//! Windows 固有: IME 変換ウィンドウの位置ずれ対策
//!
//! wxStyledTextCtrl (Windows) は、IME の変換ウィンドウ (未確定の文字が出る窓) の位置を
//! `WM_IME_STARTCOMPOSITION` を受けたときにしか決めない (ScintillaWX::ImeStartComposition。
//! wx の Scintilla は、未確定の文字を IME の窓に出させる方式だけを持つ)。そのため、次の 2 つで位置がずれる。
//!
//! 1. **範囲選択中に入力を始めたとき**: 変換が始まった時点ではまだ選択が残っているので、キャレット
//!    (= 選択の終端) に窓が出るが、確定した文字は選択を消した先頭に入る。
//!    → `WM_IME_STARTCOMPOSITION` を Scintilla より先に受け取り、選択範囲を消しておく
//!    (通常の編集として Undo できる)。
//! 2. **確定してすぐ次を打ったとき**: 確定した文字が入るとキャレットは右へ進むが、窓の位置は前の変換が
//!    始まった場所のまま。続けて打つと、IME は変換を終えずに次の未確定の文字を出すことがあり
//!    (`WM_IME_STARTCOMPOSITION` が来ない)、未確定の文字が、いま確定した文の頭に重なって出る。
//!    確定すると正しい場所に入るので、文字が右へ飛んだように見える。
//!    → 確定した文字が入った後 (`WM_IME_COMPOSITION` の `GCS_RESULTSTR` の後、変換中に届いた `WM_CHAR` の後) に、
//!    窓の位置をキャレットへ移し直す。本家の Scintilla (ScintillaWin::HandleCompositionWindowed の
//!    "Set new position after converted") と同じ対処。
//!
//! エディタの HWND をサブクラス化して行う。

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;

use wxdragon::prelude::*;

use crate::scintilla;

type Hwnd = *mut c_void;
type Himc = *mut c_void;
type SubclassProc = unsafe extern "system" fn(Hwnd, u32, usize, isize, usize, usize) -> isize;

#[repr(C)]
struct Point {
    x: i32,
    y: i32,
}

#[repr(C)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

/// Win32 の COMPOSITIONFORM
#[repr(C)]
struct CompositionForm {
    style: u32,
    pos: Point,
    area: Rect,
}

#[link(name = "comctl32")]
extern "system" {
    fn SetWindowSubclass(hwnd: Hwnd, pfn: SubclassProc, id: usize, ref_data: usize) -> i32;
    fn RemoveWindowSubclass(hwnd: Hwnd, pfn: SubclassProc, id: usize) -> i32;
    fn DefSubclassProc(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
}

#[link(name = "imm32")]
extern "system" {
    fn ImmGetContext(hwnd: Hwnd) -> Himc;
    fn ImmReleaseContext(hwnd: Hwnd, himc: Himc) -> i32;
    fn ImmSetCompositionWindow(himc: Himc, form: *const CompositionForm) -> i32;
}

#[link(name = "user32")]
extern "system" {
    fn PostMessageW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> i32;
}

const WM_NCDESTROY: u32 = 0x0082;
const WM_CHAR: u32 = 0x0102;
const WM_IME_STARTCOMPOSITION: u32 = 0x010D;
const WM_IME_ENDCOMPOSITION: u32 = 0x010E;
const WM_IME_COMPOSITION: u32 = 0x010F;
const WM_IME_CHAR: u32 = 0x0286;
/// 自分宛ての知らせ: たまっている文字のメッセージが片付いた後で、窓の位置を合わせ直す (WM_APP の範囲)
const WM_APP_PLACE_IME: u32 = 0x8000 + 0x2E7;
/// `WM_IME_COMPOSITION` の lParam: 確定した文字が入っている
const GCS_RESULTSTR: isize = 0x0800;
const CFS_POINT: u32 = 0x0002;
const SCI_POINTXFROMPOSITION: i32 = 2164;
const SCI_POINTYFROMPOSITION: i32 = 2165;
const SUBCLASS_ID: usize = 0x7479_7062; // "typb"

thread_local! {
    static EDITORS: RefCell<HashMap<usize, StyledTextCtrl>> = RefCell::new(HashMap::new());
    /// 変換の途中 (`WM_IME_STARTCOMPOSITION` から `WM_IME_ENDCOMPOSITION` まで) のエディタ
    static COMPOSING: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
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

fn editor(hwnd: Hwnd) -> Option<StyledTextCtrl> {
    EDITORS.with(|m| m.borrow().get(&(hwnd as usize)).copied())
}

fn composing(hwnd: Hwnd) -> bool {
    COMPOSING.with(|s| s.borrow().contains(&(hwnd as usize)))
}

/// IME の変換ウィンドウを、いまのキャレットの位置へ移す
///
/// # Safety
/// `hwnd` は生存中のエディタの HWND で、その UI スレッドから呼ぶこと。
unsafe fn place_ime_window(hwnd: Hwnd) {
    let Some(stc) = editor(hwnd) else {
        return;
    };
    let pos = stc.get_current_pos() as isize;
    let form = CompositionForm {
        style: CFS_POINT,
        pos: Point {
            x: scintilla::send(&stc, SCI_POINTXFROMPOSITION, 0, pos) as i32,
            y: scintilla::send(&stc, SCI_POINTYFROMPOSITION, 0, pos) as i32,
        },
        area: Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
    };
    let himc = ImmGetContext(hwnd);
    if himc.is_null() {
        return;
    }
    ImmSetCompositionWindow(himc, &form);
    ImmReleaseContext(hwnd, himc);
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
            if let Some(stc) = editor(hwnd) {
                let (start, end) = stc.get_selection();
                if start != end {
                    stc.replace_selection("");
                }
            }
            COMPOSING.with(|s| s.borrow_mut().insert(hwnd as usize));
        }
        WM_IME_ENDCOMPOSITION => {
            COMPOSING.with(|s| s.borrow_mut().remove(&(hwnd as usize)));
        }
        WM_IME_COMPOSITION if lparam & GCS_RESULTSTR != 0 => {
            // 確定した文字は、この中 (既定の処理) で WM_IME_CHAR / WM_CHAR になってエディタに入る
            let result = DefSubclassProc(hwnd, msg, wparam, lparam);
            place_ime_window(hwnd);
            // 文字がキューに積まれただけでまだ入っていない場合に備えて、それらの後でもう一度合わせる
            PostMessageW(hwnd, WM_APP_PLACE_IME, 0, 0);
            return result;
        }
        WM_CHAR | WM_IME_CHAR if composing(hwnd) => {
            // 変換の途中に文字が入った (前の変換の確定が遅れて届いた): キャレットが動くので、窓も付いていかせる
            let result = DefSubclassProc(hwnd, msg, wparam, lparam);
            place_ime_window(hwnd);
            return result;
        }
        WM_APP_PLACE_IME => {
            place_ime_window(hwnd);
            return 0;
        }
        WM_NCDESTROY => {
            EDITORS.with(|m| m.borrow_mut().remove(&(hwnd as usize)));
            COMPOSING.with(|s| s.borrow_mut().remove(&(hwnd as usize)));
            RemoveWindowSubclass(hwnd, subclass_proc, SUBCLASS_ID);
        }
        _ => {}
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}
