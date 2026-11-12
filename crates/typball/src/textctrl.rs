//! 複数行のテキスト欄 (wxTextCtrl。Windows では普通の EDIT コントロール) の補助
//!
//! タブ幅を設定に合わせる: プレビューウィンドウのテキスト表示は `[preview] tab_width`、設定画面の雛形の欄
//! (エディタに入る中身) は `[editor] tab_width`。EDIT の既定は半角 8 文字ぶんで、そのままだと違って見える。
//! EDIT はタブ位置を「設定した時点のフォント」でピクセルに直して覚えるので、フォントが変わるたびに
//! (設定の変更、別の拡大率のモニターへの移動) 設定し直す。

use std::cell::Cell;

use wxdragon::prelude::*;

use crate::settings::{self, Settings};

/// どの設定のタブ幅 (半角の文字数) に合わせるか
pub type TabWidth = fn(&Settings) -> i32;

/// エディタのタブ幅 (`[editor] tab_width`)
pub fn editor_tab_width(settings: &Settings) -> i32 {
    settings.editor.tab_width
}

/// プレビューのタブ幅 (`[preview] tab_width`)
pub fn preview_tab_width(settings: &Settings) -> i32 {
    settings.preview.tab_width
}

/// タブ幅を `settings` の値 (1〜16 に収める) と今のフォントで設定し直す。
/// フォントやタブ幅の設定を替えた直後に呼ぶ
pub fn apply_tab_width(text: &TextCtrl, settings: &Settings, width: TabWidth) {
    set_tab_stops(text, width(settings).clamp(1, 16));
}

/// タブ幅を設定に合わせ、ウィンドウが別の拡大率のモニターへ移ったときも合わせ直す。
/// 欄を作ってフォントを当てた後に 1 回呼ぶ
pub fn keep_tab_width(text: &TextCtrl, settings: &Settings, width: TabWidth) {
    apply_tab_width(text, settings, width);
    let text = *text;
    let scale = Cell::new(text.from_dip_int(9600));
    text.on_size(move |_| {
        let now = text.from_dip_int(9600);
        if now == scale.get() {
            return;
        }
        scale.set(now);
        // wx は拡大率が変わるとウィンドウの大きさを先に変え、フォントはその後で当て直す。
        // ここではまだ古いフォントなので、フォントが替わった後 (アイドル時) に設定する
        wxdragon::call_after(Box::new(move || {
            apply_tab_width(&text, &settings::get(), width);
        }));
        wxdragon::wake_up_idle();
    });
}

#[cfg(windows)]
fn set_tab_stops(text: &TextCtrl, chars: i32) {
    use windows_sys::Win32::Foundation::SIZE;
    use windows_sys::Win32::Graphics::Gdi::{
        GetDC, GetTextExtentPoint32W, ReleaseDC, SelectObject,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_GETFONT};
    /// EDIT コントロールのタブ位置 (winuser.h。windows-sys には無い)
    const EM_SETTABSTOPS: u32 = 0x00CB;
    // 欄が先に壊されていたら null (wxDragon のハンドルは破棄で無効になる)
    let hwnd = text.get_handle();
    if hwnd.is_null() {
        return;
    }
    // タブ幅はエディタ (Scintilla) と同じく「空白の幅 × 文字数」にする。EDIT に渡す単位は
    // ダイアログ単位 (欄のフォントの平均の字幅の 1/4。平均は英字 52 文字の幅から求める) なので、
    // 欄が実際に使っているフォントで両方を測って換算する。等幅フォントなら文字数 × 4 になる
    let letters: Vec<u16> = ('A'..='Z').chain('a'..='z').map(|c| c as u16).collect();
    let space = [b' ' as u16];
    let mut all = SIZE { cx: 0, cy: 0 };
    let mut one = SIZE { cx: 0, cy: 0 };
    // SAFETY: hwnd は生きている wxTextCtrl のウィンドウハンドルで、UI スレッドから呼んでいる。
    // DC は使い終わったら元のフォントに戻して返す
    unsafe {
        let font = SendMessageW(hwnd as _, WM_GETFONT, 0, 0);
        let dc = GetDC(hwnd as _);
        if dc.is_null() {
            return;
        }
        let old = if font != 0 {
            SelectObject(dc, font as _)
        } else {
            std::ptr::null_mut()
        };
        GetTextExtentPoint32W(dc, letters.as_ptr(), letters.len() as i32, &mut all);
        GetTextExtentPoint32W(dc, space.as_ptr(), 1, &mut one);
        if !old.is_null() {
            SelectObject(dc, old);
        }
        ReleaseDC(hwnd as _, dc);
    }
    let average = ((all.cx / 26 + 1) / 2).max(1);
    let width = chars.max(1) * one.cx.max(1);
    let stops: u32 = ((width * 4 + average / 2) / average).max(1) as u32;
    // SAFETY: 上と同じ。stops はこの呼び出しの間だけ読まれる
    unsafe {
        SendMessageW(hwnd as _, EM_SETTABSTOPS, 1, &stops as *const u32 as isize);
    }
    // タブ位置を変えても描き直されない
    text.refresh(true, None);
}

/// GTK / macOS のテキスト欄にはタブ幅を変える手段が無い (wx の既定のまま)
#[cfg(not(windows))]
fn set_tab_stops(_text: &TextCtrl, _chars: i32) {}
