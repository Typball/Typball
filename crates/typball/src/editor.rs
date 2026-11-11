//! エディタ (wxStyledTextCtrl) の見た目と Typst シンタックスハイライト
//!
//! Scintilla のレクサは使わず、Typst 本体と同じ `typst-syntax` で構文木を作り、
//! `highlight()` が返すタグを Scintilla のスタイル番号に割り当てる。
//!
//! 左側のマージンは左から順に
//! 0. 編集した行のマーカー (帯。オレンジ = まだ保存していない編集、青 = 編集して保存した行。
//!    設定 `highlight_changes`、保存後の扱いは `saved_changes`)
//! 1. 行番号 (テキストマージン。Number マージンと違って行ごとにスタイルを変えられるので、
//!    編集した行だけ太字にできる。設定 `line_numbers`)
//! 2. 本文との境目の 1px の黒線 (MarginType::Fore は STYLE_DEFAULT の前景色で塗られる)
//!
//! 右側の行番号と上部のルーラーは Scintilla のマージンでは作れないので `gutter.rs` で自前描画する。
//! 「編集した行」は [`ChangeTracker`] が変更のたびに前回との行差分を取ってマーカーを付ける
//! (この wxWidgets が同梱する Scintilla 5.0 には変更履歴機能 SCI_SETCHANGEHISTORY が無い)。
//! 保存すると未保存のマーカーは「保存済み」のマーカーに替わる (または消える)。
//! 「指定文字数で折り返し」は Scintilla に無いので、本文の右余白を広げて折り返し幅を文字数ぶんにする。

use std::hash::{DefaultHasher, Hash, Hasher};

use typst_syntax::{highlight, LinkedNode, Tag};
use wxdragon::prelude::*;

use crate::filetype::FileType;
use crate::scintilla;
use crate::settings::{SavedChanges, Settings, Wrap};

// Scintilla の予約スタイル番号
pub const STYLE_DEFAULT: i32 = 32;
const STYLE_LINENUMBER: i32 = 33;
const STYLE_BRACELIGHT: i32 = 34;
const STYLE_BRACEBAD: i32 = 35;
/// 行番号マージン用のスタイル (STYLE_LASTPREDEFINED = 39 より後ろの空き番号)
const STYLE_MARGIN: i32 = 40;
const STYLE_MARGIN_MODIFIED: i32 = 41;
const STYLE_MARGIN_SAVED: i32 = 42;

const MARGIN_CHANGES: i32 = 0;
const MARGIN_LINENUMBERS: i32 = 1;
const MARGIN_SEPARATOR: i32 = 2;

/// 「開いてから編集した行」のマーカー番号 (0..=24 のうち折りたたみ用 25..=31 以外)
const MARKER_MODIFIED: i32 = 1;
const MARKER_MODIFIED_MASK: i32 = 1 << MARKER_MODIFIED;
/// 「編集してから保存した行」のマーカー番号
const MARKER_SAVED: i32 = 2;
const MARKER_SAVED_MASK: i32 = 1 << MARKER_SAVED;

/// 編集した行の帯の幅 (px)。右側の行番号 (`gutter.rs`) も同じ幅で描く
pub const CHANGE_STRIP_WIDTH: i32 = 4;
/// 境界線と本文の間の余白 (DIP)
const TEXT_GAP: i32 = 3;
/// キャレットの太さ (DIP)
const CARET_WIDTH: i32 = 2;

/// 編集行の帯の幅 (実ピクセル。強調オフなら 0)
pub fn change_strip_width(stc: &StyledTextCtrl, settings: &Settings) -> i32 {
    if settings.editor.highlight_changes {
        stc.from_dip_int(CHANGE_STRIP_WIDTH)
    } else {
        0
    }
}

/// エディタの配色 (本文・行番号・ルーラー。設定 `editor.theme` で明暗を選ぶ)
pub struct EditorColors {
    /// 本文の背景
    pub bg: u32,
    /// 本文 (タグの無いテキスト)
    pub text: u32,
    /// 行番号まわり (左マージンと右側の自前描画で共通) の地
    pub gutter_bg: u32,
    pub gutter_fg: u32,
    pub gutter_fg_modified: u32,
    pub gutter_fg_saved: u32,
    /// 編集行の帯 (未保存 / 保存済み)
    pub modified: u32,
    pub saved: u32,
    /// 行番号と本文の境界線、ルーラーのキャレット位置
    pub separator: u32,
    /// ルーラーの目盛りと下線
    pub ruler_tick: u32,
    pub ruler_line: u32,
    pub caret: u32,
    pub caret_line: u32,
    pub selection: u32,
    /// raw ブロックの背景
    pub raw_bg: u32,
    /// 対応括弧の強調
    pub brace_bg: u32,
    pub brace_fg: u32,
    pub brace_bad: u32,
}

