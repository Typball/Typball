//! サイドペイン (アウトライン・プレビュー) の部品
//!
//! - [`PaneHeader`]: ペイン上部のヘッダ。タイトルと「配置メニュー」「切り離す / 戻す」「折りたたむ」の
//!   3 つのボタン。ヘッダをつかんでドラッグすると、動いている間は [`PaneHeader::on_drag_move`] に、
//!   離したときは [`PaneHeader::on_drop`] に画面座標を渡す。行き先 (左 / 右 / 下の置き場、既にある
//!   ペインの下に並べる、ウィンドウの外なら切り離す) を決めて [`DropIndicator`] で示すのは持ち主 (`tab.rs`)
//! - [`PaneBox`]: ヘッダ + 中身を入れる箱。[`FloatingPane`]: 切り離したときの浮動ウィンドウ。
//! - [`Strip`]: 折りたたんだペインを表す、ウィンドウの端の細いバー。クリックで元に戻す。
//!
//! どの操作で何をするかはペインの持ち主 (`tab.rs`) が決める。

use std::cell::RefCell;
use std::rc::Rc;

use wxdragon::dc::{Point as DcPoint, PolygonFillMode};
use wxdragon::prelude::*;

use crate::editor::rgb;
use crate::gutter::drawing_panel;
use crate::i18n::tr;
use crate::settings::PanePosition;
use crate::theme::{self, contains, fill_round_rect, mouse_pos, Dip};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HeaderButton {
    /// 配置 (左 / 右 / 下) などのメニュー
    Menu,
    /// 切り離す (浮動中なら戻す)
    Float,
    /// 折りたたむ (端のバーにしまう)
    Collapse,
}

const BUTTONS: [HeaderButton; 3] = [
    HeaderButton::Menu,
    HeaderButton::Float,
    HeaderButton::Collapse,
];
// 寸法は DIP (拡大率 100% 換算)。使うときに `Dip::px` で実ピクセルに直す
const BTN: i32 = 18;
const DRAG_THRESHOLD: i32 = 6;

struct State {
    title: String,
    floating: bool,
    /// 折りたたむ向き (ドッキングしている / する端)
    edge: PanePosition,
    hover: Option<HeaderButton>,
    pressed: Option<HeaderButton>,
    /// (押した位置, しきい値を超えて動いたか)
    drag: Option<(Point, bool)>,
    on_button: Option<Rc<dyn Fn(HeaderButton)>>,
    on_drop: Option<Rc<dyn Fn(Point)>>,
    on_drag_move: Option<Rc<dyn Fn(Point)>>,
    font: Option<Font>,
}

#[derive(Clone)]
pub struct PaneHeader {
    panel: Panel,
    state: Rc<RefCell<State>>,
}

impl PaneHeader {
    /// `title_key` は i18n のキー ("pane.outline" など)
    pub fn new(parent: &Panel, title_key: &str, floating: bool, edge: PanePosition) -> PaneHeader {
        let font = theme::ui_font(true, false, 0);
        let height =
            parent.get_char_height().max(parent.from_dip_int(12)) + parent.from_dip_int(10);
        let panel = drawing_panel(parent, Size::new(-1, height));
        if let Some(f) = &font {
            panel.set_font(f);
        }
        let header = PaneHeader {
            panel,
            state: Rc::new(RefCell::new(State {
                title: tr(title_key),
                floating,
                edge,
                hover: None,
                pressed: None,
                drag: None,
                on_button: None,
                on_drop: None,
                on_drag_move: None,
                font,
            })),
        };
        header.bind();
        header
    }

    pub fn window(&self) -> &Panel {
        &self.panel
    }

    pub fn on_button(&self, f: impl Fn(HeaderButton) + 'static) {
        self.state.borrow_mut().on_button = Some(Rc::new(f));
    }

    /// ヘッダをドラッグして離したとき (画面座標)
    pub fn on_drop(&self, f: impl Fn(Point) + 'static) {
        self.state.borrow_mut().on_drop = Some(Rc::new(f));
    }

