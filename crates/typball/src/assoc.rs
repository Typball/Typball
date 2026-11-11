//! ファイルの関連付け (設定 → 関連付け)
//!
//! 「.txt / .md / .typ をダブルクリックしたら Typball で開く」を OS に登録・解除する。
//! 仕組みは OS ごとに違うので、中身は OS ごとのモジュールに分ける:
//!
//! - Windows (`windows.rs`): レジストリの `HKCU\Software\Classes`。インストーラー (installer/typball.iss) が
//!   書くのと同じキーを、今動いている exe のパスで書く。関連付けのアイコンも選べる
//! - Linux など (`xdg.rs`): freedesktop.org の決まり。`~/.local/share/applications/typball.desktop` と
//!   `~/.config/mimeapps.list` (MIME タイプ → 既定のアプリ)
//! - macOS (`macos.rs`): Launch Services。書類の種類は .app の Info.plist が宣言し (CI が書く)、
//!   ここでは種類ごとの既定のアプリを切り替えるだけ
//!
//! 設定ファイル (Settings.toml) には何も保存しない。状態は毎回 OS に問い合わせる。
//!
//! このモジュールは Typball の他のモジュールに依存させない (画面の文字列は呼び出し側が [`Labels`] で渡す)。
//! 他の OS 向けのコードを、このモジュールだけ取り出して型検査できるようにするため。

use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;
#[cfg(any(all(unix, not(target_os = "macos")), test))]
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
mod xdg;

#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(windows)]
use windows as imp;
#[cfg(all(unix, not(target_os = "macos")))]
use xdg as imp;

/// 関連付けの対象 (Typball が扱う形式)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Txt,
    Md,
    Typ,
}

impl Kind {
    /// 拡張子 (ドット無し)
    pub fn ext(self) -> &'static str {
        match self {
            Self::Txt => "txt",
            Self::Md => "md",
            Self::Typ => "typ",
        }
    }

    /// プレビューのある形式か (右クリックに「編集」「プレビュー」を分けて出す)
    #[cfg(windows)]
    pub fn has_preview(self) -> bool {
        !matches!(self, Self::Txt)
    }

    /// エクスプローラーなどの「種類」に出る名前。インストーラーと同じ文字列にする
    /// (更新のたびにインストーラーが書き直すので、違うと行き来する)
    #[cfg(windows)]
    pub fn type_name(self) -> &'static str {
        match self {
            Self::Txt => "Text Document",
            Self::Md => "Markdown Document",
            Self::Typ => "Typst Document",
        }
    }

    /// MIME タイプ (Linux)
    #[cfg(any(all(unix, not(target_os = "macos")), test))]
    pub fn mime(self) -> &'static str {
        match self {
            Self::Txt => "text/plain",
            Self::Md => "text/markdown",
            Self::Typ => "text/x-typst",
        }
    }
}

/// 関連付けたファイルに付けるアイコン
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Icon {
    /// 形式ごとの書類のアイコン (Typball に入っている)
    Document,
    /// Typball のアプリのアイコン
    App,
    /// 利用者が選んだもの。OS に渡す文字列そのまま (Windows なら `C:\a.ico` や `C:\b.dll,3`)
    Custom(String),
}

/// OS に登録する文字列 (右クリックメニューの項目名)。呼び出し側が表示言語で作る (使うのは Windows だけ)
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone)]
pub struct Labels {
    pub open: String,
    pub edit: String,
    pub view: String,
}

/// Typball がこの形式に登録されているか
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Registered {
    No,
    /// 今動いているこの Typball
    This,
    /// 別の場所の Typball (インストール版と開発版など)
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Other(PathBuf),
}