const LIGHT: EditorColors = EditorColors {
    bg: 0xffffff,
    text: 0x1a1a1a,
    gutter_bg: 0xf5f5f5,
    gutter_fg: 0x8a8a8a,
    gutter_fg_modified: 0x1a1a1a,
    gutter_fg_saved: 0x2f6db5,
    modified: 0xf0a030,
    saved: 0x5aa0e0,
    separator: 0x000000,
    ruler_tick: 0x6a6a6a,
    ruler_line: 0x9e9e9e,
    caret: 0x000000,
    caret_line: 0xf3f3f3,
    selection: 0xc0c0c0,
    raw_bg: 0xf2f2f2,
    brace_bg: 0xffe97a,
    brace_fg: 0x1a1a1a,
    brace_bad: 0xff0000,
};

/// VS Code Dark+ 風
const DARK: EditorColors = EditorColors {
    bg: 0x1e1e1e,
    text: 0xd4d4d4,
    gutter_bg: 0x252526,
    gutter_fg: 0x858585,
    gutter_fg_modified: 0xe6e6e6,
    gutter_fg_saved: 0x6aa7ff,
    modified: 0xd7962c,
    saved: 0x3d7fc8,
    separator: 0x9a9a9a,
    ruler_tick: 0x8a8a8a,
    ruler_line: 0x4a4a4a,
    caret: 0xaeafad,
    caret_line: 0x2a2d2e,
    selection: 0x264f78,
    raw_bg: 0x2a2a2a,
    brace_bg: 0x515c6a,
    brace_fg: 0xffffff,
    brace_bad: 0xf44747,
};

/// 今のエディタの配色
pub fn colors() -> &'static EditorColors {
    if crate::theme::editor_dark() {
        &DARK
    } else {
        &LIGHT
    }
}

pub fn rgb(hex: u32) -> Colour {
    Colour::new(
        ((hex >> 16) & 0xff) as u8,
        ((hex >> 8) & 0xff) as u8,
        (hex & 0xff) as u8,
        255,
    )
}

/// typst-syntax の Tag → Scintilla スタイル番号 (0 は既定スタイル、1.. が各タグ)
fn style_of(tag: Tag) -> i32 {
    tag as i32 + 1
}

/// 各タグの配色 (ライトは VS Code Light+ 風、ダークは Dark+ 風)。(前景色, 太字, 斜体, 下線)
fn tag_appearance(tag: Tag, dark: bool) -> (u32, bool, bool, bool) {
    let (light, dark_fg, bold, italic, underline) = match tag {
        Tag::Comment => (0x008000, 0x6a9955, false, true, false),
        Tag::Punctuation => (0x5a5a5a, 0x9a9a9a, false, false, false),
        Tag::Escape => (0xee0000, 0xd7ba7d, false, false, false),
        Tag::Strong => (0x1a1a1a, 0xe6e6e6, true, false, false),
        Tag::Emph => (0x1a1a1a, 0xe6e6e6, false, true, false),
        Tag::Link => (0x0066cc, 0x4daafc, false, false, true),
        Tag::Raw => (0x6a3e3e, 0xce9178, false, false, false),
        Tag::Label => (0x8b008b, 0xc586c0, false, false, false),
        Tag::Ref => (0x8b008b, 0xc586c0, false, false, false),
        Tag::Heading => (0x0033aa, 0x569cd6, true, false, false),
        Tag::ListMarker => (0x8b008b, 0xc586c0, true, false, false),
        Tag::ListTerm => (0x1a1a1a, 0xe6e6e6, true, false, false),
        Tag::MathDelimiter => (0x1a6e9a, 0x4ec9b0, false, false, false),
        Tag::MathOperator => (0x1a6e9a, 0x4ec9b0, false, false, false),
        Tag::MathGroupingParens => (0x1a6e9a, 0x4ec9b0, false, false, false),
        Tag::Keyword => (0xaf00db, 0xc586c0, false, false, false),
        Tag::Operator => (0x4a4a4a, 0xb4b4b4, false, false, false),
        Tag::Number => (0x098658, 0xb5cea8, false, false, false),
        Tag::String => (0xa31515, 0xce9178, false, false, false),
        Tag::Function => (0x795e26, 0xdcdcaa, false, false, false),
        Tag::Interpolated => (0x0070c1, 0x9cdcfe, false, false, false),
        Tag::Error => (0xff0000, 0xf44747, false, false, true),
    };
    (if dark { dark_fg } else { light }, bold, italic, underline)
}