    /// ドラッグ中にポインタが動いたとき (画面座標)。しきい値を超えてから呼ばれる
    pub fn on_drag_move(&self, f: impl Fn(Point) + 'static) {
        self.state.borrow_mut().on_drag_move = Some(Rc::new(f));
    }

    fn button_rect(&self, button: HeaderButton) -> Rect {
        let Size { width, height } = self.panel.get_client_size();
        let dip = Dip::of(&self.panel);
        let btn = dip.px(BTN);
        let y = (height - btn) / 2;
        // 右から Collapse, Float, Menu の順
        let slot = BUTTONS.iter().rev().position(|b| *b == button).unwrap_or(0) as i32;
        Rect::new(width - dip.px(4 + BTN + slot * (BTN + 2)), y, btn, btn)
    }

    fn hit_button(&self, p: Point) -> Option<HeaderButton> {
        BUTTONS
            .into_iter()
            .find(|b| contains(&self.button_rect(*b), p))
    }

    fn tooltip(&self, button: Option<HeaderButton>) -> String {
        let floating = self.state.borrow().floating;
        match button {
            Some(HeaderButton::Menu) => tr("pane.menu"),
            Some(HeaderButton::Float) if floating => tr("pane.dock"),
            Some(HeaderButton::Float) => tr("pane.float"),
            Some(HeaderButton::Collapse) => tr("pane.collapse"),
            None => String::new(),
        }
    }

    fn bind(&self) {
        let panel = self.panel;
        panel.on_paint({
            let h = self.clone();
            move |_| h.paint()
        });
        panel.on_mouse_left_down({
            let h = self.clone();
            move |event| {
                let Some(p) = mouse_pos(&event) else {
                    return;
                };
                let button = h.hit_button(p);
                {
                    let mut st = h.state.borrow_mut();
                    st.pressed = button;
                    if button.is_none() {
                        st.drag = Some((p, false));
                    }
                }
                h.panel.capture_mouse();
                h.panel.refresh(false, None);
            }
        });
        panel.on_mouse_motion({
            let h = self.clone();
            move |event| {
                let Some(p) = mouse_pos(&event) else {
                    return;
                };
                let hover = h.hit_button(p);
                let threshold = h.panel.from_dip_int(DRAG_THRESHOLD);
                let (changed, dragging) = {
                    let mut st = h.state.borrow_mut();
                    let mut changed = st.hover != hover;
                    st.hover = hover;
                    let mut dragging = false;
                    if let Some((start, moving)) = st.drag.as_mut() {
                        if !*moving
                            && ((p.x - start.x).abs() > threshold
                                || (p.y - start.y).abs() > threshold)
                        {
                            *moving = true;
                            changed = true;
                        }
                        dragging = *moving;
                    }
                    (changed, dragging)
                };
                if changed {
                    h.panel.set_tooltip(&h.tooltip(hover));
                    h.panel.refresh(false, None);
                }
                if dragging {
                    let f = h.state.borrow().on_drag_move.clone();
                    if let Some(f) = f {
                        f(h.panel.client_to_screen(p));
                    }
                }
            }
        });
        panel.on_mouse_left_up({
            let h = self.clone();
            move |event| {
                let Some(p) = mouse_pos(&event) else {
                    return;
                };
                if h.panel.has_capture() {
                    h.panel.release_mouse();
                }
                let (clicked, dropped) = {
                    let mut st = h.state.borrow_mut();
                    let clicked = match st.pressed.take() {
                        Some(b) if h.hit_button(p) == Some(b) => Some(b),
                        _ => None,
                    };
                    let dropped = matches!(st.drag.take(), Some((_, true)));
                    (clicked, dropped)
                };
                h.panel.refresh(false, None);
                if let Some(b) = clicked {
                    let f = h.state.borrow().on_button.clone();
                    if let Some(f) = f {
                        f(b);
                    }
                }
                if dropped {
                    let screen = h.panel.client_to_screen(p);
                    let f = h.state.borrow().on_drop.clone();
                    if let Some(f) = f {
                        f(screen);
                    }
                }
            }
        });
        panel.on_mouse_leave({
            let h = self.clone();
            move |_| {
                let mut st = h.state.borrow_mut();
                if st.drag.is_none() {
                    st.hover = None;
                    drop(st);
                    h.panel.refresh(false, None);
                }
            }
        });
    }

