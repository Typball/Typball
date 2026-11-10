//! wxDragon が公開していない Scintilla メッセージを送る
//!
//! wxDragon の `StyledTextCtrl` は wxStyledTextCtrl のメソッドの一部しか包んでいない。
//! ルーラーや右側の行番号を自前で描くには「先頭の表示行」「行の高さ」「文書位置 → 画面座標」
//! などが必要なので、`wxStyledTextCtrl::SendMsg` (Scintilla の生メッセージ) を C++ の
//! シンボル名で直接呼ぶ。非仮想メンバ関数なので `this` を第 1 引数に取る C 関数として呼べる。
//!
//! シンボル名は ABI ごとに異なる。Itanium ABI では引数型 (int, wxUIntPtr, wxIntPtr) が
//! Win64 では `unsigned long long` / `long long` (= "yx")、LP64 (Linux / macOS) では
//! `unsigned long` / `long` (= "ml") になる。MSVC は独自の装飾名。
//! wxDragon 側に SendMsg が追加されたらこのモジュールはそれに置き換える。

use std::ffi::{c_void, CString};

use wxdragon::prelude::*;

extern "C" {
    #[cfg_attr(
        all(windows, target_env = "gnu"),
        link_name = "_ZNK16wxStyledTextCtrl7SendMsgEiyx"
    )]
    #[cfg_attr(
        all(windows, target_env = "msvc"),
        link_name = "?SendMsg@wxStyledTextCtrl@@QEBA_JH_K_J@Z"
    )]
    #[cfg_attr(not(windows), link_name = "_ZNK16wxStyledTextCtrl7SendMsgEiml")]
    fn wx_stc_send_msg(this: *const c_void, msg: i32, wp: usize, lp: isize) -> isize;
}

// Scintilla.h の定数 (使うものだけ)
pub const SCI_GETFIRSTVISIBLELINE: i32 = 2152;
pub const SCI_BEGINUNDOACTION: i32 = 2078;
pub const SCI_ENDUNDOACTION: i32 = 2079;
pub const SCI_GETLINEENDPOSITION: i32 = 2136;
pub const SCI_SETMARGINLEFT: i32 = 2155;
pub const SCI_GETMARGINLEFT: i32 = 2156;
pub const SCI_SETMARGINRIGHT: i32 = 2157;
pub const SCI_POINTXFROMPOSITION: i32 = 2164;
pub const SCI_POINTYFROMPOSITION: i32 = 2165;
pub const SCI_VISIBLEFROMDOCLINE: i32 = 2220;
pub const SCI_DOCLINEFROMVISIBLE: i32 = 2221;
pub const SCI_SETFIRSTVISIBLELINE: i32 = 2613;
pub const SCI_WRAPCOUNT: i32 = 2235;
pub const SCI_GETMARGINWIDTHN: i32 = 2243;
pub const SCI_SETMARGINMASKN: i32 = 2244;
pub const SCI_TEXTWIDTH: i32 = 2276;
pub const SCI_TEXTHEIGHT: i32 = 2279;
pub const SCI_SETMODEVENTMASK: i32 = 2359;
pub const SCI_LINESONSCREEN: i32 = 2370;
pub const SCI_MARGINSETTEXT: i32 = 2530;
pub const SCI_MARGINSETSTYLE: i32 = 2532;
pub const SCI_COUNTCHARACTERS: i32 = 2633;
pub const SCI_CANPASTE: i32 = 2173;
pub const SCI_CLEAR: i32 = 2180;
pub const SCI_REPLACETARGET: i32 = 2194;
pub const SCI_REPLACETARGETRE: i32 = 2195;
pub const SCI_SEARCHINTARGET: i32 = 2197;
pub const SCI_SETSEARCHFLAGS: i32 = 2198;
pub const SCI_USEPOPUP: i32 = 2371;
pub const SC_POPUP_NEVER: usize = 0;
pub const SCFIND_WHOLEWORD: i32 = 0x2;
pub const SCFIND_MATCHCASE: i32 = 0x4;
pub const SCFIND_REGEXP: i32 = 0x0020_0000;
pub const SCFIND_POSIX: i32 = 0x0040_0000;

