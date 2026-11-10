//! ツールバー: メニューバーの直下、タブの帯の上に並ぶアイコンのボタン
//!
//! 新規・開く・保存、元に戻す・やり直し、切り取り・コピー・貼り付け、検索・置換。
//! ボタンが押されたらメニューと同じ処理 (`window.rs` の `handle_menu`) へ渡す。
//! 新規は右側の ▼ で形式を選べる (メニューの「形式を選んで新規」と同じ)。
//! 有効 / 無効はウィンドウのアイドル時に `set_enabled` で合わせる (編集項目はフォーカス先で変わる)。
//! 表示するかどうかは設定 `[general] show_toolbar` (表示メニューと設定ダイアログの「表示」)。
//! アクションバー (`actionbar.rs`) と同じく自前描画で、寸法は DIP で書く。

use std::cell::RefCell;
use std::rc::Rc;

use wxdragon::dc::{Point as DcPoint, PolygonFillMode};
use wxdragon::prelude::*;

use crate::editor::rgb;
use crate::gutter::drawing_panel;
use crate::i18n::tr;
use crate::theme::{self, contains, fill_round_rect, mouse_pos, Dip};

// 寸法は DIP (拡大率 100% 換算)。使うときに `Dip::px` で実ピクセルに直す
const BTN: i32 = 24;
/// ▼ の部分の幅
const ARROW: i32 = 12;
const GAP: i32 = 1;
/// 区切り線の前後の余白を含めた幅
const SEP: i32 = 9;
const PAD: i32 = 4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToolItem {
    New,
    Open,
    Save,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Find,
    Replace,
}

impl ToolItem {
    /// 並び順
    pub const ALL: [ToolItem; 10] = [
        Self::New,
        Self::Open,
        Self::Save,
        Self::Undo,
        Self::Redo,
        Self::Cut,
        Self::Copy,
        Self::Paste,
        Self::Find,
        Self::Replace,
    ];

    /// 前に区切り線を入れるか
    fn separator_before(self) -> bool {
        matches!(self, Self::Undo | Self::Cut | Self::Find)
    }

    /// 右側に ▼ (選択肢のメニュー) があるか
    fn has_dropdown(self) -> bool {
        self == Self::New
    }

    /// ツールチップのロケールキー
    fn tip_key(self) -> &'static str {
        match self {
            Self::New => "toolbar.new",
            Self::Open => "toolbar.open",
            Self::Save => "toolbar.save",
            Self::Undo => "toolbar.undo",
            Self::Redo => "toolbar.redo",
            Self::Cut => "toolbar.cut",
            Self::Copy => "toolbar.copy",
            Self::Paste => "toolbar.paste",
            Self::Find => "toolbar.find",
            Self::Replace => "toolbar.replace",
        }
    }
}

/// マウスの乗っている (押された) 場所: (ボタンの添字, ▼ の部分か)
type Spot = (usize, bool);

struct State {
    enabled: [bool; ToolItem::ALL.len()],
    hover: Option<Spot>,
    pressed: Option<Spot>,
    on_click: Option<Rc<dyn Fn(ToolItem)>>,
    /// ▼ が押された (位置はメニューを出す画面座標 = ボタンの左下)
    on_dropdown: Option<Rc<dyn Fn(ToolItem, Point)>>,
}

#[derive(Clone)]
pub struct ToolBar {
    panel: Panel,
    state: Rc<RefCell<State>>,
}

impl ToolBar {
    pub fn new(parent: &Panel) -> ToolBar {
        let panel = drawing_panel(parent, Size::new(-1, parent.from_dip_int(BTN + PAD + 2)));
        // クリックでフォーカスを取らない (押下のハンドラでも skip(false) する。下の bind を参照)
        panel.set_can_focus(false);
        let bar = ToolBar {
            panel,
            state: Rc::new(RefCell::new(State {
                enabled: [true; ToolItem::ALL.len()],
                hover: None,
                pressed: None,
                on_click: None,
                on_dropdown: None,
            })),
        };
        bar.bind();
        bar
    }

    pub fn window(&self) -> &Panel {
        &self.panel
    }

    pub fn on_click(&self, f: impl Fn(ToolItem) + 'static) {
        self.state.borrow_mut().on_click = Some(Rc::new(f));
    }

    pub fn on_dropdown(&self, f: impl Fn(ToolItem, Point) + 'static) {
        self.state.borrow_mut().on_dropdown = Some(Rc::new(f));
    }