    fn paint(&self) {
        let dc = AutoBufferedPaintDC::new(&self.panel);
        let Size { width, height } = self.panel.get_client_size();
        let dip = Dip::of(&self.panel);
        let d = |v: i32| dip.px(v);
        dc.set_background(rgb(theme::ui().header_bg));
        dc.clear();
        dc.set_pen(rgb(theme::ui().border), 1, PenStyle::Solid);
        dc.draw_line(0, height - 1, width, height - 1);

        let st = self.state.borrow();
        if let Some(f) = &st.font {
            dc.set_font(f);
        }
        // つかめることが分かるように左端に 2 列の点々 (グリップ)
        dc.set_pen(rgb(theme::ui().grip), 1, PenStyle::Solid);
        dc.set_brush(rgb(theme::ui().grip), BrushStyle::Solid);
        let dot = d(2);
        for row in 0..3 {
            for col in 0..2 {
                dc.draw_rectangle(d(5 + col * 4), height / 2 + d(-5 + row * 4), dot, dot);
            }
        }
        let title = theme::ellipsize(&self.panel, &st.title, (width - d(BTN * 3 + 28)).max(0));
        let (_, th) = dc.get_text_extent(&title);
        dc.set_text_foreground(rgb(theme::ui().text_dim));
        dc.draw_text(&title, d(16), (height - th) / 2);

        for button in BUTTONS {
            let r = self.button_rect(button);
            let hot = st.hover == Some(button);
            let pressed = st.pressed == Some(button) && hot;
            if hot {
                fill_round_rect(
                    &dc,
                    r.x,
                    r.y,
                    r.width,
                    r.height,
                    dip.pxf(3.0),
                    if pressed {
                        theme::ui().pressed_bg
                    } else {
                        theme::ui().hover_bg
                    },
                );
            }
            let (cx, cy) = (r.x + r.width / 2, r.y + r.height / 2);
            dc.set_pen(rgb(theme::ui().text_dim), dip.pen(), PenStyle::Solid);
            dc.set_brush(rgb(theme::ui().text_dim), BrushStyle::Solid);
            match button {
                HeaderButton::Menu => {
                    // 下向きの小さな三角
                    let pts = [
                        DcPoint::new(cx - d(4), cy - d(2)),
                        DcPoint::new(cx + d(4), cy - d(2)),
                        DcPoint::new(cx, cy + d(3)),
                    ];
                    dc.draw_polygon(&pts, 0, 0, PolygonFillMode::OddEven);
                }
                HeaderButton::Float => {
                    paint_float_icon(&dc, cx, cy, theme::ui().text_dim, st.floating, dip)
                }
                HeaderButton::Collapse => paint_chevron(&dc, cx, cy, st.edge, dip),
            }
        }
    }
}

/// 切り離す: 枠から右上へ出る矢印。戻す: 枠へ向かう矢印
fn paint_float_icon(
    dc: &dyn DeviceContext,
    cx: i32,
    cy: i32,
    colour: u32,
    floating: bool,
    dip: Dip,
) {
    let d = |v: i32| dip.px(v);
    let line = |x1: i32, y1: i32, x2: i32, y2: i32| {
        dc.draw_line(cx + d(x1), cy + d(y1), cx + d(x2), cy + d(y2));
    };
    dc.set_pen(rgb(colour), dip.pen(), PenStyle::Solid);
    dc.set_brush(rgb(colour), BrushStyle::Transparent);
    dc.draw_rectangle(cx - d(6), cy - d(2), d(8), d(8));
    if floating {
        line(5, -5, -1, 1);
        line(-1, 1, -1, -3);
        line(-1, 1, 3, 1);
    } else {
        line(-1, 1, 5, -5);
        line(5, -5, 1, -5);
        line(5, -5, 5, -1);
    }
}

