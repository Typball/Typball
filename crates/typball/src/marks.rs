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
//! - 記号の位置は、できるだけ Scintilla に聞かずに計算する ([`paint`] の「位置の求め方」)。位置を聞く
//!   問い合わせは 1 回が遅く、空白 1 つごとに聞くと空白の多い文書で描画が 0.5 秒かかった。
//!   画面の行の配置は覚えさせておく (`SCI_SETLAYOUTCACHE`)。

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;

use wxdragon::dc::Point as DcPoint;
use wxdragon::prelude::*;

use crate::fileio::LineEnding;
use crate::scintilla;
use crate::settings::Settings;

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

/// 文字の幅 (px) の控え。エディタの既定のスタイルで 1 文字ずつ測ったもの (下の「位置の求め方」)。
/// フォント・ズーム・画面の拡大率が変わったら捨てる
#[derive(Default)]
struct Widths {
    /// (フォント名, サイズ, ズーム, 拡大率)
    key: (String, i32, i32, i32),
    px: HashMap<char, i32>,
}

thread_local! {
    static WIDTHS: RefCell<Widths> = RefCell::new(Widths::default());
}

impl Widths {
    fn of(&mut self, stc: &StyledTextCtrl, c: char) -> i32 {
        *self.px.entry(c).or_insert_with(|| {
            scintilla::text_width(
                stc,
                crate::editor::STYLE_DEFAULT,
                c.encode_utf8(&mut [0; 4]),
            )
        })
    }
}

/// 描く記号 1 つ (空白・タブ・全角空白)
struct Mark {
    kind: u8,
    x: i32,
    width: i32,
}

/// 画面の 1 段 (折り返していない行は 1 行ぶん、折り返した行はその中の 1 段) の記号の位置を、
/// 問い合わせずに計算で求める。合っているかは呼び出し側が確かめる
///
/// `text` は段の中身、`cum` は行の先頭からこの段の先頭までの幅 (タブ位置は行の先頭から数える)。
/// 戻り値は (記号の一覧, 最後の記号の直後までの幅, 段の終わりまでの幅)。段の終わりまでの幅は、
/// 最後の記号より後ろが ASCII だけのときにだけ求める (それ以外は None。漢字を 1 字ずつ測らないため)
fn layout_segment(
    stc: &StyledTextCtrl,
    widths: &mut Widths,
    text: &str,
    cum: &mut i32,
    tab_px: i32,
) -> (Vec<Mark>, i32, Option<i32>) {
    let base = *cum;
    let last_mark = text.rfind([' ', '\t', '\u{3000}']);
    let mut marks = Vec::new();
    let mut after_last = 0;
    for (i, c) in text.char_indices() {
        if last_mark.is_none_or(|last| i > last) {
            break;
        }
        let width = match c {
            // Scintilla のタブ位置: 行の先頭から数えて、タブ幅の倍数のうち今の位置より 2 px 以上先のもの
            '\t' => ((*cum + 2) / tab_px + 1) * tab_px - *cum,
            _ => widths.of(stc, c),
        };
        if matches!(c, ' ' | '\t' | '\u{3000}') {
            marks.push(Mark {
                kind: if c == '\u{3000}' { b'w' } else { c as u8 },
                x: *cum - base,
                width,
            });
        }
        *cum += width;
        after_last = *cum - base;
    }
    // 最後の記号より後ろ
    let tail = last_mark.map_or(text, |last| {
        let len = text[last..].chars().next().map_or(1, char::len_utf8);
        &text[last + len..]
    });
    let to_end = if tail.is_ascii() && !tail.contains('\t') {
        for c in tail.chars() {
            *cum += widths.of(stc, c);
        }
        Some(*cum - base)
    } else {
        None
    };
    (marks, after_last, to_end)
}

