//! ファイル種別 (拡張子で決まる)
//!
//! 構文ハイライト・プレビューの有無・アウトラインの取り方・「アクション」メニューの中身は
//! すべてここで決まる種別から引く。新しい形式を足すときはここに variant を追加し、
//! `outline.rs` / `editor.rs` / `window.rs` の `match` を埋める。

use std::path::Path;

use crate::i18n::tr;
use crate::settings::{OpenMode, PaneSizes, Panes, PanesOverride, Settings};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Txt,
    Md,
    Typ,
}

impl FileType {
    /// メニュー・設定ダイアログに並べる順
    pub const ALL: [FileType; 3] = [Self::Txt, Self::Md, Self::Typ];

    /// 拡張子 (ドット無し) から判定する。知らない拡張子はプレーンテキスト扱い
    pub fn from_extension(ext: &str) -> FileType {
        match ext.to_ascii_lowercase().as_str() {
            "typ" => Self::Typ,
            "md" | "markdown" => Self::Md,
            _ => Self::Txt,
        }
    }

    pub fn from_path(path: &Path) -> FileType {
        path.extension()
            .and_then(|e| e.to_str())
            .map(Self::from_extension)
            .unwrap_or(Self::Txt)
    }

    /// 既定の拡張子 (ドット無し)
    pub fn extension(self) -> &'static str {
        match self {
            Self::Txt => "txt",
            Self::Md => "md",
            Self::Typ => "typ",
        }
    }

    /// 表示名の i18n キー
    pub fn name_key(self) -> &'static str {
        match self {
            Self::Txt => "filetype.txt",
            Self::Md => "filetype.md",
            Self::Typ => "filetype.typ",
        }
    }

    /// ステータスバーに出す短い名前の i18n キー
    pub fn short_name_key(self) -> &'static str {
        match self {
            Self::Txt => "filetype.short.txt",
            Self::Md => "filetype.short.md",
            Self::Typ => "filetype.short.typ",
        }
    }

    /// プレビューができるか (Typst はコンパイルしてページ画像、Markdown は HTML にして WebView)
    pub fn has_preview(self) -> bool {
        matches!(self, Self::Md | Self::Typ)
    }

    /// アウトライン (見出し一覧) を取れるか
    pub fn has_outline(self) -> bool {
        matches!(self, Self::Md | Self::Typ)
    }

    /// コンソール (コンパイラ / 変換の出力) があるか。プレビューを作る形式と同じ
    pub fn has_console(self) -> bool {
        self.has_preview()
    }

    /// エディタとプレビューのスクロール同期ができるか (Markdown の WebView だけ。Typst のページ画像は未対応)
    pub fn has_scroll_sync(self) -> bool {
        matches!(self, Self::Md)
    }

    /// アクションバーに出す操作があるか (`actions::Action::for_filetype` が空でない)
    pub fn has_actionbar(self) -> bool {
        !crate::actions::Action::for_filetype(self).is_empty()
    }

    /// 設定に何も書かれていないときのペイン構成
    pub fn default_panes(self) -> Panes {
        match self {
            Self::Txt => Panes {
                show_outline: false,
                show_preview: false,
                show_console: false,
                show_actionbar: false,
                sync_scroll_to_preview: false,
                sync_scroll_to_editor: false,
            },
            Self::Md => Panes {
                show_outline: true,
                show_preview: true,
                show_console: true,
                show_actionbar: true,
                sync_scroll_to_preview: true,
                sync_scroll_to_editor: true,
            },
            Self::Typ => Panes {
                show_outline: true,
                show_preview: true,
                show_console: true,
                show_actionbar: true,
                sync_scroll_to_preview: true,
                sync_scroll_to_editor: true,
            },
        }
    }

    /// 設定ファイルの `[filetypes.*]` テーブル
    pub fn overrides(self, settings: &Settings) -> PanesOverride {
        match self {
            Self::Txt => &settings.filetypes.txt,
            Self::Md => &settings.filetypes.md,
            Self::Typ => &settings.filetypes.typ,
        }
        .clone()
    }

    /// 同じテーブルを書き換えるために借りる
    pub fn overrides_mut(self, settings: &mut Settings) -> &mut PanesOverride {
        match self {
            Self::Txt => &mut settings.filetypes.txt,
            Self::Md => &mut settings.filetypes.md,
            Self::Typ => &mut settings.filetypes.typ,
        }
    }

    /// この形式のペインの置き場の大きさ (分割線を動かすと `Tab::save_pane_sizes` が更新する)
    pub fn pane_sizes(self, settings: &Settings) -> PaneSizes {
        self.overrides(settings).pane_sizes
    }

    /// この種別のタブを開いたときのペイン構成 (設定の上書きを既定に重ねたもの)
    pub fn panes(self, settings: &Settings) -> Panes {
        self.overrides(settings).resolve(self.default_panes())
    }

    /// ファイルを渡されて起動したときの開き方 (エディタのタブ / プレビューウィンドウ)
    pub fn open_mode(self, settings: &Settings) -> OpenMode {
        self.overrides(settings).open_mode
    }

    /// 新しいファイル (Ctrl+N、起動時の新規タブ) の雛形。設定に無ければ言語ごとの既定
    pub fn template(self, settings: &Settings) -> String {
        self.overrides(settings)
            .template
            .unwrap_or_else(|| self.default_template())
    }

    /// 雛形の既定 (ロケールの `[template]`)。Typst は A4 縦・同梱の Noto Sans JP・日本語の指定と見出し、
    /// Markdown は空、テキストは無し
    pub fn default_template(self) -> String {
        match self {
            Self::Txt => String::new(),
            Self::Md => tr("template.md"),
            Self::Typ => tr("template.typ"),
        }
    }

    /// 保存ダイアログのフィルタ (この種別を先頭にする)
    pub fn save_wildcard(self) -> String {
        let mut order = vec![self];
        order.extend(Self::ALL.iter().copied().filter(|t| *t != self));
        let mut parts: Vec<String> = order.iter().map(|t| t.filter_entry()).collect();
        parts.push("*.*|*.*".to_string());
        parts.join("|")
    }

    /// 「開く」ダイアログのフィルタ (対応形式すべて → 各形式 → すべて)
    pub fn open_wildcard() -> String {
        let all: Vec<String> = Self::ALL
            .iter()
            .map(|t| format!("*.{}", t.extension()))
            .collect();
        let mut parts = vec![format!("{}|{}", all.join(";"), all.join(";"))];
        parts.extend(Self::ALL.iter().map(|t| t.filter_entry()));
        parts.push("*.*|*.*".to_string());
        parts.join("|")
    }

    fn filter_entry(self) -> String {
        let ext = self.extension();
        format!("*.{ext}|*.{ext}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_mapping() {
        assert_eq!(FileType::from_path(Path::new("a/b.TYP")), FileType::Typ);
        assert_eq!(FileType::from_path(Path::new("x.markdown")), FileType::Md);
        assert_eq!(FileType::from_path(Path::new("noext")), FileType::Txt);
        assert_eq!(FileType::from_extension("rs"), FileType::Txt);
    }

    #[test]
    fn wildcard_puts_own_type_first() {
        assert!(FileType::Md.save_wildcard().starts_with("*.md|*.md|"));
        assert!(FileType::open_wildcard().starts_with("*.txt;*.md;*.typ|"));
    }
}
