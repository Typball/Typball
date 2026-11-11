//! wxDragon が公開していないファイルダイアログの機能
//!
//! 保存ダイアログの中に選択リストを 1 つ足す (文字コードの選択)。wxWidgets の
//! `wxFileDialogCustomizeHook` を使うので、Windows では OS 標準の保存ダイアログにコンボボックスが入り、
//! GTK / macOS でもそれぞれのダイアログの追加部品になる。C++ 側は vendored の wxdragon-sys に足した
//! `wxd_FileDialog_ShowModalWithChoice` ("Typball patch")。

use std::ffi::{c_char, CString};

use wxdragon::ffi;
use wxdragon::prelude::*;

/// ダイアログに `label` 付きの選択リスト (`items`、最初は `selection` 番目) を足して表示する。
/// (ShowModal の戻り値, 閉じたときに選ばれていた項目) を返す
pub fn show_modal_with_choice(
    dialog: &FileDialog,
    label: &str,
    items: &[String],
    selection: usize,
) -> (i32, usize) {
    let label = CString::new(label).unwrap_or_default();
    let items: Vec<CString> = items
        .iter()
        .map(|s| CString::new(s.as_str()).unwrap_or_default())
        .collect();
    let pointers: Vec<*const c_char> = items.iter().map(|s| s.as_ptr()).collect();
    let mut chosen = selection.min(items.len().saturating_sub(1)) as i32;
    // SAFETY: dialog は生きている wxFileDialog で、文字列とその配列は呼び出しの間ずっと生きている。
    // C++ 側は呼び出しの中で文字列をコピーし、戻った後は参照しない
    let result = unsafe {
        ffi::wxd_FileDialog_ShowModalWithChoice(
            dialog.handle_ptr() as *mut ffi::wxd_FileDialog_t,
            label.as_ptr(),
            pointers.as_ptr(),
            pointers.len() as i32,
            &mut chosen,
        )
    };
    let chosen = usize::try_from(chosen).unwrap_or(selection);
    (result, chosen)
}