/// この形式を今ダブルクリックしたときに開くアプリ
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultApp {
    /// 表示名 ("メモ帳" など)
    pub name: String,
    /// 実行ファイルの場所 (分かるときだけ)
    pub path: Option<PathBuf>,
    /// 今動いているこの Typball か
    pub is_self: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub default_app: Option<DefaultApp>,
    pub registered: Registered,
    /// ここから解除できるか (自分で書き換えられない場所の登録、たとえば全ユーザー向けの
    /// インストールによる登録だけが残っているときは false)
    pub removable: bool,
    /// 登録されているアイコン (未登録なら [`Icon::Document`])
    pub icon: Icon,
}

/// この OS・この起動のしかたでできること
#[derive(Debug, Clone, Copy)]
pub struct Capabilities {
    /// 関連付けを変えられない理由 (i18n のキー)。変えられるなら None
    pub unavailable: Option<&'static str>,
    /// アイコンを選べるか
    pub icons: bool,
    /// OS の「既定のアプリ」の設定画面を開けるか
    pub system_settings: bool,
    /// 画面に添える OS ごとの注意書き (i18n のキー)
    pub note: &'static str,
}

/// 関連付け先として登録するもの (今動いている実行ファイル。macOS は .app)
pub fn target() -> Option<PathBuf> {
    imp::target()
}

pub fn capabilities() -> Capabilities {
    imp::capabilities()
}

pub fn status(kind: Kind) -> Status {
    imp::status(kind)
}

/// この Typball を関連付ける。既に別の場所の Typball が登録されていれば付け替える
pub fn register(kind: Kind, icon: &Icon, labels: &Labels) -> Result<(), String> {
    imp::register(kind, icon, labels)
}

/// 登録済みの関連付けのアイコンだけを変える (未登録なら何もしない)
pub fn set_icon(kind: Kind, icon: &Icon) -> Result<(), String> {
    imp::set_icon(kind, icon)
}

pub fn unregister(kind: Kind) -> Result<(), String> {
    imp::unregister(kind)
}

/// この Typball の関連付けを解除して終わるコマンドライン引数 (Windows のアンインストーラーが呼ぶ)
pub const UNREGISTER_ARG: &str = "--unregister-associations";

/// 今動いているこの Typball に向いている関連付けをすべて解除する。別の場所の Typball の登録には触らない
/// (開発版をアンインストールしても、インストール版の関連付けは残る)
pub fn unregister_own() {
    for kind in [Kind::Txt, Kind::Md, Kind::Typ] {
        let status = status(kind);
        if status.registered == Registered::This && status.removable {
            if let Err(e) = unregister(kind) {
                eprintln!(
                    "Typball: could not remove the .{} association: {e}",
                    kind.ext()
                );
            }
        }
    }
}

/// OS の「既定のアプリ」の設定画面を開く
pub fn open_system_settings() {
    imp::open_system_settings();
}

/// アイコンの見本 (`size` × `size` の RGBA)。取り出せない OS・アイコンでは None
pub fn icon_rgba(kind: Kind, icon: &Icon, size: u32) -> Option<Vec<u8>> {
    imp::icon_rgba(kind, icon, size)
}

/// アイコンに使えるファイルを選ぶダイアログのフィルタ (wx の書式)。アイコンを選べない OS では空
pub fn icon_wildcard() -> &'static str {
    imp::ICON_WILDCARD
}

/// 利用者が選んだファイルを [`Icon::Custom`] にする
pub fn custom_icon(path: &Path) -> Icon {
    Icon::Custom(imp::custom_icon_value(path))
}

/// 2 つのパスが同じファイルを指すか (存在すれば正規化して比べる。Windows は大文字小文字を区別しない)
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn same_file(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        let p = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        let s = p.to_string_lossy().replace('/', "\\");
        let s = s.strip_prefix(r"\\?\").unwrap_or(&s).to_string();
        if cfg!(windows) {
            s.to_lowercase()
        } else {
            s
        }
    };
    norm(a) == norm(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_file_ignores_case_on_windows_only() {
        let a = Path::new("Z:/no/such/Typball.exe");
        let b = Path::new("z:\\no\\such\\typball.EXE");
        assert_eq!(same_file(a, b), cfg!(windows));
        assert!(same_file(a, a));
    }
}