const ALL_TAGS: [Tag; 22] = [
    Tag::Comment,
    Tag::Punctuation,
    Tag::Escape,
    Tag::Strong,
    Tag::Emph,
    Tag::Link,
    Tag::Raw,
    Tag::Label,
    Tag::Ref,
    Tag::Heading,
    Tag::ListMarker,
    Tag::ListTerm,
    Tag::MathDelimiter,
    Tag::MathOperator,
    Tag::MathGroupingParens,
    Tag::Keyword,
    Tag::Operator,
    Tag::Number,
    Tag::String,
    Tag::Function,
    Tag::Interpolated,
    Tag::Error,
];

/// 設定からエディタ用フォントを作る。`extra_pt` はポイント数の増減 (ズームや小さめの表示用)
pub fn font(settings: &Settings, extra_pt: i32, bold: bool) -> Option<Font> {
    let face = if settings.editor.font_face.trim().is_empty() {
        crate::settings::DEFAULT_FONT_FACE
    } else {
        settings.editor.font_face.as_str()
    };
    Font::builder()
        .with_face_name(face)
        .with_point_size((settings.editor.font_size.clamp(6, 72) + extra_pt).clamp(4, 96))
        .with_weight(if bold {
            FontWeight::Bold
        } else {
            FontWeight::Normal
        })
        .build()
}

/// エディタの見た目 (フォント・マージン・配色) を初期化する
pub fn setup_editor(stc: &StyledTextCtrl, settings: &Settings) {
    // レクサは使わず、アプリ側で全文にスタイルを付ける
    stc.set_lexer_typed(Lexer::Container);

    // 変更イベントはテキストの挿入・削除 (が終わった後) だけにする。
    // - 既定ではマーカーやマージン文字列の変更でも EVT_STC_CHANGE が飛ぶので、変更ハンドラで
    //   行番号を付け直すと無限ループになる
    // - SC_PERFORMED_USER などをマスクに含めると、挿入・削除の「直前」の通知 (SC_MOD_BEFOREINSERT /
    //   BEFOREDELETE にも SC_PERFORMED_USER が立っている) まで届き、まだ変わっていないテキストで
    //   変更行を推定してしまう
    scintilla::set_mod_event_mask(
        stc,
        scintilla::SC_MOD_INSERTTEXT | scintilla::SC_MOD_DELETETEXT,
    );

    stc.set_margin_type_typed(MARGIN_CHANGES, MarginType::Symbol);
    scintilla::set_margin_mask(
        stc,
        MARGIN_CHANGES,
        MARKER_MODIFIED_MASK | MARKER_SAVED_MASK,
    );
    stc.set_margin_type_typed(MARGIN_LINENUMBERS, MarginType::RText);
    scintilla::set_margin_mask(stc, MARGIN_LINENUMBERS, 0);
    stc.set_margin_type_typed(MARGIN_SEPARATOR, MarginType::Fore);
    scintilla::set_margin_mask(stc, MARGIN_SEPARATOR, 0);

    apply_colors(stc, settings);

    // 現在行の強調 (色は apply_colors)
    stc.set_caret_line_visible(true);

    apply_settings(stc, settings);
}

/// 配色をセットする。フォント変更後は style_clear_all で全スタイルが既定に戻るので
/// そのたびに呼び直す。
fn apply_colors(stc: &StyledTextCtrl, settings: &Settings) {
    if let Some(f) = font(settings, 0, false) {
        stc.style_set_font(STYLE_DEFAULT, &f);
    }
    let c = colors();
    // 既定スタイルの前景色は境界線マージン (MarginType::Fore) の色にもなるので境界線の色にし、
    // 本文 (タグの無いテキスト = スタイル 0) は style_clear_all の後で少し柔らかい色にする
    stc.style_set_foreground(STYLE_DEFAULT, rgb(c.separator));
    stc.style_set_background(STYLE_DEFAULT, rgb(c.bg));
    // 既定スタイルを全スタイルにコピーしてから個別に上書き
    stc.style_clear_all();
    stc.style_set_foreground(0, rgb(c.text));

    let dark = crate::theme::editor_dark();
    for tag in ALL_TAGS {
        let (fg, bold, italic, underline) = tag_appearance(tag, dark);
        let s = style_of(tag);
        stc.style_set_foreground(s, rgb(fg));
        stc.style_set_bold(s, bold);
        stc.style_set_italic(s, italic);
        stc.style_set_underline(s, underline);
    }
    // raw ブロックは背景を薄く
    stc.style_set_background(style_of(Tag::Raw), rgb(c.raw_bg));

    // 行番号マージン。STYLE_LINENUMBER の背景色がマージン全体の地になり、
    // 文字は行ごとに STYLE_MARGIN / STYLE_MARGIN_MODIFIED (太字) / STYLE_MARGIN_SAVED で描かれる
    stc.style_set_foreground(STYLE_LINENUMBER, rgb(c.gutter_fg));
    stc.style_set_background(STYLE_LINENUMBER, rgb(c.gutter_bg));
    for (style, fg, bold) in [
        (STYLE_MARGIN, c.gutter_fg, false),
        (STYLE_MARGIN_MODIFIED, c.gutter_fg_modified, true),
        (STYLE_MARGIN_SAVED, c.gutter_fg_saved, false),
    ] {
        stc.style_set_foreground(style, rgb(fg));
        stc.style_set_background(style, rgb(c.gutter_bg));
        stc.style_set_bold(style, bold);
    }

    // 対応括弧の強調 (黄色背景 + 太字) / 不一致 (赤)
    stc.style_set_background(STYLE_BRACELIGHT, rgb(c.brace_bg));
    stc.style_set_foreground(STYLE_BRACELIGHT, rgb(c.brace_fg));
    stc.style_set_bold(STYLE_BRACELIGHT, true);
    stc.style_set_foreground(STYLE_BRACEBAD, rgb(c.brace_bad));
    stc.style_set_bold(STYLE_BRACEBAD, true);

    // キャレット・現在行・選択範囲
    scintilla::set_caret_fore(stc, c.caret);
    stc.set_caret_line_background(rgb(c.caret_line));
    scintilla::set_sel_back(stc, c.selection);
}

