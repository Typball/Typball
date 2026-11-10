//! アクションバー: 形式ごとの操作 (`actions.rs`) をアイコンで並べる
//!
//! タブの下、ルーラーの上に置く (エディタの幅だけなので左右のペインには被らない)。
//! 中身はメニューバーの「アクション」と同じ一覧の [`Action::in_bar`] なもの。
//! ボタンはアイコンだけで、マウスを乗せるとツールチップで名前が出る。
//! 種別ごとに出すかどうかは設定 `[filetypes.*] show_actionbar`。
//! 書式付けの実際の編集 ([`apply_format`]) もここにある。

use std::cell::RefCell;
use std::rc::Rc;

use wxdragon::dc::{Point as DcPoint, PolygonFillMode};
use wxdragon::prelude::*;

use crate::actions::Action;
use crate::editor::rgb;
use crate::filetype::FileType;
use crate::gutter::drawing_panel;
use crate::i18n::tr;
use crate::scintilla;
use crate::theme::{self, contains, fill_round_rect, mouse_pos, Dip};

// 寸法は DIP (拡大率 100% 換算)。使うときに `Dip::px` で実ピクセルに直す
const BTN: i32 = 24;
const GAP: i32 = 2;
const PAD: i32 = 4;

struct State {
    items: Vec<Action>,
    hover: Option<usize>,
    pressed: Option<usize>,
    on_action: Option<Rc<dyn Fn(Action)>>,
}

#[derive(Clone)]
pub struct ActionBar {
    panel: Panel,
    state: Rc<RefCell<State>>,
}

impl ActionBar {
    pub fn new(parent: &Panel, filetype: FileType) -> ActionBar {
        let panel = drawing_panel(parent, Size::new(-1, parent.from_dip_int(BTN + PAD)));
        let bar = ActionBar {
            panel,
            state: Rc::new(RefCell::new(State {
                items: bar_items(filetype),
                hover: None,
                pressed: None,
                on_action: None,
            })),
        };
        bar.bind();
        bar
    }

    pub fn window(&self) -> &Panel {
        &self.panel
    }

    pub fn on_action(&self, f: impl Fn(Action) + 'static) {
        self.state.borrow_mut().on_action = Some(Rc::new(f));
    }

    pub fn set_filetype(&self, filetype: FileType) {
        let mut st = self.state.borrow_mut();
        st.items = bar_items(filetype);
        st.hover = None;
        st.pressed = None;
        drop(st);
        self.panel.refresh(false, None);
    }

    fn rect(index: usize, dip: Dip) -> Rect {
        let d = |v: i32| dip.px(v);
        Rect::new(
            d(PAD + index as i32 * (BTN + GAP)),
            d(PAD / 2),
            d(BTN),
            d(BTN),
        )
    }