pub const SC_MOD_INSERTTEXT: i32 = 0x1;
pub const SC_MOD_DELETETEXT: i32 = 0x2;

/// Scintilla メッセージを送る。エディタが破棄済みなら 0。
pub fn send(stc: &StyledTextCtrl, msg: i32, wp: usize, lp: isize) -> isize {
    let ptr = stc.handle_ptr();
    if ptr.is_null() {
        return 0;
    }
    // SAFETY: ptr は wxdragon が `new wxStyledTextCtrl` で作ったオブジェクトそのもので
    // (wxWindow は第 1 基底クラスなのでアドレスは一致する)、同じ UI スレッドから呼んでいる。
    unsafe { wx_stc_send_msg(ptr as *const c_void, msg, wp, lp) }
}

/// 文字列を lParam で渡すメッセージ
fn send_str(stc: &StyledTextCtrl, msg: i32, wp: usize, text: &str) -> isize {
    let Ok(c) = CString::new(text) else {
        return 0;
    };
    send(stc, msg, wp, c.as_ptr() as isize)
}

/// Scintilla 自身の右クリックメニュー (英語の Undo / Cut / Copy ...) を出さないようにする。
/// 代わりに wx の wxEVT_CONTEXT_MENU が親へ伝わるので、そこで自前のメニューを出す (tab.rs)
pub fn disable_popup(stc: &StyledTextCtrl) {
    send(stc, SCI_USEPOPUP, SC_POPUP_NEVER, 0);
}

/// 選択範囲を削除する (選択が無ければ何もしない)
pub fn clear_selection(stc: &StyledTextCtrl) {
    send(stc, SCI_CLEAR, 0, 0);
}

/// クリップボードに貼り付けられる内容があるか
pub fn can_paste(stc: &StyledTextCtrl) -> bool {
    send(stc, SCI_CANPASTE, 0, 0) != 0
}

/// 検索範囲 (target) を `start`..`end` にして検索する。`start > end` なら後ろ向きに探す。
/// 見つかれば一致範囲の (先頭, 末尾) を返す (正規表現や大文字小文字の無視でも一致の長さが分かる)。
/// `flags` は SCFIND_* の組み合わせ
pub fn search(
    stc: &StyledTextCtrl,
    start: i32,
    end: i32,
    text: &str,
    flags: i32,
) -> Option<(i32, i32)> {
    if text.is_empty() {
        return None;
    }
    send(stc, SCI_SETSEARCHFLAGS, flags as usize, 0);
    stc.set_target_start(start);
    stc.set_target_end(end);
    if send_str(stc, SCI_SEARCHINTARGET, text.len(), text) < 0 {
        return None;
    }
    Some((stc.get_target_start(), stc.get_target_end()))
}

/// 検索範囲 (直前の `search` の一致範囲) を `text` で置き換え、置き換えた後の長さを返す。
/// `regex` なら `` などの後方参照を展開する
pub fn replace_target(stc: &StyledTextCtrl, text: &str, regex: bool) -> i32 {
    let msg = if regex {
        SCI_REPLACETARGETRE
    } else {
        SCI_REPLACETARGET
    };
    send_str(stc, msg, usize::MAX, text) as i32
}

/// 先頭に表示されている表示行 (折り返し後の行番号)
pub fn first_visible_line(stc: &StyledTextCtrl) -> i32 {
    send(stc, SCI_GETFIRSTVISIBLELINE, 0, 0) as i32
}

/// 画面に収まる表示行数
pub fn lines_on_screen(stc: &StyledTextCtrl) -> i32 {
    send(stc, SCI_LINESONSCREEN, 0, 0) as i32
}

/// 表示行 → 文書の行
pub fn doc_line_from_visible(stc: &StyledTextCtrl, visible: i32) -> i32 {
    send(stc, SCI_DOCLINEFROMVISIBLE, visible.max(0) as usize, 0) as i32
}

/// 文書の行 → 表示行 (折り返し後の行番号)
pub fn visible_from_doc_line(stc: &StyledTextCtrl, line: i32) -> i32 {
    send(stc, SCI_VISIBLEFROMDOCLINE, line.max(0) as usize, 0) as i32
}

