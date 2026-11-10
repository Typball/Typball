//! 検索 / 置換バー (エディタの上に出る 1 〜 2 段のバー)
//!
//! ```text
//! [検索:  ______________ ] [前へ] [次へ]  □大文字小文字 □単語 □正規表現   見つかりません   [×]
//! [置換:  ______________ ] [置換] [すべて置換]                       (置換モードのときだけ)
//! ```
//!
//! - 入力するたびに選択範囲の先頭から探し直す (インクリメンタル検索)。
//!   Enter / F3 で次へ、Shift+Enter / Shift+F3 で前へ、Esc で閉じてエディタに戻る
//! - 文書の端まで来たら反対側から続ける (折り返し検索)
//! - 検索は Scintilla の target 検索 (`scintilla::search`) で行い、正規表現と大文字小文字の
//!   無視でも一致の長さが分かるようにしている。正規表現は POSIX 構文 (`( )` でグループ、
//!   置換側は `\1`)
//! - メニューの「検索」(`window.rs`) はタブのバーを呼ぶだけで、状態はこのバーが持つ

use std::cell::RefCell;
use std::rc::Rc;

use wxdragon::prelude::*;

use crate::i18n::{tr, tr_args};
use crate::scintilla;

const GAP: i32 = 4;
/// 入力欄の幅
const FIELD_WIDTH: i32 = 220;

struct State {
    /// 置換の段を出しているか
    replace_mode: bool,
    /// 閉じるときに呼ぶ (エディタの再レイアウト)
    on_layout: Option<Rc<dyn Fn()>>,
}

#[derive(Clone)]
pub struct FindBar {
    panel: Panel,
    stc: StyledTextCtrl,
    find: TextCtrl,
    replace: TextCtrl,
    replace_row: Panel,
    match_case: CheckBox,
    whole_word: CheckBox,
    regex: CheckBox,
    status: StaticText,
    state: Rc<RefCell<State>>,
}

