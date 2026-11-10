//! Typst プレビューのツールバー (拡大縮小とページ送り)
//!
//! ```text
//! [−] [幅に合わせる (87%) ▾] [+]   [←] 2 / 5 [→]
//! ```
//!
//! - 倍率の一覧は「ページ全体」「幅に合わせる」と固定の倍率 ([`ZOOM_PERCENTS`])。合わせるモードの
//!   ときはそのときの実際の倍率を項目名に添える
//! - [−] / [+] と Ctrl+ホイール (`preview.rs`) は固定の倍率を 1 段ずつ動く
//! - ページ番号は見えているページ (画面の上 1/3 の位置にあるページ) を示す
//!
//! ボタンは自前描画 (アクションバーと同じ見た目)、倍率の一覧はネイティブの wxChoice。
//! 状態は持たず、`preview.rs` が [`PreviewBar::show_state`] で表示を合わせる。

use std::cell::RefCell;
use std::rc::Rc;

use wxdragon::dc::{Point as DcPoint, PolygonFillMode};
use wxdragon::prelude::*;

use crate::editor::rgb;
use crate::gutter::drawing_panel;
use crate::i18n::{tr, tr_args};
use crate::theme::{self, fill_round_rect, Dip};

/// ボタンの大きさと部品の間隔 (DIP)
const BTN: i32 = 24;
const GAP: i32 = 2;
const PAD: i32 = 3;

/// 倍率の指定
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Zoom {
    /// ページ全体が見える倍率 (一番大きいページの幅と高さの両方が収まる)
    FitPage,
    /// 幅に合わせる (既定)
    FitWidth,
    /// 固定の倍率 (%)。100% は紙の実寸 (1 pt = 1/72 インチ、画面の拡大率込み)
    Percent(u32),
}

/// 一覧と [−] / [+] で選べる倍率 (%)
pub const ZOOM_PERCENTS: [u32; 10] = [25, 50, 67, 75, 100, 125, 150, 200, 300, 400];

impl Zoom {
    /// 設定ファイルの値 ("fit_page" / "fit_width" / "125") から
    pub fn from_setting(s: &str) -> Zoom {
        match s.trim() {
            "fit_page" => Zoom::FitPage,
            "fit_width" | "" => Zoom::FitWidth,
            other => other
                .trim_end_matches('%')
                .parse::<u32>()
                .ok()
                .filter(|p| (10..=800).contains(p))
                .map(Zoom::Percent)
                .unwrap_or(Zoom::FitWidth),
        }
    }

    pub fn to_setting(self) -> String {
        match self {
            Zoom::FitPage => "fit_page".to_string(),
            Zoom::FitWidth => "fit_width".to_string(),
            Zoom::Percent(p) => p.to_string(),
        }
    }

