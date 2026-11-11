//! wxDragon が公開していないファイルダイアログの機能
//!
//! 保存ダイアログの中に選択リストを足す (文字コードと改行コードの選択)。C++ 側は vendored の wxdragon-sys に
//! 足した `wxd_FileDialog_ShowModalWithChoices` ("Typball patch")。
//!
//! - Windows: OS 標準のダイアログ (IFileDialog) を直接組み立て、リストを全部「保存」ボタンの左に並べる
//!   (メモ帳の「エンコード」と同じ行)。Windows がそこに置けるのは 1 組だけなので、最後のリスト以外は同じ行に
//!   自前の部品を作り、幅が足りないときは Windows の正規の段 (ファイルの種類の下) に切り替える。
//!   wxWidgets の `wxFileDialogCustomizeHook` 経由だと、ラベルとリストをひとまとまりにできず、
//!   ファイルの種類の下に 1 行空いた別の段に離れて並ぶ。
//! - GTK / macOS: `wxFileDialogCustomizeHook` で、それぞれのダイアログの追加部品になる。
//!
//! どちらでも、閉じた後は `FileDialog::get_path` などがそのまま使える。

use std::ffi::{c_char, CString};

use wxdragon::ffi;
use wxdragon::prelude::*;

/// ダイアログに足す選択リスト 1 つ
pub struct Choice<'a> {
    pub label: &'a str,
    pub items: &'a [String],
    /// 最初に選んでおく項目
    pub selection: usize,
}

/// ダイアログに `choices` の選択リストを (この順に) 足して表示する。
/// (ShowModal の戻り値, 閉じたときに各リストで選ばれていた項目) を返す
pub fn show_modal_with_choices(dialog: &FileDialog, choices: &[Choice]) -> (i32, Vec<usize>) {
    let to_c = |s: &str| CString::new(s).unwrap_or_default();
    let labels: Vec<CString> = choices.iter().map(|c| to_c(c.label)).collect();
    let items: Vec<CString> = choices
        .iter()
        .flat_map(|c| c.items.iter().map(|s| to_c(s)))
        .collect();
    let label_pointers: Vec<*const c_char> = labels.iter().map(|s| s.as_ptr()).collect();
    let item_pointers: Vec<*const c_char> = items.iter().map(|s| s.as_ptr()).collect();
    let counts: Vec<i32> = choices.iter().map(|c| c.items.len() as i32).collect();
    let mut selections: Vec<i32> = choices
        .iter()
        .map(|c| c.selection.min(c.items.len().saturating_sub(1)) as i32)
        .collect();
    // SAFETY: dialog は生きている wxFileDialog で、文字列と配列は呼び出しの間ずっと生きている。
    // 配列の長さは choices (items は counts の合計) と一致している。C++ 側は呼び出しの中で文字列を
    // コピーし、戻った後は参照しない
    let result = unsafe {
        ffi::wxd_FileDialog_ShowModalWithChoices(
            dialog.handle_ptr() as *mut ffi::wxd_FileDialog_t,
            choices.len() as i32,
            label_pointers.as_ptr(),
            item_pointers.as_ptr(),
            counts.as_ptr(),
            selections.as_mut_ptr(),
        )
    };
    let chosen = choices
        .iter()
        .zip(&selections)
        .map(|(c, s)| usize::try_from(*s).unwrap_or(c.selection))
        .collect();
    (result, chosen)
}
