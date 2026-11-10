//! 形式ごとの操作 (アクション)
//!
//! メニューバーの「アクション」と、エディタ上部のアクションバー (`actionbar.rs`) は同じ一覧から
//! 作る。メニューには形式で使えるアクションがすべて並び、バーにはそのうちアイコン向きのもの
//! ([`Action::in_bar`]) だけが並ぶ。新しい形式の操作 (たとえば .cpp のコンパイル) を足すときは
//! ここに variant を追加し、`for_filetype` と `window.rs` の実行部分を埋める。

use wxdragon::id::ID_HIGHEST;

use crate::filetype::FileType;

/// アクション ID の先頭 (ID_HIGHEST + 100 ..)。`window.rs` の他の ID と重ならないようにする
pub const ID_ACTION_BASE: i32 = ID_HIGHEST + 100;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Typst を PDF に書き出す
    ExportPdf,
    Bold,
    Italic,
    Heading,
    Code,
    Link,
    List,
    Quote,
    Math,
}

impl Action {
    /// ID の割り当て順 (全アクション)
    pub const ALL: [Action; 9] = [
        Self::ExportPdf,
        Self::Bold,
        Self::Italic,
        Self::Heading,
        Self::Code,
        Self::Link,
        Self::List,
        Self::Quote,
        Self::Math,
    ];

    /// 形式で使えるアクション (メニューとバーに並ぶ順)
    pub fn for_filetype(filetype: FileType) -> &'static [Action] {
        use Action::*;
        match filetype {
            FileType::Typ => &[Bold, Italic, Heading, Code, Math, Link, List, ExportPdf],
            FileType::Md => &[Bold, Italic, Heading, Code, Quote, Link, List],
            FileType::Txt => &[],
        }
    }

    /// アクションバーにアイコンで出すか (今はすべて)
    pub fn in_bar(self) -> bool {
        true
    }

    /// 選択範囲への書式付けか (エディタのテキストを変える)
    pub fn is_format(self) -> bool {
        !matches!(self, Self::ExportPdf)
    }

    /// メニュー項目の前に区切り線を入れるか
    pub fn separator_before(self) -> bool {
        matches!(self, Self::ExportPdf)
    }

    pub fn id(self) -> i32 {
        ID_ACTION_BASE + Self::ALL.iter().position(|a| *a == self).unwrap_or(0) as i32
    }

    pub fn from_id(id: i32) -> Option<Action> {
        let index = usize::try_from(id - ID_ACTION_BASE).ok()?;
        Self::ALL.get(index).copied()
    }

    fn key(self) -> &'static str {
        match self {
            Self::ExportPdf => "export_pdf",
            Self::Bold => "bold",
            Self::Italic => "italic",
            Self::Heading => "heading",
            Self::Code => "code",
            Self::Link => "link",
            Self::List => "list",
            Self::Quote => "quote",
            Self::Math => "math",
        }
    }

    /// メニュー項目の文字列 (アクセスキー・ショートカット付き) の i18n キー
    pub fn menu_key(self) -> String {
        format!("action.{}.menu", self.key())
    }

    /// バーのツールチップとメニューのヘルプ文字列の i18n キー
    pub fn tip_key(self) -> String {
        format!("action.{}.tip", self.key())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_roundtrip() {
        for a in Action::ALL {
            assert_eq!(Action::from_id(a.id()), Some(a));
        }
        assert_eq!(Action::from_id(ID_ACTION_BASE - 1), None);
        assert_eq!(Action::from_id(ID_ACTION_BASE + 99), None);
    }

    #[test]
    fn txt_has_no_actions() {
        assert!(Action::for_filetype(FileType::Txt).is_empty());
        assert!(Action::for_filetype(FileType::Typ).contains(&Action::ExportPdf));
    }
}
