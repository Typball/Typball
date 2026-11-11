//! 文書 = ファイル 1 つ分のメタ情報 (パス・種別・文字コード・最終更新時刻)
//!
//! テキスト本体は今のところエディタ (Scintilla) が持ち、ここでは持たない。
//! プレビュー専用ウィンドウ (エディタ無し) を作るときにテキストをこちらへ移す (文書と表示の分離)。
//!
//! 読み込み ([`load`]) と書き込み ([`fileio::write`]) は GUI に依存しないので別スレッドで呼べる
//! (`window.rs` が `background::run` で待つ)。保存は「行き先 ([`SaveTarget`]) を決める → 書く →
//! 成功したら文書に反映する ([`Document::saved_to`])」の順で、書けるまで文書は変えない。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::fileio::{self, DecodeOptions, Encoding, LineEnding};
use crate::filetype::FileType;
use crate::i18n::tr;
use crate::settings;

#[derive(Debug, Clone)]
pub struct Document {
    pub path: Option<PathBuf>,
    pub filetype: FileType,
    /// 無題の文書の番号 (1 始まり)。表示名は [`untitled_title`] で言語と設定から組み立てる
    pub untitled: usize,
    /// 最後に読み書きしたときのファイルの状態。外部変更の検知に使う
    stamp: Option<Stamp>,
    /// 読んだ (保存する) 文字コード
    pub encoding: Encoding,
    /// 「指定して開き直す」で選んだ文字コード。読み直しでもこれで読む
    pub forced_encoding: Option<Encoding>,
    /// 読み込みで置き換え文字 (U+FFFD) が入った: このまま保存すると元のバイト列は失われる。
    /// ユーザーが保存を了承したら false にする
    pub lossy: bool,
}

/// ディスク上のファイルの状態 (更新時刻と大きさ)。更新時刻だけだと、時刻の粒度が粗いファイルシステム
/// (FAT は 2 秒) や時刻を戻すツールでの書き換えを見逃すので、大きさも比べる
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    modified: Option<SystemTime>,
    len: u64,
}

impl Stamp {
    /// 今の状態。ファイルが無い (読めない) なら None
    fn of(path: &Path) -> Option<Stamp> {
        let meta = fs::metadata(path).ok()?;
        Some(Stamp {
            modified: meta.modified().ok(),
            len: meta.len(),
        })
    }
}

/// 保存 1 回分の行き先。書き込みが成功してから [`Document::saved_to`] で文書に反映する
/// (失敗したり取り消したりした保存で、文書のパスや文字コードが変わらないように)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveTarget {
    pub path: PathBuf,
    pub encoding: Encoding,
    /// 保存ダイアログで今と違う改行コードを選んだとき: その改行コードにそろえて書いた。文書の改行も
    /// 書けた後にそろえる (`Tab::convert_line_endings`)。None は文書の改行のまま書いた
    pub line_ending: Option<LineEnding>,
}

/// 無題の文書の表示名: "無題-3.txt"、設定 `untitled_numbering` が false なら "無題.txt"
pub fn untitled_title(number: usize, filetype: FileType) -> String {
    let base = tr("document.untitled");
    let ext = filetype.extension();
    if settings::get().files.untitled_numbering {
        format!("{base}-{number}.{ext}")
    } else {
        format!("{base}.{ext}")
    }
}

/// ファイルから読んだ結果
pub struct Loaded {
    pub doc: Document,
    pub text: String,
    /// 置き換え文字が入った
    pub lossy: bool,
    /// NUL バイトがあった (バイナリらしい)
    pub binary: bool,
}

/// ファイルを読んで文書にする。GUI に触らないので別スレッドから呼べる
pub fn load(path: &Path, options: DecodeOptions) -> io::Result<Loaded> {
    // 更新時刻は読む前に取る: 読んでいる間に書き換えられても次の確認で気付ける
    let stamp = Stamp::of(path);
    let bytes = fs::read(path)?;
    let decoded = fileio::decode(bytes, options);
    let doc = Document {
        path: Some(path.to_path_buf()),
        filetype: FileType::from_path(path),
        untitled: 0,
        stamp,
        encoding: decoded.encoding,
        forced_encoding: options.forced,
        lossy: decoded.lossy,
    };
    Ok(Loaded {
        doc,
        text: decoded.text,
        lossy: decoded.lossy,
        binary: decoded.binary,
    })
}

/// 読み込みの既定の指示を設定から作る (UTF-8 でないときの既定の文字コードと推定のヒント)
pub fn decode_options(settings: &settings::Settings) -> DecodeOptions {
    DecodeOptions {
        forced: None,
        fallback: Encoding::parse(&settings.files.fallback_encoding),
        hint: if crate::i18n::current_language().starts_with("ja") {
            Some(b"jp")
        } else {
            None
        },
    }
}

impl Document {
    pub fn untitled(filetype: FileType, number: usize) -> Self {
        let encoding = Encoding::parse(&settings::get().files.default_encoding).unwrap_or_default();
        Self {
            path: None,
            filetype,
            untitled: number,
            stamp: None,
            encoding,
            forced_encoding: None,
            lossy: false,
        }
    }

    /// ファイルを読み、文書とテキストを返す (同期。プレビューウィンドウ用)
    pub fn open(path: &Path, options: DecodeOptions) -> io::Result<(Self, String)> {
        let loaded = load(path, options)?;
        Ok((loaded.doc, loaded.text))
    }

