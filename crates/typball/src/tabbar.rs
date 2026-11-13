//! 自前描画のタブバー
//!
//! wxNotebook のタブはアイコンと閉じるボタンを一緒に出せず、クリック位置のタブも
//! (wxDragon からは) 取れないので、タブの帯を wxPanel に自分で描く。ページ本体は
//! [`SimpleBook`] が持ち、タブバーは「何番目が選ばれているか」をコールバックで知らせるだけ。
//!
//! できること:
//! - 左クリックで選択、× か中ボタンで閉じる、右クリックでコンテキストメニュー
//! - 空いている場所をダブルクリックで新規タブ
//! - ドラッグで並べ替え。バーの外で離すと [`TabBar::on_drag_end`] に画面座標を渡す
//!   (呼び出し側が「別ウィンドウへ移動」か「新しいウィンドウを作る」かを決める)。
//!   ドラッグ中はカーソルにタブのアイコンと行き先のバッジ (+ / → / 禁止) を出す。
//!   行き先は [`TabBar::on_drag_target`] で呼び出し側に問い合わせる

use std::cell::RefCell;
use std::rc::Rc;

use wxdragon::prelude::*;

use crate::editor::rgb;
use crate::filetype::FileType;
use crate::gutter::drawing_panel;
use wxdragon::dc::{Point as DcPoint, PolygonFillMode};

use crate::theme::{self, contains, draw_cross, ellipsize, fill_round_rect, mouse_pos, Badge, Dip};

/// バーの外へドラッグしたときの行き先 (呼び出し側が判定する)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DragTarget {
    /// 離すとその位置に新しいウィンドウができる
    NewWindow,
    /// 離すと別のウィンドウのタブバーへ移る (そのバーには挿入位置のマーカーが出ている)
    OtherWindow,
    /// 離しても何も起きない (タブが 1 つしか無いウィンドウなど)
    Nothing,
}

/// タブ 1 つ分の表示情報
pub struct TabItem {
    /// ページを識別する値 (ページのウィンドウハンドル)
    pub key: usize,
    pub title: String,
    pub filetype: FileType,
    pub modified: bool,
}

// 寸法はすべて DIP (拡大率 100% 換算)。使うときに `Dip::px` で実ピクセルに直す
const PAD_X: i32 = 8;
const ICON: i32 = 16;
const CLOSE: i32 = 16;
const MIN_TAB: i32 = 72;
const MAX_TAB: i32 = 240;
const MIN_SHRUNK: i32 = 48;
/// これ以上動かしたらドラッグとみなす
const DRAG_THRESHOLD: i32 = 6;
/// バーからこれ以上離れて離すと「外へ出した」とみなす
const DETACH_MARGIN: i32 = 32;

type Handler<T> = Option<Rc<dyn Fn(T)>>;

#[derive(Default)]
struct Handlers {
    select: Handler<usize>,
    close: Handler<usize>,
    context: Handler<usize>,
    reorder: Option<Rc<dyn Fn(usize, usize)>>,
    drag_end: Option<Rc<dyn Fn(usize, Point)>>,
    /// 引数の Point が None ならポインタは自分のバーの中 (他のバーのマーカーを消してもらう)
    drag_target: Option<Rc<dyn Fn(usize, Option<Point>) -> DragTarget>>,
    new_tab: Handler<()>,
}

struct Drag {
    index: usize,
    /// タブの左端からつかんだ位置までの距離
    grab_x: i32,
    start: Point,
    /// しきい値を超えて動いたか
    moving: bool,
    /// 現在のポインタの x
    x: i32,
    /// 今出しているカーソルのバッジ (変わったときだけ作り直す)
    badge: Option<Badge>,
}

struct State {
    items: Vec<TabItem>,
    /// 直近のレイアウトで決めたタブの矩形 (items と同じ並び)
    rects: Vec<Rect>,
    selected: Option<usize>,
    hover: Option<usize>,
    hover_close: bool,
    press_close: Option<usize>,
    drag: Option<Drag>,
    /// 別のウィンドウからタブをドラッグされているときの挿入位置 (この位置のタブの左に線を出す)
    drop_marker: Option<usize>,
    handlers: Handlers,
    font: Option<Font>,
}

#[derive(Clone)]
pub struct TabBar {
    panel: Panel,
    state: Rc<RefCell<State>>,
}