/// 設定ダイアログで変えられる項目を既存のエディタに反映する
pub fn apply_settings(stc: &StyledTextCtrl, settings: &Settings) {
    apply_colors(stc, settings);
    stc.set_wrap_mode(match settings.editor.wrap {
        Wrap::None => WrapMode::None,
        Wrap::Window | Wrap::Column => WrapMode::Word,
    });
    let tab = settings.editor.tab_width.clamp(1, 16);
    stc.set_tab_width(tab);
    stc.set_use_tabs(false);
    stc.set_indent(tab);
    update_margins(stc, settings);
    refresh_line_numbers(stc, settings);
}

/// 行番号の桁数 (最低 2 桁)
pub fn digits(stc: &StyledTextCtrl) -> i32 {
    stc.get_line_count().max(1).to_string().len().max(2) as i32
}

/// 太字の行番号 1 桁ぶんの幅 (px、ズーム込み)。左右の行番号の幅を同じ基準でそろえる
pub fn digit_width(stc: &StyledTextCtrl) -> i32 {
    scintilla::text_width(stc, STYLE_MARGIN_MODIFIED, "0").max(4)
}

/// 左側マージンの幅を設定・行数・ズームに合わせる (変更が無ければ Scintilla 側で no-op)
pub fn update_margins(stc: &StyledTextCtrl, settings: &Settings) {
    // 寸法は DIP で決めて、エディタのある画面の拡大率で実ピクセルにする。ウィンドウが拡大率の
    // 違うモニターへ移ったときは wx がマージン幅を比例で直す (wxStyledTextCtrl::OnDPIChanged) が、
    // 余白とキャレットは直さないので、ここで毎回合わせる (値が同じなら何もしない)
    let gap = stc.from_dip_int(TEXT_GAP);
    if scintilla::margin_left(stc) != gap {
        // 境界線と本文の間を少し空ける
        scintilla::set_margin_left(stc, gap);
    }
    let caret = stc.from_dip_int(CARET_WIDTH);
    if stc.get_caret_width() != caret {
        stc.set_caret_width(caret);
    }
    let strip = change_strip_width(stc, settings);
    let numbers = if settings.editor.line_numbers.left() {
        // RText マージンは右端に 3px の余白を取る。左にも少し足す
        digits(stc) * digit_width(stc) + stc.from_dip_int(7)
    } else {
        0
    };
    // 左に何も無いときは境界線も出さない
    let separator = if strip + numbers > 0 { 1 } else { 0 };
    stc.set_margin_width(MARGIN_CHANGES, strip);
    stc.set_margin_width(MARGIN_LINENUMBERS, numbers);
    stc.set_margin_width(MARGIN_SEPARATOR, separator);
    // 強調オフのときはマーカーを描かない記号にする。幅 0 のマージンにしか割り当てられていない
    // マーカーは Scintilla が行全体の背景色として描いてしまうため、幅 0 にするだけでは足りない
    let symbol = if strip > 0 {
        MarkerSymbol::FullRect
    } else {
        MarkerSymbol::Empty
    };
    for (marker, colour) in [
        (MARKER_MODIFIED, colors().modified),
        (MARKER_SAVED, colors().saved),
    ] {
        stc.marker_define_symbol(marker, symbol, rgb(colour), rgb(colour));
    }
    update_wrap_margin(stc, settings);
}