/// 折りたたむ向きの二重の山形 (左端へ «、右端へ »、下へ ⌄⌄)
fn paint_chevron(dc: &dyn DeviceContext, cx: i32, cy: i32, edge: PanePosition, dip: Dip) {
    let d = |v: i32| dip.px(v);
    let (dx, dy) = match edge {
        PanePosition::Left => (-1, 0),
        PanePosition::Right => (1, 0),
        PanePosition::Bottom => (0, 1),
    };
    for k in [-2, 2] {
        let (ox, oy) = (cx + dx * d(k), cy + dy * d(k));
        if dy == 0 {
            dc.draw_line(ox - dx * d(2), oy - d(4), ox + dx * d(2), oy);
            dc.draw_line(ox + dx * d(2), oy, ox - dx * d(2), oy + d(4));
        } else {
            dc.draw_line(ox - d(4), oy - d(2), ox, oy + d(2));
            dc.draw_line(ox, oy + d(2), ox + d(4), oy - d(2));
        }
    }
}

/// ヘッダ付きの箱: ペインの中身を入れる `body` とその上のヘッダ
#[derive(Clone)]
pub struct PaneBox {
    pub panel: Panel,
    pub header: PaneHeader,
    pub body: Panel,
}

impl PaneBox {
    pub fn new(
        parent: &dyn WxWidget,
        title_key: &str,
        floating: bool,
        edge: PanePosition,
    ) -> PaneBox {
        let panel = Panel::builder(parent).build();
        let header = PaneHeader::new(&panel, title_key, floating, edge);
        let body = Panel::builder(&panel).build();
        let sizer = BoxSizer::builder(Orientation::Vertical).build();
        sizer.add(header.window(), 0, SizerFlag::Expand, 0);
        sizer.add(&body, 1, SizerFlag::Expand, 0);
        panel.set_sizer(sizer, true);
        PaneBox {
            panel,
            header,
            body,
        }
    }

    /// 中身のウィジェットを body いっぱいに置く (作った直後に 1 回呼ぶ)
    pub fn set_content<W: WxWidget>(&self, content: &W) {
        let sizer = BoxSizer::builder(Orientation::Vertical).build();
        sizer.add(content, 1, SizerFlag::Expand, 0);
        self.body.set_sizer(sizer, true);
        self.panel.layout();
    }
}

/// 切り離したペインの浮動ウィンドウ (メインウィンドウの手前に浮く)
#[derive(Clone)]
pub struct FloatingPane {
    pub frame: Frame,
    pub pane: PaneBox,
}

impl FloatingPane {
    pub fn new(
        parent: &Frame,
        title: &str,
        title_key: &str,
        pos: Point,
        size: Size,
        edge: PanePosition,
    ) -> FloatingPane {
        let min = parent.from_dip(Size::new(240, 200));
        let frame = Frame::builder()
            .with_parent(parent)
            .with_title(title)
            .with_position(pos)
            .with_size(Size::new(
                size.width.max(min.width),
                size.height.max(min.height),
            ))
            .with_style(FrameStyle::Default | FrameStyle::FloatOnParent)
            .build();
        crate::appicon::apply(&frame);
        let pane = PaneBox::new(&frame, title_key, true, edge);
        let sizer = BoxSizer::builder(Orientation::Vertical).build();
        sizer.add(&pane.panel, 1, SizerFlag::Expand, 0);
        frame.set_sizer(sizer, true);
        FloatingPane { frame, pane }
    }

    pub fn show(&self) {
        self.frame.layout();
        self.frame.show(true);
    }