impl TabBar {
    pub fn new(parent: &Panel) -> TabBar {
        let font = theme::ui_font(false, false, 0);
        let text_h = font
            .as_ref()
            .map(|_| parent.get_char_height())
            .unwrap_or(16)
            .max(12);
        let height = text_h + parent.from_dip_int(14);
        let panel = drawing_panel(parent, Size::new(-1, height));
        if let Some(f) = &font {
            panel.set_font(f);
        }
        let bar = TabBar {
            panel,
            state: Rc::new(RefCell::new(State {
                items: Vec::new(),
                rects: Vec::new(),
                selected: None,
                hover: None,
                hover_close: false,
                press_close: None,
                drag: None,
                drop_marker: None,
                handlers: Handlers::default(),
                font,
            })),
        };
        bar.bind();
        bar
    }

    pub fn window(&self) -> &Panel {
        &self.panel
    }

    // -----------------------------------------------------------------------
    // コールバック
    // -----------------------------------------------------------------------

    pub fn on_select(&self, f: impl Fn(usize) + 'static) {
        self.state.borrow_mut().handlers.select = Some(Rc::new(f));
    }

    pub fn on_close(&self, f: impl Fn(usize) + 'static) {
        self.state.borrow_mut().handlers.close = Some(Rc::new(f));
    }

    pub fn on_context(&self, f: impl Fn(usize) + 'static) {
        self.state.borrow_mut().handlers.context = Some(Rc::new(f));
    }

    /// ドラッグで並べ替えたとき (from → to)。内部の並びは変更済み
    pub fn on_reorder(&self, f: impl Fn(usize, usize) + 'static) {
        self.state.borrow_mut().handlers.reorder = Some(Rc::new(f));
    }

    /// バーの外で離したとき (タブの位置, 離した画面座標)
    pub fn on_drag_end(&self, f: impl Fn(usize, Point) + 'static) {
        self.state.borrow_mut().handlers.drag_end = Some(Rc::new(f));
    }

    /// 空いている場所をダブルクリックしたとき
    pub fn on_new_tab(&self, f: impl Fn(()) + 'static) {
        self.state.borrow_mut().handlers.new_tab = Some(Rc::new(f));
    }

    /// バーの外にあるポインタ (画面座標) で離したら何が起きるか (カーソルのバッジに使う)。
    /// ポインタがバーの中に戻ったときとドラッグが終わったときは None で呼ぶ
    pub fn on_drag_target(&self, f: impl Fn(usize, Option<Point>) -> DragTarget + 'static) {
        self.state.borrow_mut().handlers.drag_target = Some(Rc::new(f));
    }

    /// 別のウィンドウからのドラッグ中に出す挿入位置のマーカー (None で消す)
    pub fn set_drop_marker(&self, index: Option<usize>) {
        let changed = {
            let mut st = self.state.borrow_mut();
            let changed = st.drop_marker != index;
            st.drop_marker = index;
            changed
        };
        if changed {
            self.panel.refresh(false, None);
        }
    }

    // -----------------------------------------------------------------------
    // 項目の操作 (コールバックは呼ばない)
    // -----------------------------------------------------------------------

    pub fn len(&self) -> usize {
        self.state.borrow().items.len()
    }

    pub fn insert(&self, index: usize, item: TabItem) {
        {
            let mut st = self.state.borrow_mut();
            let index = index.min(st.items.len());
            st.items.insert(index, item);
            if let Some(sel) = st.selected {
                if sel >= index {
                    st.selected = Some(sel + 1);
                }
            }
        }
        self.relayout();
    }

    pub fn remove(&self, index: usize) {
        {
            let mut st = self.state.borrow_mut();
            if index >= st.items.len() {
                return;
            }
            st.items.remove(index);
            st.hover = None;
            st.drag = None;
            st.press_close = None;
            st.selected = match st.selected {
                Some(sel) if st.items.is_empty() => {
                    let _ = sel;
                    None
                }
                Some(sel) if sel > index => Some(sel - 1),
                Some(sel) if sel == index => Some(sel.min(st.items.len() - 1)),
                other => other,
            };
        }
        self.relayout();
    }

