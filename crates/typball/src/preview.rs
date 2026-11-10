//! プレビュー (Typst をコンパイルしてページ画像を表示する)

use std::cell::RefCell;
use std::rc::Rc;

use wxdragon::prelude::*;
use wxdragon::widgets::scrolled_window::ScrollBarConfig;

use crate::i18n::tr;

const MAX_PPP: f32 = 1.5;
/// ビットマップの周りの余白 (DIP)
const MARGIN: i32 = 8;
pub const SCROLL_UNIT: i32 = 16;
/// ページの周り (とページ間のすき間) の色。typball_core::merge_pages のすき間と同じ
const BG_LIGHT: u8 = 230;

/// プレビューの地の色 (暗いときはすき間を反転した色)
pub fn bg() -> Colour {
    let v = if crate::theme::preview_dark() {
        darken_level(BG_LIGHT, BG_LIGHT, BG_LIGHT)
    } else {
        BG_LIGHT
    };
    Colour::new(v, v, v, 255)
}

/// 暗いときの明るさの範囲: 白い紙 → DARK_LO、黒い文字 → DARK_HI
const DARK_LO: f32 = 30.0;
const DARK_HI: f32 = 222.0;

/// 色相と彩度を保ったまま明るさ (HSL の L) を反転し、DARK_LO..DARK_HI に収める。
/// 白い紙は暗い灰色に、黒い文字は明るい灰色に、色付きの見出しやリンクは同じ色相のまま明るさだけ変わる
/// (画像も同じように変わる)。L は max+min だけで決まるので、その値ごとのずらし量を表にしておく
fn darken_table() -> &'static [i16; 511] {
    static TABLE: std::sync::OnceLock<[i16; 511]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0i16; 511];
        for (sum, shift) in t.iter_mut().enumerate() {
            let l = sum as f32 / 2.0;
            let target = DARK_LO + (DARK_HI - DARK_LO) * (255.0 - l) / 255.0;
            *shift = (target - l).round() as i16;
        }
        t
    })
}

fn darken_level(r: u8, g: u8, b: u8) -> u8 {
    let sum = r.max(g).max(b) as usize + r.min(g).min(b) as usize;
    (r as i16 + darken_table()[sum]).clamp(0, 255) as u8
}

/// RGBA の画像を暗い配色にする (その場で書き換える)
fn darken(rgba: &mut [u8]) {
    let table = darken_table();
    for px in rgba.chunks_exact_mut(4) {
        let (r, g, b) = (px[0], px[1], px[2]);
        let shift = table[r.max(g).max(b) as usize + r.min(g).min(b) as usize];
        px[0] = (r as i16 + shift).clamp(0, 255) as u8;
        px[1] = (g as i16 + shift).clamp(0, 255) as u8;
        px[2] = (b as i16 + shift).clamp(0, 255) as u8;
    }
}

/// プレビュー 1 枚分の状態。ビットマップは paint ハンドラから参照される。
pub struct Preview {
    pub scrolled: ScrolledWindow,
    pub bitmap: Option<Bitmap>,
    pub size: (i32, i32),
    /// 前回コンパイルしたときのクライアント幅 (幅が変わったときだけ組み直す)
    pub rendered_width: i32,
    pub frame: Frame,
}

/// 1 回のコンパイルの結果 (コンソールペインに出す)
pub struct Report {
    pub ok: bool,
    pub pages: usize,
    pub diagnostics: Vec<String>,
    pub elapsed_ms: u128,
}

/// プレビューを再コンパイルし、ステータスバーに診断メッセージを出す。
/// ScrolledWindow に BackgroundStyle::Paint を設定し、paint イベントで
/// AutoBufferedPaintDC (ダブルバッファ) に描くことでチラつきを抑える。
pub fn render_preview(source: &str, state: &Rc<RefCell<Preview>>) -> Report {
    let (scrolled, frame) = {
        let st = state.borrow();
        (st.scrolled.clone(), st.frame.clone())
    };
    // レイアウト前はクライアント幅が 0 近辺なので最低幅を保証する
    let client_width = scrolled.get_client_size().width;
    let margin = scrolled.from_dip_int(MARGIN);
    let avail = (client_width - margin * 3).max(480) as u32;
    let started = std::time::Instant::now();
    let out = typball_core::compile_fit(source, avail, MAX_PPP);
    let elapsed_ms = started.elapsed().as_millis();
    let mut merged = typball_core::merge_pages(&out.pages, 10);
    if crate::theme::preview_dark() {
        darken(&mut merged.rgba);
    }
    let new_size = (merged.width as i32, merged.height as i32);

    let size_changed = {
        let mut st = state.borrow_mut();
        st.bitmap = Bitmap::from_rgba(&merged.rgba, merged.width, merged.height);
        let changed = st.size != new_size;
        st.size = new_size;
        st.rendered_width = client_width;
        changed
    };

    // 仮想サイズが変わったときだけスクロールバーを設定し直す (スクロール位置は維持)
    if size_changed {
        let (vx, vy) = scrolled.get_view_start();
        scrolled.set_scrollbars(ScrollBarConfig {
            pixels_per_unit_x: SCROLL_UNIT,
            pixels_per_unit_y: SCROLL_UNIT,
            no_units_x: (new_size.0 + margin * 2) / SCROLL_UNIT + 1,
            no_units_y: (new_size.1 + margin * 2) / SCROLL_UNIT + 1,
            x_pos: vx,
            y_pos: vy,
            no_refresh: true,
        });
    }
    scrolled.refresh(false, None);

    let msg = if out.diagnostics.is_empty() {
        tr("status.ok")
    } else {
        out.diagnostics.join(" | ")
    };
    frame.set_status_text(&msg, 0);
    Report {
        ok: out.ok,
        pages: out.pages.len(),
        diagnostics: out.diagnostics,
        elapsed_ms,
    }
}

/// paint イベント: ダブルバッファに背景とビットマップを描く
pub fn paint_preview(state: &Rc<RefCell<Preview>>) {
    let st = state.borrow();
    let dc = AutoBufferedPaintDC::new(&st.scrolled);
    dc.set_background(bg());
    dc.clear();
    if let Some(bmp) = st.bitmap.as_ref() {
        let (vx, vy) = st.scrolled.get_view_start();
        let (ux, uy) = st.scrolled.get_scroll_pixels_per_unit();
        let margin = st.scrolled.from_dip_int(MARGIN);
        dc.draw_bitmap(bmp, margin - vx * ux, margin - vy * uy, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn darken_inverts_lightness_and_keeps_hue() {
        let mut px = [255, 255, 255, 255, 0, 0, 0, 255, 200, 40, 40, 255];
        darken(&mut px);
        // 白 → 暗い灰色、黒 → 明るい灰色
        assert_eq!(&px[0..3], &[30, 30, 30]);
        assert_eq!(&px[4..7], &[222, 222, 222]);
        // 赤は赤のまま (R が一番大きく、G と B は等しい)
        assert!(px[8] > px[9] && px[9] == px[10]);
        assert_eq!(px[11], 255);
    }
}
