//! 「編集」メニューとエディタの右クリックメニュー
//!
//! 項目の一覧は [`EditItem`] の 1 か所に置き、メニューバーの「編集」とエディタのコンテキスト
//! メニューの両方をここから作る。コンテキストメニューには編集項目のあとに形式ごとのアクション
//! (`actions.rs`、メニューバーの「アクション」と同じ一覧) が並ぶ。
//!
//! 項目を足すには:
//! - 全体に共通の操作なら [`EditItem`] に variant を足し、`ALL` / `key` / `enabled` / `apply` を埋める
//!   (ロケールの `menu.<key>` も追加)。メニューバーとコンテキストメニューの両方に出る
//! - 形式ごとの操作 (Typst のコンパイルなど) なら `actions.rs` の [`Action`] に足す。
//!   コンテキストメニューの「アクション」の並びとメニューバーの両方に出る
//!
//! メニューの Ctrl+C などは wx のアクセラレータとしてフレームが先に受け取るので、フォーカスの
//! あるコントロール (エディタ / 検索バーの入力欄 / プレビューの WebView) に合わせて振り分ける
//! ([`EditTarget`])。振り分けないと検索欄で Ctrl+V を押してもエディタに貼り付いてしまう。

use wxdragon::id::ID_HIGHEST;
use wxdragon::prelude::*;
use wxdragon::widgets::webview::WebView;

use crate::actions::Action;
use crate::filetype::FileType;
use crate::i18n::tr;
use crate::scintilla;

/// 編集項目の ID の先頭 (ID_HIGHEST + 60 ..)。`window.rs` の ID (+1 .. +59)、
/// アクション (+100 ..)、ペインのメニュー (+200 ..) と重ならないようにする
pub const ID_EDIT_BASE: i32 = ID_HIGHEST + 60;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EditItem {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Delete,
    SelectAll,
}

/// 編集操作の相手 (フォーカスのあるコントロール)
#[derive(Clone)]
pub enum EditTarget {
    Editor(StyledTextCtrl),
    /// 検索バーなどの 1 行入力欄
    Text(TextCtrl),
    /// Markdown プレビューの WebView
    Web(WebView),
}

impl EditItem {
    /// ID の割り当て順 = メニューに並ぶ順
    pub const ALL: [EditItem; 7] = [
        Self::Undo,
        Self::Redo,
        Self::Cut,
        Self::Copy,
        Self::Paste,
        Self::Delete,
        Self::SelectAll,
    ];

    pub fn id(self) -> i32 {
        ID_EDIT_BASE + Self::ALL.iter().position(|a| *a == self).unwrap_or(0) as i32
    }

    pub fn from_id(id: i32) -> Option<EditItem> {
        let index = usize::try_from(id - ID_EDIT_BASE).ok()?;
        Self::ALL.get(index).copied()
    }

    /// メニュー項目の前に区切り線を入れるか
    fn separator_before(self) -> bool {
        matches!(self, Self::Cut | Self::SelectAll)
    }

    /// ロケールのキー (`menu.undo` など)。メニューバーとコンテキストメニューで共通
    fn key(self) -> &'static str {
        match self {
            Self::Undo => "menu.undo",
            Self::Redo => "menu.redo",
            Self::Cut => "menu.cut",
            Self::Copy => "menu.copy",
            Self::Paste => "menu.paste",
            Self::Delete => "menu.delete",
            Self::SelectAll => "menu.select_all",
        }
    }

    /// いま実行できるか (メニューの有効 / 無効)
    pub fn enabled(self, target: &EditTarget) -> bool {
        match target {
            EditTarget::Editor(stc) => {
                let has_selection = stc.get_selection_start() != stc.get_selection_end();
                match self {
                    Self::Undo => stc.can_undo(),
                    Self::Redo => stc.can_redo(),
                    Self::Cut | Self::Copy | Self::Delete => has_selection,
                    Self::Paste => scintilla::can_paste(stc),
                    Self::SelectAll => stc.get_length() > 0,
                }
            }
            EditTarget::Text(text) => {
                let (from, to) = text.get_selection();
                match self {
                    Self::Undo => text_can_undo(text),
                    Self::Redo => false,
                    Self::Cut | Self::Copy | Self::Delete => from != to,
                    Self::Paste => true,
                    Self::SelectAll => !text.get_value().is_empty(),
                }
            }
            EditTarget::Web(web) => match self {
                Self::Undo => web.can_undo(),
                Self::Redo => web.can_redo(),
                Self::Cut => web.can_cut(),
                Self::Copy => web.can_copy(),
                Self::Paste => web.can_paste(),
                Self::Delete => false,
                Self::SelectAll => true,
            },
        }
    }

    pub fn apply(self, target: &EditTarget) {
        match target {
            EditTarget::Editor(stc) => match self {
                Self::Undo => stc.undo(),
                Self::Redo => stc.redo(),
                Self::Cut => stc.cut(),
                Self::Copy => stc.copy(),
                Self::Paste => stc.paste(),
                Self::Delete => scintilla::clear_selection(stc),
                Self::SelectAll => stc.select_all(),
            },
            EditTarget::Text(text) => text_apply(text, self),
            EditTarget::Web(web) => match self {
                Self::Undo => web.undo(),
                Self::Redo => web.redo(),
                Self::Cut => web.cut(),
                Self::Copy => web.copy(),
                Self::Paste => web.paste(),
                Self::Delete => {}
                Self::SelectAll => web.select_all(),
            },
        }
    }
}