    /// 並べ替える (from の項目を to の位置へ)
    pub fn move_item(&self, from: usize, to: usize) {
        {
            let mut st = self.state.borrow_mut();
            if from >= st.items.len() || to >= st.items.len() || from == to {
                return;
            }
            let item = st.items.remove(from);
            st.items.insert(to, item);
            st.selected = st.selected.map(|sel| {
                if sel == from {
                    to
                } else if from < sel && sel <= to {
                    sel - 1
                } else if to <= sel && sel < from {
                    sel + 1
                } else {
                    sel
                }
            });
        }
        self.relayout();
    }

    /// 表示情報を更新する (変わっていなければ描き直さない)
    pub fn update(&self, key: usize, title: &str, modified: bool, filetype: FileType) {
        let changed = {
            let mut st = self.state.borrow_mut();
            match st.items.iter_mut().find(|it| it.key == key) {
                Some(it)
                    if it.title != title || it.modified != modified || it.filetype != filetype =>
                {
                    it.title = title.to_string();
                    it.modified = modified;
                    it.filetype = filetype;
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.relayout();
        }
    }

    pub fn select(&self, index: usize) {
        {
            let mut st = self.state.borrow_mut();
            if index >= st.items.len() || st.selected == Some(index) {
                return;
            }
            st.selected = Some(index);
        }
        self.panel.refresh(false, None);
    }

    pub fn selected(&self) -> Option<usize> {
        self.state.borrow().selected
    }

    pub fn index_of(&self, key: usize) -> Option<usize> {
        self.state
            .borrow()
            .items
            .iter()
            .position(|it| it.key == key)
    }

    /// バーの画面上の矩形 (別ウィンドウへのドロップ判定用)
    pub fn screen_rect(&self) -> Rect {
        let origin = self.panel.client_to_screen(Point::new(0, 0));
        let size = self.panel.get_client_size();
        Rect::new(origin.x, origin.y, size.width, size.height)
    }

    /// 画面座標 `screen` にドロップしたときの挿入位置 (ポインタに一番近いタブの隙間)
    pub fn drop_index(&self, screen: Point) -> usize {
        let p = self.panel.screen_to_client(screen);
        let st = self.state.borrow();
        st.rects
            .iter()
            .position(|r| p.x < r.x + r.width / 2)
            .unwrap_or(st.items.len())
    }

    // -----------------------------------------------------------------------
    // レイアウト
    // -----------------------------------------------------------------------

    fn relayout(&self) {
        let width = self.panel.get_client_size().width;
        let height = self.panel.get_client_size().height;
        let dip = Dip::of(&self.panel);
        let d = |v: i32| dip.px(v);
        let mut st = self.state.borrow_mut();
        let n = st.items.len() as i32;
        let mut widths: Vec<i32> = st
            .items
            .iter()
            .map(|it| {
                let label = label_of(it);
                let tw = self.panel.get_text_extent(&label).width;
                (d(PAD_X + ICON + 6) + tw + d(6 + CLOSE + 6)).clamp(d(MIN_TAB), d(MAX_TAB))
            })
            .collect();
        let total: i32 = widths.iter().sum::<i32>() + d(4);
        if n > 0 && total > width {
            let each = ((width - d(4)) / n).max(d(MIN_SHRUNK));
            for w in &mut widths {
                *w = (*w).min(each);
            }
        }
        let mut x = d(4);
        st.rects = widths
            .iter()
            .map(|w| {
                let r = Rect::new(x, 0, *w, height);
                x += *w;
                r
            })
            .collect();
        drop(st);
        self.panel.refresh(false, None);
    }

    fn hit_test(&self, p: Point) -> Option<usize> {
        let st = self.state.borrow();
        st.rects.iter().position(|r| contains(r, p))
    }

    fn close_rect(rect: &Rect, dip: Dip) -> Rect {
        let size = dip.px(CLOSE);
        Rect::new(
            rect.x + rect.width - size - dip.px(6),
            rect.y + (rect.height - size) / 2 + dip.px(1),
            size,
            size,
        )
    }

    fn hit_close(&self, p: Point) -> Option<usize> {
        let dip = Dip::of(&self.panel);
        let st = self.state.borrow();
        st.rects
            .iter()
            .position(|r| contains(&Self::close_rect(r, dip), p))
    }

    // -----------------------------------------------------------------------
    // イベント
    // -----------------------------------------------------------------------

    fn bind(&self) {
        let panel = self.panel;

        panel.on_paint({
            let bar = self.clone();
            move |_| bar.paint()
        });
        panel.on_size({
            let bar = self.clone();
            move |_| bar.relayout()
        });
        panel.on_mouse_left_down({
            let bar = self.clone();
            move |event| {
                if let Some(p) = mouse_pos(&event) {
                    bar.left_down(p);
                }
                // ここで処理済みにしないと wx の既定処理がこのパネルにフォーカスを移し、
                // タブを選んだ直後のキー入力がエディタに届かなくなる
                event.skip(false);
            }
        });
        panel.on_mouse_motion({
            let bar = self.clone();
            move |event| {
                if let Some(p) = mouse_pos(&event) {
                    bar.motion(p);
                }
            }
        });
        panel.on_mouse_left_up({
            let bar = self.clone();
            move |event| {
                if let Some(p) = mouse_pos(&event) {
                    bar.left_up(p);
                }
            }
        });
        panel.on_mouse_leave({
            let bar = self.clone();
            move |_| {
                let mut st = bar.state.borrow_mut();
                if st.drag.is_none() {
                    st.hover = None;
                    st.hover_close = false;
                    drop(st);
                    bar.panel.refresh(false, None);
                }
            }
        });
        panel.on_mouse_middle_down({
            let bar = self.clone();
            move |event| {
                let hit = mouse_pos(&event).and_then(|p| bar.hit_test(p));
                let handler = bar.state.borrow().handlers.close.clone();
                if let (Some(i), Some(f)) = (hit, handler) {
                    f(i);
                }
            }
        });
        panel.on_mouse_right_down({
            let bar = self.clone();
            move |event| {
                let Some(i) = mouse_pos(&event).and_then(|p| bar.hit_test(p)) else {
                    return;
                };
                bar.select_and_notify(i);
                let handler = bar.state.borrow().handlers.context.clone();
                if let Some(f) = handler {
                    f(i);
                }
            }
        });
        panel.on_mouse_left_double_click({
            let bar = self.clone();
            move |event| {
                let hit = mouse_pos(&event).and_then(|p| bar.hit_test(p));
                if hit.is_none() {
                    let handler = bar.state.borrow().handlers.new_tab.clone();
                    if let Some(f) = handler {
                        f(());
                    }
                }
            }
        });
    }

    /// 選択を変え、変わったら on_select を呼ぶ
    fn select_and_notify(&self, index: usize) {
        let handler = {
            let mut st = self.state.borrow_mut();
            if st.selected == Some(index) {
                None
            } else {
                st.selected = Some(index);
                st.handlers.select.clone()
            }
        };
        self.panel.refresh(false, None);
        if let Some(f) = handler {
            f(index);
        }
    }

    fn left_down(&self, p: Point) {
        if let Some(i) = self.hit_close(p) {
            self.state.borrow_mut().press_close = Some(i);
            return;
        }
        let Some(i) = self.hit_test(p) else {
            return;
        };
        self.select_and_notify(i);
        let grab_x = self
            .state
            .borrow()
            .rects
            .get(i)
            .map(|r| p.x - r.x)
            .unwrap_or(0);
        self.state.borrow_mut().drag = Some(Drag {
            index: i,
            grab_x,
            start: p,
            moving: false,
            x: p.x,
            badge: None,
        });
        self.panel.capture_mouse();
    }

    /// バーの矩形 (クライアント座標) の外か。少し余裕を持たせる
    fn is_outside(&self, p: Point) -> bool {
        let size = self.panel.get_client_size();
        let margin = self.panel.from_dip_int(DETACH_MARGIN);
        p.y < -margin
            || p.y > size.height + margin
            || p.x < -margin * 2
            || p.x > size.width + margin * 2
    }

    /// ドラッグ中のカーソルを行き先に合わせる
    fn update_drag_cursor(&self, p: Point) {
        let (index, filetype, current) = {
            let st = self.state.borrow();
            let Some(d) = st.drag.as_ref().filter(|d| d.moving) else {
                return;
            };
            let Some(item) = st.items.get(d.index) else {
                return;
            };
            (d.index, item.filetype, d.badge)
        };
        let handler = self.state.borrow().handlers.drag_target.clone();
        let badge = if self.is_outside(p) {
            match handler.map(|f| f(index, Some(self.panel.client_to_screen(p)))) {
                Some(DragTarget::NewWindow) => Badge::NewWindow,
                Some(DragTarget::OtherWindow) => Badge::Insert,
                Some(DragTarget::Nothing) | None => Badge::Forbidden,
            }
        } else {
            // 外から戻ってきたら他のバーのマーカーを消してもらう
            if current.is_some_and(|b| b != Badge::None) {
                if let Some(f) = handler {
                    f(index, None);
                }
            }
            Badge::None
        };
        if current == Some(badge) {
            return;
        }
        if let Some(d) = self.state.borrow_mut().drag.as_mut() {
            d.badge = Some(badge);
        }
        let cursor = theme::drag_cursor(
            &|dc, x, y, size| paint_icon(dc, filetype, x, y, size),
            badge,
            Dip::of(&self.panel),
        );
        theme::show_drag_cursor(cursor.as_ref());
    }

    fn motion(&self, p: Point) {
        let hover = self.hit_test(p);
        let hover_close = self.hit_close(p).is_some();
        let threshold = self.panel.from_dip_int(DRAG_THRESHOLD);
        let mut reorder: Option<(usize, usize)> = None;
        {
            let mut st = self.state.borrow_mut();
            let changed = st.hover != hover || st.hover_close != hover_close;
            st.hover = hover;
            st.hover_close = hover_close;
            if let Some(drag) = st.drag.as_mut() {
                if !drag.moving
                    && ((p.x - drag.start.x).abs() > threshold
                        || (p.y - drag.start.y).abs() > threshold)
                {
                    drag.moving = true;
                }
                if drag.moving {
                    drag.x = p.x;
                    let i = drag.index;
                    // つかんでいるタブの中心が隣のタブの中心を越えたら入れ替える
                    let center = p.x - drag.grab_x + st.rects[i].width / 2;
                    if i > 0 && center < st.rects[i - 1].x + st.rects[i - 1].width / 2 {
                        reorder = Some((i, i - 1));
                    } else if i + 1 < st.rects.len()
                        && center > st.rects[i + 1].x + st.rects[i + 1].width / 2
                    {
                        reorder = Some((i, i + 1));
                    }
                }
                drop(st);
                self.panel.refresh(false, None);
            } else if changed {
                drop(st);
                self.panel.refresh(false, None);
            }
        }
        if let Some((from, to)) = reorder {
            self.move_item(from, to);
            if let Some(drag) = self.state.borrow_mut().drag.as_mut() {
                drag.index = to;
            }
            let handler = self.state.borrow().handlers.reorder.clone();
            if let Some(f) = handler {
                f(from, to);
            }
        }
        self.update_drag_cursor(p);
    }

    fn left_up(&self, p: Point) {
        if self.panel.has_capture() {
            self.panel.release_mouse();
        }
        theme::show_drag_cursor(None);
        let outside = self.is_outside(p);
        let (close, moved) = {
            let mut st = self.state.borrow_mut();
            let close = match st.press_close.take() {
                Some(i)
                    if st
                        .rects
                        .get(i)
                        .map(|r| contains(&Self::close_rect(r, Dip::of(&self.panel)), p))
                        == Some(true) =>
                {
                    Some(i)
                }
                _ => None,
            };
            let moved = match st.drag.take() {
                Some(d) if d.moving => Some(d.index),
                _ => None,
            };
            (close, moved)
        };
        let drag_end = outside.then_some(moved).flatten();
        self.panel.refresh(false, None);
        if let Some(i) = close {
            let handler = self.state.borrow().handlers.close.clone();
            if let Some(f) = handler {
                f(i);
            }
        }
        if let Some(i) = moved {
            // ドラッグが終わった: 他のバーのマーカーを消してもらう
            let handler = self.state.borrow().handlers.drag_target.clone();
            if let Some(f) = handler {
                f(i, None);
            }
        }
        if let Some(i) = drag_end {
            let screen = self.panel.client_to_screen(p);
            let handler = self.state.borrow().handlers.drag_end.clone();
            if let Some(f) = handler {
                f(i, screen);
            }
        }
    }

    // -----------------------------------------------------------------------
    // 描画
    // -----------------------------------------------------------------------

    fn paint(&self) {
        let dc = AutoBufferedPaintDC::new(&self.panel);
        let Size { width, height } = self.panel.get_client_size();
        let dip = Dip::of(&self.panel);
        let d = |v: i32| dip.px(v);
        dc.set_background(rgb(theme::ui().bar_bg));
        dc.clear();
        let st = self.state.borrow();
        if let Some(f) = &st.font {
            dc.set_font(f);
        }
        // 下の境界線 (選択中のタブの下だけ空ける)
        dc.set_pen(rgb(theme::ui().border), 1, PenStyle::Solid);
        dc.draw_line(0, height - 1, width, height - 1);

        let dragging = st.drag.as_ref().filter(|d| d.moving);
        for (i, (item, rect)) in st.items.iter().zip(&st.rects).enumerate() {
            if dragging.map(|d| d.index) == Some(i) {
                continue;
            }
            self.paint_tab(&dc, &st, i, item, rect, false);
        }
        // 別のウィンドウからドラッグされているときの挿入位置
        if let Some(i) = st.drop_marker {
            let x = match st.rects.get(i) {
                Some(r) => r.x,
                None => st.rects.last().map(|r| r.x + r.width).unwrap_or(d(4)),
            };
            dc.set_pen(rgb(theme::ui().drop_marker), 1, PenStyle::Solid);
            dc.set_brush(rgb(theme::ui().drop_marker), BrushStyle::Solid);
            let bar = d(3);
            dc.draw_rectangle(x - bar / 2, d(2), bar, height - d(3));
            let tri = [
                DcPoint::new(x - d(5), 0),
                DcPoint::new(x + d(6), 0),
                DcPoint::new(x, d(6)),
            ];
            dc.draw_polygon(&tri, 0, 0, PolygonFillMode::OddEven);
        }
        // つかんでいるタブはポインタに追従させて最後に (一番手前に) 描く
        if let Some(d) = dragging {
            if let (Some(item), Some(rect)) = (st.items.get(d.index), st.rects.get(d.index)) {
                let x = (d.x - d.grab_x).clamp(0, (width - rect.width).max(0));
                let moved = Rect::new(x, rect.y, rect.width, rect.height);
                self.paint_tab(&dc, &st, d.index, item, &moved, true);
            }
        }
    }

    fn paint_tab(
        &self,
        dc: &AutoBufferedPaintDC,
        st: &State,
        index: usize,
        item: &TabItem,
        rect: &Rect,
        floating: bool,
    ) {
        let selected = st.selected == Some(index);
        let hovered = st.hover == Some(index);
        let dip = Dip::of(&self.panel);
        let d = |v: i32| dip.px(v);
        let radius = dip.pxf(5.0);
        let top = if selected { d(2) } else { d(4) };
        let bg = if selected {
            theme::ui().tab_active
        } else if hovered || floating {
            theme::ui().tab_hover
        } else {
            theme::ui().tab_inactive
        };
        // タブ本体 (上だけ角丸: 角丸矩形を下へはみ出させて描く)
        dc.set_clipping_region(rect.x, top, rect.width, rect.height - top);
        let below = d(6);
        fill_round_rect(
            dc,
            rect.x,
            top,
            rect.width,
            rect.height - top + below,
            radius,
            bg,
        );
        dc.set_pen(rgb(theme::ui().border), 1, PenStyle::Solid);
        dc.set_brush(rgb(bg), BrushStyle::Transparent);
        dc.draw_rounded_rectangle(rect.x, top, rect.width, rect.height - top + below, radius);
        dc.destroy_clipping_region();
        if selected && !floating {
            // ページとつなげる: 下の境界線を消す
            dc.set_pen(rgb(theme::ui().tab_active), 1, PenStyle::Solid);
            dc.draw_line(
                rect.x + 1,
                rect.height - 1,
                rect.x + rect.width - 1,
                rect.height - 1,
            );
        } else {
            // 選択されていないタブは下の境界線の奥に置く (タブ本体が線を塗りつぶすので引き直す)。
            // 選択中のタブだけがページとつながり、手前にあるように見える
            dc.set_pen(rgb(theme::ui().border), 1, PenStyle::Solid);
            dc.draw_line(
                rect.x,
                rect.height - 1,
                rect.x + rect.width,
                rect.height - 1,
            );
        }

        // アイコン
        let icon = d(ICON);
        let icon_y = top + (rect.height - top - icon) / 2;
        paint_icon(dc, item.filetype, rect.x + d(PAD_X), icon_y, icon);

        // 文字 (閉じるボタンの手前まで)
        let text_x = rect.x + d(PAD_X + ICON + 6);
        let show_close = selected || hovered || floating;
        let text_right = if show_close {
            rect.x + rect.width - d(CLOSE + 8)
        } else {
            rect.x + rect.width - d(6)
        };
        let label = ellipsize(&self.panel, &label_of(item), (text_right - text_x).max(0));
        let (_, th) = dc.get_text_extent(&label);
        dc.set_text_foreground(rgb(if selected {
            theme::ui().text
        } else {
            theme::ui().text_dim
        }));
        dc.set_clipping_region(text_x, top, (text_right - text_x).max(0), rect.height - top);
        dc.draw_text(&label, text_x, top + (rect.height - top - th) / 2);
        dc.destroy_clipping_region();

        // 閉じるボタン
        if show_close {
            let c = Self::close_rect(rect, dip);
            let hot = hovered && st.hover_close;
            if hot {
                fill_round_rect(
                    dc,
                    c.x,
                    c.y,
                    c.width,
                    c.height,
                    dip.pxf(3.0),
                    theme::ui().close_hover,
                );
            }
            draw_cross(
                dc,
                c.x + c.width / 2,
                c.y + c.height / 2,
                d(7),
                dip.pen(),
                if hot { 0xffffff } else { theme::ui().text_dim },
            );
        }
    }
}

/// タブに出す文字列 (未保存なら " *")
fn label_of(item: &TabItem) -> String {
    if item.modified {
        format!("{} *", item.title)
    } else {
        item.title.clone()
    }
}

/// 種別のアイコン: テキストは罫線入りの紙、Markdown は青の "M"、Typst は青緑の "T"、図は橙の "D"。
/// 左上 (x, y) に一辺 `size` px で描く (絵は 16px で設計してあり、`size` に合わせて拡大する)
pub fn paint_icon(dc: &dyn DeviceContext, filetype: FileType, x: i32, y: i32, size: i32) {
    let scale = size as f64 / ICON as f64;
    let s = |v: i32| (v as f64 * scale).round() as i32;
    let pen = s(1).max(1);
    match filetype {
        FileType::Txt => {
            dc.set_pen(rgb(theme::ui().icon_edge), pen, PenStyle::Solid);
            dc.set_brush(rgb(theme::ui().icon_paper), BrushStyle::Solid);
            dc.draw_rectangle(x + s(2), y + s(1), s(12), s(14));
            dc.set_pen(rgb(theme::ui().icon_line), pen, PenStyle::Solid);
            for i in 0..4 {
                let ly = y + s(4 + i * 3);
                dc.draw_line(x + s(4), ly, x + s(12 - if i == 3 { 4 } else { 0 }), ly);
            }
        }
        FileType::Md => badge(dc, x, y, size, 0x0b5cad, "M"),
        FileType::Typ => badge(dc, x, y, size, 0x239dad, "T"),
        FileType::Tbdg => badge(dc, x, y, size, 0xc2610f, "D"),
    }
}

fn badge(dc: &dyn DeviceContext, x: i32, y: i32, size: i32, colour: u32, letter: &str) {
    fill_round_rect(
        dc,
        x,
        y,
        size,
        size,
        3.0 * size as f64 / ICON as f64,
        colour,
    );
    dc.set_text_foreground(rgb(0xffffff));
    // 文字の大きさは描く先の DC で決まる (ウィンドウの DC は wx がそのウィンドウの DPI に合わせ、
    // カーソル用の MemoryDC は主モニターの DPI のまま)。どちらでも台座に収まるよう、
    // 行の高さが台座を明らかに超えるあいだポイント数を下げる (行の高さには上下の余白が
    // 含まれるので、台座の 1.25 倍までは字が収まる)
    let mut extent = (0, 0);
    for delta in [0, -1, -2, -3, -4, -5] {
        if let Some(font) = theme::ui_font(true, false, delta) {
            dc.set_font(&font);
        }
        extent = dc.get_text_extent(letter);
        if extent.1 * 4 <= size * 5 {
            break;
        }
    }
    let (tw, th) = extent;
    dc.draw_text(letter, x + (size - tw) / 2, y + (size - th) / 2);
}