    /// 1 段大きく (`dir` > 0) / 小さく した倍率。`current` は今の実際の倍率 (%)
    pub fn step(current: u32, dir: i32) -> Zoom {
        let next = if dir > 0 {
            ZOOM_PERCENTS.iter().copied().find(|p| *p > current)
        } else {
            ZOOM_PERCENTS.iter().rev().copied().find(|p| *p < current)
        };
        Zoom::Percent(next.unwrap_or(if dir > 0 {
            ZOOM_PERCENTS[ZOOM_PERCENTS.len() - 1]
        } else {
            ZOOM_PERCENTS[0]
        }))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Glyph {
    ZoomOut,
    ZoomIn,
    Prev,
    Next,
}

/// 自前描画のアイコンボタン (押して離すと `on_click`)
#[derive(Clone)]
struct IconButton {
    panel: Panel,
    state: Rc<RefCell<ButtonState>>,
}

struct ButtonState {
    glyph: Glyph,
    hover: bool,
    pressed: bool,
    enabled: bool,
    on_click: Option<Rc<dyn Fn()>>,
}

impl IconButton {
    fn new(parent: &Panel, glyph: Glyph, tip: &str) -> IconButton {
        let size = parent.from_dip_int(BTN);
        let panel = drawing_panel(parent, Size::new(size, size));
        panel.set_tooltip(tip);
        let button = IconButton {
            panel,
            state: Rc::new(RefCell::new(ButtonState {
                glyph,
                hover: false,
                pressed: false,
                enabled: true,
                on_click: None,
            })),
        };
        button.bind();
        button
    }

    fn on_click(&self, f: impl Fn() + 'static) {
        self.state.borrow_mut().on_click = Some(Rc::new(f));
    }

    fn set_enabled(&self, enabled: bool) {
        let mut st = self.state.borrow_mut();
        if st.enabled != enabled {
            st.enabled = enabled;
            st.pressed = false;
            drop(st);
            self.panel.refresh(false, None);
        }
    }

    fn bind(&self) {
        let panel = self.panel;
        panel.on_paint({
            let b = self.clone();
            move |_| b.paint()
        });
        panel.on_mouse_left_down({
            let b = self.clone();
            move |_| {
                if !b.state.borrow().enabled {
                    return;
                }
                b.state.borrow_mut().pressed = true;
                b.panel.capture_mouse();
                b.panel.refresh(false, None);
            }
        });
        // ダブルクリックも 2 回目の押下として扱う (連打でページを送れるように)
        panel.on_mouse_left_double_click({
            let b = self.clone();
            move |_| {
                if !b.state.borrow().enabled {
                    return;
                }
                b.state.borrow_mut().pressed = true;
                b.panel.capture_mouse();
                b.panel.refresh(false, None);
            }
        });
        panel.on_mouse_left_up({
            let b = self.clone();
            move |event| {
                if b.panel.has_capture() {
                    b.panel.release_mouse();
                }
                let inside = theme::mouse_pos(&event)
                    .map(|p| {
                        let s = b.panel.get_client_size();
                        p.x >= 0 && p.y >= 0 && p.x < s.width && p.y < s.height
                    })
                    .unwrap_or(false);
                let fire = {
                    let mut st = b.state.borrow_mut();
                    let was = st.pressed;
                    st.pressed = false;
                    was && inside && st.enabled
                };
                b.panel.refresh(false, None);
                if fire {
                    let f = b.state.borrow().on_click.clone();
                    if let Some(f) = f {
                        f();
                    }
                }
            }
        });
        panel.on_mouse_enter({
            let b = self.clone();
            move |_| {
                b.state.borrow_mut().hover = true;
                b.panel.refresh(false, None);
            }
        });
        panel.on_mouse_leave({
            let b = self.clone();
            move |_| {
                b.state.borrow_mut().hover = false;
                b.panel.refresh(false, None);
            }
        });
    }

    fn paint(&self) {
        let dc = AutoBufferedPaintDC::new(&self.panel);
        let Size { width, height } = self.panel.get_client_size();
        let dip = Dip::of(&self.panel);
        let ui = theme::ui();
        dc.set_background(rgb(ui.bar_bg));
        dc.clear();
        let st = self.state.borrow();
        if st.enabled && (st.hover || st.pressed) {
            fill_round_rect(
                &dc,
                0,
                0,
                width,
                height,
                dip.pxf(4.0),
                if st.pressed {
                    ui.pressed_bg
                } else {
                    ui.hover_bg
                },
            );
        }
        let fg = if st.enabled { ui.text } else { ui.text_dim };
        paint_glyph(&dc, st.glyph, width / 2, height / 2, dip, fg, ui.bar_bg);
    }
}

/// ボタンの絵 (中心座標)。座標は DIP で書き、`dip` で実ピクセルに直す
fn paint_glyph(dc: &dyn DeviceContext, glyph: Glyph, cx: i32, cy: i32, dip: Dip, fg: u32, bg: u32) {
    let d = |v: i32| dip.px(v);
    let pen = dip.pen();
    dc.set_pen(rgb(fg), pen, PenStyle::Solid);
    match glyph {
        Glyph::ZoomOut | Glyph::ZoomIn => {
            // 虫眼鏡: 円 + 柄、中に − / +
            dc.set_brush(rgb(bg), BrushStyle::Transparent);
            let (ox, oy) = (cx - d(2), cy - d(2));
            dc.draw_circle(ox, oy, d(6));
            dc.set_pen(rgb(fg), (pen * 2).max(2), PenStyle::Solid);
            dc.draw_line(ox + d(4), oy + d(4), cx + d(7), cy + d(7));
            dc.set_pen(rgb(fg), pen, PenStyle::Solid);
            dc.draw_line(ox - d(3), oy, ox + d(4), oy);
            if glyph == Glyph::ZoomIn {
                dc.draw_line(ox, oy - d(3), ox, oy + d(4));
            }
        }
        Glyph::Prev | Glyph::Next => {
            // 矢印: 軸 + 三角の矢じり
            let s = if glyph == Glyph::Prev { -1 } else { 1 };
            dc.draw_line(cx - s * d(7), cy, cx + s * d(3), cy);
            dc.set_brush(rgb(fg), BrushStyle::Solid);
            let pts =
                [(3, -5), (8, 0), (3, 5)].map(|(x, y)| DcPoint::new(cx + s * d(x), cy + d(y)));
            dc.draw_polygon(&pts, 0, 0, PolygonFillMode::OddEven);
        }
    }
}

/// ツールバー本体
#[derive(Clone)]
pub struct PreviewBar {
    pub panel: Panel,
    zoom_out: IconButton,
    zoom_in: IconButton,
    choice: Choice,
    prev: IconButton,
    next: IconButton,
    label: StaticText,
    /// 一覧に今入っている項目 (変わったときだけ入れ直す)
    items: Rc<RefCell<Vec<String>>>,
    /// 一覧に対応する倍率 (項目の順)
    zooms: Rc<Vec<Zoom>>,
}

impl PreviewBar {
    pub fn new(parent: &Panel) -> PreviewBar {
        let panel = Panel::builder(parent).build();
        panel.set_background_color(rgb(theme::ui().bar_bg));
        let zoom_out = IconButton::new(&panel, Glyph::ZoomOut, &tr("preview.zoom_out"));
        let zoom_in = IconButton::new(&panel, Glyph::ZoomIn, &tr("preview.zoom_in"));
        let choice = Choice::builder(&panel).build();
        choice.set_tooltip(&tr("preview.zoom"));
        // 項目名 (「幅に合わせる (87%)」⇔「100%」) で幅が変わってボタンが動かないよう、広めに取っておく
        choice.set_min_size(Size::new(panel.from_dip_int(150), -1));
        let prev = IconButton::new(&panel, Glyph::Prev, &tr("preview.prev_page"));
        let next = IconButton::new(&panel, Glyph::Next, &tr("preview.next_page"));
        let label = StaticText::builder(&panel).with_label("").build();
        // 下の境界線 (ページの灰色との区切り)
        panel.on_paint(move |_| {
            let dc = PaintDC::new(&panel);
            let Size { width, height } = panel.get_client_size();
            dc.set_pen(rgb(theme::ui().border), 1, PenStyle::Solid);
            dc.draw_line(0, height - 1, width, height - 1);
        });

        let px = |v: i32| panel.from_dip_int(v);
        let row = BoxSizer::builder(Orientation::Horizontal).build();
        let center = SizerFlag::AlignCenterVertical;
        row.add(&zoom_out.panel, 0, center | SizerFlag::Left, px(PAD + 1));
        row.add(&choice, 0, center | SizerFlag::Left, px(GAP));
        row.add(&zoom_in.panel, 0, center | SizerFlag::Left, px(GAP));
        row.add_spacer(px(14));
        row.add(&prev.panel, 0, center, 0);
        row.add(
            &label,
            0,
            center | SizerFlag::Left | SizerFlag::Right,
            px(6),
        );
        row.add(&next.panel, 0, center, 0);
        let outer = BoxSizer::builder(Orientation::Vertical).build();
        outer.add_sizer(
            &row,
            0,
            SizerFlag::Expand | SizerFlag::Top | SizerFlag::Bottom,
            px(PAD),
        );
        outer.add_spacer(1);
        panel.set_sizer(outer, true);

        let mut zooms = vec![Zoom::FitPage, Zoom::FitWidth];
        zooms.extend(ZOOM_PERCENTS.iter().map(|p| Zoom::Percent(*p)));
        PreviewBar {
            panel,
            zoom_out,
            zoom_in,
            choice,
            prev,
            next,
            label,
            items: Rc::new(RefCell::new(Vec::new())),
            zooms: Rc::new(zooms),
        }
    }

    /// 倍率の変更 (一覧・[−]・[+])。`f` には新しい指定が渡る。[−] / [+] は今の実際の倍率から 1 段動く
    pub fn on_zoom(&self, current_percent: impl Fn() -> u32 + 'static, f: impl Fn(Zoom) + 'static) {
        let f: Rc<dyn Fn(Zoom)> = Rc::new(f);
        let current: Rc<dyn Fn() -> u32> = Rc::new(current_percent);
        self.zoom_out.on_click({
            let (f, current) = (f.clone(), current.clone());
            move || f(Zoom::step(current(), -1))
        });
        self.zoom_in.on_click({
            let (f, current) = (f.clone(), current.clone());
            move || f(Zoom::step(current(), 1))
        });
        let choice = self.choice;
        let zooms = self.zooms.clone();
        self.choice.on_selection_changed(move |_| {
            if let Some(z) = choice.get_selection().and_then(|i| zooms.get(i as usize)) {
                f(*z);
            }
        });
    }

    /// ページ送り。`f` には -1 (前) / +1 (次) が渡る
    pub fn on_page(&self, f: impl Fn(i32) + 'static) {
        let f: Rc<dyn Fn(i32)> = Rc::new(f);
        self.prev.on_click({
            let f = f.clone();
            move || f(-1)
        });
        self.next.on_click(move || f(1));
    }

    /// 表示を今の状態に合わせる。`page` は 0 始まり、`pages` が 0 なら文書なし
    pub fn show_state(&self, zoom: Zoom, percent: u32, page: usize, pages: usize) {
        // 一覧の項目 (合わせるモードは実際の倍率を添える)
        let items: Vec<String> = self
            .zooms
            .iter()
            .map(|z| match z {
                Zoom::FitPage | Zoom::FitWidth => {
                    let key = if *z == Zoom::FitPage {
                        "preview.fit_page"
                    } else {
                        "preview.fit_width"
                    };
                    if *z == zoom && pages > 0 {
                        tr_args(
                            &format!("{key}_percent"),
                            &[("percent", &percent.to_string())],
                        )
                    } else {
                        tr(key)
                    }
                }
                Zoom::Percent(p) => format!("{p}%"),
            })
            .collect();
        let mut relayout = false;
        if *self.items.borrow() != items {
            self.choice.freeze();
            self.choice.clear();
            for item in &items {
                self.choice.append(item);
            }
            self.choice.thaw();
            *self.items.borrow_mut() = items;
            relayout = true;
        }
        // 固定の倍率が一覧に無い値 (設定ファイルで 110 など) なら何も選ばない
        match self.zooms.iter().position(|z| *z == zoom) {
            Some(i) => {
                if self.choice.get_selection() != Some(i as u32) {
                    self.choice.set_selection(i as u32);
                }
            }
            None => self.choice.set_selection(u32::MAX),
        }
        self.zoom_out
            .set_enabled(pages > 0 && percent > ZOOM_PERCENTS[0]);
        self.zoom_in
            .set_enabled(pages > 0 && percent < ZOOM_PERCENTS[ZOOM_PERCENTS.len() - 1]);
        self.prev.set_enabled(page > 0);
        self.next.set_enabled(page + 1 < pages);
        let text = if pages == 0 {
            String::new()
        } else {
            tr_args(
                "preview.page_of",
                &[
                    ("page", &(page + 1).to_string()),
                    ("pages", &pages.to_string()),
                ],
            )
        };
        if self.label.get_label() != text {
            self.label.set_label(&text);
            relayout = true;
        }
        if relayout {
            self.panel.layout();
        }
    }

    /// 配色が変わったときに背景を合わせる
    pub fn apply_theme(&self) {
        self.panel.set_background_color(rgb(theme::ui().bar_bg));
        self.panel.refresh(true, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_settings_roundtrip() {
        for z in [Zoom::FitPage, Zoom::FitWidth, Zoom::Percent(125)] {
            assert_eq!(Zoom::from_setting(&z.to_setting()), z);
        }
        assert_eq!(Zoom::from_setting("150%"), Zoom::Percent(150));
        assert_eq!(Zoom::from_setting("5000"), Zoom::FitWidth);
        assert_eq!(Zoom::from_setting("nonsense"), Zoom::FitWidth);
    }

    #[test]
    fn zoom_steps_move_between_fixed_percents() {
        assert_eq!(Zoom::step(100, 1), Zoom::Percent(125));
        assert_eq!(Zoom::step(100, -1), Zoom::Percent(75));
        // 合わせるモードの中途半端な倍率からは隣の固定の倍率へ
        assert_eq!(Zoom::step(87, 1), Zoom::Percent(100));
        assert_eq!(Zoom::step(87, -1), Zoom::Percent(75));
        // 端では止まる
        assert_eq!(Zoom::step(400, 1), Zoom::Percent(400));
        assert_eq!(Zoom::step(25, -1), Zoom::Percent(25));
    }
}
