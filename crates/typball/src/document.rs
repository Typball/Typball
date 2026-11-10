//! 文書 = ファイル 1 つ分のメタ情報 (パス・種別・最終更新時刻)
//!
//! テキスト本体は今のところエディタ (Scintilla) が持ち、ここでは持たない。
//! プレビュー専用ウィンドウ (エディタ無し) を作るときにテキストをこちらへ移す (文書と表示の分離)。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::filetype::FileType;
use crate::i18n::tr;
use crate::settings;

#[derive(Debug, Clone)]
pub struct Document {
    pub path: Option<PathBuf>,
    pub filetype: FileType,
    /// 無題の文書の番号 (1 始まり)。表示名は [`untitled_title`] で言語と設定から組み立てる
    pub untitled: usize,
    /// 最後に読み書きしたときのファイルの更新時刻。外部変更の検知に使う
    mtime: Option<SystemTime>,
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

impl Document {
    pub fn untitled(filetype: FileType, number: usize) -> Self {
        Self {
            path: None,
            filetype,
            untitled: number,
            mtime: None,
        }
    }

    /// ファイルを読み、文書とテキストを返す。UTF-8 以外のバイト列は U+FFFD に置き換える
    pub fn open(path: &Path) -> io::Result<(Self, String)> {
        let bytes = fs::read(path)?;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let doc = Self {
            path: Some(path.to_path_buf()),
            filetype: FileType::from_path(path),
            untitled: 0,
            mtime: modified_time(path),
        };
        Ok((doc, text))
    }

    /// 現在のパスから読み直す
    pub fn reload(&mut self) -> io::Result<String> {
        let Some(path) = self.path.clone() else {
            return Err(io::Error::new(io::ErrorKind::NotFound, "untitled"));
        };
        let bytes = fs::read(&path)?;
        self.mtime = modified_time(&path);
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// 現在のパスに UTF-8 で保存する
    pub fn save(&mut self, text: &str) -> io::Result<()> {
        let Some(path) = self.path.clone() else {
            return Err(io::Error::new(io::ErrorKind::NotFound, "untitled"));
        };
        fs::write(&path, text.as_bytes())?;
        self.mtime = modified_time(&path);
        Ok(())
    }

    /// パスを付け替える (名前を付けて保存)。種別は新しい拡張子で決め直す
    pub fn set_path(&mut self, path: PathBuf) {
        self.filetype = FileType::from_path(&path);
        self.path = Some(path);
        self.mtime = None;
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
        modified_time(path) != self.mtime
    }

    /// 外部変更を確認済みにする (今のファイルの状態を基準にし直す)
    pub fn acknowledge_external(&mut self) {
        if let Some(path) = &self.path {
            self.mtime = modified_time(path);
        }
    }
}

fn modified_time(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}
