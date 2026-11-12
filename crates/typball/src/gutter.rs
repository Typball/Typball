//! エディタの周りに自前で描く部品: 上部のルーラーと右側の行番号
//!
//! Scintilla のマージンは左側にしか置けないので、右側の行番号と上部のルーラーは wxPanel に
//! 自前で描く。表示位置は Scintilla に問い合わせる (`scintilla.rs`: 先頭の表示行・行の高さ・
//! 文書位置の座標) ので、折り返し行やズーム (Ctrl+ホイール) にも追従する。
//! 再描画はエディタの UpdateUI イベント (キャレット移動・スクロール・変更) で行う。

use wxdragon::prelude::*;

use crate::editor::{self, rgb, LineChange};
use crate::scintilla;
use crate::settings::RulerTabs;
use crate::settings::{self, Settings};
use crate::theme;

/// 自前描画用のパネルを作る (背景は paint ハンドラで塗るのでシステムには消させない)。
/// サイズが変わったら全体を描き直す (既定では新しく見えた部分しか再描画されず、
/// 目盛りの数字が途中で切れたり消えたりする)
pub fn drawing_panel(parent: &Panel, size: Size) -> Panel {
    let panel = Panel::builder(parent).with_size(size).build();
    panel.set_min_size(size);
    panel.set_background_style(BackgroundStyle::Paint);
    panel.on_erase_background(|_| {});
    panel.on_size(move |_| panel.refresh(false, None));
    panel
}

// ---------------------------------------------------------------------------
// ルーラー
// ---------------------------------------------------------------------------

/// エディタ上部のルーラー。本文の列位置に目盛りを打ち、キャレットの位置に黒いマーカーを出す。
/// 目盛りの間隔は数字 "0" の幅 (プロポーショナルフォントでは目安)。
#[derive(Clone)]
pub struct Ruler {
    panel: Panel,
    container: Panel,
    stc: StyledTextCtrl,
}

impl Ruler {
    pub fn new(container: &Panel, stc: StyledTextCtrl) -> Self {
        let panel = drawing_panel(container, Size::new(-1, ruler_height(&stc)));
        panel.on_paint({
            let panel = panel;
            move |_| paint_ruler(&panel, &stc)
        });
        Self {
            panel,
            container: *container,
            stc,
        }
    }

    pub fn window(&self) -> &Panel {
        &self.panel
    }

    /// 高さをフォントに合わせてから再描画する
    pub fn refresh(&self) {
        let h = ruler_height(&self.stc);
        if self.panel.get_min_size().height != h {
            self.panel.set_min_size(Size::new(-1, h));
            self.container.layout();
        }
        self.panel.refresh(false, None);
    }
}

/// ルーラーの高さ = 本文 1 行の高さ
fn ruler_height(stc: &StyledTextCtrl) -> i32 {
    scintilla::text_height(stc, 0).clamp(stc.from_dip_int(14), stc.from_dip_int(64))
}

