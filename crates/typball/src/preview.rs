//! プレビュー (Typst をコンパイルしてページ画像を表示する)
//!
//! ```text
//! container (Panel)
//! ├ bar (PreviewBar): [−] [倍率 ▾] [+]  [←] 2 / 5 [→]
//! └ scrolled (ScrolledWindow): ページを縦に並べて描く
//! ```
//!
//! - コンパイル済みの文書 (`typball_core::Document`) を持っておき、倍率やペインの大きさが変わったときは
//!   コンパイルし直さずに並べ直す ([`TypstView::relayout`])
//! - ページは描くときに、見えているものだけラスタライズしてキャッシュする (大きな倍率でも全ページ分の
//!   画像を持たない)。並べ直し・再コンパイルでキャッシュは捨てる
//! - コンパイルに失敗したときは直前に成功した文書を出したままにする (診断はステータスバーとコンソール)
//! - 倍率は「幅に合わせる」「ページ全体」「固定の %」。最後に選んだものを設定 `[preview] typst_zoom` に
//!   覚えて、次に開くタブとプレビューウィンドウで使う

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use wxdragon::prelude::*;
use wxdragon::widgets::scrolled_window::ScrollBarConfig;

use crate::i18n::tr;
use crate::previewbar::{PreviewBar, Zoom};
use crate::settings;
use crate::theme::Dip;

/// ページの周りの余白とページ間のすき間 (DIP)
const MARGIN: i32 = 8;
const PAGE_GAP: i32 = 10;
pub const SCROLL_UNIT: i32 = 16;
/// 1 pt あたりのピクセル数の範囲 (小さすぎる / 大きすぎる画像を作らない)
const MIN_PPP: f32 = 0.05;
const MAX_PPP: f32 = 8.0;
/// ページの周り (とページ間のすき間) の色
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

/// 1 回のコンパイルの結果 (コンソールペインに出す)
pub struct Report {
    pub ok: bool,
    pub pages: usize,
    pub diagnostics: Vec<String>,
    pub elapsed_ms: u128,
}

/// 並べたページ 1 枚分 (座標は仮想領域のピクセル、x は描くときに中央へ寄せる)
struct PageSlot {
    width: i32,
    height: i32,
    top: i32,
    bitmap: Option<Bitmap>,
}

struct State {
    doc: Option<typball_core::Document>,
    zoom: Zoom,
    /// 実際の 1 pt あたりのピクセル数
    ppp: f32,
    pages: Vec<PageSlot>,
    /// ページを並べた領域の大きさ (余白込み)
    content: Size,
    /// 並べたときのクライアント領域 (合わせるモードはこれが変わると並べ直す)
    laid_out_for: Size,
    /// ラスタライズしたときの配色 (変わったらキャッシュを捨てる)
    dark: bool,
}

/// Typst のプレビュー 1 つ (タブのペイン、プレビューウィンドウ)
pub struct TypstView {
    /// ペインの中身として置くパネル (バー + ページ)
    pub container: Panel,
    scrolled: ScrolledWindow,
    bar: PreviewBar,
    frame: Frame,
    state: RefCell<State>,
    /// 見えているページ (0 始まり)。描くたびに求め直し、変わったらバーの表示を直す
    current: Cell<usize>,
}

impl TypstView {
    pub fn new(parent: &Panel, frame: Frame) -> Rc<TypstView> {
        let container = Panel::builder(parent).build();
        let bar = PreviewBar::new(&container);
        let scrolled = ScrolledWindow::builder(&container).build();
        scrolled.set_scroll_rate(SCROLL_UNIT, SCROLL_UNIT);
        scrolled.set_background_color(bg());
        scrolled.set_background_style(BackgroundStyle::Paint);
        let sizer = BoxSizer::builder(Orientation::Vertical).build();
        sizer.add(&bar.panel, 0, SizerFlag::Expand, 0);
        sizer.add(&scrolled, 1, SizerFlag::Expand, 0);
        container.set_sizer(sizer, true);

        let view = Rc::new(TypstView {
            container,
            scrolled,
            bar,
            frame,
            state: RefCell::new(State {
                doc: None,
                zoom: Zoom::from_setting(&settings::get().preview.typst_zoom),
                ppp: 1.0,
                pages: Vec::new(),
                content: Size::new(0, 0),
                laid_out_for: Size::new(-1, -1),
                dark: false,
            }),
            current: Cell::new(0),
        });
        view.bind();
        view.update_bar();
        view
    }