    fn hit_test(&self, p: Point) -> Option<usize> {
        let n = self.state.borrow().items.len();
        let dip = Dip::of(&self.panel);
        (0..n).find(|i| contains(&Self::rect(*i, dip), p))
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
                let hit = mouse_pos(&event).and_then(|p| bar.hit_test(p));
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
                    st.hover = hover;
                    let tip = hover
                        .and_then(|i| st.items.get(i))
                        .map(|a| tr(&a.tip_key()))
                        .unwrap_or_default();
                    drop(st);
                    bar.panel.set_tooltip(&tip);
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
                let action = {
                    let mut st = bar.state.borrow_mut();
                    let pressed = st.pressed.take();
                    match (pressed, hit) {
                        (Some(a), Some(b)) if a == b => st.items.get(a).copied(),
                        _ => None,
                    }
                };
                bar.panel.refresh(false, None);
                if let Some(action) = action {
                    let f = bar.state.borrow().on_action.clone();
                    if let Some(f) = f {
                        f(action);
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
        dc.set_background(rgb(theme::ui().bar_bg));
        dc.clear();
        dc.set_pen(rgb(theme::ui().border), 1, PenStyle::Solid);
        dc.draw_line(0, height - 1, width, height - 1);

        let st = self.state.borrow();
        for (i, action) in st.items.iter().enumerate() {
            let r = Self::rect(i, dip);
            let hot = st.hover == Some(i);
            let pressed = st.pressed == Some(i) && hot;
            if hot || pressed {
                fill_round_rect(
                    &dc,
                    r.x,
                    r.y,
                    r.width,
                    r.height,
                    dip.pxf(4.0),
                    if pressed {
                        theme::ui().pressed_bg
                    } else {
                        theme::ui().hover_bg
                    },
                );
            }
            paint_glyph(&dc, *action, r.x + r.width / 2, r.y + r.height / 2, dip);
        }
    }
}

fn bar_items(filetype: FileType) -> Vec<Action> {
    Action::for_filetype(filetype)
        .iter()
        .copied()
        .filter(|a| a.in_bar())
        .collect()
}

/// ボタンの絵 (中心座標)。座標は DIP で書き、`dip` で実ピクセルに直す。
/// 文字の大きさは wx が描く先のウィンドウの DPI に合わせるので、ここでは換算しない
fn paint_glyph(dc: &dyn DeviceContext, action: Action, cx: i32, cy: i32, dip: Dip) {
    let d = |v: i32| dip.px(v);
    let pen = dip.pen();
    let fg = theme::ui().text;
    let text = |s: &str, bold: bool, italic: bool, delta: i32| {
        if let Some(font) = theme::ui_font(bold, italic, delta) {
            dc.set_font(&font);
        }
        dc.set_text_foreground(rgb(fg));
        let (w, h) = dc.get_text_extent(s);
        dc.draw_text(s, cx - w / 2, cy - h / 2);
    };
    dc.set_pen(rgb(fg), pen, PenStyle::Solid);
    dc.set_brush(rgb(fg), BrushStyle::Solid);
    match action {
        Action::Bold => text("B", true, false, 2),
        Action::Italic => text("I", false, true, 2),
        Action::Heading => text("H", true, false, 1),
        Action::Code => text("</>", false, false, -1),
        Action::Math => text("∑", false, false, 2),
        Action::Link => {
            // 鎖の輪 2 つ
            dc.set_brush(rgb(fg), BrushStyle::Transparent);
            let r = dip.pxf(3.0);
            dc.draw_rounded_rectangle(cx - d(9), cy - d(3), d(10), d(7), r);
            dc.draw_rounded_rectangle(cx - d(1), cy - d(3), d(10), d(7), r);
            dc.set_pen(rgb(theme::ui().bar_bg), d(2), PenStyle::Solid);
            dc.draw_line(cx - d(1), cy - d(4), cx - d(1), cy + d(5));
            dc.set_pen(rgb(fg), pen, PenStyle::Solid);
            dc.draw_line(cx - d(3), cy, cx + d(3), cy);
        }
        Action::List => {
            for i in 0..3 {
                let y = cy + d(-6 + i * 6);
                dc.draw_rectangle(cx - d(8), y - d(1), d(3), d(3));
                dc.draw_line(cx - d(3), y, cx + d(8), y);
            }
        }
        Action::Quote => {
            dc.draw_rectangle(cx - d(8), cy - d(7), d(3), d(14));
            for i in 0..3 {
                let y = cy + d(-5 + i * 5);
                dc.draw_line(cx - d(2), y, cx + d(8), y);
            }
        }
        Action::Export => {
            // 右上が折れた紙と、右へ出ていく矢印
            dc.set_brush(rgb(theme::ui().icon_paper), BrushStyle::Solid);
            dc.set_pen(rgb(theme::ui().icon_edge), pen, PenStyle::Solid);
            let pts = [(-8, -9), (1, -9), (5, -5), (5, 9), (-8, 9)]
                .map(|(x, y)| DcPoint::new(cx + d(x), cy + d(y)));
            dc.draw_polygon(&pts, 0, 0, PolygonFillMode::OddEven);
            let accent = 0xc0392b;
            dc.set_pen(rgb(accent), (pen * 2).max(2), PenStyle::Solid);
            dc.draw_line(cx - d(3), cy + d(2), cx + d(6), cy + d(2));
            dc.set_pen(rgb(accent), pen, PenStyle::Solid);
            dc.set_brush(rgb(accent), BrushStyle::Solid);
            let head = [(5, -2), (9, 2), (5, 6)].map(|(x, y)| DcPoint::new(cx + d(x), cy + d(y)));
            dc.draw_polygon(&head, 0, 0, PolygonFillMode::OddEven);
        }
    }
}

// ---------------------------------------------------------------------------
// 書式付け
// ---------------------------------------------------------------------------

/// 選択範囲 (無ければキャレット位置) にマークアップを適用する。
/// 書式付けでないアクション (PDF 書き出しなど) は呼び出し側 (`window.rs`) が処理する
pub fn apply_format(stc: &StyledTextCtrl, action: Action, filetype: FileType) {
    let md = filetype == FileType::Md;
    match action {
        Action::Bold => wrap(
            stc,
            if md { "**" } else { "*" },
            if md { "**" } else { "*" },
        ),
        Action::Italic => wrap(stc, if md { "*" } else { "_" }, if md { "*" } else { "_" }),
        Action::Code => wrap(stc, "`", "`"),
        Action::Math => wrap(stc, "$", "$"),
        Action::Link => {
            if md {
                wrap(stc, "[", "](https://)")
            } else {
                wrap(stc, "#link(\"https://\")[", "]")
            }
        }
        Action::Heading => heading(stc, if md { '#' } else { '=' }),
        Action::List => toggle_prefix(stc, "- "),
        Action::Quote => toggle_prefix(stc, "> "),
        Action::Export => return,
    }
    stc.set_focus();
}

/// 選択範囲を `before` と `after` で囲み、元の範囲を選び直す
fn wrap(stc: &StyledTextCtrl, before: &str, after: &str) {
    let start = stc.get_selection_start();
    let selected = stc.get_selected_text();
    scintilla::begin_undo_action(stc);
    stc.replace_selection(&format!("{before}{selected}{after}"));
    scintilla::end_undo_action(stc);
    let inner = start + before.len() as i32;
    stc.set_selection(inner, inner + selected.len() as i32);
}

/// 選択範囲にかかる行の番号 (選択の終わりが行頭にあればその行は含めない)
fn selected_lines(stc: &StyledTextCtrl) -> (i32, i32) {
    let start = stc.get_selection_start();
    let end = stc.get_selection_end();
    let first = stc.line_from_position(start);
    let mut last = stc.line_from_position(end);
    if last > first && stc.position_from_line(last) == end {
        last -= 1;
    }
    (first, last)
}

/// 行頭のマーカーを 1 つ増やす (`#` / `=`)。6 個を超えたら外す
fn heading(stc: &StyledTextCtrl, marker: char) {
    let text = stc.get_text();
    let (first, last) = selected_lines(stc);
    let caret = stc.get_current_pos();
    let mut delta_at_caret = 0;
    scintilla::begin_undo_action(stc);
    for line in (first..=last).rev() {
        let start = stc.position_from_line(line) as usize;
        let end = scintilla::line_end_position(stc, line) as usize;
        let Some(content) = text.get(start..end) else {
            continue;
        };
        let count = content.chars().take_while(|c| *c == marker).count();
        let has_space = content[count..].starts_with(' ');
        let old_len = if count > 0 {
            count + usize::from(has_space)
        } else {
            0
        };
        let new_prefix = if count >= 6 {
            String::new()
        } else {
            format!("{}{} ", marker.to_string().repeat(count), marker)
        };
        stc.set_selection(start as i32, (start + old_len) as i32);
        stc.replace_selection(&new_prefix);
        if (start as i32) <= caret {
            delta_at_caret += new_prefix.len() as i32 - old_len as i32;
        }
    }
    scintilla::end_undo_action(stc);
    stc.goto_pos((caret + delta_at_caret).max(0));
}

/// 行頭の `prefix` を付け外しする (選択行のどれかに無ければ全部に付け、全部にあれば外す)
fn toggle_prefix(stc: &StyledTextCtrl, prefix: &str) {
    let text = stc.get_text();
    let (first, last) = selected_lines(stc);
    let caret = stc.get_current_pos();
    let line_text = |line: i32| -> &str {
        let start = stc.position_from_line(line) as usize;
        let end = scintilla::line_end_position(stc, line) as usize;
        text.get(start..end).unwrap_or("")
    };
    let all_have = (first..=last).all(|l| line_text(l).starts_with(prefix));
    let mut delta_at_caret = 0;
    scintilla::begin_undo_action(stc);
    for line in (first..=last).rev() {
        let start = stc.position_from_line(line);
        if all_have {
            stc.set_selection(start, start + prefix.len() as i32);
            stc.replace_selection("");
            if start < caret {
                delta_at_caret -= prefix.len() as i32;
            }
        } else if !line_text(line).starts_with(prefix) {
            stc.insert_text(start, prefix);
            if start <= caret {
                delta_at_caret += prefix.len() as i32;
            }
        }
    }
    scintilla::end_undo_action(stc);
    stc.goto_pos((caret + delta_at_caret).max(0));
}