fn paint_ruler(panel: &Panel, stc: &StyledTextCtrl) {
    let dc = AutoBufferedPaintDC::new(panel);
    let Size { width, height } = panel.get_client_size();
    let c = editor::colors();
    dc.set_background(rgb(c.gutter_bg));
    dc.clear();
    dc.set_pen(rgb(c.ruler_line), 1, PenStyle::Solid);
    dc.draw_line(0, height - 1, width, height - 1);

    // 目盛りの数字は UI フォント (設定のフォントはエディタの中身だけ)。ズームには追従する
    let Some(label_font) = theme::ui_font(false, false, stc.get_zoom()) else {
        return;
    };
    dc.set_font(&label_font);
    dc.set_text_foreground(rgb(c.gutter_fg));

    // 列 0 の x 座標 (横スクロールしていれば負になる) と 1 列の幅
    let first_doc = scintilla::doc_line_from_visible(stc, scintilla::first_visible_line(stc));
    let origin = scintilla::point_x_from_position(stc, stc.position_from_line(first_doc));
    let unit = scintilla::text_width(stc, editor::STYLE_DEFAULT, "0").max(1);
    let right = stc.get_client_size().width.min(width);

    // 画面左にはみ出している列は飛ばす
    let mut col = if origin < 0 {
        (-origin + unit - 1) / unit
    } else {
        0
    };
    dc.set_pen(rgb(c.ruler_tick), 1, PenStyle::Solid);
    // タブ位置 (タブ幅ごとの桁) の示し方
    let (tab, tab_marks) = {
        let s = crate::settings::get();
        (s.editor.tab_width.clamp(1, 16), s.editor.ruler_tabs)
    };
    let px = panel.from_dip_int(1).max(1);
    loop {
        let x = origin + col * unit;
        if x >= right {
            break;
        }
        // 10 列ごとに長い目盛りと数字、5 列ごとに中、それ以外は短い目盛り
        let len = if col % 10 == 0 {
            height / 2
        } else if col % 5 == 0 {
            height / 3
        } else {
            height / 6
        };
        dc.draw_line(x, height - 1 - len, x, height - 1);
        if col > 0 && col % tab == 0 {
            match tab_marks {
                RulerTabs::None => {}
                // 目盛りの上に、下向きの小さな三角 (下線から少し浮かせる)
                RulerTabs::Arrow => {
                    let rows = 3 * px;
                    for row in 0..rows {
                        let half = rows - 1 - row;
                        let y = height - 1 - 2 * px - rows + row;
                        dc.draw_line(x - half, y, x + half + 1, y);
                    }
                }
            }
        }
        if col % 10 == 0 && col > 0 {
            // 数字は目盛りの真上に、目盛りを中心にして置く。字が大きいので目盛りの上の方に重なるが、
            // 横の位置は桁と合わせる (ユーザーの希望。右にずらすと、どの桁の数字か分かりにくい)
            let label = col.to_string();
            let (tw, _) = dc.get_text_extent(&label);
            dc.draw_text(&label, x - tw / 2, -px);
        }
        col += 1;
    }

    // キャレット位置の黒いマーカー (下半分、幅 5 DIP)
    let caret_x = scintilla::point_x_from_position(stc, stc.get_current_pos());
    if caret_x >= 0 && caret_x < right {
        dc.set_pen(rgb(c.separator), 1, PenStyle::Solid);
        dc.set_brush(rgb(c.separator), BrushStyle::Solid);
        let w = panel.from_dip_int(5) | 1;
        dc.draw_rectangle(caret_x - w / 2, height / 2, w, height - 1 - height / 2);
    }
}

// ---------------------------------------------------------------------------
// 右側の行番号
// ---------------------------------------------------------------------------

/// エディタ右側の行番号。左の行番号マージンと同じ配色・桁幅で、左端に 1px の境界線を引く。
/// 未保存の編集行は太字にして右端にオレンジの帯、保存済みの編集行は青の帯を出す
/// (設定 `highlight_changes`)。
#[derive(Clone)]
pub struct RightGutter {
    panel: Panel,
    container: Panel,
    stc: StyledTextCtrl,
}

impl RightGutter {
    pub fn new(container: &Panel, stc: StyledTextCtrl) -> Self {
        let width = gutter_width(&stc, &settings::get());
        let panel = drawing_panel(container, Size::new(width, -1));
        panel.on_paint({
            let panel = panel;
            move |_| paint_gutter(&panel, &stc)
        });
        Self {
            panel,
            container: *container,
            stc,
        }
    }

    pub fn window(&self) -> &Panel {
        &self.panel
    }

    /// 幅を桁数・ズームに合わせてから再描画する
    pub fn refresh(&self) {
        let w = gutter_width(&self.stc, &settings::get());
        if self.panel.get_min_size().width != w {
            self.panel.set_min_size(Size::new(w, -1));
            self.container.layout();
        }
        self.panel.refresh(false, None);
    }
}