/// 描き直された範囲 (`update`) にかかる行の記号を `dc` に描く。エディタが自分を描き終えるたびに呼ぶ
/// ([`on_painted`])。
///
/// IME で入力している途中 (`composing`) は、キャレットのある行の改行と EOF の記号を描かない。
/// 未確定の文字は IME の窓がエディタの上に重ねて出しているだけで、文書にはまだ入っていない。
/// 確定すると先にその窓が消え、少し遅れて文字が入るので、記号を描いたままだと、窓の下に隠れていた
/// 記号が元の位置に一瞬見えてから行末へ動く
///
/// # 位置の求め方
///
/// 文字の画面上の位置を Scintilla に聞く問い合わせ (`SCI_POINTXFROMPOSITION` など) は、1 回ごとに
/// 計測用の描画面を作るので遅い (1 回 0.2 ms 近く。行の配置を覚えさせていても変わらない)。以前は空白
/// 1 つにつき 4 回聞いていて、空白の多い文書では 1 回の描画に 0.5 秒かかった。今は聞く回数を
/// 1 行あたり 1〜2 回に抑える:
///
/// - 行の上端は聞かずに計算する (画面の何段目かは速く分かる)。描き直された範囲の外の行は何も聞かない
/// - 空白・タブの x は、行の先頭から文字の幅を足して求める ([`layout_segment`])。文字の幅は既定の
///   スタイルで 1 文字ずつ測って控えておく (同じ文字は 1 回だけ)
/// - 足した結果が合っているかを、段ごとに 1 か所だけ Scintilla に聞いて確かめる。合わなければ
///   (太字などで幅の違うスタイルが混ざっている、文字が合成されている、など) その段は空白の並びごとに
///   位置を聞く
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

    let mut widths = WIDTHS.take();
    let key = (
        e.font_face.clone(),
        e.font_size,
        stc.get_zoom(),
        stc.from_dip_int(9600),
    );
    if widths.key != key {
        widths.key = key;
        widths.px.clear();
    }
    let cell = widths.of(stc, ' ').max(3);
    let thin = stc.from_dip_int(1).max(1);
    let m = Metrics {
        cell,
        thin,
        head: (cell * 3 / 8).max(2),
    };
    let tab_px = (widths.of(stc, ' ').max(1)) * e.tab_width.clamp(1, 16);
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
    // ペンは変わるときだけ作り直す
    let current_pen = Cell::new((u32::MAX, 0));
    let pen = |colour: u32, width: i32| {
        if current_pen.replace((colour, width)) != (colour, width) {
            dc.set_pen(rgb(colour), width, PenStyle::Solid);
        }
    };

    let (update_top, update_bottom) = (update.y, update.y + update.height);
    // 行間 (設定) で足した高さは記号の大きさに入れない: 文字のある高さの中に描く
    let (extra_top, extra_bottom) = crate::editor::line_extra(stc, settings);
    let composing_line = composing.then(|| stc.get_current_line());
    let row_height = scintilla::text_height(stc, from).max(1);
    // 記号の高さ (行の中の文字のある部分)
    let height = (row_height - extra_top - extra_bottom).max(4);
    // 行の先頭の x (どの行も同じ)。最初に要るときに 1 回だけ聞く
    let origin: Cell<Option<i32>> = Cell::new(None);
    let origin_x = |line_start: i32| {
        origin.get().unwrap_or_else(|| {
            let x = scintilla::point_x_from_position(stc, line_start);
            origin.set(Some(x));
            x
        })
    };
    // スタイルごとの空白の幅 (計算が合わなかった段で使う)
    let mut style_widths: HashMap<(i32, char), i32> = HashMap::new();

    let mut row = scintilla::visible_from_doc_line(stc, from);
    for line in from..=to {
        // 行の上端と段の数は、画面の何段目かから計算する (聞くと遅い)
        let next_row = scintilla::visible_from_doc_line(stc, line + 1);
        let rows = next_row - row;
        let line_top = (row - first) * row_height;
        row = next_row;
        // 折りたたまれて見えない行 / 描き直されなかった行 (前に描いた記号がそのまま残っている)
        if rows <= 0 || line_top + rows * row_height <= update_top || line_top >= update_bottom {
            continue;
        }
        let start = stc.position_from_line(line);
        let end = scintilla::line_end_position(stc, line);
        let eol = line_ending_of(stc, line, count);
        let draw_end = composing_line != Some(line)
            && match eol {
                Some(_) => e.show_eol,
                None => e.show_eof,
            };
        let bytes = if e.show_whitespace && end > start {
            let mut bytes = scintilla::line_bytes(stc, line);
            bytes.truncate((end - start) as usize);
            bytes
        } else {
            Vec::new()
        };
        let text = std::str::from_utf8(&bytes).unwrap_or("");
        let has_marks = text.contains([' ', '\t', '\u{3000}']);
        if !has_marks && !draw_end {
            continue;
        }

        // 段の先頭の位置 (折り返していなければ行の先頭だけ)
        let mut starts = vec![start];
        if rows > 1 && has_marks {
            let x = origin_x(start);
            for k in 1..rows {
                let y = line_top + k * row_height + row_height / 2;
                let pos = scintilla::position_from_point(stc, x, y);
                starts.push(pos.clamp(*starts.last().unwrap_or(&start), end));
            }
        }

        let mut cum = 0;
        let mut end_x = None;
        for (k, seg_start) in starts.iter().copied().enumerate() {
            let seg_end = starts.get(k + 1).copied().unwrap_or(end);
            let last = k + 1 == starts.len();
            let y = line_top + k as i32 * row_height + extra_top;
            let seg = text
                .get((seg_start - start) as usize..(seg_end - start) as usize)
                .unwrap_or("");
            if !seg.contains([' ', '\t', '\u{3000}']) {
                // 記号の無い段: 次の段のタブ位置のために幅だけ進めることはしない (タブが出てきたら
                // 計算が合わず、その段は聞き直しになるだけ)
                continue;
            }
            let x0 = origin_x(start);
            let (marks, after_last, to_end) =
                layout_segment(stc, &mut widths, seg, &mut cum, tab_px);
            // 計算が合っているか 1 か所で確かめる: 段の終わりまで分かっていて最後の段ならそこ
            // (改行の記号の位置にも使う)、そうでなければ最後の記号の直後
            let ok = match to_end {
                Some(width) if last => {
                    let actual = scintilla::point_x_from_position(stc, end);
                    end_x = Some(actual);
                    actual == x0 + width
                }
                _ => {
                    let last_mark = seg.rfind([' ', '\t', '\u{3000}']).unwrap_or(0);
                    let len = seg[last_mark..].chars().next().map_or(1, char::len_utf8);
                    let pos = seg_start + (last_mark + len) as i32;
                    if pos >= seg_end && !last {
                        // 段の終わりの位置は次の段の先頭として返るので、最後の記号の頭で確かめる
                        let width = marks.last().map_or(0, |mark| mark.width);
                        scintilla::point_x_from_position(stc, seg_start + last_mark as i32)
                            == x0 + after_last - width
                    } else {
                        scintilla::point_x_from_position(stc, pos) == x0 + after_last
                    }
                }
            };
            pen(colours.mark, 1);
            if ok {
                for mark in &marks {
                    draw_mark(
                        dc,
                        &m,
                        &pen,
                        colours.mark,
                        mark.kind,
                        x0 + mark.x,
                        y,
                        mark.width,
                        height,
                    );
                }
                continue;
            }
            // 計算が合わなかった段: 空白の並びごとに先頭の位置を聞き、並びの中は幅を足していく
            let mut next_x: Option<i32> = None;
            let mut chars = seg.char_indices().peekable();
            while let Some((i, c)) = chars.next() {
                if !matches!(c, ' ' | '\t' | '\u{3000}') {
                    next_x = None;
                    continue;
                }
                let pos = seg_start + i as i32;
                let x = next_x.unwrap_or_else(|| scintilla::point_x_from_position(stc, pos));
                let after = pos + c.len_utf8() as i32;
                let width = if c == '\t' {
                    // タブの幅は位置で決まるので、次の文字の位置を聞く (段の終わりなら半角 1 つぶん)
                    if after < seg_end || last {
                        (scintilla::point_x_from_position(stc, after) - x).max(1)
                    } else {
                        m.cell
                    }
                } else {
                    let style = scintilla::style_at(stc, pos);
                    *style_widths.entry((style, c)).or_insert_with(|| {
                        scintilla::text_width(stc, style, c.encode_utf8(&mut [0; 4]))
                    })
                };
                draw_mark(
                    dc,
                    &m,
                    &pen,
                    colours.mark,
                    if c == '\u{3000}' { b'w' } else { c as u8 },
                    x,
                    y,
                    width,
                    height,
                );
                next_x = Some(x + width);
            }
        }

        if !draw_end {
            continue;
        }
        // 改行と EOF は行末のすぐ後ろ (折り返した行は最後の段)
        let x = end_x.unwrap_or_else(|| scintilla::point_x_from_position(stc, end));
        let y = line_top + (rows - 1) * row_height + extra_top;
        match eol {
            Some(eol) => {
                // 設定と違う改行は色で強調する。線は同じ細さ: 太いペンは線の左右どちらかに寄って
                // 描かれるので、矢印が左右対称にならない。矢印は太さ 1 のペンで塗って描く (fill)
                let colour = if eol == base {
                    colours.mark
                } else {
                    different_colour(eol)
                };
                pen(colour, 1);
                draw_line_ending(dc, &m, eol, x + m.thin, y, height);
            }
            None => {
                draw_eof(dc, stc, settings, &m, x, y, height);
                // draw_eof はペンを替える
                current_pen.set((u32::MAX, 0));
            }
        }
    }
    dc.destroy_clipping_region();
    WIDTHS.set(widths);
}