/// 「指定文字数で折り返し」のために本文の右余白を調整する。Scintilla は本文領域の幅で
/// 折り返すので、エディタの幅から文字数ぶんを引いた残りを右余白にする (幅が足りなければ
/// そのままエディタの右端で折り返す)。文字幅は数字 "0" の幅 (プロポーショナルフォントでは目安)。
/// エディタの幅・マージン幅・ズームが変わるたびに呼ぶ
pub fn update_wrap_margin(stc: &StyledTextCtrl, settings: &Settings) {
    let right = if settings.editor.wrap == Wrap::Column {
        let unit = scintilla::text_width(stc, STYLE_DEFAULT, "0").max(1);
        let text_start: i32 = (MARGIN_CHANGES..=MARGIN_SEPARATOR)
            .map(|m| scintilla::margin_width(stc, m))
            .sum::<i32>()
            + scintilla::margin_left(stc);
        // 半文字ぶんの余裕: Scintilla は文字の右端が幅に達すると手前で折り返す
        let want = unit * settings.editor.wrap_column.clamp(10, 1000) + unit / 2;
        (stc.get_client_size().width - text_start - want).max(1)
    } else {
        1
    };
    scintilla::set_margin_right(stc, right);
}

/// 左側マージンの行番号文字列を全行付け直す (編集した行は太字)。
/// テキストマージンの文字列は行の挿入・削除でずれるので、変更のたびに呼ぶ。
pub fn refresh_line_numbers(stc: &StyledTextCtrl, settings: &Settings) {
    if !settings.editor.line_numbers.left() {
        return;
    }
    // 大きなファイルでは Scintilla 自身の行番号マージンにしてある (全行の文字列を付け直さない)
    if scintilla::margin_type(stc, MARGIN_LINENUMBERS) == scintilla::SC_MARGIN_NUMBER {
        return;
    }
    let highlight = settings.editor.highlight_changes;
    for line in 0..stc.get_line_count() {
        let style = match line_change(stc, line) {
            LineChange::Modified if highlight => STYLE_MARGIN_MODIFIED,
            LineChange::Saved if highlight => STYLE_MARGIN_SAVED,
            _ => STYLE_MARGIN,
        };
        scintilla::margin_set_text(stc, line, &(line + 1).to_string());
        scintilla::margin_set_style(stc, line, style);
    }
}

/// 大きなファイル向けの表示に切り替える: 行番号は Scintilla 自身の Number マージンで描く
/// (行ごとの文字列・スタイルを付け直す O(行数) の処理をしない)。編集行の強調は付かない
pub fn set_large_mode(stc: &StyledTextCtrl, large: bool) {
    let want = if large {
        MarginType::Number
    } else {
        MarginType::RText
    };
    let is_number = scintilla::margin_type(stc, MARGIN_LINENUMBERS) == scintilla::SC_MARGIN_NUMBER;
    if is_number != large {
        stc.set_margin_type_typed(MARGIN_LINENUMBERS, want);
    }
}

/// 行の編集状態
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineChange {
    Unchanged,
    /// 編集してから保存した
    Saved,
    /// 編集して、まだ保存していない (保存済みの印より優先)
    Modified,
}

pub fn line_change(stc: &StyledTextCtrl, line: i32) -> LineChange {
    let markers = stc.marker_get(line);
    if markers & MARKER_MODIFIED_MASK != 0 {
        LineChange::Modified
    } else if markers & MARKER_SAVED_MASK != 0 {
        LineChange::Saved
    } else {
        LineChange::Unchanged
    }
}

/// 開いてから編集して、まだ保存していない行か
pub fn is_modified(stc: &StyledTextCtrl, line: i32) -> bool {
    stc.marker_get(line) & MARKER_MODIFIED_MASK != 0
}

/// 保存した: 未保存の編集行の印を、設定に従って「保存済み」の印に替えるか消す
pub fn mark_changes_saved(stc: &StyledTextCtrl, settings: &Settings) {
    let keep = settings.editor.saved_changes == SavedChanges::Keep;
    let mut line = stc.marker_next(0, MARKER_MODIFIED_MASK);
    while line >= 0 {
        stc.marker_delete(line, MARKER_MODIFIED);
        if keep {
            stc.marker_add(line, MARKER_SAVED);
        }
        line = stc.marker_next(line + 1, MARKER_MODIFIED_MASK);
    }
}

/// 編集行の印を消す。`unsaved` で未保存の編集行の印、`saved` で保存済みの印
pub fn clear_change_marks(stc: &StyledTextCtrl, unsaved: bool, saved: bool) {
    if unsaved {
        stc.marker_delete_all(MARKER_MODIFIED);
    }
    if saved {
        stc.marker_delete_all(MARKER_SAVED);
    }
}

