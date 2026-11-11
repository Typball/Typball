//! 不可視文字の記号表示 (改行・ファイルの終わり・空白とタブ・全角空白)
//!
//! 設定 `editor.show_eol` / `show_eof` / `show_whitespace` (表示 > 記号を表示)。
//! 新規文書の改行コード (設定) と違う改行は、種類ごとに違う色で強調する。
//!
//! 記号はフォントの文字ではなく図形で描く。フォントにある文字 (中点や矢印) で出すと、打ち込んだ文字と
//! 見分けにくい (EmEditor も記号は図形で描いている)。半角空白は受け皿の形、タブは矢印、全角空白は枠、
//! 改行は矢印 (LF ↓ / CR ← / CRLF は下りてから左へ折れる)、ファイルの終わりは `EOF` の札。
//! 改行と EOF は行末のすぐ後ろに描く。
//!
//! # 描き方
//!
//! エディタ (Scintilla) には記号を図形で描く機能が無い (同梱の 5.0 は改行記号の色も変えられない) ので、
//! エディタが自分を描いた直後に、同じ描画の中で上から描き足す ([`on_painted`] → [`paint`])。
//!
//! - 描き足す先は、エディタが今使っている描画 (WM_PAINT) の DC。描き直された範囲にだけ描けるので、
//!   描き直された行の記号だけが引き直される。
//! - 別に画面へ描く (`ClientDC`) と、エディタが画面に出した後に記号を重ねることになり、その間の
//!   一瞬だけ記号の無い絵が見える (クリックや入力のたびにちらつく)。エディタのウィンドウを二重バッファに
//!   してあるので (`wxd_Window_BindAfterPaint`)、エディタの絵と記号は 1 回でまとめて画面に出る。
//! - 記号の位置は Scintilla に聞く (`SCI_POINTXFROMPOSITION` など)。1 回の描画で何百回も聞くので、
//!   画面の行の配置を覚えさせておく (`SCI_SETLAYOUTCACHE`)。

use std::ffi::c_void;

use wxdragon::prelude::*;

use crate::fileio::LineEnding;
use crate::scintilla;
use crate::settings::Settings;

const WIDE_SPACE: &str = "\u{3000}";

const SCI_GETCHARAT: i32 = 2007;
const SCI_SETVIEWWS: i32 = 2021;
const SCI_GETMARGINS: i32 = 2253;
const SCI_SETLAYOUTCACHE: i32 = 2272;
/// SC_CACHE_PAGE: 画面に出ている行の配置を覚えておく (既定はキャレットのある行だけ)
const SC_CACHE_PAGE: usize = 2;

/// 新規文書の改行コードと違う改行の色
fn different_colour(eol: LineEnding) -> u32 {
    let c = crate::editor::colors();
    match eol {
        LineEnding::Crlf => c.mark_crlf,
        LineEnding::Lf => c.mark_lf,
        LineEnding::Cr => c.mark_cr,
    }
}

/// エディタの設定 (`editor::apply_settings` から呼ぶ)
pub fn apply_settings(stc: &StyledTextCtrl) {
    // 記号の位置を聞くたびに行を組み直さないように、画面の行の配置を覚えさせる
    scintilla::send(stc, SCI_SETLAYOUTCACHE, SC_CACHE_PAGE, 0);
    // Scintilla 自身の空白表示 (点) は使わない
    scintilla::send(stc, SCI_SETVIEWWS, 0, 0);
}

/// エディタの描画で使っている DC (描画の間だけ有効)
pub struct PaintDc(*mut wxdragon::ffi::wxd_DC_t);

impl DeviceContext for PaintDc {
    fn dc_ptr(&self) -> *mut wxdragon::ffi::wxd_DC_t {
        self.0
    }
}