    fn bind(self: &Rc<Self>) {
        let weak: Weak<Self> = Rc::downgrade(self);
        let scrolled = self.scrolled;
        scrolled.on_paint({
            let weak = weak.clone();
            move |_| {
                if let Some(v) = weak.upgrade() {
                    v.paint();
                }
            }
        });
        scrolled.on_erase_background(|_| {});
        // 合わせるモードは大きさが変わると倍率が変わるので並べ直す (コンパイルはしない)
        scrolled.on_size({
            let weak = weak.clone();
            move |event| {
                event.skip(true);
                if let Some(v) = weak.upgrade() {
                    v.on_resized();
                }
            }
        });
        // Ctrl+ホイールで拡大縮小
        scrolled.on_mouse_wheel({
            let weak = weak.clone();
            move |event| {
                let WindowEventData::MouseButton(e) = &event else {
                    event.skip(true);
                    return;
                };
                if !ctrl_down() {
                    event.skip(true);
                    return;
                }
                if let Some(v) = weak.upgrade() {
                    let dir = e.event.get_wheel_rotation().signum();
                    if dir != 0 {
                        v.set_zoom(Zoom::step(v.percent(), dir));
                    }
                }
            }
        });
        self.bar.on_zoom(
            {
                let weak = weak.clone();
                move || weak.upgrade().map(|v| v.percent()).unwrap_or(100)
            },
            {
                let weak = weak.clone();
                move |zoom| {
                    if let Some(v) = weak.upgrade() {
                        v.set_zoom(zoom);
                        v.scrolled.set_focus();
                    }
                }
            },
        );
        self.bar.on_page(move |delta| {
            if let Some(v) = weak.upgrade() {
                let target = v.current.get() as i64 + delta as i64;
                v.go_to_page(target.max(0) as usize);
            }
        });
    }

    /// ソースをコンパイルして表示し、ステータスバーに診断を出す。
    /// `path` は文書の保存先 (未保存なら None)。画像などはそのフォルダを基準に読む
    pub fn render(&self, source: &str, path: Option<&std::path::Path>) -> Report {
        let started = std::time::Instant::now();
        let (doc, diagnostics) = typball_core::compile_doc(source, path);
        let elapsed_ms = started.elapsed().as_millis();
        let ok = doc.is_some();
        let pages = doc.as_ref().map(|d| d.page_count()).unwrap_or(0);
        if let Some(doc) = doc {
            self.state.borrow_mut().doc = Some(doc);
            // 同じ倍率なら位置はピクセルのまま保つ (入力のたびに跳ばない)
            self.relayout(false);
        }
        let msg = if diagnostics.is_empty() {
            tr("status.ok")
        } else {
            diagnostics.join(" | ")
        };
        self.frame.set_status_text(&msg, 0);
        Report {
            ok,
            pages,
            diagnostics,
            elapsed_ms,
        }
    }

    /// 配色が変わった (呼び出し側が続けて render する)
    pub fn apply_theme(&self) {
        self.scrolled.set_background_color(bg());
        self.bar.apply_theme();
        self.relayout(false);
    }

    /// 今の実際の倍率 (%)
    fn percent(&self) -> u32 {
        let ppp = self.state.borrow().ppp;
        (ppp / self.ppp_at_100() * 100.0).round().max(1.0) as u32
    }

    /// 100% (実寸) のときの 1 pt あたりのピクセル数: 96 DPI × 画面の拡大率 / 72
    fn ppp_at_100(&self) -> f32 {
        (Dip::of(&self.scrolled).pxf(96.0) / 72.0) as f32
    }

    fn set_zoom(&self, zoom: Zoom) {
        if self.state.borrow().zoom == zoom {
            return;
        }
        self.state.borrow_mut().zoom = zoom;
        self.relayout(true);
        // ページ全体はページの切れ目で止まると意味が無いので、見ていたページの頭に合わせる
        if zoom == Zoom::FitPage {
            self.go_to_page(self.current.get());
        }
        // 次に開くプレビューも同じ倍率で
        let mut s = settings::get();
        let value = zoom.to_setting();
        if s.preview.typst_zoom != value {
            s.preview.typst_zoom = value;
            settings::set(s.clone());
            let _ = s.save();
        }
    }