    /// 有効 / 無効を合わせる。変わったときだけ描き直す (アイドルのたびに呼ばれる)
    pub fn set_enabled(&self, enabled: impl Fn(ToolItem) -> bool) {
        if !self.panel.is_shown() {
            return;
        }
        let Ok(mut st) = self.state.try_borrow_mut() else {
            return;
        };
        let now = ToolItem::ALL.map(enabled);
        if now != st.enabled {
            st.enabled = now;
            drop(st);
            self.panel.refresh(false, None);
        }
    }

    /// ボタンの矩形 (▼ を含む)。`ToolItem::ALL` の順
    fn rects(dip: Dip) -> Vec<Rect> {
        let d = |v: i32| dip.px(v);
        let mut x = PAD;
        let mut out = Vec::with_capacity(ToolItem::ALL.len());
        for item in ToolItem::ALL {
            if item.separator_before() {
                x += SEP - GAP;
            }
            let w = BTN + if item.has_dropdown() { ARROW } else { 0 };
            out.push(Rect::new(d(x), d(PAD / 2 + 1), d(w), d(BTN)));
            x += w + GAP;
        }
        out
    }

    fn hit_test(&self, p: Point) -> Option<Spot> {
        let dip = Dip::of(&self.panel);
        let rects = Self::rects(dip);
        let index = rects.iter().position(|r| contains(r, p))?;
        let arrow = ToolItem::ALL[index].has_dropdown() && p.x >= rects[index].x + dip.px(BTN);
        Some((index, arrow))
    }