/// エディタ (`stc`) が自分を描き終えるたびに、同じ描画の中で `f` を呼ぶ。`f` は渡された DC に
/// 描き足す (2 つ目の引数は描き直された範囲、3 つ目は IME で入力している途中かどうか)。
///
/// vendored の wxdragon-sys に足した `wxd_Window_BindAfterPaint` を使う。Scintilla の「描き終えた」
/// 通知 (`EVT_STC_PAINTED`) では早すぎる: wxStyledTextCtrl は裏のビットマップに描いてから、描画
/// ハンドラの終わりにまとめて写す (wxBufferedPaintDC) ので、通知の中で描いたものはその転写で消える
pub fn on_painted(stc: &StyledTextCtrl, f: impl Fn(&PaintDc, Rect, bool) + 'static) {
    type Callback = Box<dyn Fn(&PaintDc, Rect, bool)>;

    unsafe extern "C" fn call(
        userdata: *mut c_void,
        dc: *mut wxdragon::ffi::wxd_DC_t,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        composing: i32,
    ) {
        if dc.is_null() {
            return;
        }
        // SAFETY: userdata は下で Box::into_raw した Callback で、release が呼ばれるまで生きている
        let f = unsafe { &*(userdata as *const Callback) };
        let dc = PaintDc(dc);
        // パニックを C++ へ抜けさせない
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f(&dc, Rect::new(x, y, width, height), composing != 0)
        }));
    }
    unsafe extern "C" fn release(userdata: *mut c_void) {
        // SAFETY: ウィンドウが破棄されるときに 1 回だけ呼ばれる
        drop(unsafe { Box::from_raw(userdata as *mut Callback) });
    }

    let boxed: Box<Callback> = Box::new(Box::new(f));
    // SAFETY: stc は生きているウィンドウ。userdata の持ち主は C++ 側に移り、release で返ってくる
    unsafe {
        wxdragon::ffi::wxd_Window_BindAfterPaint(
            stc.handle_ptr(),
            Some(call),
            Box::into_raw(boxed) as *mut c_void,
            Some(release),
        );
    }
}

/// 行の終わりの改行コード。最後の行 (改行で終わらない) は None
fn line_ending_of(stc: &StyledTextCtrl, line: i32, count: i32) -> Option<LineEnding> {
    if line + 1 >= count {
        return None;
    }
    let end = scintilla::line_end_position(stc, line);
    let next = stc.position_from_line(line + 1);
    match next - end {
        2 => Some(LineEnding::Crlf),
        1 => {
            if scintilla::send(stc, SCI_GETCHARAT, end as usize, 0) as u8 == b'\r' {
                Some(LineEnding::Cr)
            } else {
                Some(LineEnding::Lf)
            }
        }
        _ => None,
    }
}

/// 記号を描くときの寸法 (px)。半角 1 文字の幅と行の高さから決める (ズームにも付いていく)
struct Metrics {
    /// 半角 1 文字の幅
    cell: i32,
    /// 線の太さ
    thin: i32,
    /// 矢じりの大きさ
    head: i32,
}