impl FindBar {
    pub fn new(parent: &Panel, stc: StyledTextCtrl) -> FindBar {
        let panel = Panel::builder(parent).build();
        let find = TextCtrl::builder(&panel)
            .with_style(TextCtrlStyle::ProcessEnter)
            .with_size(Size::new(FIELD_WIDTH, -1))
            .build();
        let prev = Button::builder(&panel).with_label(&tr("find.prev")).build();
        let next = Button::builder(&panel).with_label(&tr("find.next")).build();
        let match_case = CheckBox::builder(&panel)
            .with_label(&tr("find.match_case"))
            .build();
        let whole_word = CheckBox::builder(&panel)
            .with_label(&tr("find.whole_word"))
            .build();
        let regex = CheckBox::builder(&panel)
            .with_label(&tr("find.regex"))
            .build();
        let status = StaticText::builder(&panel).build();
        let close = Button::builder(&panel)
            .with_label("×")
            .with_size(Size::new(24, -1))
            .build();
        close.set_tooltip(&tr("find.close_tip"));

        let row1 = BoxSizer::builder(Orientation::Horizontal).build();
        let center = SizerFlag::AlignCenterVertical;
        let label = StaticText::builder(&panel)
            .with_label(&tr("find.label"))
            .build();
        row1.add(&label, 0, center | SizerFlag::Left, GAP * 2);
        row1.add(&find, 0, center | SizerFlag::Left, GAP);
        row1.add(&prev, 0, center | SizerFlag::Left, GAP);
        row1.add(&next, 0, center | SizerFlag::Left, GAP);
        row1.add(&match_case, 0, center | SizerFlag::Left, GAP * 3);
        row1.add(&whole_word, 0, center | SizerFlag::Left, GAP * 2);
        row1.add(&regex, 0, center | SizerFlag::Left, GAP * 2);
        row1.add(&status, 1, center | SizerFlag::Left, GAP * 3);
        row1.add(&close, 0, center | SizerFlag::Left | SizerFlag::Right, GAP);

        // 置換の段は別のパネルにまとめて、まとめて出し入れする
        let replace_row = Panel::builder(&panel).build();
        let replace_label = StaticText::builder(&replace_row)
            .with_label(&tr("find.replace_label"))
            .build();
        let replace = TextCtrl::builder(&replace_row)
            .with_style(TextCtrlStyle::ProcessEnter)
            .with_size(Size::new(FIELD_WIDTH, -1))
            .build();
        let replace_one = Button::builder(&replace_row)
            .with_label(&tr("find.replace_one"))
            .build();
        let replace_all = Button::builder(&replace_row)
            .with_label(&tr("find.replace_all"))
            .build();
        let row2 = BoxSizer::builder(Orientation::Horizontal).build();
        row2.add(&replace_label, 0, center | SizerFlag::Left, GAP * 2);
        row2.add(&replace, 0, center | SizerFlag::Left, GAP);
        row2.add(&replace_one, 0, center | SizerFlag::Left, GAP);
        row2.add(&replace_all, 0, center | SizerFlag::Left, GAP);
        replace_row.set_sizer(row2, true);
        replace_row.show(false);

        let col = BoxSizer::builder(Orientation::Vertical).build();
        col.add_sizer(
            &row1,
            0,
            SizerFlag::Expand | SizerFlag::Top | SizerFlag::Bottom,
            GAP / 2,
        );
        col.add(
            &replace_row,
            0,
            SizerFlag::Expand | SizerFlag::Bottom,
            GAP / 2,
        );
        panel.set_sizer(col, true);
        panel.show(false);

        let bar = FindBar {
            panel,
            stc,
            find,
            replace,
            replace_row,
            match_case,
            whole_word,
            regex,
            status,
            state: Rc::new(RefCell::new(State {
                replace_mode: false,
                on_layout: None,
            })),
        };

        // --- イベント ---
        find.on_text_updated({
            let bar = bar.clone();
            move |_| bar.search_incremental()
        });
        for cb in [match_case, whole_word, regex] {
            // チェックボックスの変更 (wxEVT_CHECKBOX)。条件が変わったので探し直す
            WxEvtHandler::bind_internal(&cb, EventType::CHECKBOX, {
                let bar = bar.clone();
                move |_| bar.search_incremental()
            });
        }
        for (field, is_replace) in [(find, false), (replace, true)] {
            field.on_key_down({
                let bar = bar.clone();
                move |event| {
                    let WindowEventData::Keyboard(key) = &event else {
                        event.skip(true);
                        return;
                    };
                    match key.get_key_code() {
                        Some(WXK_ESCAPE) => bar.hide(),
                        Some(WXK_RETURN) | Some(WXK_NUMPAD_ENTER) => {
                            if is_replace {
                                bar.replace_one();
                            } else {
                                bar.find_next(!key.shift_down());
                            }
                        }
                        _ => event.skip(true),
                    }
                }
            });
        }
        next.on_click({
            let bar = bar.clone();
            move |_| {
                bar.find_next(true);
            }
        });
        prev.on_click({
            let bar = bar.clone();
            move |_| {
                bar.find_next(false);
            }
        });
        replace_one.on_click({
            let bar = bar.clone();
            move |_| bar.replace_one()
        });
        replace_all.on_click({
            let bar = bar.clone();
            move |_| bar.replace_all()
        });
        close.on_click({
            let bar = bar.clone();
            move |_| bar.hide()
        });
        bar
    }

    pub fn window(&self) -> &Panel {
        &self.panel
    }

    /// バーを出し入れしたときに呼ばれる処理 (持ち主がエディタ部分を並べ直す)
    pub fn on_layout(&self, f: impl Fn() + 'static) {
        self.state.borrow_mut().on_layout = Some(Rc::new(f));
    }

    pub fn is_shown(&self) -> bool {
        self.panel.is_shown()
    }

    /// 入力欄のどちらかにフォーカスがあればそれを返す (編集メニューの振り分け用)
    pub fn focused_text(&self) -> Option<TextCtrl> {
        if self.find.has_focus() {
            Some(self.find)
        } else if self.replace.has_focus() {
            Some(self.replace)
        } else {
            None
        }
    }

    /// バーを出す (`replace` なら置換の段も)。エディタで 1 行以内の選択があればそれを検索語にする
    pub fn show(&self, replace: bool) {
        let selected = self.stc.get_selected_text();
        if !selected.is_empty() && !selected.contains(['\n', '\r']) {
            self.find.set_value(&selected);
        }
        self.state.borrow_mut().replace_mode = replace;
        self.replace_row.show(replace);
        self.panel.show(true);
        self.set_status("");
        self.relayout();
        self.find.set_focus();
        self.find.select_all();
    }