    fn bind(&self) {
        let panel = self.panel;
        panel.on_paint({
            let bar = self.clone();
            move |_| bar.paint()
        });
        panel.on_mouse_left_down({
            let bar = self.clone();
            move |event| {
                // 処理済みにする。wxDragon はハンドラの前に Skip(true) するので、そのままだと
                // wxMSW が「未処理の左クリック」としてこのパネルに SetFocus し、エディタから
                // フォーカスが外れる (編集ボタンの相手がプレビューに変わり、元に戻すが効かない)
                event.skip(false);
                let hit = mouse_pos(&event)
                    .and_then(|p| bar.hit_test(p))
                    .filter(|(i, _)| bar.state.borrow().enabled[*i]);
                // ▼ は押した時点でメニューを出す (メニューが閉じるまで戻らない)
                if let Some((index, true)) = hit {
                    let dip = Dip::of(&bar.panel);
                    let r = Self::rects(dip)[index];
                    let pos = bar.panel.client_to_screen(Point::new(r.x, r.y + r.height));
                    let f = {
                        let mut st = bar.state.borrow_mut();
                        st.pressed = hit;
                        st.on_dropdown.clone()
                    };
                    bar.panel.refresh(false, None);
                    bar.panel.update();
                    if let Some(f) = f {
                        f(ToolItem::ALL[index], pos);
                    }
                    let mut st = bar.state.borrow_mut();
                    st.pressed = None;
                    st.hover = None;
                    drop(st);
                    bar.panel.refresh(false, None);
                    return;
                }
                bar.state.borrow_mut().pressed = hit;
                if hit.is_some() {
                    bar.panel.capture_mouse();
                }
                bar.panel.refresh(false, None);
            }
        });
        panel.on_mouse_motion({
            let bar = self.clone();
            move |event| {
                let Some(p) = mouse_pos(&event) else {
                    return;
                };
                let hover = bar.hit_test(p);
                let mut st = bar.state.borrow_mut();
                if st.hover != hover {
                    let tip_changed = st.hover.map(|h| h.0) != hover.map(|h| h.0);
                    st.hover = hover;
                    drop(st);
                    if tip_changed {
                        let tip = hover
                            .map(|(i, _)| tr(ToolItem::ALL[i].tip_key()))
                            .unwrap_or_default();
                        bar.panel.set_tooltip(&tip);
                    }
                    bar.panel.refresh(false, None);
                }
            }
        });
        panel.on_mouse_left_up({
            let bar = self.clone();
            move |event| {
                if bar.panel.has_capture() {
                    bar.panel.release_mouse();
                }
                let hit = mouse_pos(&event).and_then(|p| bar.hit_test(p));
                let item = {
                    let mut st = bar.state.borrow_mut();
                    let pressed = st.pressed.take();
                    match (pressed, hit) {
                        (Some((a, false)), Some((b, false))) if a == b && st.enabled[a] => {
                            Some(ToolItem::ALL[a])
                        }
                        _ => None,
                    }
                };
                bar.panel.refresh(false, None);
                if let Some(item) = item {
                    let f = bar.state.borrow().on_click.clone();
                    if let Some(f) = f {
                        f(item);
                    }
                }
            }
        });
        panel.on_mouse_leave({
            let bar = self.clone();
            move |_| {
                bar.state.borrow_mut().hover = None;
                bar.panel.refresh(false, None);
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
        dc.set_pen(rgb(ui.border), 1, PenStyle::Solid);
        dc.draw_line(0, height - 1, width, height - 1);

        let st = self.state.borrow();
        let rects = Self::rects(dip);
        for (i, (item, r)) in ToolItem::ALL.iter().zip(&rects).enumerate() {
            let enabled = st.enabled[i];
            if item.separator_before() {
                let x = r.x - dip.px((SEP - GAP) / 2 + GAP);
                dc.set_pen(rgb(ui.border), 1, PenStyle::Solid);
                dc.draw_line(x, r.y + dip.px(3), x, r.y + r.height - dip.px(3));
            }
            let hot = enabled && st.hover.is_some_and(|h| h.0 == i);
            let pressed = st.pressed.filter(|p| p.0 == i && (hot || p.1));
            if hot || pressed.is_some() {
                let radius = dip.pxf(4.0);
                if let Some((_, arrow)) = pressed {
                    // 押している部分 (本体 / ▼) だけを濃くする
                    fill_round_rect(&dc, r.x, r.y, r.width, r.height, radius, ui.hover_bg);
                    let (x, w) = if item.has_dropdown() {
                        let split = dip.px(BTN);
                        if arrow {
                            (r.x + split, r.width - split)
                        } else {
                            (r.x, split)
                        }
                    } else {
                        (r.x, r.width)
                    };
                    fill_round_rect(&dc, x, r.y, w, r.height, radius, ui.pressed_bg);
                } else {
                    fill_round_rect(&dc, r.x, r.y, r.width, r.height, radius, ui.hover_bg);
                }
                if item.has_dropdown() {
                    // 本体と ▼ の境目
                    let x = r.x + dip.px(BTN);
                    dc.set_pen(rgb(ui.border), 1, PenStyle::Solid);
                    dc.draw_line(x, r.y + dip.px(3), x, r.y + r.height - dip.px(3));
                }
            }
            let cx = r.x + dip.px(BTN) / 2;
            let cy = r.y + r.height / 2;
            paint_glyph(&dc, *item, cx, cy, dip, enabled);
            if item.has_dropdown() {
                let ax = r.x + dip.px(BTN + ARROW / 2);
                let colour = if enabled { ui.text } else { disabled_colour() };
                dc.set_pen(rgb(colour), 1, PenStyle::Solid);
                dc.set_brush(rgb(colour), BrushStyle::Solid);
                let d = |v: i32| dip.px(v);
                let pts =
                    [(-3, -1), (3, -1), (0, 2)].map(|(x, y)| DcPoint::new(ax + d(x), cy + d(y)));
                dc.draw_polygon(&pts, 0, 0, PolygonFillMode::OddEven);
            }
        }
    }
}

/// 無効なボタンの絵の色 (文字色と背景の間)
fn disabled_colour() -> u32 {
    blend(theme::ui().text, theme::ui().bar_bg, 0.6)
}

/// `a` と `b` を混ぜる (`t` = 0 で a、1 で b)
fn blend(a: u32, b: u32, t: f64) -> u32 {
    let ch = |c: u32, shift: u32| ((c >> shift) & 0xff) as f64;
    let mix = |shift: u32| {
        let v = ch(a, shift) * (1.0 - t) + ch(b, shift) * t;
        (v.round().clamp(0.0, 255.0) as u32) << shift
    };
    mix(16) | mix(8) | mix(0)
}

/// アイコンの色。無効なら全部を灰色にする
struct Palette {
    fg: u32,
    paper: u32,
    edge: u32,
    line: u32,
    folder: u32,
    folder_edge: u32,
    floppy: u32,
    floppy_edge: u32,
    board: u32,
    accent: u32,
}

impl Palette {
    fn new(enabled: bool) -> Palette {
        let ui = theme::ui();
        if !enabled {
            let g = disabled_colour();
            let bg = ui.bar_bg;
            return Palette {
                fg: g,
                paper: bg,
                edge: g,
                line: g,
                folder: bg,
                folder_edge: g,
                floppy: g,
                floppy_edge: g,
                board: g,
                accent: g,
            };
        }
        Palette {
            fg: ui.text,
            paper: ui.icon_paper,
            edge: ui.icon_edge,
            line: ui.icon_line,
            folder: 0xe8b64c,
            folder_edge: 0xa07418,
            floppy: 0x3d72c4,
            floppy_edge: 0x24508f,
            board: 0xa8743a,
            accent: 0x2e8b3a,
        }
    }
}

/// ボタンの絵 (中心座標)。座標は DIP で書き、`dip` で実ピクセルに直す
fn paint_glyph(dc: &dyn DeviceContext, item: ToolItem, cx: i32, cy: i32, dip: Dip, enabled: bool) {
    let d = |v: i32| dip.px(v);
    let pen = dip.pen();
    let c = Palette::new(enabled);
    let pts = |list: &[(i32, i32)]| -> Vec<DcPoint> {
        list.iter()
            .map(|(x, y)| DcPoint::new(cx + d(*x), cy + d(*y)))
            .collect()
    };
    // 右上が折れた紙 (左上 x, y・幅 w・高さ h)
    let paper = |x: i32, y: i32, w: i32, h: i32| {
        dc.set_pen(rgb(c.edge), pen, PenStyle::Solid);
        dc.set_brush(rgb(c.paper), BrushStyle::Solid);
        let fold = 4;
        dc.draw_polygon(
            &pts(&[
                (x, y),
                (x + w - fold, y),
                (x + w, y + fold),
                (x + w, y + h),
                (x, y + h),
            ]),
            0,
            0,
            PolygonFillMode::OddEven,
        );
        dc.draw_lines(
            &pts(&[
                (x + w - fold, y),
                (x + w - fold, y + fold),
                (x + w, y + fold),
            ]),
            0,
            0,
        );
    };
    // 曲がった矢印 (元に戻す = 左向き、やり直し = 右向き)
    let curved_arrow = |dir: i32| {
        dc.set_pen(rgb(c.fg), (pen * 2).max(2), PenStyle::Solid);
        dc.set_brush(rgb(c.fg), BrushStyle::Transparent);
        // 上半分の弧 (中心 (cx, cy+2)、半径 6)
        dc.draw_elliptic_arc(cx - d(6), cy - d(4), d(12), d(12), 0.0, 180.0);
        // 弧の片方の端から下へ伸ばす
        let tail_x = cx + d(6) * dir;
        dc.draw_line(tail_x, cy + d(2), tail_x, cy + d(5));
        // 矢じり (弧のもう一方の端、下向き)
        dc.set_pen(rgb(c.fg), pen, PenStyle::Solid);
        dc.set_brush(rgb(c.fg), BrushStyle::Solid);
        let hx = -6 * dir;
        dc.draw_polygon(
            &pts(&[(hx - 4, 1), (hx + 4, 1), (hx, 6)]),
            0,
            0,
            PolygonFillMode::OddEven,
        );
    };
    match item {
        ToolItem::New => {
            paper(-6, -8, 12, 16);
            // 右下の緑の +
            dc.set_pen(rgb(c.accent), (pen * 2).max(2), PenStyle::Solid);
            dc.draw_line(cx + d(4), cy + d(1), cx + d(4), cy + d(9));
            dc.draw_line(cx, cy + d(5), cx + d(8), cy + d(5));
        }
        ToolItem::Open => {
            // 奥の板 (つまみ付き) と手前に開いた板
            dc.set_pen(rgb(c.folder_edge), pen, PenStyle::Solid);
            dc.set_brush(rgb(c.folder), BrushStyle::Solid);
            dc.draw_polygon(
                &pts(&[(-9, -6), (-4, -6), (-2, -4), (7, -4), (7, 7), (-9, 7)]),
                0,
                0,
                PolygonFillMode::OddEven,
            );
            dc.set_brush(rgb(blend(c.folder, 0xffffff, 0.35)), BrushStyle::Solid);
            dc.draw_polygon(
                &pts(&[(-6, -1), (10, -1), (7, 7), (-9, 7)]),
                0,
                0,
                PolygonFillMode::OddEven,
            );
        }
        ToolItem::Save => {
            // フロッピーディスク: 本体・上のシャッター・下のラベル
            dc.set_pen(rgb(c.floppy_edge), pen, PenStyle::Solid);
            dc.set_brush(rgb(c.floppy), BrushStyle::Solid);
            dc.draw_polygon(
                &pts(&[(-8, -8), (6, -8), (8, -6), (8, 8), (-8, 8)]),
                0,
                0,
                PolygonFillMode::OddEven,
            );
            dc.set_pen(rgb(c.floppy_edge), pen, PenStyle::Solid);
            dc.set_brush(rgb(c.paper), BrushStyle::Solid);
            dc.draw_rectangle(cx - d(5), cy + d(1), d(10), d(7));
            dc.set_brush(rgb(blend(c.floppy, 0xffffff, 0.55)), BrushStyle::Solid);
            dc.draw_rectangle(cx - d(4), cy - d(8), d(8), d(6));
            dc.set_brush(rgb(c.floppy_edge), BrushStyle::Solid);
            dc.draw_rectangle(cx + d(1), cy - d(7), d(2), d(4));
        }
        ToolItem::Undo => curved_arrow(1),
        ToolItem::Redo => curved_arrow(-1),
        ToolItem::Cut => {
            // はさみ: 交差する 2 枚の刃と下の 2 つの輪
            dc.set_pen(rgb(c.fg), pen, PenStyle::Solid);
            dc.draw_line(cx - d(4), cy - d(9), cx + d(3), cy + d(3));
            dc.draw_line(cx + d(4), cy - d(9), cx - d(3), cy + d(3));
            dc.set_brush(rgb(c.fg), BrushStyle::Transparent);
            dc.set_pen(rgb(c.fg), (pen * 2).max(2), PenStyle::Solid);
            dc.draw_circle(cx - d(4), cy + d(5), d(3));
            dc.draw_circle(cx + d(4), cy + d(5), d(3));
        }
        ToolItem::Copy => {
            paper(-8, -8, 10, 12);
            paper(-2, -3, 10, 12);
            dc.set_pen(rgb(c.line), pen, PenStyle::Solid);
            for y in [2, 5] {
                dc.draw_line(cx + d(0), cy + d(y), cx + d(6), cy + d(y));
            }
        }
        ToolItem::Paste => {
            // クリップボードの板と、挟まれた紙
            dc.set_pen(rgb(blend(c.board, 0x000000, 0.3)), pen, PenStyle::Solid);
            dc.set_brush(rgb(c.board), BrushStyle::Solid);
            dc.draw_rounded_rectangle(cx - d(8), cy - d(7), d(13), d(16), dip.pxf(2.0));
            dc.set_brush(rgb(c.fg), BrushStyle::Solid);
            dc.set_pen(rgb(c.fg), pen, PenStyle::Solid);
            dc.draw_rectangle(cx - d(4), cy - d(9), d(5), d(3));
            paper(-1, -2, 9, 11);
        }
        ToolItem::Find | ToolItem::Replace => {
            // 虫眼鏡 (置換は小さくして、右上に入れ替えの矢印)
            let (lx, ly, r) = if item == ToolItem::Find {
                (-2, -2, 6)
            } else {
                (-4, 0, 5)
            };
            dc.set_pen(rgb(c.fg), (pen * 2).max(2), PenStyle::Solid);
            dc.set_brush(rgb(c.paper), BrushStyle::Solid);
            dc.draw_circle(cx + d(lx), cy + d(ly), d(r));
            dc.set_pen(rgb(c.fg), (pen * 3).max(3), PenStyle::Solid);
            let k = r * 7 / 10;
            dc.draw_line(
                cx + d(lx + k + 1),
                cy + d(ly + k + 1),
                cx + d(lx + k + 5),
                cy + d(ly + k + 5),
            );
            if item == ToolItem::Replace {
                // 右上: 右向きの矢印 (下) と左向きの矢印 (上)
                dc.set_pen(rgb(c.accent), pen, PenStyle::Solid);
                dc.set_brush(rgb(c.accent), BrushStyle::Solid);
                dc.draw_line(cx + d(1), cy - d(7), cx + d(9), cy - d(7));
                dc.draw_polygon(
                    &pts(&[(1, -7), (4, -10), (4, -4)]),
                    0,
                    0,
                    PolygonFillMode::OddEven,
                );
                dc.draw_line(cx + d(1), cy - d(1), cx + d(9), cy - d(1));
                dc.draw_polygon(
                    &pts(&[(9, -1), (6, -4), (6, 2)]),
                    0,
                    0,
                    PolygonFillMode::OddEven,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_mixes_each_channel() {
        assert_eq!(blend(0x000000, 0xffffff, 0.0), 0x000000);
        assert_eq!(blend(0x000000, 0xffffff, 1.0), 0xffffff);
        assert_eq!(blend(0x204060, 0x4080c0, 0.5), 0x3060_90);
    }
}