    fn on_resized(&self) {
        let client = self.scrolled.get_client_size();
        let (zoom, laid_out) = {
            let st = self.state.borrow();
            (st.zoom, st.laid_out_for)
        };
        let changed = match zoom {
            Zoom::FitWidth => client.width != laid_out.width,
            Zoom::FitPage => client != laid_out,
            Zoom::Percent(_) => false,
        };
        if changed {
            self.relayout(false);
        } else {
            self.scrolled.refresh(false, None);
        }
    }

    /// 倍率を決めてページを並べ直し、スクロール範囲を設定し直す。`keep_relative` なら倍率が変わっても
    /// 画面の中央にあった位置 (ページ番号とページ内の割合) を保つ
    fn relayout(&self, keep_relative: bool) {
        let client = self.scrolled.get_client_size();
        let dip = Dip::of(&self.scrolled);
        let margin = dip.px(MARGIN);
        let gap = dip.px(PAGE_GAP);
        let ppp_100 = self.ppp_at_100();
        let (vx, vy) = self.view_start_px();
        let anchor = if keep_relative {
            self.anchor_at(vy + client.height / 2)
        } else {
            None
        };

        let mut st = self.state.borrow_mut();
        let sizes = st
            .doc
            .as_ref()
            .map(|d| d.page_sizes_pt())
            .unwrap_or_default();
        let widest = sizes.iter().map(|s| s.0).fold(0.0_f64, f64::max).max(1.0);
        let tallest = sizes.iter().map(|s| s.1).fold(0.0_f64, f64::max).max(1.0);
        let avail_w = (client.width - margin * 2).max(40) as f64;
        let avail_h = (client.height - margin * 2).max(40) as f64;
        let ppp = match st.zoom {
            Zoom::FitWidth => (avail_w / widest) as f32,
            Zoom::FitPage => (avail_w / widest).min(avail_h / tallest) as f32,
            Zoom::Percent(p) => p as f32 / 100.0 * ppp_100,
        }
        .clamp(MIN_PPP, MAX_PPP);
        let fit = matches!(st.zoom, Zoom::FitWidth | Zoom::FitPage);

        let mut top = margin;
        let mut max_w = 0;
        let pages: Vec<PageSlot> = sizes
            .iter()
            .map(|(w, h)| {
                // 合わせるモードは切り捨てて、横スクロールバーが出ないようにする
                let width = if fit {
                    (w * ppp as f64).floor() as i32
                } else {
                    (w * ppp as f64).round() as i32
                }
                .max(1);
                let height = ((h * ppp as f64).round() as i32).max(1);
                let slot = PageSlot {
                    width,
                    height,
                    top,
                    bitmap: None,
                };
                top += height + gap;
                max_w = max_w.max(width);
                slot
            })
            .collect();
        let content = if pages.is_empty() {
            Size::new(0, 0)
        } else {
            Size::new(max_w + margin * 2, top - gap + margin)
        };
        st.ppp = ppp;
        st.pages = pages;
        st.content = content;
        st.laid_out_for = client;
        st.dark = crate::theme::preview_dark();
        drop(st);

        // スクロール範囲。合わせるモードの幅は切り捨てて、クライアント幅を超えないようにする
        let units = |px: i32, round_up: bool| {
            if round_up {
                (px + SCROLL_UNIT - 1) / SCROLL_UNIT
            } else {
                px / SCROLL_UNIT
            }
        };
        let (new_x, new_y) = match anchor {
            Some((page, frac)) => {
                let st = self.state.borrow();
                match st.pages.get(page) {
                    Some(p) => {
                        let y = p.top + (p.height as f64 * frac) as i32 - client.height / 2;
                        let x = (content.width - client.width).max(0) / 2;
                        (x, y.max(0))
                    }
                    None => (vx, vy),
                }
            }
            None => (vx, vy),
        };
        self.scrolled.set_scrollbars(ScrollBarConfig {
            pixels_per_unit_x: SCROLL_UNIT,
            pixels_per_unit_y: SCROLL_UNIT,
            no_units_x: units(content.width, !fit),
            no_units_y: units(content.height, true),
            x_pos: new_x / SCROLL_UNIT,
            y_pos: new_y / SCROLL_UNIT,
            no_refresh: true,
        });
        self.scrolled.refresh(false, None);
        self.update_bar();
    }

