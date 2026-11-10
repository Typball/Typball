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
pub const BG: (u8, u8, u8) = (230, 230, 230);

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
    let merged = typball_core::merge_pages(&out.pages, 10);
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
    dc.set_background(Colour::new(BG.0, BG.1, BG.2, 255));
    dc.clear();
    if let Some(bmp) = st.bitmap.as_ref() {
        let (vx, vy) = st.scrolled.get_view_start();
        let (ux, uy) = st.scrolled.get_scroll_pixels_per_unit();
        let margin = st.scrolled.from_dip_int(MARGIN);
        dc.draw_bitmap(bmp, margin - vx * ux, margin - vy * uy, false);
    }
}