/// 編集した行の追跡
///
/// 変更イベントには位置情報が付いてこない (wxDragon が公開していない) ので、次のように推定する。
///
/// - 挿入 (全文が長くなった): Scintilla は挿入の通知をキャレットを動かす前に出すので、
///   通知時点のキャレット位置が挿入位置。そこから増えたバイト数ぶん進んだ位置までの行に
///   マーカーを付ける。ただし挿入が改行で終わるとき (Enter など) は、その次の行は
///   「改行でできただけ」なので含めない。
/// - 削除 (全文が短くなった): 前回の全文と行単位のハッシュで比べ、共通の先頭・末尾を除いた
///   範囲 (行が減っただけならつなぎ目の行) にマーカーを付ける。
///
/// マーカーは Scintilla が行の挿入・削除に合わせて動かしてくれる。
/// 全文が開いた (または最後に保存した) ときの内容に戻ったら未保存のマーカーをすべて消す。
pub struct ChangeTracker {
    original: u64,
    lines: Vec<u64>,
    len: usize,
}

impl ChangeTracker {
    pub fn new(text: &str) -> Self {
        Self {
            original: hash_of(text),
            lines: line_hashes(text),
            len: text.len(),
        }
    }

    /// 保存した: 今の全文を「元の状態」にする (ここへ戻れば未保存の編集行の印が消える)
    pub fn rebase(&mut self, text: &str) {
        self.original = hash_of(text);
        self.lines = line_hashes(text);
        self.len = text.len();
    }

    /// 編集後の全文を渡し、変わった行にマーカーを付ける
    pub fn update(&mut self, stc: &StyledTextCtrl, text: &str) {
        let new = line_hashes(text);
        if hash_of(text) == self.original {
            stc.marker_delete_all(MARKER_MODIFIED);
            self.lines = new;
            self.len = text.len();
            return;
        }
        if text.len() == self.len && new == self.lines {
            // 中身の変わらない通知 (念のため)。何も付けない
            return;
        }
        let caret_pos = stc.get_current_pos().max(0) as usize;
        let (first, last) = if text.len() > self.len && caret_pos <= self.len {
            inserted_range(text, caret_pos, text.len() - self.len)
        } else {
            let caret_line = stc.get_current_line().max(0) as usize;
            changed_range(&self.lines, &new, caret_line)
        };
        for line in first..last {
            if !is_modified(stc, line as i32) {
                stc.marker_add(line as i32, MARKER_MODIFIED);
            }
        }
        self.lines = new;
        self.len = text.len();
    }
}