    /// 読み直しの指示: 指定された文字コードがあればそれ、無ければ設定どおり
    pub fn reload_options(&self, settings: &settings::Settings) -> DecodeOptions {
        DecodeOptions {
            forced: self.forced_encoding,
            ..decode_options(settings)
        }
    }

    /// 現在のパスから同期で読み直す (プレビューウィンドウ用。エディタは window.rs が別スレッドで読む)
    pub fn reload(&mut self, settings: &settings::Settings) -> io::Result<String> {
        let Some(path) = self.path.clone() else {
            return Err(io::Error::new(io::ErrorKind::NotFound, "untitled"));
        };
        let loaded = load(&path, self.reload_options(settings))?;
        self.apply_reloaded(&loaded);
        Ok(loaded.text)
    }

    /// 読み直した結果を取り込む (パスと種別はそのまま)
    pub fn apply_reloaded(&mut self, loaded: &Loaded) {
        self.stamp = loaded.doc.stamp;
        self.encoding = loaded.doc.encoding;
        self.lossy = loaded.lossy;
    }

    /// `target` に書き終えた: 保存先と文字コードを文書に反映し、ファイルの状態を取り直し、
    /// 置き換え文字の警告を解く。保存先が変わったら true (別の名前で保存した: 種別やプレビューの
    /// 基準フォルダが変わりうる)
    pub fn saved_to(&mut self, target: &SaveTarget) -> bool {
        let moved = self.path.as_deref() != Some(target.path.as_path());
        // 選んで保存した文字コードは内容からの推定より確かなので、読み直しでもそれで読む
        // (短い Shift_JIS のファイルは推定が外れやすい)
        if self.forced_encoding.is_some() || self.encoding != target.encoding {
            self.forced_encoding = Some(target.encoding);
        }
        if moved {
            self.filetype = FileType::from_path(&target.path);
            self.path = Some(target.path.clone());
        }
        self.encoding = target.encoding;
        self.stamp = Stamp::of(&target.path);
        self.lossy = false;
        moved
    }

    /// パスを付け替える (名前を付けて保存)。種別は新しい拡張子で決め直す
    pub fn set_path(&mut self, path: PathBuf) {
        self.filetype = FileType::from_path(&path);
        self.path = Some(path);
        self.stamp = None;
    }

    /// タブやタイトルバーに出す名前
    pub fn title(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| untitled_title(self.untitled, self.filetype))
    }

    /// ファイルが最後に読み書きした後に外部で変更 (または削除) されたか
    pub fn changed_externally(&self) -> bool {
        let Some(path) = &self.path else {
            return false;
        };
        Stamp::of(path) != self.stamp
    }

    /// 外部変更を確認済みにする (今のファイルの状態を基準にし直す)
    pub fn acknowledge_external(&mut self) {
        if let Some(path) = &self.path {
            self.stamp = Stamp::of(path);
        }
    }

    /// ディスク上のファイルが、最後に読み書きした後に別のプログラムで書き換えられているか
    /// (消されたのは含まない: 保存すれば作り直すだけなので、上書きの確認は要らない)
    pub fn overwritten_externally(&self) -> bool {
        let Some(path) = &self.path else {
            return false;
        };
        match Stamp::of(path) {
            Some(now) => self.stamp.is_some_and(|s| s != now),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("typball-document-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_save_changes_the_document_only_when_it_is_applied() {
        let dir = temp_dir("saved-to");
        let old = dir.join("a.txt");
        fs::write(&old, "old").unwrap();
        let mut doc = load(&old, DecodeOptions::default()).unwrap().doc;
        doc.lossy = true;
        assert_eq!(doc.filetype, FileType::Txt);

        // 別の名前・別の文字コードで書けた後に反映する
        let new = dir.join("b.md");
        let sjis = Encoding::parse("shift_jis").unwrap();
        fs::write(&new, "new").unwrap();
        let target = SaveTarget {
            path: new.clone(),
            encoding: sjis,
            line_ending: None,
        };
        assert!(doc.saved_to(&target), "the document moved to another file");
        assert_eq!(doc.path.as_deref(), Some(new.as_path()));
        assert_eq!(doc.filetype, FileType::Md);
        assert_eq!(doc.encoding, sjis);
        // 選んだ文字コードは読み直しにも使う
        assert_eq!(doc.forced_encoding, Some(sjis));
        assert!(!doc.lossy);
        assert!(!doc.changed_externally());
        // 同じ行き先への保存は「移動」ではない
        assert!(!doc.saved_to(&target));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn external_rewrites_are_noticed_but_deletion_is_not_an_overwrite() {
        let dir = temp_dir("external");
        let path = dir.join("a.txt");
        fs::write(&path, "one").unwrap();
        let mut doc = load(&path, DecodeOptions::default()).unwrap().doc;
        assert!(!doc.changed_externally() && !doc.overwritten_externally());

        // 大きさが変われば、更新時刻が同じでも気付く
        fs::write(&path, "one, two").unwrap();
        assert!(doc.changed_externally() && doc.overwritten_externally());
        doc.acknowledge_external();
        assert!(!doc.changed_externally() && !doc.overwritten_externally());

        // 消されたのは「外部変更」だが、保存で上書きするものは無い
        fs::remove_file(&path).unwrap();
        assert!(doc.changed_externally());
        assert!(!doc.overwritten_externally());
        let _ = fs::remove_dir_all(&dir);
    }
}