/// 描き直された範囲 (`update`) にかかる行の記号を `dc` に描く。エディタが自分を描き終えるたびに呼ぶ
/// ([`on_painted`])。
///
/// IME で入力している途中 (`composing`) は、キャレットのある行の改行と EOF の記号を描かない。
/// 未確定の文字は IME の窓がエディタの上に重ねて出しているだけで、文書にはまだ入っていない。
/// 確定すると先にその窓が消え、少し遅れて文字が入るので、記号を描いたままだと、窓の下に隠れていた
/// 記号が元の位置に一瞬見えてから行末へ動く
pub fn paint(
    stc: &StyledTextCtrl,
    settings: &Settings,
    dc: &PaintDc,
    update: Rect,
    composing: bool,
) {
    let e = &settings.editor;
    if !e.show_eol && !e.show_eof && !e.show_whitespace {
        return;
    }
    let count = stc.get_line_count();
    if count <= 0 || update.width <= 0 || update.height <= 0 {
        return;
    }
    let first = scintilla::first_visible_line(stc);
    let screen = scintilla::lines_on_screen(stc);
    let from = scintilla::doc_line_from_visible(stc, first).clamp(0, count - 1);
    let to = scintilla::doc_line_from_visible(stc, first + screen + 1).clamp(from, count - 1);

    let cell = scintilla::text_width(stc, crate::editor::STYLE_DEFAULT, " ").max(3);
    let thin = stc.from_dip_int(1).max(1);
    let m = Metrics {
        cell,
        thin,
        head: (cell * 3 / 8).max(2),
    };
    let colours = crate::editor::colors();
    let rgb = crate::editor::rgb;
    let base = settings.files.line_ending();

    // 行番号などのマージンには描かない (横にスクロールした行の記号がはみ出す)
    let margins: i32 = (0..scintilla::send(stc, SCI_GETMARGINS, 0, 0) as i32)
        .map(|margin| scintilla::margin_width(stc, margin))
        .sum();
    let size = stc.get_client_size();
    dc.set_clipping_region(margins, 0, (size.width - margins).max(0), size.height);
    dc.set_brush(rgb(colours.mark), BrushStyle::Transparent);
    let pen = |colour: u32, width: i32| dc.set_pen(rgb(colour), width, PenStyle::Solid);

    let (update_top, update_bottom) = (update.y, update.y + update.height);
    // 行間 (設定) で足した高さは記号の大きさに入れない: 文字のある高さの中に描く
    let (extra_top, extra_bottom) = crate::editor::line_extra(stc, settings);
    let composing_line = composing.then(|| stc.get_current_line());
    for line in from..=to {
        let start = stc.position_from_line(line);
        let end = scintilla::line_end_position(stc, line);
        let height = scintilla::text_height(stc, line);
        // 描き直されなかった行は、前に描いた記号がそのまま残っている (折り返した行は段の数だけ高い)
        let line_top = scintilla::point_y_from_position(stc, start);
        let line_bottom = scintilla::point_y_from_position(stc, end) + height;
        if line_bottom <= update_top || line_top >= update_bottom {
            continue;
        }
        // ここから下の y と height は、行の中の文字のある部分
        let height = (height - extra_top - extra_bottom).max(4);
        let point = |pos: i32| {
            (
                scintilla::point_x_from_position(stc, pos),
                scintilla::point_y_from_position(stc, pos) + extra_top,
            )
        };
        // 記号の幅: 次の文字の位置まで。折り返しで次の文字が次の段にあるときは `fallback`
        let width_at = |pos: i32, len: i32, x: i32, y: i32, fallback: i32| {
            let (next_x, next_y) = point(pos + len);
            if next_y == y && next_x > x {
                next_x - x
            } else {
                fallback
            }
        };

        if e.show_whitespace && end > start {
            // 半角空白は太さ 1 のペンで塗る (fill)。タブと全角空白はその後で線の太さに戻す
            pen(colours.mark, 1);
            let bytes = scintilla::line_bytes(stc, line);
            let text = &bytes[..((end - start) as usize).min(bytes.len())];
            let mut i = 0;
            while i < text.len() {
                let pos = start + i as i32;
                match text[i] {
                    b' ' => {
                        let (x, y) = point(pos);
                        let width = width_at(pos, 1, x, y, m.cell);
                        draw_space(dc, m.thin, x, y, width, height);
                        i += 1;
                    }
                    b'\t' => {
                        let (x, y) = point(pos);
                        let width = width_at(pos, 1, x, y, m.cell);
                        pen(colours.mark, m.thin);
                        draw_tab(dc, &m, x, y, width, height);
                        pen(colours.mark, 1);
                        i += 1;
                    }
                    _ if text[i..].starts_with(WIDE_SPACE.as_bytes()) => {
                        let (x, y) = point(pos);
                        let width = width_at(pos, 3, x, y, m.cell * 2);
                        pen(colours.mark, m.thin);
                        draw_wide_space(dc, x, y, width, height);
                        pen(colours.mark, 1);
                        i += WIDE_SPACE.len();
                    }
                    _ => i += 1,
                }
            }
        }

        if composing_line == Some(line) {
            continue;
        }
        match line_ending_of(stc, line, count) {
            Some(eol) if e.show_eol => {
                // 設定と違う改行は色で強調する。線は同じ細さ: 太いペンは線の左右どちらかに寄って
                // 描かれるので、矢印が左右対称にならない。矢印は太さ 1 のペンで塗って描く (fill)
                let colour = if eol == base {
                    colours.mark
                } else {
                    different_colour(eol)
                };
                pen(colour, 1);
                let (x, y) = point(end);
                draw_line_ending(dc, &m, eol, x + m.thin, y, height);
            }
            None if e.show_eof => {
                let (x, y) = point(end);
                draw_eof(dc, stc, settings, &m, x, y, height);
            }
            _ => {}
        }
    }
    dc.destroy_clipping_region();
}