/// 右余白 3 DIP (+ 帯と余白 2 DIP)
fn right_pad(stc: &StyledTextCtrl, strip: i32) -> i32 {
    stc.from_dip_int(3)
        + if strip > 0 {
            strip + stc.from_dip_int(2)
        } else {
            0
        }
}

/// 境界線 1px + 左余白 4 DIP + 数字 + 右余白
fn gutter_width(stc: &StyledTextCtrl, settings: &Settings) -> i32 {
    let strip = editor::change_strip_width(stc, settings);
    1 + stc.from_dip_int(4) + editor::digits(stc) * editor::digit_width(stc) + right_pad(stc, strip)
}

fn paint_gutter(panel: &Panel, stc: &StyledTextCtrl) {
    let dc = AutoBufferedPaintDC::new(panel);
    let Size { width, height } = panel.get_client_size();
    let c = editor::colors();
    dc.set_background(rgb(c.gutter_bg));
    dc.clear();
    dc.set_pen(rgb(c.separator), 1, PenStyle::Solid);
    dc.draw_line(0, 0, 0, height);

    let settings = settings::get();
    let zoom = stc.get_zoom();
    // 行番号は UI フォント (設定のフォントはエディタの中身だけ)。ズームには追従する
    let (Some(normal), Some(bold)) = (
        theme::ui_font(false, false, zoom),
        theme::ui_font(true, false, zoom),
    ) else {
        return;
    };
    let strip = editor::change_strip_width(stc, &settings);
    let right_pad = right_pad(stc, strip);

    let first_visible = scintilla::first_visible_line(stc);
    let last_line = stc.get_line_count() - 1;
    let first_doc = scintilla::doc_line_from_visible(stc, first_visible);
    let last_doc =
        scintilla::doc_line_from_visible(stc, first_visible + scintilla::lines_on_screen(stc))
            .min(last_line);

    let extra_top = editor::line_extra(stc, &crate::settings::get()).0;
    // 行の上端と段の数は、画面の何段目かから計算する。Scintilla に位置を聞く問い合わせ
    // (SCI_POINTYFROMPOSITION / SCI_WRAPCOUNT) は 1 回ごとに計測用の描画面を作るので遅い (marks.rs)
    let line_h = scintilla::text_height(stc, first_doc);
    let mut row = scintilla::visible_from_doc_line(stc, first_doc);
    for line in first_doc..=last_doc {
        let next_row = scintilla::visible_from_doc_line(stc, line + 1);
        let rows = (next_row - row).max(1);
        let y = (row - first_visible) * line_h;
        row = next_row;
        if y >= height {
            break;
        }
        let change = if strip > 0 {
            editor::line_change(stc, line)
        } else {
            LineChange::Unchanged
        };
        let strip_colour = match change {
            LineChange::Modified => Some(c.modified),
            LineChange::Saved => Some(c.saved),
            LineChange::Unchanged => None,
        };
        if let Some(colour) = strip_colour {
            dc.set_pen(rgb(colour), 1, PenStyle::Solid);
            dc.set_brush(rgb(colour), BrushStyle::Solid);
            dc.draw_rectangle(width - strip, y, strip, line_h * rows);
        }
        let (font, fg) = match change {
            LineChange::Modified => (&bold, c.gutter_fg_modified),
            LineChange::Saved => (&normal, c.gutter_fg_saved),
            LineChange::Unchanged => (&normal, c.gutter_fg),
        };
        dc.set_font(font);
        dc.set_text_foreground(rgb(fg));
        let label = (line + 1).to_string();
        let (tw, _) = dc.get_text_extent(&label);
        // 行間 (設定) で行の上に足した高さのぶん、本文と同じだけ下げる
        dc.draw_text(&label, width - right_pad - tw, y + extra_top);
    }
}