/// 表示行 `visible` が先頭に来るようにスクロールする (文書の末尾付近は収まる位置まで)
pub fn set_first_visible_line(stc: &StyledTextCtrl, visible: i32) {
    send(stc, SCI_SETFIRSTVISIBLELINE, visible.max(0) as usize, 0);
}

/// 文書の行が折り返しで何段になっているか
pub fn wrap_count(stc: &StyledTextCtrl, line: i32) -> i32 {
    send(stc, SCI_WRAPCOUNT, line as usize, 0) as i32
}

/// 行の高さ (px)
pub fn text_height(stc: &StyledTextCtrl, line: i32) -> i32 {
    send(stc, SCI_TEXTHEIGHT, line as usize, 0) as i32
}

/// スタイルのフォントで文字列を描いたときの幅 (px、ズーム込み)
pub fn text_width(stc: &StyledTextCtrl, style: i32, text: &str) -> i32 {
    send_str(stc, SCI_TEXTWIDTH, style as usize, text) as i32
}

/// 文書位置のクライアント座標 x / y (行の先頭、折り返し段の先頭)
pub fn point_x_from_position(stc: &StyledTextCtrl, pos: i32) -> i32 {
    send(stc, SCI_POINTXFROMPOSITION, 0, pos as isize) as i32
}

pub fn point_y_from_position(stc: &StyledTextCtrl, pos: i32) -> i32 {
    send(stc, SCI_POINTYFROMPOSITION, 0, pos as isize) as i32
}

/// マージンに表示するマーカーのビットマスク
pub fn set_margin_mask(stc: &StyledTextCtrl, margin: i32, mask: i32) {
    send(stc, SCI_SETMARGINMASKN, margin as usize, mask as isize);
}

/// マージンと本文の間隔 (px)
pub fn set_margin_left(stc: &StyledTextCtrl, px: i32) {
    send(stc, SCI_SETMARGINLEFT, 0, px as isize);
}

pub fn margin_left(stc: &StyledTextCtrl) -> i32 {
    send(stc, SCI_GETMARGINLEFT, 0, 0) as i32
}

/// 本文の右側の余白 (px)。折り返しの幅はこのぶん狭くなる
pub fn set_margin_right(stc: &StyledTextCtrl, px: i32) {
    send(stc, SCI_SETMARGINRIGHT, 0, px as isize);
}

/// マージンの幅 (px)
pub fn margin_width(stc: &StyledTextCtrl, margin: i32) -> i32 {
    send(stc, SCI_GETMARGINWIDTHN, margin as usize, 0) as i32
}

/// テキストマージンに表示する行ごとの文字列とそのスタイル
pub fn margin_set_text(stc: &StyledTextCtrl, line: i32, text: &str) {
    send_str(stc, SCI_MARGINSETTEXT, line as usize, text);
}

pub fn margin_set_style(stc: &StyledTextCtrl, line: i32, style: i32) {
    send(stc, SCI_MARGINSETSTYLE, line as usize, style as isize);
}

/// EVT_STC_CHANGE / EVT_STC_MODIFIED を飛ばす変更の種類を絞る
pub fn set_mod_event_mask(stc: &StyledTextCtrl, mask: i32) {
    send(stc, SCI_SETMODEVENTMASK, mask as usize, 0);
}

/// 行末 (改行文字の手前) の位置
pub fn line_end_position(stc: &StyledTextCtrl, line: i32) -> i32 {
    send(stc, SCI_GETLINEENDPOSITION, line.max(0) as usize, 0) as i32
}

/// 2 つの位置の間の文字数 (バイト数ではなく、UTF-8 の文字数)
pub fn count_characters(stc: &StyledTextCtrl, start: i32, end: i32) -> i32 {
    send(
        stc,
        SCI_COUNTCHARACTERS,
        start.max(0) as usize,
        end.max(0) as isize,
    ) as i32
}

/// 複数の編集を 1 回の Undo にまとめる
pub fn begin_undo_action(stc: &StyledTextCtrl) {
    send(stc, SCI_BEGINUNDOACTION, 0, 0);
}

pub fn end_undo_action(stc: &StyledTextCtrl) {
    send(stc, SCI_ENDUNDOACTION, 0, 0);
}