/// 線分。wx の線は終点の画素を描かないので、終点も打つ
fn stroke(dc: &PaintDc, x1: i32, y1: i32, x2: i32, y2: i32) {
    dc.draw_line(x1, y1, x2, y2);
    dc.draw_line(x2, y2, x1, y1);
}

/// 塗った長方形 (左上 `x`, `y`・幅 `w`・高さ `h`)。太さ 1 のペンで 1 行ずつ引く: 線の端の画素や
/// 太いペンの角の扱いに頼らないので、角まで欠けずに出る (呼ぶ前にペンの太さを 1 にしておくこと)
fn fill(dc: &PaintDc, x: i32, y: i32, w: i32, h: i32) {
    for row in 0..h.max(0) {
        dc.draw_line(x, y + row, x + w, y + row);
    }
}

/// 半角空白: 文字の下の方に受け皿の形 (両端が少し立った下線)。`t` は線の太さ
fn draw_space(dc: &PaintDc, t: i32, x: i32, y: i32, width: i32, height: i32) {
    let left = x + t;
    let w = width - 2 * t;
    if w < 3 * t {
        return;
    }
    // 底の線の位置と、両端の立ち上がりの高さ
    let bottom = y + height - (height / 5).max(2) + t;
    let rise = ((height / 7).max(2) - t).max(t);
    fill(dc, left, bottom - rise, t, rise + t);
    fill(dc, left, bottom, w, t);
    fill(dc, left + w - t, bottom - rise, t, rise + t);
}

/// タブ: 先頭に右向きの矢印 (タブの幅が広くても矢印は 1 文字半まで)
fn draw_tab(dc: &PaintDc, m: &Metrics, x: i32, y: i32, width: i32, height: i32) {
    let left = x + 1;
    let right = x + width.min(m.cell * 3 / 2).max(m.cell) - 2;
    let mid = y + height * 11 / 20;
    stroke(dc, left, mid, right, mid);
    stroke(dc, right, mid, right - m.head, mid - m.head);
    stroke(dc, right, mid, right - m.head, mid + m.head);
}

/// 全角空白: 枠
fn draw_wide_space(dc: &PaintDc, x: i32, y: i32, width: i32, height: i32) {
    let inset = (height / 6).max(2);
    dc.draw_rectangle(
        x + 1,
        y + inset,
        (width - 2).max(2),
        (height - inset * 2).max(2),
    );
}

