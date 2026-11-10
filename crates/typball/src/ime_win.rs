//! Windows 固有: IME 変換ウィンドウの位置ずれ対策
//!
//! wxStyledTextCtrl (Windows) は `WM_IME_STARTCOMPOSITION` を受けた時点のキャレット位置に
//! IME の変換ウィンドウを置く (ScintillaWX::ImeStartComposition)。範囲選択中に IME で
//! 入力を始めると、この時点ではまだ選択が残っているためキャレット (= 選択の終端) に
//! 変換ウィンドウが出るが、確定文字は選択を消した先頭位置に入るので表示がずれる。
//!
//! ここではエディタの HWND をサブクラス化し、`WM_IME_STARTCOMPOSITION` を Scintilla より
//! 先に受け取って選択範囲を削除しておく。これでキャレットが先頭に移り、変換ウィンドウも
//! 実際の挿入位置に出る。削除は通常の編集として Undo できる。

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;

use wxdragon::prelude::*;

type Hwnd = *mut c_void;
type SubclassProc = unsafe extern "system" fn(Hwnd, u32, usize, isize, usize, usize) -> isize;

#[link(name = "comctl32")]
extern "system" {
    fn SetWindowSubclass(hwnd: Hwnd, pfn: SubclassProc, id: usize, ref_data: usize) -> i32;
    fn RemoveWindowSubclass(hwnd: Hwnd, pfn: SubclassProc, id: usize) -> i32;
    fn DefSubclassProc(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
}

const WM_NCDESTROY: u32 = 0x0082;
const WM_IME_STARTCOMPOSITION: u32 = 0x010D;
const SUBCLASS_ID: usize = 0x7479_7062; // "typb"

thread_local! {
    static EDITORS: RefCell<HashMap<usize, StyledTextCtrl>> = RefCell::new(HashMap::new());
}

/// エディタに対策を適用する。ウィンドウ破棄時に自動で解除される。
pub fn install(stc: &StyledTextCtrl) {
    let hwnd = stc.get_handle();
    if hwnd.is_null() {
        return;
    }
    EDITORS.with(|m| m.borrow_mut().insert(hwnd as usize, *stc));
    // SAFETY: hwnd は生存中のエディタの HWND。コールバックは同じ UI スレッドで呼ばれる。
    unsafe {
        SetWindowSubclass(hwnd, subclass_proc, SUBCLASS_ID, 0);
    }
}

unsafe extern "system" fn subclass_proc(
    hwnd: Hwnd,
    msg: u32,
    wparam: usize,
    lparam: isize,
    _id: usize,
    _data: usize,
) -> isize {
    match msg {
        WM_IME_STARTCOMPOSITION => {
            let stc = EDITORS.with(|m| m.borrow().get(&(hwnd as usize)).copied());
            if let Some(stc) = stc {
                let (start, end) = stc.get_selection();
                if start != end {
                    stc.replace_selection("");
                }
            }
        }
        WM_NCDESTROY => {
            EDITORS.with(|m| m.borrow_mut().remove(&(hwnd as usize)));
            RemoveWindowSubclass(hwnd, subclass_proc, SUBCLASS_ID);
        }
        _ => {}
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}