    /// クライアント領域 (ヘッダの上端) が画面上の `pos` / `size` に来るように、
    /// タイトルバーと枠のぶんだけウィンドウを広げてずらす
    pub fn fit_client_to(&self, pos: Point, size: Size) {
        let outer = self.frame.get_size();
        let client = self.frame.get_client_size();
        let dx = (outer.width - client.width).max(0);
        let dy = (outer.height - client.height).max(0);
        // 左右の枠は dx / 2 ずつ、残り (タイトルバー) は上に付く
        self.frame.set_size_with_pos(
            pos.x - dx / 2,
            pos.y - (dy - dx / 2),
            size.width + dx,
            size.height + dy,
        );
    }
}

// ---------------------------------------------------------------------------
// ドロップ先の表示
// ---------------------------------------------------------------------------

/// ペインのヘッダをドラッグしている間、行き先の場所に重ねて出す半透明の青い板 (説明文付き)。
/// 枠無しの最前面ウィンドウで、Windows ではレイヤードウィンドウにして透かす (他の OS では不透明)。
/// ドラッグが終わったら [`DropIndicator::destroy`] で消す (隠したまま残すと、ウィンドウを
/// 全部閉じてもアプリが終わらない原因になる)
pub struct DropIndicator {
    frame: Frame,
    panel: Panel,
    label: Rc<RefCell<String>>,
}

impl DropIndicator {
    pub fn new(parent: &Frame) -> DropIndicator {
        let frame = Frame::builder()
            .with_parent(parent)
            .with_title("")
            .with_style(FrameStyle::StayOnTop | FrameStyle::NoTaskbar)
            .with_size(Size::new(10, 10))
            .build();
        let panel = drawing_panel_in(&frame);
        let label = Rc::new(RefCell::new(String::new()));
        let sizer = BoxSizer::builder(Orientation::Vertical).build();
        sizer.add(&panel, 1, SizerFlag::Expand, 0);
        frame.set_sizer(sizer, true);
        make_translucent(&frame);
        panel.on_paint({
            let label = label.clone();
            let font = theme::ui_font(true, false, 1);
            move |_| {
                let dc = AutoBufferedPaintDC::new(&panel);
                let Size { width, height } = panel.get_client_size();
                let dip = Dip::of(&panel);
                let border = dip.px(2).max(2);
                dc.set_background(rgb(theme::ui().drop_marker));
                dc.clear();
                dc.set_pen(rgb(0xffffff), border, PenStyle::Solid);
                dc.set_brush(rgb(0xffffff), BrushStyle::Transparent);
                let inset = border / 2;
                dc.draw_rectangle(inset, inset, width - border, height - border);
                if let Some(f) = &font {
                    dc.set_font(f);
                }
                let text = label.borrow().clone();
                let (tw, th) = dc.get_text_extent(&text);
                dc.set_text_foreground(rgb(0xffffff));
                dc.draw_text(
                    &text,
                    ((width - tw) / 2).max(dip.px(4)),
                    ((height - th) / 2).max(dip.px(2)),
                );
            }
        });
        DropIndicator {
            frame,
            panel,
            label,
        }
    }

    /// 画面上の `rect` に説明 `label` 付きで出す (出ていれば動かす)
    pub fn show_at(&self, rect: &Rect, label: &str) {
        *self.label.borrow_mut() = label.to_string();
        self.frame
            .set_size_with_pos(rect.x, rect.y, rect.width.max(8), rect.height.max(8));
        if !self.frame.is_shown() {
            self.frame.show(true);
        }
        self.panel.refresh(false, None);
    }

    pub fn hide(&self) {
        if self.frame.is_shown() {
            self.frame.show(false);
        }
    }

    pub fn destroy(&self) {
        self.frame.destroy();
    }
}

/// フレーム直下に置く自前描画のパネル (`gutter::drawing_panel` は Panel の子専用なので同じことをここで)
fn drawing_panel_in(parent: &Frame) -> Panel {
    let panel = Panel::builder(parent).build();
    panel.set_background_style(BackgroundStyle::Paint);
    panel.on_erase_background(|_| {});
    panel.on_size(move |_| panel.refresh(false, None));
    panel
}