/// 改行: 行末のすぐ後ろに矢印。LF は下向き、CR は左向き、CRLF は下りてから左へ折れる。
/// どれも半角 1 文字の幅に収める
fn draw_line_ending(dc: &PaintDc, m: &Metrics, eol: LineEnding, x: i32, y: i32, height: i32) {
    let t = m.thin;
    let top = y + height / 5;
    // 矢印の下の端 (この行まで描く)
    let bottom = y + height - (height / 5).max(2);
    // 横向きの矢印の左右の端 (半角 1 文字ぶんの中。`right` の列まで描く)
    let left = x + 1;
    let right = x + m.cell - 2;
    // 横向きの矢じりは、幅に収まるように小さめにする
    let head = m.head.min((right - left) / 2).max(2);
    match eol {
        LineEnding::Lf => {
            // 軸は 1 文字の幅の中央。矢じりは軸の左右に同じだけ開く
            let shaft = x + (m.cell - t) / 2;
            fill(dc, shaft, top, t, bottom + 1 - top);
            head_down(dc, shaft, bottom, t, m.head);
        }
        LineEnding::Cr => {
            let shaft = y + height * 11 / 20 - t / 2;
            fill(dc, left, shaft, right + 1 - left, t);
            // 折れ曲がりが無いぶん、矢じりは LF と同じ大きさまで取れる
            head_left(dc, left, shaft, t, m.head.min(right - left - 2).max(2));
        }
        LineEnding::Crlf => {
            // 矢じりの下の端が `bottom` に来る高さで左へ折れる
            let shaft = bottom + 1 - t - head;
            fill(dc, right + 1 - t, top, t, shaft + t - top);
            fill(dc, left, shaft, right + 1 - left, t);
            head_left(dc, left, shaft, t, head);
        }
    }
}

/// 斜めの線 (45°) の、1 行 (1 列) ぶんの長さ。太さ `t` の線に見える長さ (t × √2 を丸めたもの)
fn diagonal(t: i32) -> i32 {
    ((t * 14 + 5) / 10).max(1)
}

/// 下向きの矢じり: 軸 (左端 `shaft`・太さ `t`) の先 (`tip` の行) から、左右へ `size` 画素ずつ斜めに上がる。
/// 1 行ずつ、軸をはさんで左右同じ位置に打つので、必ず左右対称になる
fn head_down(dc: &PaintDc, shaft: i32, tip: i32, t: i32, size: i32) {
    let d = diagonal(t);
    for step in 1..=size {
        fill(dc, shaft - step, tip - step, d, 1);
        fill(dc, shaft + t + step - d, tip - step, d, 1);
    }
}

/// 左向きの矢じり: 軸 (上端 `shaft`・太さ `t`) の先 (`tip` の列) から、上下へ `size` 画素ずつ斜めに開く
fn head_left(dc: &PaintDc, tip: i32, shaft: i32, t: i32, size: i32) {
    let d = diagonal(t);
    for step in 1..=size {
        fill(dc, tip + step, shaft - step, 1, d);
        fill(dc, tip + step, shaft + t + step - d, 1, d);
    }
}

/// ファイルの終わり: 記号の色で塗った角の丸い札に、太字の `EOF` を地の色で抜く
fn draw_eof(
    dc: &PaintDc,
    stc: &StyledTextCtrl,
    settings: &Settings,
    m: &Metrics,
    x: i32,
    y: i32,
    height: i32,
) {
    let colours = crate::editor::colors();
    let rgb = crate::editor::rgb;
    // 本文より少し小さい太字 (ズームにも付いていく)
    let Some(font) = crate::editor::font(settings, stc.get_zoom() - 2, true) else {
        return;
    };
    dc.set_font(&font);
    let (text_width, text_height) = dc.get_text_extent("EOF");
    let pad = (m.cell / 2).max(3);
    let badge_height = text_height.min(height - m.thin * 2).max(4);
    let left = x + m.thin * 3;
    let top = y + (height - badge_height) / 2;
    dc.set_pen(rgb(colours.mark), m.thin, PenStyle::Solid);
    dc.set_brush(rgb(colours.mark), BrushStyle::Solid);
    dc.draw_rounded_rectangle(
        left,
        top,
        text_width + pad * 2,
        badge_height,
        (badge_height / 4).max(2) as f64,
    );
    dc.set_background_mode(BackgroundMode::Transparent);
    dc.set_text_foreground(rgb(colours.bg));
    dc.draw_text("EOF", left + pad, top + (badge_height - text_height) / 2);
    dc.set_brush(rgb(colours.mark), BrushStyle::Transparent);
}