    /// 仮想領域の y にあるページと、そのページ内の割合 (0..1)
    fn anchor_at(&self, y: i32) -> Option<(usize, f64)> {
        let st = self.state.borrow();
        let i = st.pages.iter().rposition(|p| p.top <= y).unwrap_or(0);
        let p = st.pages.get(i)?;
        Some((i, ((y - p.top) as f64 / p.height as f64).clamp(0.0, 1.0)))
    }

    fn view_start_px(&self) -> (i32, i32) {
        let (vx, vy) = self.scrolled.get_view_start();
        let (ux, uy) = self.scrolled.get_scroll_pixels_per_unit();
        (vx * ux.max(1), vy * uy.max(1))
    }

    /// ページの先頭を画面の上端へ
    fn go_to_page(&self, index: usize) {
        let top = {
            let st = self.state.borrow();
            let Some(p) = st.pages.get(index.min(st.pages.len().saturating_sub(1))) else {
                return;
            };
            p.top - Dip::of(&self.scrolled).px(MARGIN)
        };
        let (vx, _) = self.scrolled.get_view_start();
        // 単位 (16 px) に丸めると前のページの末尾が少し見えるので、切り上げてページの頭に揃える
        self.scrolled
            .scroll_coords(vx, (top.max(0) + SCROLL_UNIT - 1) / SCROLL_UNIT);
        self.current.set(index);
        self.scrolled.refresh(false, None);
        self.update_bar();
    }

    fn update_bar(&self) {
        let (zoom, pages) = {
            let st = self.state.borrow();
            (st.zoom, st.pages.len())
        };
        let page = self.current.get().min(pages.saturating_sub(1));
        self.bar.show_state(zoom, self.percent(), page, pages);
    }

    /// paint: 見えているページだけ (無ければラスタライズして) 描く
    fn paint(&self) {
        let dc = AutoBufferedPaintDC::new(&self.scrolled);
        dc.set_background(bg());
        dc.clear();
        let client = self.scrolled.get_client_size();
        let (vx, vy) = self.view_start_px();
        let dark = crate::theme::preview_dark();
        let current = {
            let mut st = self.state.borrow_mut();
            if st.dark != dark {
                st.dark = dark;
                for p in &mut st.pages {
                    p.bitmap = None;
                }
            }
            let total_w = st.content.width.max(client.width);
            let ppp = st.ppp;
            let probe = vy + client.height / 3;
            let mut current = 0;
            let State { doc, pages, .. } = &mut *st;
            for (i, p) in pages.iter_mut().enumerate() {
                if p.top <= probe {
                    current = i;
                }
                let visible = p.top + p.height >= vy && p.top <= vy + client.height;
                if !visible {
                    // 画面から 2 画面以上離れたページの画像は捨てる (メモリを抑える)
                    let far =
                        p.top + p.height < vy - client.height * 2 || p.top > vy + client.height * 3;
                    if far {
                        p.bitmap = None;
                    }
                    continue;
                }
                if p.bitmap.is_none() {
                    p.bitmap = doc
                        .as_ref()
                        .and_then(|d| d.rasterize_page(i, ppp))
                        .and_then(|mut img| {
                            if dark {
                                darken(&mut img.rgba);
                            }
                            Bitmap::from_rgba(&img.rgba, img.width, img.height)
                        });
                }
                if let Some(bmp) = &p.bitmap {
                    let x = (total_w - p.width) / 2;
                    dc.draw_bitmap(bmp, x - vx, p.top - vy, false);
                }
            }
            current
        };
        if current != self.current.get() {
            self.current.set(current);
            self.update_bar();
        }
    }
}

/// Ctrl キーが押されているか (wxDragon のマウスイベントからは修飾キーが取れないので OS に聞く)
#[cfg(windows)]
fn ctrl_down() -> bool {
    #[link(name = "user32")]
    extern "system" {
        fn GetKeyState(vk: i32) -> i16;
    }
    const VK_CONTROL: i32 = 0x11;
    unsafe { GetKeyState(VK_CONTROL) < 0 }
}

#[cfg(not(windows))]
fn ctrl_down() -> bool {
    false
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