/// Windows: レイヤードウィンドウにして半透明に、クリックを素通しし、表示してもアクティブにならないようにする
/// (ドラッグ中のマウスキャプチャを奪わない)
#[cfg(windows)]
fn make_translucent(frame: &Frame) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetLayeredWindowAttributes, SetWindowLongPtrW, GWL_EXSTYLE, LWA_ALPHA,
        WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
    };
    let hwnd = frame.get_handle();
    if hwnd.is_null() {
        return;
    }
    // SAFETY: hwnd は作ったばかりの wxFrame のハンドルで、UI スレッドから呼んでいる
    unsafe {
        let ex = GetWindowLongPtrW(hwnd as _, GWL_EXSTYLE);
        let add =
            (WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW) as isize;
        SetWindowLongPtrW(hwnd as _, GWL_EXSTYLE, ex | add);
        SetLayeredWindowAttributes(hwnd as _, 0, 150, LWA_ALPHA);
    }
}

#[cfg(not(windows))]
fn make_translucent(_frame: &Frame) {}

/// ウィンドウの画面上の矩形
pub fn frame_rect(frame: &Frame) -> Rect {
    let pos = frame.get_position();
    let size = frame.get_size();
    Rect::new(pos.x, pos.y, size.width, size.height)
}

/// ウィジェットの画面上の矩形
pub fn widget_rect<W: WxWidget>(w: &W) -> Rect {
    let origin = w.client_to_screen(Point::new(0, 0));
    let size = w.get_client_size();
    Rect::new(origin.x, origin.y, size.width, size.height)
}

// ---------------------------------------------------------------------------
// 折りたたんだペインのバー
// ---------------------------------------------------------------------------

struct StripState {
    edge: PanePosition,
    /// (ペインの識別子, タイトル)
    items: Vec<(usize, String)>,
    rects: Vec<Rect>,
    hover: Option<usize>,
    on_click: Option<Rc<dyn Fn(usize)>>,
    font: Option<Font>,
}

/// ウィンドウの端 (左 / 右 / 下) に出る細いバー。折りたたんだペインの名前が並び、
/// クリックするとそのペインが元の大きさに戻る
#[derive(Clone)]
pub struct Strip {
    panel: Panel,
    state: Rc<RefCell<StripState>>,
}

const STRIP_PAD: i32 = 4;

impl Strip {
    pub fn new(parent: &Panel, edge: PanePosition) -> Strip {
        let font = theme::ui_font(false, false, 0);
        let thickness =
            parent.get_char_height().max(parent.from_dip_int(12)) + parent.from_dip_int(10);
        let size = if edge.is_side() {
            Size::new(thickness, -1)
        } else {
            Size::new(-1, thickness)
        };
        let panel = drawing_panel(parent, size);
        if let Some(f) = &font {
            panel.set_font(f);
        }
        panel.show(false);
        let strip = Strip {
            panel,
            state: Rc::new(RefCell::new(StripState {
                edge,
                items: Vec::new(),
                rects: Vec::new(),
                hover: None,
                on_click: None,
                font,
            })),
        };
        strip.bind();
        strip
    }

    pub fn window(&self) -> &Panel {
        &self.panel
    }

    pub fn on_click(&self, f: impl Fn(usize) + 'static) {
        self.state.borrow_mut().on_click = Some(Rc::new(f));
    }

    /// 並べる項目を入れ替える。空なら隠れる。呼び出し側で親の layout() を呼ぶこと
    pub fn set_items(&self, items: Vec<(usize, String)>) {
        let visible = !items.is_empty();
        {
            let mut st = self.state.borrow_mut();
            st.items = items;
            st.hover = None;
        }
        self.relayout();
        self.panel.show(visible);
    }