/// 空白・タブ・全角空白の記号を 1 つ描く。半角空白は太さ 1 のペンで塗る (fill)。タブと全角空白は線の太さ
#[allow(clippy::too_many_arguments)]
fn draw_mark(
    dc: &PaintDc,
    m: &Metrics,
    pen: &impl Fn(u32, i32),
    colour: u32,
    kind: u8,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    match kind {
        b' ' => {
            pen(colour, 1);
            draw_space(dc, m.thin, x, y, width, height);
        }
        b'\t' => {
            pen(colour, m.thin);
            draw_tab(dc, m, x, y, width, height);
        }
        _ => {
            pen(colour, m.thin);
            draw_wide_space(dc, x, y, width, height);
        }
    }
}

// 描く呼び出しは 1 回ごとに時間がかかる (数 µs。空白は画面に何百個も出る) ので、記号 1 つを
// なるべく少ない呼び出しで描く。折れ線 (`draw_lines`) は最後の点の画素だけ描かない

/// 塗った長方形 (左上 `x`, `y`・幅 `w`・高さ `h`)。太さ 1 のペンで 1 本ずつ引く: 線の端の画素や
/// 太いペンの角の扱いに頼らないので、角まで欠けずに出る (呼ぶ前にペンの太さを 1 にしておくこと)。
/// 縦長なら縦線、横長なら横線で引く (本数の少ない方)
fn fill(dc: &PaintDc, x: i32, y: i32, w: i32, h: i32) {
    if h > w {
        for column in 0..w.max(0) {
            dc.draw_line(x + column, y, x + column, y + h);
        }
    } else {
        for row in 0..h.max(0) {
            dc.draw_line(x, y + row, x + w, y + row);
        }
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
    if t == 1 {
        // 太さ 1 なら折れ線 1 本で同じ形になる: 左の立ち上がり → 底 → 右の立ち上がり
        let right = left + w - 1;
        dc.draw_lines(
            &[
                DcPoint::new(left, bottom - rise),
                DcPoint::new(left, bottom),
                DcPoint::new(right, bottom),
                DcPoint::new(right, bottom - rise - 1),
            ],
            0,
            0,
        );
        return;
    }
    fill(dc, left, bottom - rise, t, rise + t);
    fill(dc, left, bottom, w, t);
    fill(dc, left + w - t, bottom - rise, t, rise + t);
}

/// タブ: 先頭に右向きの矢印 (タブの幅が広くても矢印は 1 文字半まで)。折れ線 1 本で描く:
/// 軸 → 上の矢じり → 先へ戻る → 下の矢じり → 先へ戻る
fn draw_tab(dc: &PaintDc, m: &Metrics, x: i32, y: i32, width: i32, height: i32) {
    let left = x + 1;
    let right = x + width.min(m.cell * 3 / 2).max(m.cell) - 2;
    let mid = y + height * 11 / 20;
    let tip = DcPoint::new(right, mid);
    dc.draw_lines(
        &[
            DcPoint::new(left, mid),
            tip,
            DcPoint::new(right - m.head, mid - m.head),
            tip,
            DcPoint::new(right - m.head, mid + m.head),
            tip,
        ],
        0,
        0,
    );
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