    /// バーを隠してエディタにフォーカスを戻す
    pub fn hide(&self) {
        if !self.panel.is_shown() {
            return;
        }
        self.panel.show(false);
        self.relayout();
        self.stc.set_focus();
    }

    fn relayout(&self) {
        self.panel.layout();
        let f = self.state.borrow().on_layout.clone();
        if let Some(f) = f {
            f();
        }
    }

    fn set_status(&self, text: &str) {
        self.status.set_label(text);
        self.panel.layout();
    }

    fn flags(&self) -> i32 {
        let mut flags = 0;
        if self.match_case.is_checked() {
            flags |= scintilla::SCFIND_MATCHCASE;
        }
        if self.whole_word.is_checked() {
            flags |= scintilla::SCFIND_WHOLEWORD;
        }
        if self.regex.is_checked() {
            flags |= scintilla::SCFIND_REGEXP | scintilla::SCFIND_POSIX;
        }
        flags
    }

    /// 選択範囲 (小さい方, 大きい方)
    fn selection(&self) -> (i32, i32) {
        let (a, b) = (self.stc.get_selection_start(), self.stc.get_selection_end());
        (a.min(b), a.max(b))
    }

    fn search(&self, start: i32, end: i32) -> Option<(i32, i32)> {
        let text = self.find.get_value();
        scintilla::search(&self.stc, start, end, &text, self.flags())
    }

    /// `start` から前後に探し、端まで来たら反対側から続ける
    fn search_wrapping(&self, start: i32, forward: bool) -> Option<(i32, i32)> {
        let len = self.stc.get_length();
        if forward {
            self.search(start, len).or_else(|| self.search(0, len))
        } else {
            self.search(start, 0).or_else(|| self.search(len, 0))
        }
    }

    fn select_match(&self, found: Option<(i32, i32)>) -> bool {
        match found {
            Some((s, e)) => {
                self.stc.set_selection(s, e);
                self.stc.ensure_caret_visible();
                self.set_status("");
                true
            }
            None => {
                if !self.find.get_value().is_empty() {
                    self.set_status(&tr("find.not_found"));
                }
                false
            }
        }
    }

    /// 入力中: 選択範囲の先頭から探し直す (いま選んでいる一致がまだ一致するならそこに留まる)
    fn search_incremental(&self) {
        if self.find.get_value().is_empty() {
            self.set_status("");
            return;
        }
        let (lo, _) = self.selection();
        let found = self.search_wrapping(lo, true);
        self.select_match(found);
    }

    /// 次 (前) の一致へ。検索語が空ならバーを出すだけ。見つかれば true
    pub fn find_next(&self, forward: bool) -> bool {
        if self.find.get_value().is_empty() {
            self.show(self.state.borrow().replace_mode);
            return false;
        }
        let (lo, hi) = self.selection();
        let start = if forward { hi } else { lo };
        let found = self.search_wrapping(start, forward);
        self.select_match(found)
    }

    /// 選択範囲が検索語に一致していれば置き換えて、次の一致へ進む
    pub fn replace_one(&self) {
        let (lo, hi) = self.selection();
        if lo != hi {
            if let Some((s, e)) = self.search(lo, hi) {
                if s == lo && e == hi {
                    let replacement = self.replace.get_value();
                    let n =
                        scintilla::replace_target(&self.stc, &replacement, self.regex.is_checked());
                    self.stc.set_selection(s, s + n.max(0));
                }
            }
        }
        self.find_next(true);
    }

    /// 文書全体で置き換える (1 回の「元に戻す」で戻せる)
    pub fn replace_all(&self) {
        if self.find.get_value().is_empty() {
            return;
        }
        let replacement = self.replace.get_value();
        let regex = self.regex.is_checked();
        let mut count = 0;
        let mut pos = 0;
        scintilla::begin_undo_action(&self.stc);
        while let Some((s, e)) = self.search(pos, self.stc.get_length()) {
            let n = scintilla::replace_target(&self.stc, &replacement, regex).max(0);
            count += 1;
            // 空文字に一致する正規表現 (`a*` など) で止まらなくならないように 1 つ進める
            pos = s + n + if e == s { 1 } else { 0 };
            if pos > self.stc.get_length() {
                break;
            }
        }
        scintilla::end_undo_action(&self.stc);
        self.set_status(&tr_args("find.replaced", &[("n", &count.to_string())]));
    }
}