/// 編集項目を `menu` に並べる (メニューバーの「編集」とコンテキストメニューの共通部分)
fn append_edit_items(menu: &Menu) {
    for item in EditItem::ALL {
        if item.separator_before() {
            menu.append_separator();
        }
        menu.append(item.id(), &tr(item.key()), "", ItemKind::Normal);
    }
}

/// メニューバーの「編集」。有効 / 無効はメニューを開くときに [`update_enabled`] で合わせる
pub fn edit_menu() -> Menu {
    let menu = Menu::builder().build();
    append_edit_items(&menu);
    menu
}

/// 編集項目の有効 / 無効を `target` に合わせる (`target` が無ければすべて無効)
pub fn update_enabled(menu: &Menu, target: Option<&EditTarget>) {
    for item in EditItem::ALL {
        let enabled = target.map(|t| item.enabled(t)).unwrap_or(false);
        menu.enable_item(item.id(), enabled);
    }
}

/// エディタの右クリックメニュー: 編集項目 + 形式ごとのアクション。
/// 選ばれた項目はポップアップ元のウィンドウからフレームへ伝わり、`window.rs` の `handle_menu` に届く
pub fn context_menu(stc: &StyledTextCtrl, filetype: FileType) -> Menu {
    let menu = Menu::builder().build();
    append_edit_items(&menu);
    update_enabled(&menu, Some(&EditTarget::Editor(*stc)));
    let actions = Action::for_filetype(filetype);
    if !actions.is_empty() {
        menu.append_separator();
        for action in actions {
            if action.separator_before() {
                menu.append_separator();
            }
            menu.append(
                action.id(),
                &tr(&action.menu_key()),
                &tr(&action.tip_key()),
                ItemKind::Normal,
            );
        }
    }
    menu
}

// ---------------------------------------------------------------------------
// 1 行入力欄 (wxTextCtrl)
// ---------------------------------------------------------------------------
// wxDragon の TextCtrl には Cut / Copy / Paste / Undo が無い。Windows ではネイティブの
// EDIT コントロールに WM_CUT などを送れば同じことができる (元に戻すも効く)。
// 他の OS ではクリップボードを使って同等の操作を組み立てる (元に戻すは使えない)

#[cfg(windows)]
fn text_can_undo(text: &TextCtrl) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW;
    /// EDIT コントロールの「元に戻せるか」(winuser.h。windows-sys には無い)
    const EM_CANUNDO: u32 = 0x00C6;
    let hwnd = text.get_handle();
    if hwnd.is_null() {
        return false;
    }
    // SAFETY: hwnd は生きている wxTextCtrl のウィンドウハンドルで、UI スレッドから呼んでいる
    unsafe { SendMessageW(hwnd as _, EM_CANUNDO, 0, 0) != 0 }
}

#[cfg(windows)]
fn text_apply(text: &TextCtrl, item: EditItem) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SendMessageW, WM_CLEAR, WM_COPY, WM_CUT, WM_PASTE, WM_UNDO,
    };
    let hwnd = text.get_handle();
    if hwnd.is_null() {
        return;
    }
    let msg = match item {
        EditItem::Undo => WM_UNDO,
        EditItem::Cut => WM_CUT,
        EditItem::Copy => WM_COPY,
        EditItem::Paste => WM_PASTE,
        EditItem::Delete => WM_CLEAR,
        EditItem::SelectAll => {
            text.select_all();
            return;
        }
        EditItem::Redo => return,
    };
    // SAFETY: 上と同じ
    unsafe {
        SendMessageW(hwnd as _, msg, 0, 0);
    }
}

#[cfg(not(windows))]
fn text_can_undo(_text: &TextCtrl) -> bool {
    false
}

#[cfg(not(windows))]
fn text_apply(text: &TextCtrl, item: EditItem) {
    let (from, to) = text.get_selection();
    let value = text.get_value();
    let selected = || -> String {
        let chars: Vec<char> = value.chars().collect();
        let (a, b) = (from.max(0) as usize, to.max(0) as usize);
        chars
            .get(a.min(chars.len())..b.min(chars.len()))
            .map(|c| c.iter().collect())
            .unwrap_or_default()
    };
    match item {
        EditItem::Copy | EditItem::Cut => {
            let s = selected();
            if !s.is_empty() {
                let clipboard = Clipboard::get();
                if clipboard.open() {
                    let mut data = TextDataObject::new(&s);
                    clipboard.set_data(&mut data);
                    clipboard.close();
                }
                if item == EditItem::Cut {
                    text.remove(from, to);
                }
            }
        }
        EditItem::Delete => text.remove(from, to),
        EditItem::Paste => {
            let clipboard = Clipboard::get();
            if clipboard.open() {
                if let Some(s) = clipboard.get_text() {
                    text.replace(from, to, &s);
                }
                clipboard.close();
            }
        }
        EditItem::SelectAll => text.select_all(),
        EditItem::Undo | EditItem::Redo => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_do_not_collide_with_actions() {
        for item in EditItem::ALL {
            assert_eq!(EditItem::from_id(item.id()), Some(item));
        }
        assert_eq!(EditItem::from_id(ID_EDIT_BASE - 1), None);
        assert!(ID_EDIT_BASE + EditItem::ALL.len() as i32 <= crate::actions::ID_ACTION_BASE);
    }
}