/// 位置 `start` に `len` バイト挿入した後のテキスト `text` で、マーカーを付ける行の範囲
/// [first, last) を返す。
///
/// 規則は「挿入前と比べて内容 (改行文字を除く) が変わった行、または新しくできた行」。
/// EmEditor と同じで、改行文字そのものを入れたことは変更に数えない:
/// - 行末で Enter: 元の行はそのまま (印なし)、新しくできた次の行に印
/// - 行頭で Enter: 新しくできた空行に印。下に送られた元の行はそのまま
/// - 行の途中で Enter: 前半も後半も元の行とは内容が違うので両方に印
/// - 空行で Enter: どちらの空行が新しいかは決められないので、キャレットが移る下の行に印
/// - 文字を入力・貼り付け: 内容が入った行すべて
fn inserted_range(text: &str, start: usize, len: usize) -> (usize, usize) {
    let end = (start + len).min(text.len());
    let start = start.min(end);
    let first = line_of(text, start);
    let last = line_of(text, end);
    if first == last {
        return (first, first + 1);
    }
    // 挿入前にこの位置にあった行 (挿入範囲を抜いた 1 行) の内容
    let line_start = text[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end = text[end..]
        .find('\n')
        .map(|i| end + i)
        .unwrap_or(text.len());
    let old_line = format!("{}{}", &text[line_start..start], &text[end..line_end]);
    let old_line = old_line.trim_end_matches('\r');
    // 挿入後の最初の行 (挿入した文字列の最初の改行まで) と最後の行 (最後の改行から行末まで)
    let first_nl = start + text[start..end].find('\n').unwrap_or(0);
    let first_line = text[line_start..first_nl].trim_end_matches('\r');
    let last_start = text[..end].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let last_line = text[last_start..line_end].trim_end_matches('\r');
    let first_changed = first_line != old_line;
    let last_changed = last_line != old_line;

    let mut lo = if first_changed { first } else { first + 1 };
    let mut hi = if last_changed { last + 1 } else { last };
    if lo >= hi {
        // 空行で Enter など、どの行も内容は変わっていない: キャレットが移る行に印
        lo = last;
        hi = last + 1;
    }
    (lo, hi)
}

/// バイトオフセットが属する 0 始まりの行番号
fn line_of(text: &str, offset: usize) -> usize {
    text.as_bytes()[..offset.min(text.len())]
        .iter()
        .filter(|b| **b == b'\n')
        .count()
}

pub fn hash_of<T: Hash + ?Sized>(value: &T) -> u64 {
    let mut h = DefaultHasher::new();
    value.hash(&mut h);
    h.finish()
}

/// 行ごとのハッシュ。Scintilla と同じく "\n" で行を数え、CRLF の "\r" は無視する
fn line_hashes(text: &str) -> Vec<u64> {
    text.split('\n')
        .map(|l| hash_of(l.strip_suffix('\r').unwrap_or(l)))
        .collect()
}

/// 旧→新で変わった行の範囲 [first, last) を新しい行番号で返す。
/// 行が減っただけのときはつなぎ目の 1 行を返す。
///
/// 同じ内容の行 (空行など) が並んでいると、どの行が挿入・削除されたかは差分からは決まらない。
/// 共通の先頭を貪欲に取ると変更範囲が実際より下に寄るので、範囲を同じ内容の行の上でずらして
/// キャレットのある行 (= 編集した場所) に近づける。
fn changed_range(old: &[u64], new: &[u64], caret: usize) -> (usize, usize) {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let max_suffix = old.len().min(new.len()) - prefix;
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(max_suffix)
        .take_while(|(a, b)| a == b)
        .count();
    let mut first = prefix;
    let mut last = new.len() - suffix;
    // 旧側の範囲は [first, old_last)。新旧の範囲を同時に 1 行ずらせるのは、
    // 範囲の直前の行と範囲の最後の行が新旧とも同じ内容のとき
    let mut old_last = old.len() - suffix;
    while first > 0 && first > caret {
        let same_new = last <= first || new[first - 1] == new[last - 1];
        let same_old = old_last <= first || old[first - 1] == old[old_last - 1];
        if !(same_new && same_old) {
            break;
        }
        first -= 1;
        last -= 1;
        old_last -= 1;
    }
    // 範囲が空 (削除のみ) のときはつなぎ目 first の 1 行とみなす
    while caret >= last.max(first + 1) && last < new.len() && old_last < old.len() {
        let same_new = last <= first || new[first] == new[last];
        let same_old = old_last <= first || old[first] == old[old_last];
        if !(same_new && same_old) {
            break;
        }
        first += 1;
        last += 1;
        old_last += 1;
    }
    if first >= last {
        let line = first.min(new.len().saturating_sub(1));
        (line, line + 1)
    } else {
        (first, last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(lines: &[&str]) -> Vec<u64> {
        lines.iter().map(|l| hash_of(*l)).collect()
    }

    #[test]
    fn single_line_edit() {
        assert_eq!(
            changed_range(&h(&["a", "b", "c"]), &h(&["a", "B", "c"]), 1),
            (1, 2)
        );
    }

    #[test]
    fn split_line_marks_both_halves() {
        assert_eq!(
            changed_range(&h(&["ab", "c"]), &h(&["a", "b", "c"]), 0),
            (0, 2)
        );
    }

    #[test]
    fn pure_deletion_marks_join_line() {
        assert_eq!(
            changed_range(&h(&["a", "b", "c"]), &h(&["a", "c"]), 1),
            (1, 2)
        );
        assert_eq!(changed_range(&h(&["a", "b"]), &h(&["a"]), 0), (0, 1));
    }

    #[test]
    fn append_at_end() {
        assert_eq!(
            changed_range(&h(&["a", ""]), &h(&["a", "x", ""]), 1),
            (1, 2)
        );
    }

    #[test]
    fn inserted_blank_line_is_attributed_to_the_caret() {
        // 空行の直後で Enter: 貪欲な共通接頭辞では 2 行目 (idx 2) が新しい行に見えるが、
        // 実際に挿入されたのはキャレット (idx 0) の直後の idx 1
        let old = h(&["text", "", "next"]);
        let new = h(&["text", "", "", "next"]);
        assert_eq!(changed_range(&old, &new, 0), (1, 2));
        // キャレットが下にあれば下の空行
        assert_eq!(changed_range(&old, &new, 2), (2, 3));
    }

    #[test]
    fn enter_at_end_of_line_marks_the_new_line() {
        // "abc" の末尾で Enter (CRLF): "abc" はそのまま、新しくできた 1 行目に印
        let text = "abc\r\n";
        assert_eq!(inserted_range(text, 3, 2), (1, 2));
        // 続けて空行で Enter: 空行同士は区別できないのでキャレットが移る 2 行目
        let text = "abc\r\n\r\n";
        assert_eq!(inserted_range(text, 5, 2), (2, 3));
        // 最終行 (改行無し) の末尾で Enter
        let text = "a\nbc\n";
        assert_eq!(inserted_range(text, 4, 1), (2, 3));
    }

    #[test]
    fn enter_in_the_middle_marks_both_halves() {
        // "abcd" の 2 文字目で Enter → "ab" と "cd" の両方が元の行とは違う
        let text = "ab\r\ncd";
        assert_eq!(inserted_range(text, 2, 2), (0, 2));
        // 行頭で Enter → 新しい空行 (0 行目) に印。下に送られた "abc" はそのまま
        let text = "\r\nabc";
        assert_eq!(inserted_range(text, 0, 2), (0, 1));
        // 2 行目の行頭で Enter
        let text = "x\n\nabc";
        assert_eq!(inserted_range(text, 2, 1), (1, 2));
    }

    #[test]
    fn typing_and_pasting() {
        assert_eq!(inserted_range("hello", 4, 1), (0, 1));
        // "a\nz" の 1 行目末尾に "x\ny" を貼り付け: 2 行とも内容が入った
        assert_eq!(inserted_range("ax\ny\nz", 1, 3), (0, 2));
        // "a\nz" の 1 行目末尾に "x\n" を貼り付け: "ax" が変わり、次の空行は新しくできた行
        assert_eq!(inserted_range("ax\n\nz", 1, 2), (0, 2));
        // "a\nz" の 1 行目末尾に "\ny" を貼り付け: "a" はそのまま、"y" が新しい行
        assert_eq!(inserted_range("a\ny\nz", 1, 2), (1, 2));
        // 行頭に 2 行をまるごと貼り付け: その 2 行だけ ("old" は下に送られただけ)
        assert_eq!(inserted_range("p\nq\nold", 0, 4), (0, 2));
    }

    #[test]
    fn deleted_blank_line_is_attributed_to_the_caret() {
        let old = h(&["text", "", "", "", "next"]);
        let new = h(&["text", "", "", "next"]);
        assert_eq!(changed_range(&old, &new, 1), (1, 2));
        assert_eq!(changed_range(&old, &new, 2), (2, 3));
    }

    #[test]
    fn crlf_is_ignored() {
        assert_eq!(line_hashes("a\r\nb"), line_hashes("a\nb"));
        assert_eq!(line_hashes("a\n").len(), 2);
    }
}

/// 全文にスタイルを付ける。Typst は typst-syntax で解析して葉ノードごとに色分けし、
/// それ以外の種別は既定スタイルに戻す (Markdown のハイライトは未実装)。
/// Scintilla の位置は UTF-8 バイト単位、typst-syntax の span もバイト単位なので一致する。
pub fn restyle(stc: &StyledTextCtrl, text: &str, filetype: FileType) {
    if filetype != FileType::Typ {
        stc.start_styling(0);
        stc.set_styling(text.len() as i32, 0);
        return;
    }
    let root = typst_syntax::parse(text);
    let mut runs: Vec<(usize, i32)> = Vec::new();
    collect_runs(&LinkedNode::new(&root), 0, &mut runs);

    stc.start_styling(0);
    for (len, style) in runs {
        stc.set_styling(len as i32, style);
    }
}

fn collect_runs(node: &LinkedNode, inherited: i32, runs: &mut Vec<(usize, i32)>) {
    let style = highlight(node).map(style_of).unwrap_or(inherited);
    let mut children = node.children().peekable();
    if children.peek().is_none() {
        let len = node.len();
        if len == 0 {
            return;
        }
        // 同じスタイルが続く場合はまとめる
        if let Some(last) = runs.last_mut() {
            if last.1 == style {
                last.0 += len;
                return;
            }
        }
        runs.push((len, style));
    } else {
        for child in children {
            collect_runs(&child, style, runs);
        }
    }
}

/// キャレット前後の括弧を探し、対応する括弧を強調する
pub fn update_brace_highlight(stc: &StyledTextCtrl) {
    fn is_brace(c: i32) -> bool {
        matches!(c, 0x28 | 0x29 | 0x5b | 0x5d | 0x7b | 0x7d) // ( ) [ ] { }
    }
    let pos = stc.get_current_pos();
    let brace_pos = if pos > 0 && is_brace(stc.get_char_at(pos - 1)) {
        Some(pos - 1)
    } else if is_brace(stc.get_char_at(pos)) {
        Some(pos)
    } else {
        None
    };
    match brace_pos {
        Some(p) => {
            let m = stc.brace_match(p);
            if m >= 0 {
                stc.brace_highlight(p, m);
            } else {
                stc.brace_bad_light(p);
            }
        }
        None => stc.brace_highlight(-1, -1),
    }
}