    fn relayout(&self) {
        let Size { width, height } = self.panel.get_client_size();
        let dip = Dip::of(&self.panel);
        let d = |v: i32| dip.px(v);
        let mut st = self.state.borrow_mut();
        let side = st.edge.is_side();
        let mut offset = d(6);
        st.rects = st
            .items
            .iter()
            .map(|(_, title)| {
                let len = self.panel.get_text_extent(title).width + d(20);
                let r = if side {
                    Rect::new(d(3), offset, width - d(6), len)
                } else {
                    Rect::new(offset, d(3), len, height - d(6))
                };
                offset += len + d(STRIP_PAD);
                r
            })
            .collect();
        drop(st);
        self.panel.refresh(false, None);
    }

    fn hit_test(&self, p: Point) -> Option<usize> {
        self.state
            .borrow()
            .rects
            .iter()
            .position(|r| contains(r, p))
    }

    fn bind(&self) {
        let panel = self.panel;
        panel.on_paint({
            let s = self.clone();
            move |_| s.paint()
        });
        panel.on_size({
            let s = self.clone();
            move |_| s.relayout()
        });
        panel.on_mouse_motion({
            let s = self.clone();
            move |event| {
                let hover = mouse_pos(&event).and_then(|p| s.hit_test(p));
                let mut st = s.state.borrow_mut();
                if st.hover != hover {
                    st.hover = hover;
                    let tip = hover
                        .and_then(|i| st.items.get(i))
                        .map(|(_, t)| t.clone())
                        .unwrap_or_default();
                    drop(st);
                    s.panel.set_tooltip(&tip);
                    s.panel.refresh(false, None);
                }
            }
        });
        panel.on_mouse_leave({
            let s = self.clone();
            move |_| {
                s.state.borrow_mut().hover = None;
                s.panel.refresh(false, None);
            }
        });
        panel.on_mouse_left_up({
            let s = self.clone();
            move |event| {
                let hit = mouse_pos(&event).and_then(|p| s.hit_test(p));
                let (id, f) = {
                    let st = s.state.borrow();
                    (
                        hit.and_then(|i| st.items.get(i)).map(|(id, _)| *id),
                        st.on_click.clone(),
                    )
                };
                if let (Some(id), Some(f)) = (id, f) {
                    f(id);
                }
            }
        });
    }

    fn paint(&self) {
        let dc = AutoBufferedPaintDC::new(&self.panel);
        let Size { width, height } = self.panel.get_client_size();
        let radius = Dip::of(&self.panel).pxf(4.0);
        dc.set_background(rgb(theme::ui().bar_bg));
        dc.clear();
        let st = self.state.borrow();
        if let Some(f) = &st.font {
            dc.set_font(f);
        }
        // 内側 (エディタ側) の境界線
        dc.set_pen(rgb(theme::ui().border), 1, PenStyle::Solid);
        match st.edge {
            PanePosition::Left => dc.draw_line(width - 1, 0, width - 1, height),
            PanePosition::Right => dc.draw_line(0, 0, 0, height),
            PanePosition::Bottom => dc.draw_line(0, 0, width, 0),
        }
        for (i, ((_, title), r)) in st.items.iter().zip(&st.rects).enumerate() {
            let bg = if st.hover == Some(i) {
                theme::ui().tab_hover
            } else {
                theme::ui().tab_inactive
            };
            fill_round_rect(&dc, r.x, r.y, r.width, r.height, radius, bg);
            dc.set_pen(rgb(theme::ui().border), 1, PenStyle::Solid);
            dc.set_brush(rgb(bg), BrushStyle::Transparent);
            dc.draw_rounded_rectangle(r.x, r.y, r.width, r.height, radius);
            dc.set_text_foreground(rgb(theme::ui().text_dim));
            let (tw, th) = dc.get_text_extent(title);
            if st.edge.is_side() {
                // 下から上へ読む向きに 90 度回す。(x, y) は回転前の文字列の左上
                dc.draw_rotated_text(
                    title,
                    r.x + (r.width - th) / 2,
                    r.y + r.height - (r.height - tw) / 2,
                    90.0,
                );
            } else {
                dc.draw_text(title, r.x + (r.width - tw) / 2, r.y + (r.height - th) / 2);
            }
        }
    }
}
