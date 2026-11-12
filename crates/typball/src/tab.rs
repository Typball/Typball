//! 1 タブ分のページ: エディタと、その左右・下に置けるサイドペイン (アウトライン / プレビュー / コンソール)
//!
//! ```text
//! page (Panel)                               ページ置き場 (window.rs) の子。選択中のものだけ見える
//! ├ strips[Left] (Strip)                     折りたたんだペインの細いバー (無ければ隠れる)
//! ├ v0 (SplitterWindow)
//! │  ├ h1 (SplitterWindow)
//! │  │  ├ slots[Left] (SplitterWindow)       左に置いたペイン (1 つなら initialize、2 つなら上下に分割)
//! │  │  └ h2 (SplitterWindow)
//! │  │     ├ v1 (SplitterWindow)
//! │  │     │  ├ editor_box (Panel)
//! │  │     │  │  ├ actionbar (ActionBar)     設定 [filetypes.*] show_actionbar
//! │  │     │  │  ├ findbar (FindBar)         検索 / 置換バー (検索メニューで出す。普段は隠れている)
//! │  │     │  │  ├ ruler  (Ruler)            設定 show_ruler
//! │  │     │  │  └ hbox: stc | gutter        設定 line_numbers = right / both
//! │  │     │  └ slots[Bottom] (SplitterWindow)  下に置いたペイン (エディタ幅。[panes] bottom_width = "editor")
//! │  │     └ slots[Right] (SplitterWindow)   右に置いたペイン
//! │  └ wide_bottom (SplitterWindow)          下に置いたペイン (ウィンドウ幅。bottom_width = "window")
//! ├ strips[Right]
//! └ strips[Bottom]
//! ```
//!
//! 下の置き場は 2 つ用意してあり (v1 の中と v0 の中)、設定 `bottom_width` で使う方を選ぶ。
//! 切り替えるときは下にあるペインをいったん外して新しい方に入れ直す (`relayout_panes`)。
//!
//! ウィンドウの大きさが変わったときは、ペインの幅 (高さ) を保ってエディタ側が伸び縮みする。
//! wx の SplitterWindow は左 (上) 側を保つのが既定で gravity の設定が wxDragon に無いので、
//! 右・下の置き場はサイズイベントで分割位置を置き直して同じ動きにしている。
//! プレビューの組版はタイマーで 1 回にまとめてから行う (作った直後のサイズ変化のたびに
//! コンパイルすると新しいタブが出るまで待たされる)。
//!
//! ペイン 1 つの状態は 非表示 / 折りたたみ (端のバーに名前だけ) / ドッキング / 浮動 (別ウィンドウ) の
//! 4 つで、ヘッダのボタン・メニュー・ドラッグと表示メニューで切り替える。置く場所 (左 / 右 / 下) は
//! 設定 `[panes]` で、ヘッダのメニューから変えると全ウィンドウの全タブに反映される。
//! wxDragon に Reparent が無いので、場所を変えるときや切り離すときは中身 (ListBox / ScrolledWindow)
//! を作り直す。テキストの持ち主は Scintilla。ファイルのパスや種別は [`Document`] が持つ。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wxdragon::prelude::*;

use crate::actionbar::{self, ActionBar};
use crate::actions::Action;
#[cfg(windows)]
use crate::dnd_win;
use crate::document::Document;
use crate::editmenu::{self, EditTarget};
use crate::editor::{self, ChangeTracker};
use crate::fileio::{EolCounts, LineEnding};
use crate::filetype::FileType;
use crate::findbar::FindBar;
use crate::gutter::{RightGutter, Ruler};
use crate::i18n::{tr, tr_args};
#[cfg(windows)]
use crate::ime_win;
use crate::outline::{self, OutlineItem};
use crate::pane::{self, DropIndicator, FloatingPane, HeaderButton, PaneBox, Strip};
use crate::preview::TypstView;
use crate::scintilla;
use crate::settings::{self, BottomWidth, PanePosition, PaneSizes, Settings};
use crate::tabbar::{TabBar, TabItem};
use crate::webpreview::{self, WebPreview};

/// 置き場の最小の大きさ (DIP)
const MIN_PANE: i32 = 80;
/// プレビューを組み直すまでの待ち時間 (ms)。連続する変更とサイズイベントをまとめる
const RENDER_DELAY_MS: i32 = 20;
/// ペインのヘッダのメニューの ID: PANE_MENU_BASE + ペイン番号 * 8 + 項目
const PANE_MENU_BASE: i32 = wxdragon::id::ID_HIGHEST + 200;

/// ステータスバーの欄: 0 = メッセージ, 1 = 種別, 2 = 行・桁, 3 = 文字数, 4 = 文字コード, 5 = 改行コード。
/// 幅は DIP (-1 は残り全部)。ウィンドウを作った直後の幅で、文書を表示したら中身に合わせて右に詰める
/// ([`EolField::fit`])
pub const STATUS_FIELD_WIDTHS: [i32; 6] = [-1, 80, 150, 110, 130, 180];
/// 改行コードの欄 ([`EolField`] が文字を重ねて描く)
const STATUS_EOL_FIELD: usize = 5;
/// 欄の文字の左右に空ける幅 (DIP)。欄を中身の幅に詰めるときに足す
const STATUS_FIELD_PAD: i32 = 10;
/// 右下のサイズ変更のつまみのぶん、最後の欄に足す幅 (DIP)
const STATUS_GRIP: i32 = 14;

/// ステータスバーの改行コードの欄と、欄の幅の管理。
///
/// 改行コードは "CRLF (Windows)" のように使う OS を添えて出し、新規文書の改行コード (設定。初期値は
/// この OS の標準) と違う文書では太字にする。改行コードが混ざっている文書では「混在 (CRLF / LF)」を
/// 太字と警告の色で出す。wxStatusBar は欄ごとにフォントや色を変えられないので、欄の上に文字の
/// コントロールを重ねて描く。
///
/// 種別から右の欄は中身の幅に詰めて右に寄せ、左のメッセージの欄を広く空ける ([`EolField::fit`])
#[derive(Clone)]
pub struct EolField {
    status: StatusBar,
    label: StaticText,
    /// 今出している内容 (同じなら何もしない: キャレットが動くたびに呼ばれる)
    shown: Rc<Cell<Option<EolShown>>>,
    /// 欄 1..=4 の幅を決める文字 (拡大率が変わったら同じ文字で測り直す)
    measured: Rc<RefCell<Vec<String>>>,
    /// 今の欄の幅 (実ピクセル。空ならウィンドウを作った直後の幅のまま)
    widths: Rc<RefCell<Vec<i32>>>,
}

/// 欄の表示を決めるもの
#[derive(Clone, Copy, PartialEq, Eq)]
struct EolShown {
    /// これから入力する改行
    eol: LineEnding,
    /// 新規文書の改行コード (これと違えば太字)
    base: LineEnding,
    /// 文書で使われている改行コード: [CRLF, LF, CR] があるか (2 つ以上なら混在)
    used: [bool; 3],
    /// 警告の色は配色 (ライト / ダーク) で変える
    dark: bool,
}

/// 混在の警告の文字色 (ステータスバーの背景の上で読める濃さ)
const EOL_WARNING_LIGHT: (u8, u8, u8) = (0xb4, 0x53, 0x09);
const EOL_WARNING_DARK: (u8, u8, u8) = (0xfb, 0xbf, 0x24);

impl EolField {
    pub fn new(status: &StatusBar) -> Self {
        let field = EolField {
            status: *status,
            label: StaticText::builder(status).with_label("").build(),
            shown: Rc::new(Cell::new(None)),
            measured: Rc::new(RefCell::new(Vec::new())),
            widths: Rc::new(RefCell::new(Vec::new())),
        };
        // ステータスバーの大きさが変わると欄の位置も動く
        status.on_size({
            let field = field.clone();
            move |event| {
                field.place();
                event.skip(true);
            }
        });
        field
    }

    /// 欄に文書の改行コードを出す。`eol` はこれから入力する改行、`counts` は文書の中の改行の数
    pub fn set(&self, eol: LineEnding, counts: &EolCounts) {
        let kinds = counts.kinds();
        let base = settings::get().files.line_ending();
        let shown = EolShown {
            eol,
            base,
            used: LineEnding::ALL.map(|e| kinds.contains(&e)),
            dark: crate::theme::ui_dark(),
        };
        if self.shown.replace(Some(shown)) == Some(shown) {
            return;
        }
        let mixed = counts.mixed();
        let differs = eol != base;
        if let Some(font) = self.status.get_font() {
            let mut font = font.to_owned();
            font.set_weight(if mixed || differs {
                FontWeight::Bold
            } else {
                FontWeight::Normal
            });
            self.label.set_font(&font);
        }
        let name = tr(eol.label_key());
        let (text, tip) = if mixed {
            let used = kinds
                .iter()
                .map(|e| e.short_label())
                .collect::<Vec<_>>()
                .join(" / ");
            (
                tr_args("status.eol_mixed", &[("kinds", &used)]),
                tr_args("status.eol_tip_mixed", &[("kinds", &used), ("eol", &name)]),
            )
        } else if differs {
            let tip = tr_args(
                "status.eol_tip_differs",
                &[("eol", &name), ("base", base.short_label())],
            );
            (name, tip)
        } else {
            let tip = tr_args("status.eol_tip", &[("eol", &name)]);
            (name, tip)
        };
        let colour = if mixed {
            let (r, g, b) = if shown.dark {
                EOL_WARNING_DARK
            } else {
                EOL_WARNING_LIGHT
            };
            Colour::new(r, g, b, 255)
        } else {
            self.status.get_foreground_color()
        };
        self.label.set_foreground_color(colour);
        self.label.set_label(&text);
        self.label.set_tooltip(&tip);
        // 文字の幅が変わったので、欄の幅も合わせ直す (その中で文字も置き直す)
        self.widths.borrow_mut().clear();
        self.apply_widths();
        self.label.refresh(true, None);
    }

    /// 欄 1..=4 (種別・行桁・文字数・文字コード) の幅を `measured` の文字が収まる幅にして、欄を右に詰める。
    /// 行桁と文字数は、数字の桁が増えるたびに欄が動かないように、桁を多めに取った文字で測る
    pub fn fit(&self, measured: Vec<String>) {
        if *self.measured.borrow() != measured {
            *self.measured.borrow_mut() = measured;
            self.apply_widths();
        }
    }

    /// 拡大率が変わった (ウィンドウが別のモニターへ移った): 同じ文字で幅を測り直す
    pub fn rescale(&self) {
        self.widths.borrow_mut().clear();
        self.apply_widths();
    }

    fn apply_widths(&self) {
        let measured = self.measured.borrow();
        if measured.is_empty() {
            // まだ文書を表示していない: ウィンドウを作った直後の幅のまま
            self.place();
            return;
        }
        let pad = self.status.from_dip_int(STATUS_FIELD_PAD);
        let mut widths = vec![-1];
        widths.extend(
            measured
                .iter()
                .map(|text| self.status.get_text_extent(text).width + 2 * pad),
        );
        // 最後の欄は重ねた文字の幅 + 余白 + 右下のつまみ
        widths.push(
            self.label.get_best_size().width + 2 * pad + self.status.from_dip_int(STATUS_GRIP),
        );
        if *self.widths.borrow() != widths {
            self.status.set_status_widths(&widths);
            *self.widths.borrow_mut() = widths;
        }
        drop(measured);
        self.place();
    }

    /// 文字を欄の中に置く (左に少し空け、縦は中央)
    fn place(&self) {
        // 欄がステータスバーに収まっていない (ウィンドウがとても狭い) ときは、位置を聞けないので文字も出さない
        let needed: i32 = if self.widths.borrow().is_empty() {
            status_widths(&self.status).iter().filter(|w| **w > 0).sum()
        } else {
            self.widths.borrow().iter().filter(|w| **w > 0).sum()
        };
        let fits = self.status.get_size().width >= needed;
        self.label.show(fits);
        if !fits {
            return;
        }
        let rect = self.status.get_field_rect(STATUS_EOL_FIELD);
        let best = self.label.get_best_size();
        // ステータスバー自身が描く欄の文字は、欄の左端から 3px ほどのところに出る。それに合わせる
        let pad = self.status.from_dip_int(3);
        // 隣の欄 (ステータスバー自身が描く文字) と高さがそろうように、中央より少し下に置く
        self.label.set_size_with_pos(
            rect.x + pad,
            rect.y + (rect.height - best.height + 1) / 2 + self.status.from_dip_int(1),
            best.width.min((rect.width - pad).max(0)),
            best.height,
        );
    }
}

/// ウィンドウを作った直後のステータスバーの欄の幅 (実ピクセル)
pub fn status_widths<W: WxWidget>(window: &W) -> Vec<i32> {
    STATUS_FIELD_WIDTHS
        .iter()
        .map(|w| if *w > 0 { window.from_dip_int(*w) } else { *w })
        .collect()
}

/// タブを入れる先 (ウィンドウ側の部品)
#[derive(Clone)]
pub struct TabHost {
    /// ページの置き場。ページの表示・非表示とサイズは window.rs が管理する
    pub pages: Panel,
    pub tabbar: TabBar,
    pub frame: Frame,
    pub status: StatusBar,
    /// ステータスバーの改行コードの欄
    pub eol: EolField,
    /// ペインの表示状態が変わったとき (表示メニューのチェックを合わせる)
    pub on_panes_changed: Rc<dyn Fn()>,
    /// ペインの配置設定が変わったとき (全ウィンドウのタブを並べ直す)
    pub on_layout_changed: Rc<dyn Fn()>,
    /// アクションバーのボタンが押されたとき (メニューの「アクション」と同じ処理へ)
    pub on_action: Rc<dyn Fn(Action)>,
    /// エディタにファイルがドロップされたとき (開けたら true)
    pub on_drop_files: Rc<dyn Fn(Vec<String>) -> bool>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaneState {
    Hidden,
    /// 端のバーに名前だけ出ている
    Collapsed,
    Docked,
    Floating,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pane {
    Outline,
    Preview,
    /// Typst のコンパイル結果 (診断)・Markdown の変換結果・開発サーバーの出力
    Console,
}

fn new_splitter<W: WxWidget>(parent: &W) -> SplitterWindow {
    let s = SplitterWindow::builder(parent)
        .with_style(SplitterWindowStyle::LiveUpdate | SplitterWindowStyle::Default)
        .build();
    s.set_minimum_pane_size(parent.from_dip_int(MIN_PANE));
    s
}

/// 分割方向に沿ったスプリッタの長さ (左右なら幅、下なら高さ)
fn splitter_extent(splitter: &SplitterWindow, position: PanePosition) -> i32 {
    let size = splitter.get_client_size();
    if position.is_side() {
        size.width
    } else {
        size.height
    }
}

impl Pane {
    pub const ALL: [Pane; 3] = [Pane::Outline, Pane::Preview, Pane::Console];

    fn index(self) -> usize {
        match self {
            Pane::Outline => 0,
            Pane::Preview => 1,
            Pane::Console => 2,
        }
    }

    pub fn title_key(self) -> &'static str {
        match self {
            Pane::Outline => "pane.outline",
            Pane::Preview => "pane.preview",
            Pane::Console => "pane.console",
        }
    }

    pub fn available(self, filetype: FileType) -> bool {
        match self {
            Pane::Outline => filetype.has_outline(),
            Pane::Preview => filetype.has_preview(),
            Pane::Console => filetype.has_console(),
        }
    }

    pub fn position(self, settings: &Settings) -> PanePosition {
        match self {
            Pane::Outline => settings.panes.outline,
            Pane::Preview => settings.panes.preview,
            Pane::Console => settings.panes.console,
        }
    }
}

/// ペインの中身 (ドッキング側か浮動側のどちらか一方に存在する)
#[derive(Clone)]
enum PaneContent {
    Outline(ListBox),
    /// Typst: ページ画像
    Preview(Rc<TypstView>),
    /// Markdown: WebView (HTML / サイトモード)
    Web(Rc<WebPreview>),
    /// コンソール: 読み取り専用のテキスト
    Console(TextCtrl),
}

#[derive(Clone)]
struct Docked {
    bbox: PaneBox,
    content: PaneContent,
    position: PanePosition,
}

#[derive(Clone)]
struct Floating {
    fp: FloatingPane,
    content: PaneContent,
}

#[derive(Clone)]
struct PaneInfo {
    state: PaneState,
    docked: Option<Docked>,
    floating: Option<Floating>,
}

#[derive(Clone)]
pub struct Tab {
    pub page: Panel,
    pub stc: StyledTextCtrl,
    pub doc: Rc<RefCell<Document>>,
    host: TabHost,
    /// 下の置き場をウィンドウ幅にするときの分割 (h1 / wide_bottom)
    v0: SplitterWindow,
    h1: SplitterWindow,
    h2: SplitterWindow,
    v1: SplitterWindow,
    /// 左 / 右 / 下の置き場 (PanePosition::index の順)。下はエディタ幅のもの
    slots: [SplitterWindow; 3],
    /// 下の置き場 (ウィンドウ幅)。`bottom_wide` のときはこちらを使う
    wide_bottom: SplitterWindow,
    /// 下の置き場が今ウィンドウ幅か (設定 `[panes] bottom_width` を反映した状態)
    bottom_wide: Rc<Cell<bool>>,
    strips: [Strip; 3],
    editor_box: Panel,
    actionbar: ActionBar,
    findbar: FindBar,
    ruler: Ruler,
    gutter: RightGutter,
    panes: Rc<RefCell<[PaneInfo; 3]>>,
    /// 置き場の幅 (左 / 右) と高さ (下)。分割位置を動かすと更新して形式ごとの設定
    /// (`[filetypes.*] pane_sizes`) に保存し、置き直すときと同じ形式の新しいタブで使う。
    /// 単位は DIP (拡大率 100% 換算)。実ピクセルのまま覚えると、拡大率の違うモニターへ
    /// ウィンドウを移したときに右・下の置き場が元の見た目より狭く (広く) なる
    pane_sizes: Rc<RefCell<[i32; 3]>>,
    /// プレビューの組版をまとめて後回しにするための単発タイマー
    render_timer: Rc<Timer<Panel>>,
    tracker: Rc<RefCell<ChangeTracker>>,
    items: Rc<RefCell<Vec<OutlineItem>>>,
    /// set_text 中は変更イベントを無視する
    loading: Rc<Cell<bool>>,
    /// 全文の文字数 (変更のたびに数え直す)
    char_count: Rc<Cell<i32>>,
    /// 文書の中の改行コードごとの数 (ステータスバーの混在の警告に使う)
    eol_counts: Rc<Cell<EolCounts>>,
    /// ペインのヘッダをドラッグしている間だけ存在する行き先の表示
    drop_indicator: Rc<RefCell<Option<DropIndicator>>>,
    /// 大きなファイル (設定 `files.large_file_mb` 超): 色分け・編集行の強調・アウトライン・
    /// プレビュー・スナップショットを止めて、変更のたびに全文を取り出さない
    large: Rc<Cell<bool>>,
    /// スナップショット (backup.rs) のファイル名。プロセス内で一意
    backup_id: u64,
    /// 最後のスナップショットの後に編集があった
    backup_dirty: Rc<Cell<bool>>,
    /// スナップショットを書いてある (消すべきものがある)
    backup_written: Rc<Cell<bool>>,
    /// 待つのをやめた保存 (裏で続いている)。終わるまで次の保存はしない
    pending_save: Rc<RefCell<Option<PendingSave>>>,
}

/// 裏で続いている保存
pub struct PendingSave {
    pub receiver: crate::background::Pending<std::io::Result<()>>,
    /// 保存を始めたときの全文のハッシュ。終わったときに同じなら「保存済み」にできる
    pub hash: u64,
    /// 書いている先。書き終えたら文書に反映する
    pub target: crate::document::SaveTarget,
}

thread_local! {
    static NEXT_BACKUP_ID: Cell<u64> = const { Cell::new(1) };
}

fn next_backup_id() -> u64 {
    NEXT_BACKUP_ID.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    })
}

impl Tab {
    /// ページを作ってタブバーとページ集に追加し、選択する。
    /// `modified` が true ならテキストを「未保存の変更あり」として入れる (タブの移動用)。
    /// `index` が None なら末尾に追加する。
    pub fn new(
        host: &TabHost,
        doc: Document,
        text: &str,
        modified: bool,
        index: Option<usize>,
    ) -> Tab {
        let settings = settings::get();
        let filetype = doc.filetype;
        let panes = filetype.panes(&settings);

        // 選ばれるまで見せない (window.rs が置き場いっぱいに広げて表示する)
        let page = Panel::builder(&host.pages).build();
        page.show(false);
        let render_timer = Rc::new(Timer::new(&page));
        let v0 = new_splitter(&page);
        let h1 = new_splitter(&v0);
        let wide_bottom = new_splitter(&v0);
        let slot_left = new_splitter(&h1);
        let h2 = new_splitter(&h1);
        let v1 = new_splitter(&h2);
        let slot_right = new_splitter(&h2);
        let editor_box = Panel::builder(&v1).build();
        let slot_bottom = new_splitter(&v1);
        let slots = [slot_left, slot_right, slot_bottom];
        for s in slots.iter().chain([&wide_bottom]) {
            s.show(false);
        }
        v0.initialize(&h1);
        h1.initialize(&h2);
        h2.initialize(&v1);
        v1.initialize(&editor_box);

        // --- エディタ ---
        let actionbar = ActionBar::new(&editor_box, filetype);
        let stc = StyledTextCtrl::builder(&editor_box).build();
        editor::setup_editor(&stc, &settings);
        // 右クリックは Scintilla の英語のメニューではなく自前のメニュー (bind_events) を出す
        scintilla::disable_popup(&stc);
        let findbar = FindBar::new(&editor_box, stc);
        #[cfg(windows)]
        ime_win::install(&stc);
        // ファイルのドロップはエディタの中でも受ける (ウィンドウの他の部分は window.rs で
        // フレームに付けたドロップ先が受ける)。Windows では Scintilla 自身のテキスト用ドロップ先を
        // 残したままファイルだけ横取りするので、選択文字列をドラッグして動かす操作もそのまま使える。
        // 他の OS では wx のドロップ先と入れ替わる (そちらではテキストのドラッグ移動は効かない)
        #[cfg(windows)]
        dnd_win::install(&stc, host.on_drop_files.clone());
        #[cfg(not(windows))]
        FileDropTarget::builder(&stc)
            .with_on_enter(|_, _, _| DragResult::Copy)
            .with_on_drag_over(|_, _, _| DragResult::Copy)
            .with_on_drop_files({
                let host = host.clone();
                move |files, _x, _y| (host.on_drop_files)(files)
            })
            .build();
        let ruler = Ruler::new(&editor_box, stc);
        let gutter = RightGutter::new(&editor_box, stc);
        actionbar
            .window()
            .show(panes.show_actionbar && filetype.has_actionbar());
        ruler.window().show(settings.editor.show_ruler);
        gutter.window().show(settings.editor.line_numbers.right());

        let hbox = BoxSizer::builder(Orientation::Horizontal).build();
        hbox.add(&stc, 1, SizerFlag::Expand, 0);
        hbox.add(gutter.window(), 0, SizerFlag::Expand, 0);
        let vbox = BoxSizer::builder(Orientation::Vertical).build();
        vbox.add(actionbar.window(), 0, SizerFlag::Expand, 0);
        vbox.add(findbar.window(), 0, SizerFlag::Expand, 0);
        vbox.add(ruler.window(), 0, SizerFlag::Expand, 0);
        vbox.add_sizer(&hbox, 1, SizerFlag::Expand, 0);
        editor_box.set_sizer(vbox, true);

        // --- 折りたたみバーとページ全体の配置 ---
        let strips = [
            Strip::new(&page, PanePosition::Left),
            Strip::new(&page, PanePosition::Right),
            Strip::new(&page, PanePosition::Bottom),
        ];
        let row = BoxSizer::builder(Orientation::Horizontal).build();
        row.add(strips[0].window(), 0, SizerFlag::Expand, 0);
        row.add(&v0, 1, SizerFlag::Expand, 0);
        row.add(strips[1].window(), 0, SizerFlag::Expand, 0);
        let col = BoxSizer::builder(Orientation::Vertical).build();
        col.add_sizer(&row, 1, SizerFlag::Expand, 0);
        col.add(strips[2].window(), 0, SizerFlag::Expand, 0);
        page.set_sizer(col, true);

        let empty = PaneInfo {
            state: PaneState::Hidden,
            docked: None,
            floating: None,
        };
        let tab = Tab {
            page,
            stc,
            doc: Rc::new(RefCell::new(doc)),
            host: host.clone(),
            v0,
            h1,
            h2,
            v1,
            slots,
            wide_bottom,
            bottom_wide: Rc::new(Cell::new(
                settings.panes.bottom_width == BottomWidth::Window,
            )),
            strips,
            editor_box,
            actionbar,
            findbar,
            ruler,
            gutter,
            panes: Rc::new(RefCell::new([empty.clone(), empty.clone(), empty])),
            pane_sizes: Rc::new(RefCell::new(filetype.pane_sizes(&settings).as_array())),
            render_timer,
            tracker: Rc::new(RefCell::new(ChangeTracker::new(""))),
            items: Rc::new(RefCell::new(Vec::new())),
            loading: Rc::new(Cell::new(false)),
            char_count: Rc::new(Cell::new(0)),
            eol_counts: Rc::new(Cell::new(EolCounts::default())),
            drop_indicator: Rc::new(RefCell::new(None)),
            large: Rc::new(Cell::new(false)),
            backup_id: next_backup_id(),
            backup_dirty: Rc::new(Cell::new(modified)),
            backup_written: Rc::new(Cell::new(false)),
            pending_save: Rc::new(RefCell::new(None)),
        };

        tab.bind_events();
        for (i, strip) in tab.strips.iter().enumerate() {
            let _ = i;
            strip.on_click({
                let tab = tab.clone();
                move |id| {
                    if let Some(which) = Pane::ALL.get(id).copied() {
                        tab.expand_pane(which);
                    }
                }
            });
        }
        if panes.show_outline && filetype.has_outline() {
            tab.place(Pane::Outline);
        }
        if panes.show_preview && filetype.has_preview() {
            tab.place(Pane::Preview);
        }
        if panes.show_console && filetype.has_console() {
            tab.place(Pane::Console);
        }
        tab.load_text(text, modified);

        let item = TabItem {
            key: tab.key(),
            title: tab.doc.borrow().title(),
            filetype,
            modified,
        };
        let count = host.tabbar.len();
        let index = index.unwrap_or(count).min(count);
        host.tabbar.insert(index, item);
        host.tabbar.select(index);
        tab.update_title();
        tab.update_status();
        tab
    }

    /// タブバーとページを結び付けるキー (ページのハンドル)
    pub fn key(&self) -> usize {
        self.page.handle_ptr() as usize
    }

    fn bind_events(&self) {
        let stc = self.stc;

        // テキスト変更: 再ハイライト + 編集行マーカー + 行番号 + アウトライン + プレビュー
        stc.on_stc_change({
            let tab = self.clone();
            move |_| {
                if tab.loading.get() {
                    return;
                }
                tab.backup_dirty.set(true);
                if tab.large.get() {
                    // 全文を取り出して付け直す処理 (色分け・編集行・アウトライン・プレビュー) は
                    // しない。タイトルの「*」と行桁だけ
                    tab.update_title();
                    tab.update_status();
                    return;
                }
                let text = stc.get_text();
                tab.tracker.borrow_mut().update(&stc, &text);
                tab.refresh_from_text(&text);
                tab.update_title();
                tab.update_status();
            }
        });
        // キャレット移動・スクロール・ズームなど
        stc.on_stc_update_ui({
            let tab = self.clone();
            move |_| {
                editor::update_brace_highlight(&stc);
                editor::update_margins(&stc, &settings::get());
                tab.refresh_decorations();
                tab.update_status();
                tab.sync_preview_scroll();
                tab.sync_outline_selection();
            }
        });
        // エディタが自分を描いた直後に、同じ描画の中で不可視文字の記号を描き足す (marks.rs)
        crate::marks::on_painted(&stc, move |dc, update, composing| {
            crate::marks::paint(&stc, &settings::get(), dc, update, composing);
        });
        // エディタの大きさが変わったらルーラーと右側行番号も描き直す。
        // 「指定文字数で折り返し」の右余白もエディタの幅で決まる
        stc.on_size({
            let tab = self.clone();
            move |_| {
                // 拡大率の違うモニターへ移ったときもここに来るので、余白などの寸法ごと合わせ直す
                editor::update_margins(&stc, &settings::get());
                tab.refresh_decorations()
            }
        });
        self.actionbar.on_action({
            let host = self.host.clone();
            move |action| (host.on_action)(action)
        });
        self.render_timer.on_tick({
            let tab = self.clone();
            move |_| tab.render_preview()
        });
        // 右クリックメニュー。Scintilla 自身のメニューは止めてある (disable_popup) ので
        // wxEVT_CONTEXT_MENU がエディタから親の editor_box へ伝わってくる。
        // 選ばれた項目はフレームの on_menu (window.rs の handle_menu) に届く
        self.editor_box.on_context_menu({
            let tab = self.clone();
            move |_| {
                // メニューの対象をエディタにそろえる (検索欄にフォーカスがあっても)
                stc.set_focus();
                let mut menu = editmenu::context_menu(&stc, tab.filetype());
                tab.editor_box.popup_menu(&mut menu, None);
            }
        });
        // 検索バーを出し入れしたらエディタ部分を並べ直す
        self.findbar.on_layout({
            let editor_box = self.editor_box;
            move || editor_box.layout()
        });
        // エディタで Esc: 検索バーが出ていれば閉じる。Tab / Shift+Tab / Backspace: タブ位置を表示上の
        // 桁で扱う。
        // それ以外のキーは Scintilla に渡す
        stc.on_key_down({
            let tab = self.clone();
            move |event| {
                if let WindowEventData::Keyboard(key) = &event {
                    if key.get_key_code() == Some(WXK_ESCAPE) && tab.findbar.is_shown() {
                        tab.findbar.hide();
                        return;
                    }
                    // Tab / Shift+Tab: タブ位置を表示上の桁で数える (Scintilla は文字数で数えるので、
                    // 全角文字の後ろではタブ位置がずれる)
                    if key.get_key_code() == Some(WXK_TAB) && !key.control_down() && !key.alt_down()
                    {
                        let handled = if key.shift_down() {
                            tab.back_tab()
                        } else {
                            tab.insert_soft_tab()
                        };
                        if handled {
                            event.skip(false);
                            return;
                        }
                    }
                    // Backspace: キャレットの前のスペースをタブ位置までまとめて消す (設定の動作による)
                    if key.get_key_code() == Some(WXK_BACK)
                        && !key.control_down()
                        && !key.shift_down()
                        && !key.alt_down()
                        && tab.backspace_to_tab_stop()
                    {
                        // Scintilla に渡さない (渡すと、もう 1 文字消される)
                        event.skip(false);
                        return;
                    }
                }
                event.skip(true);
            }
        });
        self.bind_splitters();
    }

    // -----------------------------------------------------------------------
    // 編集メニュー・検索
    // -----------------------------------------------------------------------

    /// 編集メニュー (Ctrl+C など) の相手。フォーカスのあるコントロールで決める:
    /// 検索バーの入力欄 → エディタ → プレビューの WebView (あれば) → それ以外はエディタ
    pub fn edit_target(&self) -> EditTarget {
        if let Some(text) = self.findbar.focused_text() {
            return EditTarget::Text(text);
        }
        if self.stc.has_focus() {
            return EditTarget::Editor(self.stc);
        }
        if let Some(web) = self.active_web_preview().filter(|w| w.webview().is_some()) {
            // WebView2 の中にフォーカスがあるときは wx からは「どこにも無い」ように見えることが
            // あるので、エディタにフォーカスが無ければプレビューを相手にする
            return EditTarget::Web(web);
        }
        EditTarget::Editor(self.stc)
    }

    /// 検索バーを出す (`replace` なら置換の段も)
    pub fn show_find(&self, replace: bool) {
        self.findbar.show(replace);
    }

    /// 次 (前) の一致へ。検索語がまだ無ければバーを出す
    pub fn find_next(&self, forward: bool) {
        self.findbar.find_next(forward);
    }

    /// 指定行 (1 始まり) へ移動してエディタにフォーカスを移す
    pub fn go_to_line(&self, line: i32) {
        let last = self.stc.get_line_count().max(1);
        self.stc.goto_line(line.clamp(1, last) - 1);
        self.stc.ensure_caret_visible();
        self.stc.set_focus();
    }

    /// 置き場のスプリッタ。下は設定により、エディタ幅 (v1 の中) とウィンドウ幅 (v0 の中) のどちらか
    fn slot(&self, position: PanePosition) -> SplitterWindow {
        if position == PanePosition::Bottom && self.bottom_wide.get() {
            self.wide_bottom
        } else {
            self.slots[position.index()]
        }
    }

    /// 置き場を持つスプリッタ (左 = h1, 右 = h2, 下 = v1 かウィンドウ幅なら v0)
    fn outer_splitter(&self, position: PanePosition) -> SplitterWindow {
        match position {
            PanePosition::Left => self.h1,
            PanePosition::Right => self.h2,
            PanePosition::Bottom if self.bottom_wide.get() => self.v0,
            PanePosition::Bottom => self.v1,
        }
    }

    /// 分割位置の記憶と、右・下の置き場の大きさを保つサイズ処理
    fn bind_splitters(&self) {
        // 下の置き場は 2 つあるので両方に付ける (使っていない方は隠れていてイベントが来ない)
        let targets = [
            (PanePosition::Left, self.h1, self.slots[0]),
            (PanePosition::Right, self.h2, self.slots[1]),
            (PanePosition::Bottom, self.v1, self.slots[2]),
            (PanePosition::Bottom, self.v0, self.wide_bottom),
        ];
        for (position, splitter, slot) in targets {
            let sizes = self.pane_sizes.clone();
            // 利用者が分割位置を動かしたら、置き場の大きさとして覚えて形式ごとの設定に保存する
            splitter.on_sash_position_changed({
                let sizes = sizes.clone();
                let tab = self.clone();
                move |event| {
                    let Some(sash) = event.get_sash_position() else {
                        return;
                    };
                    // このイベントは親へ伝わるので、入れ子になった子のスプリッタ (v1 → h2 → h1) の
                    // 分割線を動かしたときにも来る。自分の分割線のものだけ扱う
                    if event.get_id() != splitter.get_id() {
                        event.skip(true);
                        return;
                    }
                    let total = splitter_extent(&splitter, position);
                    let size = match position {
                        PanePosition::Left => sash,
                        PanePosition::Right | PanePosition::Bottom => total - sash,
                    };
                    // 隠れているページにも wx がサッシを詰めるときにこのイベントを送ってくる
                    // (小さい古いサイズに合わせた値) ので、見えているページの操作だけを覚える
                    if size >= splitter.from_dip_int(MIN_PANE) && tab.page.is_shown() {
                        sizes.borrow_mut()[position.index()] = splitter.to_dip_int(size);
                        tab.save_pane_sizes();
                    }
                }
            });
            if position == PanePosition::Left {
                continue;
            }
            // 右・下: スプリッタの大きさが変わったら置き場の大きさを保つ位置に分割線を置き直す。
            // wx 自身のサイズ処理 (SizeWindows) はこの後に走る
            splitter.on_size(move |_| {
                if !slot.is_shown() {
                    return;
                }
                let total = splitter_extent(&splitter, position);
                if total <= 0 {
                    return;
                }
                let want = splitter.from_dip_int(sizes.borrow()[position.index()]);
                let min = splitter.from_dip_int(MIN_PANE);
                splitter.set_sash_position((total - want).max(min), false);
            });
        }
    }

    /// アクション (書式付け) を実行する。書式付け以外は呼び出し側で処理する
    pub fn apply_action(&self, action: Action) {
        if action.is_format() {
            actionbar::apply_format(&self.stc, action, self.filetype());
        }
    }

    // -----------------------------------------------------------------------
    // ペインの中身
    // -----------------------------------------------------------------------

    /// ペインの中身を `parent` の中に作る (ドッキング側・浮動側で共通)
    fn create_content(&self, which: Pane, bbox: &PaneBox) -> PaneContent {
        match which {
            Pane::Outline => {
                let list = ListBox::builder(&bbox.body).build();
                bbox.set_content(&list);
                self.bind_outline_list(list);
                PaneContent::Outline(list)
            }
            Pane::Console => {
                // 普通の EDIT にする (Rich2 = RichEdit にしない)。RichEdit は字の大きさを自分で DPI に
                // 合わせるうえ、wx がモニターとシステムの拡大率の比でズームを掛けるので、拡大率の違う
                // モニターが混ざった環境で字が二重に拡大された (175% のモニターで 1.75 倍)。EDIT なら
                // ほかの部品と同じく wx がフォントをウィンドウの DPI に合わせるだけで済む
                let text = TextCtrl::builder(&bbox.body)
                    .with_style(
                        TextCtrlStyle::MultiLine
                            | TextCtrlStyle::ReadOnly
                            | TextCtrlStyle::DontWrap,
                    )
                    .build();
                // コンソールは UI フォント (設定のフォントはエディタの中身だけ)
                if let Some(font) = crate::theme::ui_font(false, false, 0) {
                    text.set_font(&font);
                }
                text.set_value(&tr("console.hint"));
                bbox.set_content(&text);
                self.bind_console(text);
                PaneContent::Console(text)
            }
            Pane::Preview if self.filetype() == FileType::Md => {
                let path = self.doc.borrow().path.clone();
                let web = WebPreview::new(&bbox.body, self.host.frame, path.as_deref());
                bbox.set_content(&web.container);
                self.bind_web_preview(&web);
                PaneContent::Web(web)
            }
            Pane::Preview => {
                let view = TypstView::new(&bbox.body, self.host.frame);
                bbox.set_content(&view.container);
                PaneContent::Preview(view)
            }
        }
    }

    /// コンソールの行をダブルクリックすると、その行に書かれた `行:桁` の位置へ飛ぶ
    fn bind_console(&self, text: TextCtrl) {
        let stc = self.stc;
        text.on_mouse_left_double_click(move |event| {
            event.skip(true);
            let Some((_, line)) = text.position_to_xy(text.get_insertion_point()) else {
                return;
            };
            let Some((target, _col)) = parse_location(&text.get_line_text(line)) else {
                return;
            };
            stc.goto_line((target - 1).max(0));
            stc.ensure_caret_visible();
            stc.set_focus();
        });
    }

    /// アウトラインの項目を選ぶとその見出しへ飛ぶ。選択の変化 (キーボード操作) に加えて、マウスの
    /// ボタンを離したときにも飛ぶ: 今いる見出しは `sync_outline_selection` で選択済みなので、それを
    /// クリックしても選択変更イベントが来ない (選択済みの項目をクリックして章の先頭に戻る操作のため)
    fn bind_outline_list(&self, list: ListBox) {
        let jump = {
            let tab = self.clone();
            move || {
                let Some(i) = list.get_selection() else {
                    return;
                };
                let line = tab.items.borrow().get(i as usize).map(|it| it.line);
                if let Some(line) = line {
                    tab.jump_to_heading(line as i32);
                }
            }
        };
        list.on_selection_changed({
            let jump = jump.clone();
            move |_| jump()
        });
        list.on_mouse_left_up(move |event| {
            jump();
            event.skip(true);
        });
    }

    /// 見出しの行をエディタの上端に置き、キャレットもそこへ移す。
    /// `goto_line` だけだと「見えるまで最小限スクロール」なので、上から辿ると見出しが最下行に来てしまう。
    /// 文書の末尾に近い見出しは Scintilla が末尾より先へスクロールしないので、収まる位置まで
    fn jump_to_heading(&self, line: i32) {
        let stc = self.stc;
        let line = line.clamp(0, stc.get_line_count().max(1) - 1);
        scintilla::set_first_visible_line(&stc, scintilla::visible_from_doc_line(&stc, line));
        stc.goto_line(line);
        stc.set_focus();
        self.sync_outline_selection();
    }

    /// 今いる見出しをアウトラインで選択状態にして示す。基準はキャレットの行 (画面内にあるとき)、
    /// そうでなければ先頭に見えている行 (ホイールでスクロールしただけのとき)。
    /// 最初の見出しより前にいるときは何も選ばない
    fn sync_outline_selection(&self) {
        let Some(list) = self.active_outline_list() else {
            return;
        };
        let items = self.items.borrow();
        if items.is_empty() {
            return;
        }
        let stc = self.stc;
        let first_visible = scintilla::first_visible_line(&stc);
        let first = scintilla::doc_line_from_visible(&stc, first_visible) as usize;
        let last = scintilla::doc_line_from_visible(
            &stc,
            first_visible + (scintilla::lines_on_screen(&stc) - 1).max(0),
        ) as usize;
        let caret = stc.get_current_line().max(0) as usize;
        let anchor = if (first..=last).contains(&caret) {
            caret
        } else {
            first
        };
        // 基準の行以前の最後の見出し。文書の先頭など最初の見出しより前にいるときは、
        // 画面内に見えている最初の見出しがあればそれ (無ければ何も選ばない)
        let wanted = items
            .iter()
            .rposition(|it| it.line <= anchor)
            .or_else(|| items.iter().position(|it| it.line <= last));
        let current = list.get_selection().map(|i| i as usize);
        if wanted == current {
            return;
        }
        if let Some(i) = current {
            list.set_selection(i as u32, false);
        }
        if let Some(i) = wanted {
            list.set_selection(i as u32, true);
            list.ensure_visible(i as i32);
        }
    }

    /// 今表示に使っているペインの中身 (ドッキング側か浮動側)
    fn active_content(&self, which: Pane) -> Option<PaneContent> {
        let panes = self.panes.borrow();
        let info = &panes[which.index()];
        info.floating
            .as_ref()
            .map(|f| &f.content)
            .or(info.docked.as_ref().map(|d| &d.content))
            .cloned()
    }

    /// 今表示に使っているコンソール
    fn active_console(&self) -> Option<TextCtrl> {
        match self.active_content(Pane::Console)? {
            PaneContent::Console(t) => Some(t),
            _ => None,
        }
    }

    /// コンソールの中身を入れ替える (無ければ何もしない)
    fn console_set(&self, lines: &[String]) {
        if let Some(console) = self.active_console() {
            console.set_value(&lines.join("\n"));
        }
    }

    /// 今表示に使っている Web プレビュー (Markdown)
    fn active_web_preview(&self) -> Option<Rc<WebPreview>> {
        match self.active_content(Pane::Preview)? {
            PaneContent::Web(w) => Some(w),
            _ => None,
        }
    }

    /// ページ → Rust のメッセージ: リンクを開く、プレビューのスクロールをエディタに反映する
    fn bind_web_preview(&self, web: &Rc<WebPreview>) {
        let tab = self.clone();
        let weak = Rc::downgrade(web);
        web.on_message(move |msg| {
            if let Some(url) = msg.strip_prefix("open:") {
                webpreview::open_external(url);
            } else if let Some(rel) = msg.strip_prefix("file:") {
                // 相対パスのリンク (他の .md など) は Typball で開く
                let rel =
                    crate::mdhtml::percent_decode(rel.split(['?', '#']).next().unwrap_or(rel));
                let base = tab.doc.borrow().path.clone();
                let target = match base.as_ref().and_then(|p| p.parent()) {
                    Some(dir) => dir.join(&rel),
                    None => std::path::PathBuf::from(&rel),
                };
                if target.is_file() {
                    (tab.host.on_drop_files)(vec![target.display().to_string()]);
                }
            } else if let Some(rest) = msg.strip_prefix("line:") {
                // プレビューのスクロール → エディタ: "line:<行>:<基準点 0..1>" (設定 sync_scroll_to_editor が
                // 切ってあれば何もしない)。行を画面の同じ割合の位置に置く (sync_preview_scroll の逆)
                if !tab.filetype().panes(&settings::get()).sync_scroll_to_editor {
                    return;
                }
                let mut parts = rest.trim().split(':');
                let line = parts.next().and_then(|s| s.parse::<i32>().ok());
                let anchor = parts
                    .next()
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0)
                    .clamp(0.0, 1.0);
                if let Some(line) = line {
                    if let Some(web) = weak.upgrade() {
                        web.note_preview_scroll(line, anchor);
                    }
                    let stc = tab.stc;
                    let on_screen = scintilla::lines_on_screen(&stc).max(1);
                    let visible = scintilla::visible_from_doc_line(&stc, (line - 1).max(0));
                    let first = visible - ((on_screen - 1) as f64 * anchor).round() as i32;
                    scintilla::set_first_visible_line(&stc, first.max(0));
                }
            }
        });
    }

    /// エディタのスクロール位置をプレビューに知らせる (Markdown の WebView のみ、設定
    /// sync_scroll_to_preview がオンのとき)。「先頭行を上端に」だけだと末尾で最後まで届かず、
    /// 末尾だけ特別扱いすると最終行が見えた瞬間に跳んで途中の章の続きが見られないので、
    /// 基準点を位置に応じて上端から下端へ連続的に移す
    fn sync_preview_scroll(&self) {
        let Some(web) = self.active_web_preview() else {
            return;
        };
        if !self
            .filetype()
            .panes(&settings::get())
            .sync_scroll_to_preview
        {
            return;
        }
        let stc = self.stc;
        let first = scintilla::first_visible_line(&stc);
        let on_screen = scintilla::lines_on_screen(&stc).max(1);
        let last_doc = stc.get_line_count().max(1) - 1;
        let total = scintilla::visible_from_doc_line(&stc, last_doc)
            + scintilla::wrap_count(&stc, last_doc).max(1);
        // エディタのスクロール位置 (0 = 先頭, 1 = 末尾)。基準点を画面のこの割合の位置に置く:
        // 先頭付近は上端の行、末尾付近は下端の行で合わせるので、末尾でもプレビューが最後まで届き、
        // 途中で跳ぶこともない
        let max_first = (total - on_screen).max(0);
        let anchor = if max_first > 0 {
            (first as f64 / max_first as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let anchor_visible =
            (first + ((on_screen - 1) as f64 * anchor).round() as i32).min(total - 1);
        let line = scintilla::doc_line_from_visible(&stc, anchor_visible) + 1;
        web.scroll_to_line(line, anchor);
    }

    fn active_outline_list(&self) -> Option<ListBox> {
        match self.active_content(Pane::Outline)? {
            PaneContent::Outline(l) => Some(l),
            _ => None,
        }
    }

    /// プレビューを少し後で組み直す。連続して呼んでも 1 回にまとまり、
    /// エディタなどの描画が先に済んでから走る
    fn request_render(&self) {
        let has_target = self.active_content(Pane::Preview).is_some()
            || self.active_content(Pane::Console).is_some();
        if has_target && self.filetype().has_preview() {
            self.render_timer.start(RENDER_DELAY_MS, true);
        }
    }

    /// プレビューを今すぐ組み直し、結果をコンソールに出す (タイマーから呼ばれる)。
    /// プレビューが無くてコンソールだけ出ているときは診断だけ取る
    fn render_preview(&self) {
        let filetype = self.filetype();
        if !filetype.has_preview() || self.large.get() {
            return;
        }
        let text = self.text();
        let started = std::time::Instant::now();
        match self.active_content(Pane::Preview) {
            Some(PaneContent::Preview(preview)) => {
                let path = self.doc.borrow().path.clone();
                let report = preview.render(&text, path.as_deref());
                self.console_set(&typst_log(&report));
            }
            Some(PaneContent::Web(web)) => {
                let path = self.doc.borrow().path.clone();
                web.set_markdown(&text, path.as_deref());
                let ms = started.elapsed().as_millis();
                self.console_set(&markdown_log(&text, ms, Some(&web)));
            }
            _ if self.active_console().is_some() => match filetype {
                FileType::Typ => {
                    let path = self.doc.borrow().path.clone();
                    let (ok, diagnostics) = typball_core::check(&text, path.as_deref());
                    let report = crate::preview::Report {
                        ok,
                        pages: 0,
                        diagnostics,
                        elapsed_ms: started.elapsed().as_millis(),
                    };
                    self.console_set(&typst_log(&report));
                }
                _ => self.console_set(&markdown_log(&text, 0, None)),
            },
            _ => {}
        }
    }

    fn refresh_outline(&self, text: &str) {
        let Some(list) = self.active_outline_list() else {
            return;
        };
        let items = outline::outline(text, self.filetype());
        let labels: Vec<String> = items.iter().map(|it| it.label()).collect();
        let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
        list.set_items(&refs);
        *self.items.borrow_mut() = items;
        self.sync_outline_selection();
    }

    fn refresh_pane_content(&self, which: Pane) {
        match which {
            Pane::Outline => self.refresh_outline(&self.text()),
            Pane::Preview | Pane::Console => self.request_render(),
        }
    }

    // -----------------------------------------------------------------------
    // ペインの配置
    // -----------------------------------------------------------------------

    pub fn pane_state(&self, which: Pane) -> PaneState {
        self.panes.borrow()[which.index()].state
    }

    fn set_state(&self, which: Pane, state: PaneState) {
        self.panes.borrow_mut()[which.index()].state = state;
    }

    pub fn outline_shown(&self) -> bool {
        self.pane_state(Pane::Outline) != PaneState::Hidden
    }

    pub fn preview_shown(&self) -> bool {
        self.pane_state(Pane::Preview) != PaneState::Hidden
    }

    pub fn console_shown(&self) -> bool {
        self.pane_state(Pane::Console) != PaneState::Hidden
    }

    pub fn show_outline(&self, show: bool) {
        self.show_pane(Pane::Outline, show);
    }

    pub fn show_preview(&self, show: bool) {
        self.show_pane(Pane::Preview, show);
    }

    pub fn show_console(&self, show: bool) {
        self.show_pane(Pane::Console, show);
    }

    fn panes_changed(&self) {
        (self.host.on_panes_changed)();
    }

    /// 置き場 `position` にドッキングしているペインのパネル (自分以外)
    fn docked_panels_at(&self, position: PanePosition, except: Pane) -> Vec<Panel> {
        self.docked_at(position, except)
            .into_iter()
            .map(|(_, panel)| panel)
            .collect()
    }

    /// 置き場 `position` にドッキングしているペインとそのパネル (自分以外)
    fn docked_at(&self, position: PanePosition, except: Pane) -> Vec<(Pane, Panel)> {
        let panes = self.panes.borrow();
        Pane::ALL
            .iter()
            .filter(|p| **p != except)
            .filter_map(|p| panes[p.index()].docked.as_ref().map(|d| (*p, d)))
            .filter(|(_, d)| d.position == position)
            .map(|(p, d)| (p, d.bbox.panel))
            .collect()
    }

    /// 置き場をレイアウトに組み込む (その置き場に最初のペインが入ったとき)。
    /// 右・下は分割位置を負の値 (右端 / 下端からの距離) で渡す。まだ大きさが決まっていない
    /// (作った直後の) スプリッタでも、wx が最初のサイズ確定時に反映してくれる
    fn attach_slot(&self, position: PanePosition) {
        let slot = self.slot(position);
        let size = self
            .page
            .from_dip_int(self.pane_sizes.borrow()[position.index()]);
        slot.show(true);
        match position {
            PanePosition::Left => {
                self.h1.split_vertically(&slot, &self.h2, size);
            }
            PanePosition::Right => {
                self.h2.split_vertically(&self.v1, &slot, -size);
            }
            PanePosition::Bottom if self.bottom_wide.get() => {
                self.v0.split_horizontally(&self.h1, &slot, -size);
            }
            PanePosition::Bottom => {
                self.v1.split_horizontally(&self.editor_box, &slot, -size);
            }
        }
    }

    /// 置き場をレイアウトから外す (その置き場が空になったとき)
    fn detach_slot(&self, position: PanePosition) {
        let slot = self.slot(position);
        match position {
            PanePosition::Left => self.h1.unsplit(Some(&slot)),
            PanePosition::Right => self.h2.unsplit(Some(&slot)),
            PanePosition::Bottom if self.bottom_wide.get() => self.v0.unsplit(Some(&slot)),
            PanePosition::Bottom => self.v1.unsplit(Some(&slot)),
        };
        slot.show(false);
    }

    /// 設定の置き場にドッキングして表示する
    fn place(&self, which: Pane) {
        if self.panes.borrow()[which.index()].docked.is_some() {
            return;
        }
        let settings = settings::get();
        let position = which.position(&settings);
        let slot = self.slot(position);
        let bbox = PaneBox::new(&slot, which.title_key(), false, position);
        let content = self.create_content(which, &bbox);
        self.bind_pane_header(&bbox.header, which, false);
        let others = self.docked_panels_at(position, which);
        {
            let mut panes = self.panes.borrow_mut();
            let info = &mut panes[which.index()];
            info.docked = Some(Docked {
                bbox: bbox.clone(),
                content,
                position,
            });
            info.state = PaneState::Docked;
        }
        match others.first() {
            None => {
                slot.initialize(&bbox.panel);
                self.attach_slot(position);
                // 置き場の大きさが前回と同じだとサイズイベントが来ず、中身が初期サイズのまま
                // 残るので、分割位置の設定を通して中身を置き場いっぱいに広げさせる
                slot.set_sash_position(0, true);
            }
            Some(other) => {
                // 同じ置き場に 2 つ目: 左右の置き場は上下に、下の置き場は左右に並べる
                let size = slot.get_client_size();
                if position.is_side() {
                    slot.split_horizontally(other, &bbox.panel, size.height / 2);
                } else {
                    slot.split_vertically(other, &bbox.panel, size.width / 2);
                }
            }
        }
        self.page.layout();
        self.refresh_pane_content(which);
    }

    /// ドッキングをやめて中身ごと破棄する (状態は呼び出し側が決める)
    fn unplace(&self, which: Pane) {
        let docked = self.panes.borrow_mut()[which.index()].docked.take();
        let Some(docked) = docked else {
            return;
        };
        let slot = self.slot(docked.position);
        if self.docked_panels_at(docked.position, which).is_empty() {
            self.detach_slot(docked.position);
        } else {
            slot.unsplit(Some(&docked.bbox.panel));
        }
        docked.bbox.panel.destroy();
        self.page.layout();
    }

    /// 浮動ウィンドウを閉じて中身の参照も捨てる (状態は呼び出し側が決める)
    fn destroy_float(&self, which: Pane) {
        let floating = self.panes.borrow_mut()[which.index()].floating.take();
        if let Some(f) = floating {
            f.fp.frame.destroy();
        }
    }

    /// 折りたたみバーの中身を状態から作り直す
    fn update_strips(&self) {
        let settings = settings::get();
        for position in PanePosition::ALL {
            let items: Vec<(usize, String)> = Pane::ALL
                .iter()
                .filter(|p| self.pane_state(**p) == PaneState::Collapsed)
                .filter(|p| p.position(&settings) == position)
                .map(|p| (p.index(), tr(p.title_key())))
                .collect();
            self.strips[position.index()].set_items(items);
        }
        self.page.layout();
    }

    /// 表示 / 非表示 (表示メニュー)。表示するときはドッキングする
    pub fn show_pane(&self, which: Pane, show: bool) {
        let state = self.pane_state(which);
        match (state, show) {
            (PaneState::Hidden | PaneState::Collapsed, true) => self.place(which),
            (PaneState::Docked, false) => {
                self.unplace(which);
                self.set_state(which, PaneState::Hidden);
            }
            (PaneState::Floating, false) => {
                self.destroy_float(which);
                self.set_state(which, PaneState::Hidden);
            }
            (PaneState::Collapsed, false) => self.set_state(which, PaneState::Hidden),
            _ => return,
        }
        self.update_strips();
        self.panes_changed();
    }

    /// 端のバーに折りたたむ
    pub fn collapse_pane(&self, which: Pane) {
        match self.pane_state(which) {
            PaneState::Docked => self.unplace(which),
            PaneState::Floating => self.destroy_float(which),
            PaneState::Collapsed => return,
            PaneState::Hidden => {}
        }
        self.set_state(which, PaneState::Collapsed);
        self.update_strips();
        self.panes_changed();
    }

    /// 折りたたみから元に戻す
    pub fn expand_pane(&self, which: Pane) {
        if self.pane_state(which) != PaneState::Collapsed {
            return;
        }
        self.place(which);
        self.update_strips();
        self.panes_changed();
    }

    /// 別ウィンドウに切り離す。`at` はドロップした画面座標 (None ならペインの今の位置)
    pub fn float_pane(&self, which: Pane, at: Option<Point>) {
        let state = self.pane_state(which);
        if state == PaneState::Floating {
            return;
        }
        let settings = settings::get();
        let position = which.position(&settings);
        // ドッキングしていた場所にそのまま浮かせる (クライアント領域を元のペインの矩形に合わせる
        // ので、押した「切り離す」ボタンの位置に「戻す」ボタンが来る)
        let docked_rect = self.panes.borrow()[which.index()]
            .docked
            .as_ref()
            .map(|d| pane::widget_rect(&d.bbox.panel));
        let (mut pos, size) = match docked_rect {
            Some(r) => (Point::new(r.x, r.y), Size::new(r.width, r.height)),
            None => {
                let r = pane::frame_rect(&self.host.frame);
                let offset = self.host.frame.from_dip_int(60);
                (
                    Point::new(r.x + offset, r.y + offset),
                    self.host.frame.from_dip(Size::new(360, 520)),
                )
            }
        };
        let dropped = at.is_some();
        if let Some(p) = at {
            let grab = self.host.frame.from_dip_point(Point::new(40, 12));
            pos = Point::new(p.x - grab.x, p.y - grab.y);
        }
        if state == PaneState::Docked {
            self.unplace(which);
        }
        let title = format!("{} - {}", tr(which.title_key()), self.doc.borrow().title());
        let fp = FloatingPane::new(
            &self.host.frame,
            &title,
            which.title_key(),
            pos,
            size,
            position,
        );
        if !dropped && docked_rect.is_some() {
            fp.fit_client_to(pos, size);
        }
        self.bind_pane_header(&fp.pane.header, which, true);
        // ウィンドウの × で閉じられたら折りたたみ扱いにする (端のバーから戻せる)
        fp.frame.on_close({
            let tab = self.clone();
            move |event| {
                tab.panes.borrow_mut()[which.index()].floating.take();
                tab.set_state(which, PaneState::Collapsed);
                tab.update_strips();
                tab.panes_changed();
                event.skip(true);
            }
        });
        let content = self.create_content(which, &fp.pane);
        {
            let mut panes = self.panes.borrow_mut();
            let info = &mut panes[which.index()];
            info.floating = Some(Floating {
                fp: fp.clone(),
                content,
            });
            info.state = PaneState::Floating;
        }
        fp.show();
        self.refresh_pane_content(which);
        self.update_strips();
        self.panes_changed();
    }

    /// 浮動ウィンドウからメインウィンドウへ戻す
    pub fn dock_pane(&self, which: Pane) {
        if self.pane_state(which) != PaneState::Floating {
            return;
        }
        self.destroy_float(which);
        self.place(which);
        self.update_strips();
        self.panes_changed();
    }

    /// 今のペインの大きさをこの形式の設定として保存し、同じ形式の他のタブにも反映してもらう
    /// (分割線を動かしたあと)
    fn save_pane_sizes(&self) {
        let sizes = PaneSizes::from_array(*self.pane_sizes.borrow());
        let mut s = settings::get();
        let entry = self.filetype().overrides_mut(&mut s);
        if entry.pane_sizes == sizes {
            return;
        }
        entry.pane_sizes = sizes;
        settings::set(s.clone());
        let _ = s.save();
        (self.host.on_layout_changed)();
    }

    /// 覚えている大きさに分割線を置き直す (ドッキングしている置き場だけ)。隠れているページは
    /// サイズが古いままなので触らず、表示するとき (window.rs の show_tab_page) にもう一度呼ばれる
    pub fn apply_pane_sizes(&self) {
        if !self.page.is_shown() {
            return;
        }
        for position in PanePosition::ALL {
            if !self.slot(position).is_shown() {
                continue;
            }
            let splitter = self.outer_splitter(position);
            let total = splitter_extent(&splitter, position);
            if total <= 0 {
                continue;
            }
            let want = splitter.from_dip_int(self.pane_sizes.borrow()[position.index()]);
            let sash = match position {
                PanePosition::Left => want,
                PanePosition::Right | PanePosition::Bottom => {
                    (total - want).max(splitter.from_dip_int(MIN_PANE))
                }
            };
            splitter.set_sash_position(sash, true);
        }
    }

    /// 下の置き場の幅の設定を変えて保存し、全ウィンドウに反映してもらう
    fn set_bottom_width(&self, width: BottomWidth) {
        let mut s = settings::get();
        s.panes.bottom_width = width;
        settings::set(s.clone());
        let _ = s.save();
        (self.host.on_layout_changed)();
    }

    /// 置き場の設定を変えて保存し、全ウィンドウに反映してもらう
    fn set_position(&self, which: Pane, position: PanePosition) {
        let mut s = settings::get();
        match which {
            Pane::Outline => s.panes.outline = position,
            Pane::Preview => s.panes.preview = position,
            Pane::Console => s.panes.console = position,
        }
        settings::set(s.clone());
        let _ = s.save();
        (self.host.on_layout_changed)();
    }

    /// 設定の置き場と違う場所にドッキングしているペインを置き直す (設定変更後に呼ぶ)
    pub fn relayout_panes(&self) {
        let settings = settings::get();
        // この形式のペインの大きさが (同じ形式の他のタブで動かされて) 変わっていれば合わせる
        let sizes = self.filetype().pane_sizes(&settings).as_array();
        if sizes != *self.pane_sizes.borrow() {
            *self.pane_sizes.borrow_mut() = sizes;
            self.apply_pane_sizes();
        }
        // 下の置き場の幅 (エディタ幅 / ウィンドウ幅) が変わっていれば、下にあるペインを新しい方に入れ直す
        let wide = settings.panes.bottom_width == BottomWidth::Window;
        if wide != self.bottom_wide.get() {
            let at_bottom: Vec<Pane> = Pane::ALL
                .into_iter()
                .filter(|p| {
                    let panes = self.panes.borrow();
                    panes[p.index()].docked.as_ref().map(|d| d.position)
                        == Some(PanePosition::Bottom)
                })
                .collect();
            for which in &at_bottom {
                self.unplace(*which);
            }
            self.bottom_wide.set(wide);
            for which in &at_bottom {
                self.place(*which);
            }
        }
        for which in Pane::ALL {
            let current = self.panes.borrow()[which.index()]
                .docked
                .as_ref()
                .map(|d| d.position);
            if let Some(current) = current {
                if current != which.position(&settings) {
                    self.unplace(which);
                    self.place(which);
                }
            }
        }
        self.update_strips();
    }

    /// ヘッダのボタンとドラッグ。`floating` は浮動ウィンドウ側のヘッダか
    fn bind_pane_header(&self, header: &pane::PaneHeader, which: Pane, floating: bool) {
        header.on_button({
            let tab = self.clone();
            let header = header.clone();
            move |button| match (button, floating) {
                (HeaderButton::Menu, _) => tab.show_pane_menu(which, &header),
                (HeaderButton::Collapse, _) => tab.collapse_pane(which),
                (HeaderButton::Float, false) => tab.float_pane(which, None),
                (HeaderButton::Float, true) => tab.dock_pane(which),
            }
        });
        header.on_drag_move({
            let tab = self.clone();
            move |screen| tab.update_drop_indicator(which, screen)
        });
        header.on_drop({
            let tab = self.clone();
            move |screen| tab.finish_pane_drop(which, screen, floating)
        });
        header.window().on_menu_selected({
            let tab = self.clone();
            move |event| tab.pane_menu_command(event.get_id())
        });
    }

    // -----------------------------------------------------------------------
    // ヘッダのドラッグ (置き場の変更・切り離し・戻し)
    // -----------------------------------------------------------------------

    /// ドラッグ中の画面座標から行き先の置き場を決める。ページの左 / 右 / 下の端に近い帯の中なら
    /// その置き場、ページの中央やページの外なら None
    fn drop_target(&self, screen: Point) -> Option<PanePosition> {
        let r = pane::widget_rect(&self.page);
        if !crate::theme::contains(&r, screen) {
            return None;
        }
        let band_x = (r.width / 4).clamp(80, 260);
        let band_y = (r.height / 4).clamp(60, 200);
        let candidates = [
            (screen.x - r.x, band_x, PanePosition::Left),
            (r.x + r.width - screen.x, band_x, PanePosition::Right),
            (r.y + r.height - screen.y, band_y, PanePosition::Bottom),
        ];
        candidates
            .into_iter()
            .filter(|(dist, band, _)| dist < band)
            .min_by_key(|(dist, _, _)| *dist)
            .map(|(_, _, position)| position)
    }

    /// ドロップ先の見た目: ペインが来る場所の画面上の矩形と説明。
    /// その置き場に他のペインがあればその下半分 (下の置き場なら右半分) を示し、「〜の下に並べる」と書く
    fn drop_preview(&self, which: Pane, position: PanePosition) -> (Rect, String) {
        if let Some((other, panel)) = self.docked_at(position, which).into_iter().next() {
            let r = pane::widget_rect(&panel);
            let name = tr(other.title_key());
            return if position.is_side() {
                (
                    Rect::new(r.x, r.y + r.height / 2, r.width, r.height / 2),
                    tr_args("pane.drop_below", &[("name", &name)]),
                )
            } else {
                (
                    Rect::new(r.x + r.width / 2, r.y, r.width / 2, r.height),
                    tr_args("pane.drop_right_of", &[("name", &name)]),
                )
            };
        }
        let page = pane::widget_rect(&self.page);
        let size = self
            .page
            .from_dip_int(self.pane_sizes.borrow()[position.index()]);
        let rect = match position {
            PanePosition::Left => Rect::new(page.x, page.y, size, page.height),
            PanePosition::Right => Rect::new(page.x + page.width - size, page.y, size, page.height),
            PanePosition::Bottom => {
                // 置き場を持つスプリッタの幅 (エディタ幅ならエディタの列、ウィンドウ幅ならページ全体)
                let host = pane::widget_rect(&self.outer_splitter(position));
                Rect::new(host.x, host.y + host.height - size, host.width, size)
            }
        };
        let docked_here = self.panes.borrow()[which.index()]
            .docked
            .as_ref()
            .map(|d| d.position == position)
            .unwrap_or(false);
        let key = if docked_here {
            "pane.drop_same"
        } else {
            match position {
                PanePosition::Left => "pane.drop_left",
                PanePosition::Right => "pane.drop_right",
                PanePosition::Bottom => "pane.drop_bottom",
            }
        };
        (rect, tr(key))
    }

    /// ドラッグ中: 行き先があればそこに表示を出し、無ければ隠す
    fn update_drop_indicator(&self, which: Pane, screen: Point) {
        match self.drop_target(screen) {
            Some(position) => {
                let (rect, label) = self.drop_preview(which, position);
                let mut slot = self.drop_indicator.borrow_mut();
                let indicator = slot.get_or_insert_with(|| DropIndicator::new(&self.host.frame));
                indicator.show_at(&rect, &label);
            }
            None => {
                if let Some(indicator) = self.drop_indicator.borrow().as_ref() {
                    indicator.hide();
                }
            }
        }
    }

    /// ドロップ: 帯の中なら置き場を変える (浮動なら戻す)、ウィンドウの外なら切り離す、
    /// 浮動ペインをウィンドウの中 (帯の外) に離したら今の置き場に戻す
    fn finish_pane_drop(&self, which: Pane, screen: Point, floating: bool) {
        if let Some(indicator) = self.drop_indicator.borrow_mut().take() {
            indicator.destroy();
        }
        let inside = crate::theme::contains(&pane::frame_rect(&self.host.frame), screen);
        match self.drop_target(screen) {
            Some(position) => {
                if position != which.position(&settings::get()) {
                    self.set_position(which, position);
                }
                if floating {
                    self.dock_pane(which);
                }
            }
            None if floating && inside => self.dock_pane(which),
            None if !floating && !inside => self.float_pane(which, Some(screen)),
            None => {}
        }
    }

    /// ヘッダの「配置」メニュー: 左 / 右 / 下 (下ならウィンドウ幅に広げるか)、切り離す or 戻す、
    /// 折りたたむ、非表示
    fn show_pane_menu(&self, which: Pane, header: &pane::PaneHeader) {
        let base = PANE_MENU_BASE + which.index() as i32 * 8;
        let settings = settings::get();
        let current = which.position(&settings);
        let state = self.pane_state(which);
        let mut menu = Menu::builder().build();
        for (i, (position, key)) in [
            (PanePosition::Left, "pane.position_left"),
            (PanePosition::Right, "pane.position_right"),
            (PanePosition::Bottom, "pane.position_bottom"),
        ]
        .into_iter()
        .enumerate()
        {
            menu.append(base + i as i32, &tr(key), "", ItemKind::Radio);
            if position == current {
                menu.check_item(base + i as i32, true);
            }
        }
        if current == PanePosition::Bottom {
            menu.append(base + 6, &tr("pane.bottom_wide"), "", ItemKind::Check);
            menu.check_item(base + 6, settings.panes.bottom_width == BottomWidth::Window);
        }
        menu.append_separator();
        let float_key = if state == PaneState::Floating {
            "pane.dock"
        } else {
            "pane.float"
        };
        menu.append(base + 3, &tr(float_key), "", ItemKind::Normal);
        menu.append(base + 4, &tr("pane.collapse"), "", ItemKind::Normal);
        menu.append_separator();
        menu.append(base + 5, &tr("pane.hide"), "", ItemKind::Normal);
        header.window().popup_menu(&mut menu, None);
    }

    fn pane_menu_command(&self, id: i32) {
        let offset = id - PANE_MENU_BASE;
        if !(0..(Pane::ALL.len() as i32 * 8)).contains(&offset) {
            return;
        }
        let which = Pane::ALL[(offset / 8) as usize];
        match offset % 8 {
            0 => self.set_position(which, PanePosition::Left),
            1 => self.set_position(which, PanePosition::Right),
            2 => self.set_position(which, PanePosition::Bottom),
            3 => {
                if self.pane_state(which) == PaneState::Floating {
                    self.dock_pane(which);
                } else {
                    self.float_pane(which, None);
                }
            }
            4 => self.collapse_pane(which),
            5 => self.show_pane(which, false),
            6 => {
                let wide = settings::get().panes.bottom_width == BottomWidth::Window;
                self.set_bottom_width(if wide {
                    BottomWidth::Editor
                } else {
                    BottomWidth::Window
                });
            }
            _ => {}
        }
    }

    /// タブを閉じるとき: 浮動ウィンドウも一緒に閉じてからページを破棄する
    pub fn destroy(&self) {
        self.render_timer.stop();
        for which in Pane::ALL {
            self.destroy_float(which);
        }
        self.page.destroy();
    }

    // -----------------------------------------------------------------------
    // テキスト
    // -----------------------------------------------------------------------

    pub fn text(&self) -> String {
        self.stc.get_text()
    }

    pub fn filetype(&self) -> FileType {
        self.doc.borrow().filetype
    }

    /// 未保存の変更があるか
    pub fn is_modified(&self) -> bool {
        self.stc.is_modified()
    }

    /// ファイルから読んだテキストを入れる (開く・読み直し)
    pub fn set_text(&self, text: &str) {
        self.load_text(text, false);
        self.update_title();
        self.update_status();
    }

    fn load_text(&self, text: &str, modified: bool) {
        let large = is_large(text.len(), &settings::get());
        self.large.set(large);
        editor::set_large_mode(&self.stc, large);
        // これから入力する改行をファイルの改行コードに合わせる (LF のファイルに CRLF を混ぜない)
        scintilla::set_eol_mode(
            &self.stc,
            LineEnding::detect(text).unwrap_or_else(|| settings::get().files.line_ending()),
        );
        self.loading.set(true);
        self.stc.set_text(text);
        if !modified {
            // 開いた直後の内容を「元の状態」にする (Undo で消えず、変更扱いにもならない)
            self.stc.empty_undo_buffer();
            self.stc.set_save_point();
        }
        self.loading.set(false);
        *self.tracker.borrow_mut() = if large {
            ChangeTracker::new("")
        } else {
            ChangeTracker::new(text)
        };
        editor::clear_change_marks(&self.stc, true, true);
        self.refresh_from_text(text);
    }

    /// 大きなファイルとして開いている (色分けなどを止めている) か
    pub fn is_large(&self) -> bool {
        self.large.get()
    }

    /// スナップショット (backup.rs) の識別子
    pub fn backup_id(&self) -> u64 {
        self.backup_id
    }

    /// 最後のスナップショットの後に編集があったか。呼ぶと「書いた」ことになる
    pub fn take_backup_dirty(&self) -> bool {
        self.backup_dirty.replace(false)
    }

    pub fn set_backup_dirty(&self) {
        self.backup_dirty.set(true);
    }

    /// スナップショットを書いたか (消すものがあるか)
    pub fn backup_written(&self) -> &Cell<bool> {
        &self.backup_written
    }

    /// 裏で続いている保存
    pub fn pending_save(&self) -> &RefCell<Option<PendingSave>> {
        &self.pending_save
    }

    /// 保存した直後に呼ぶ: 未保存の編集行の印を「保存済み」に替える (設定によっては消す)
    pub fn mark_saved(&self) {
        self.stc.set_save_point();
        let settings = settings::get();
        editor::mark_changes_saved(&self.stc, &settings);
        if !self.large.get() {
            self.tracker.borrow_mut().rebase(&self.text());
        }
        editor::refresh_line_numbers(&self.stc, &settings);
        self.refresh_decorations();
        self.update_title();
        // Markdown の WebView: 保存で画像が変わっているかもしれないのでキャッシュを更新させる
        if let Some(web) = self.active_web_preview() {
            web.on_saved();
        }
        // Typst も、画像や取り込んだファイルが変わっているかもしれないので組み直す
        // (変わったファイルだけ読み直すので、何も変わっていなければ軽い)
        self.request_render();
    }

    /// 編集行の強調を消す (表示メニュー)。`saved_only` なら保存済みの印だけ、
    /// そうでなければ未保存の印も含めてすべて
    pub fn clear_change_marks(&self, saved_only: bool) {
        editor::clear_change_marks(&self.stc, !saved_only, true);
        editor::refresh_line_numbers(&self.stc, &settings::get());
        self.refresh_decorations();
    }

    /// 構文・行番号・アウトライン・プレビューをテキストから引き直す
    fn refresh_from_text(&self, text: &str) {
        let settings = settings::get();
        let filetype = self.filetype();
        if self.large.get() {
            // 色分け無し (全文をスタイル 0 に)。アウトラインとプレビューも作らない
            editor::restyle(&self.stc, text, FileType::Txt);
        } else {
            editor::restyle(&self.stc, text, filetype);
        }
        editor::update_margins(&self.stc, &settings);
        editor::refresh_line_numbers(&self.stc, &settings);
        self.char_count.set(scintilla::count_characters(
            &self.stc,
            0,
            self.stc.get_length(),
        ));
        // ステータスバーの「混在」の警告用。軽量モードは開いたときにしかここへ来ないので、その後の編集で
        // 混ざっても分からない
        self.eol_counts.set(EolCounts::of(text.as_bytes()));
        if self.large.get() {
            self.refresh_decorations();
            return;
        }
        self.refresh_outline(text);
        self.request_render();
        self.refresh_decorations();
    }

    /// ルーラーと右側行番号を描き直す (キャレット移動・スクロール・変更のたびに呼ぶ)
    fn refresh_decorations(&self) {
        self.ruler.refresh();
        self.gutter.refresh();
    }

    /// 種別が変わった (名前を付けて保存で拡張子が変わった) とき
    pub fn filetype_changed(&self) {
        let filetype = self.filetype();
        for which in Pane::ALL {
            if !which.available(filetype) {
                self.show_pane(which, false);
            }
        }
        // プレビューの中身の種類 (Typst の画像 / Markdown の WebView) が合わなくなったら作り直す
        let wants_web = filetype == FileType::Md;
        let has_web = self.active_web_preview().is_some();
        if self.active_content(Pane::Preview).is_some() && wants_web != has_web {
            self.show_pane(Pane::Preview, false);
            self.show_pane(Pane::Preview, true);
        }
        self.actionbar.set_filetype(filetype);
        self.apply_actionbar_setting(&settings::get());
        self.refresh_from_text(&self.text());
        self.update_title();
        self.update_status();
    }

    // -----------------------------------------------------------------------
    // タイトル・ステータス・設定
    // -----------------------------------------------------------------------

    /// タブバー内の位置
    pub fn index(&self) -> Option<usize> {
        self.host.tabbar.index_of(self.key())
    }

    fn is_current(&self) -> bool {
        self.index().is_some() && self.index() == self.host.tabbar.selected()
    }

    /// ウィンドウのタイトルバーに出す文字列 (未保存なら " *")
    pub fn display_name(&self) -> String {
        let mut name = self.doc.borrow().title();
        if self.is_modified() {
            name.push_str(" *");
        }
        name
    }

    /// タブの表示と (選択中なら) ウィンドウタイトルを更新する
    pub fn update_title(&self) {
        let (title, filetype) = {
            let doc = self.doc.borrow();
            (doc.title(), doc.filetype)
        };
        self.host
            .tabbar
            .update(self.key(), &title, self.is_modified(), filetype);
        if self.is_current() {
            self.host
                .frame
                .set_title(&format!("{} - Typball", self.display_name()));
        }
    }

    /// ステータスバーの種別・行桁・文字数・文字コード・改行コードを更新する (選択中のタブだけ)
    pub fn update_status(&self) {
        if !self.is_current() {
            return;
        }
        let stc = &self.stc;
        let pos = stc.get_current_pos();
        let line = stc.get_current_line();
        let col = scintilla::count_characters(stc, stc.position_from_line(line), pos) + 1;
        let status = &self.host.status;
        let kind = tr(self.filetype().short_name_key());
        let position =
            |line: &str, col: &str| tr_args("status.position", &[("line", line), ("col", col)]);
        let chars = |n: &str| tr_args("status.chars", &[("n", n)]);
        let (line, col) = ((line + 1).to_string(), col.to_string());
        let count = self.char_count.get().to_string();
        let encoding = self.doc.borrow().encoding.label();
        status.set_status_text(&kind, 1);
        status.set_status_text(&position(&line, &col), 2);
        status.set_status_text(&chars(&count), 3);
        status.set_status_text(&encoding, 4);
        self.host
            .eol
            .set(self.line_ending(), &self.eol_counts.get());
        // 欄を中身の幅に詰める。行桁と文字数は桁を多めに取った文字で測り、数字が増えるたびに欄が
        // 動かないようにする (行は 4 桁、桁は 3 桁、文字数は 5 桁までは同じ幅)
        let reserve = |digits: &str, least: usize| "0".repeat(digits.len().max(least));
        self.host.eol.fit(vec![
            kind,
            position(&reserve(&line, 4), &reserve(&col, 3)),
            chars(&reserve(&count, 5)),
            encoding,
        ]);
    }

    /// 文字の幅を、半角の幅を 1 として返す関数 (全角文字は 2)。エディタのフォントで測り、同じ文字は
    /// 1 回だけ測る。タブ位置の桁を数えるのに使う (エディタがタブをそろえる位置と同じ数え方になる)
    fn char_width(&self) -> impl FnMut(char) -> usize {
        let stc = self.stc;
        let space = scintilla::text_width(&stc, editor::STYLE_DEFAULT, " ").max(1);
        let mut measured: std::collections::HashMap<char, usize> = std::collections::HashMap::new();
        move |c: char| {
            if c.is_ascii() {
                return 1;
            }
            *measured.entry(c).or_insert_with(|| {
                let px =
                    scintilla::text_width(&stc, editor::STYLE_DEFAULT, c.encode_utf8(&mut [0; 4]));
                ((px + space / 2) / space).max(0) as usize
            })
        }
    }

    /// 行頭から `pos` までの文字列 (`pos` が行の中に無ければ None)
    fn line_before(&self, pos: i32) -> Option<String> {
        let stc = self.stc;
        let line = stc.line_from_position(pos);
        let start = stc.position_from_line(line);
        let bytes = scintilla::line_bytes(&stc, line);
        let before = bytes.get(..(pos - start).max(0) as usize)?;
        String::from_utf8(before.to_vec()).ok()
    }

    /// Tab キー (スペースを入力する設定のとき): 次のタブ位置までのスペースを入れる。入れたら true。
    ///
    /// Scintilla に任せると、桁を文字数で数えるので (全角文字も 1 桁)、全角文字の後ろで入るスペースの
    /// 数がずれ、表示上のタブ位置にそろわない。ここでは表示上の桁 (全角は 2 桁) で数える。
    /// タブ文字を入力する設定のときと、複数行を選択しているとき (行ごとの字下げ) は Scintilla に任せる
    fn insert_soft_tab(&self) -> bool {
        let stc = self.stc;
        let (tab, use_tabs) = {
            let s = settings::get();
            (s.editor.tab_width.clamp(1, 16) as usize, s.editor.use_tabs)
        };
        if use_tabs || scintilla::selections(&stc) > 1 {
            return false;
        }
        let (start, end) = (stc.get_selection_start(), stc.get_selection_end());
        if stc.line_from_position(start) != stc.line_from_position(end) {
            return false;
        }
        let Some(before) = self.line_before(start) else {
            return false;
        };
        let col = crate::tabs::column(&before, tab, self.char_width());
        let count = crate::tabs::to_next_stop(col, tab);
        // 選択範囲があれば置き換える (1 回の Undo で戻る)
        stc.replace_selection(&" ".repeat(count));
        stc.ensure_caret_visible();
        true
    }

    /// Shift+Tab (字下げの外): キャレットを前のタブ位置へ戻す。動かしたら true。
    /// 字下げの中 (行頭からキャレットまでが空白だけ) と、文字を選択しているときは Scintilla に任せる
    /// (字下げを 1 段戻す)
    fn back_tab(&self) -> bool {
        let stc = self.stc;
        if scintilla::selections(&stc) > 1 || stc.get_selection_start() != stc.get_selection_end() {
            return false;
        }
        let pos = stc.get_current_pos();
        let Some(before) = self.line_before(pos) else {
            return false;
        };
        if before.bytes().all(|b| b == b' ' || b == b'\t') {
            return false;
        }
        let tab = settings::get().editor.tab_width.clamp(1, 16) as usize;
        let index = crate::tabs::previous_stop_index(&before, tab, self.char_width());
        let target = pos - (before.len() - index) as i32;
        stc.set_selection(target, target);
        stc.ensure_caret_visible();
        true
    }

    /// Backspace: キャレットの前のスペースを、設定 `editor.backspace` の動作に従ってまとめて消す
    /// (どれだけ消すかは `settings::Backspace::count`)。消したら true。まとめて消すものが無ければ
    /// 何もしない (呼ぶ側がふつうの Backspace にする)
    fn backspace_to_tab_stop(&self) -> bool {
        let stc = self.stc;
        if stc.get_selection_start() != stc.get_selection_end() {
            return false;
        }
        let pos = stc.get_current_pos();
        let line = stc.get_current_line();
        let start = stc.position_from_line(line);
        if pos <= start {
            return false;
        }
        let bytes = scintilla::line_bytes(&stc, line);
        let Some(before) = bytes.get(..(pos - start) as usize) else {
            return false;
        };
        let spaces = before.iter().rev().take_while(|b| **b == b' ').count();
        // 1 個だけならふつうの Backspace と同じ
        if spaces < 2 {
            return false;
        }
        let Ok(prefix) = std::str::from_utf8(before) else {
            return false;
        };
        let (mode, tab) = {
            let s = settings::get();
            (s.editor.backspace, s.editor.tab_width.clamp(1, 16) as usize)
        };
        let col = crate::tabs::column(prefix, tab, self.char_width());
        let in_indent = before.iter().all(|b| *b == b' ' || *b == b'\t');
        let count = mode.count(col, spaces, in_indent, tab) as i32;
        if count == 0 {
            return false;
        }
        stc.delete_range(pos - count, count);
        true
    }

    /// 文書のタブをスペースに (`to_spaces`)、またはスペースの並びをタブに、一括で変換する
    /// (編集 > タブをスペースに変換 / スペースをタブに変換)。何か変えたら true。1 回の Undo で戻せる。
    ///
    /// 桁は半角の幅を 1 として、エディタのフォントで測った文字の幅で数える (全角文字は 2)。エディタが
    /// タブをそろえる位置と同じ数え方なので、変換の前後で文字の位置は変わらない
    pub fn convert_tabs(&self, to_spaces: bool) -> bool {
        let stc = self.stc;
        let tab = settings::get().editor.tab_width.clamp(1, 16) as usize;
        let text = self.text();
        let width = self.char_width();
        let converted = if to_spaces {
            crate::tabs::expand(&text, tab, width)
        } else {
            crate::tabs::tabify(&text, tab, width)
        };
        if converted == text {
            return false;
        }
        // 文書全体を 1 回で置き換える (変更の通知も 1 回)。キャレットのある行と表示位置は保つ
        let line = stc.get_current_line();
        let first = scintilla::first_visible_line(&stc);
        scintilla::begin_undo_action(&stc);
        stc.set_target_start(0);
        stc.set_target_end(stc.get_length());
        scintilla::replace_target(&stc, &converted, false);
        scintilla::end_undo_action(&stc);
        let pos = stc.position_from_line(line.min(stc.get_line_count() - 1).max(0));
        stc.set_selection(pos, pos);
        scintilla::set_first_visible_line(&stc, first);
        true
    }

    /// 文書の改行コード (Enter で入る改行。開いたときにファイルで一番多いものに合わせてある)
    pub fn line_ending(&self) -> LineEnding {
        scintilla::eol_mode(&self.stc)
    }

    /// 文書のすべての改行を `eol` にそろえ、これから入力する改行も合わせる。改行が変われば文書は
    /// 未保存になり、1 回の Undo で戻せる (Undo しても、これから入力する改行は `eol` のまま)
    pub fn convert_line_endings(&self, eol: LineEnding) {
        scintilla::convert_eols(&self.stc, eol);
        scintilla::set_eol_mode(&self.stc, eol);
        if self.large.get() {
            // 軽量モードは変更のたびに全文を見ないので、数え直す代わりに「そろった」ことだけ反映する
            let counts = self.eol_counts.get();
            let total = counts.crlf + counts.lf + counts.cr;
            self.eol_counts.set(match eol {
                LineEnding::Crlf => EolCounts {
                    crlf: total,
                    ..Default::default()
                },
                LineEnding::Lf => EolCounts {
                    lf: total,
                    ..Default::default()
                },
                LineEnding::Cr => EolCounts {
                    cr: total,
                    ..Default::default()
                },
            });
        }
        self.update_status();
    }

    fn apply_actionbar_setting(&self, settings: &Settings) {
        let filetype = self.filetype();
        let show = filetype.panes(settings).show_actionbar && filetype.has_actionbar();
        self.actionbar.window().show(show);
        self.editor_box.layout();
    }

    /// 形式ごとのペインの既定 (`[filetypes.*] show_outline / show_preview`) が変わっていれば、開いている
    /// このタブにも合わせる。変わっていない項目には触らないので、表示メニューで個別に閉じたペインは戻らない
    fn apply_pane_defaults(&self, before: &Settings, after: &Settings) {
        let filetype = self.filetype();
        let (old, new) = (filetype.panes(before), filetype.panes(after));
        if old.show_outline != new.show_outline && filetype.has_outline() {
            self.show_outline(new.show_outline);
        }
        if old.show_preview != new.show_preview && filetype.has_preview() {
            self.show_preview(new.show_preview);
        }
        if old.show_console != new.show_console && filetype.has_console() {
            self.show_console(new.show_console);
        }
    }

    /// 設定ダイアログで変えられる項目を反映する (`before` は変更前の設定)
    pub fn apply_settings(&self, before: &Settings, settings: &Settings) {
        editor::apply_settings(&self.stc, settings);
        // 不可視文字の記号 (表示の有無や、比べる改行コード) は、エディタを描き直すと変わる
        self.stc.refresh(true, None);
        self.ruler.window().show(settings.editor.show_ruler);
        self.gutter
            .window()
            .show(settings.editor.line_numbers.right());
        self.apply_actionbar_setting(settings);
        self.apply_pane_defaults(before, settings);
        // プレビューのフォントかタブ幅が変わった: Typst は組み直し、Markdown はページを読み直す
        if before.preview.font_face != settings.preview.font_face
            || before.preview.font_size != settings.preview.font_size
            || before.preview.tab_width != settings.preview.tab_width
        {
            if let Some(web) = self.active_web_preview() {
                web.reload_page();
            }
            self.request_render();
        }
        self.editor_box.layout();
        self.relayout_panes();
        self.refresh_decorations();
        self.update_title();
        self.update_status();
    }

    /// 配色の明暗が変わった (設定の変更・OS の切り替え)。エディタは色を当て直し、
    /// プレビューは Typst を組み直して Markdown はページを読み直す
    pub fn apply_theme(&self, changed: crate::theme::Changed, settings: &Settings) {
        if changed.editor {
            editor::apply_settings(&self.stc, settings);
            self.refresh_decorations();
            self.stc.refresh(true, None);
        }
        if changed.preview {
            match self.active_content(Pane::Preview) {
                Some(PaneContent::Web(web)) => web.reload_page(),
                Some(PaneContent::Preview(view)) => view.apply_theme(),
                _ => {}
            }
            self.request_render();
        }
    }
}

// ---------------------------------------------------------------------------
// コンソールの文面
// ---------------------------------------------------------------------------

/// Typst のコンパイル結果をコンソールの行にする
fn typst_log(report: &crate::preview::Report) -> Vec<String> {
    let ms = report.elapsed_ms.to_string();
    let pages = report.pages.to_string();
    let head = if !report.ok {
        tr_args("console.typst_failed", &[("ms", &ms)])
    } else if report.diagnostics.is_empty() {
        tr_args("console.typst_ok", &[("pages", &pages), ("ms", &ms)])
    } else {
        tr_args("console.typst_warnings", &[("pages", &pages), ("ms", &ms)])
    };
    let mut lines = vec![head];
    lines.extend(report.diagnostics.iter().cloned());
    lines
}

/// Markdown の変換結果 (とサイトモードの開発サーバーの出力) をコンソールの行にする
fn markdown_log(text: &str, elapsed_ms: u128, web: Option<&WebPreview>) -> Vec<String> {
    let mut lines = vec![tr_args(
        "console.markdown",
        &[
            ("lines", &text.lines().count().to_string()),
            ("chars", &text.chars().count().to_string()),
            ("ms", &elapsed_ms.to_string()),
        ],
    )];
    if let Some(log) = web.and_then(WebPreview::server_log) {
        lines.push(String::new());
        lines.extend(log.lines().map(str::to_string));
    }
    lines
}

/// 診断行の `行:桁:` (先頭の "error: " などの後) を読む
fn parse_location(line: &str) -> Option<(i32, i32)> {
    line.split_whitespace().find_map(|token| {
        let token = token.trim_end_matches(':');
        let (l, c) = token.split_once(':')?;
        Some((l.parse::<i32>().ok()?, c.parse::<i32>().ok()?))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_location_is_parsed() {
        assert_eq!(
            parse_location("error: 12:5: unknown variable"),
            Some((12, 5))
        );
        assert_eq!(parse_location("warning: 3:1: unused"), Some((3, 1)));
        assert_eq!(parse_location("Typst: OK (2 pages, 15 ms)"), None);
    }
}

/// 設定 `files.large_file_mb` を超える大きさか (0 なら無制限)
pub fn is_large(len: usize, settings: &Settings) -> bool {
    let mb = settings.files.large_file_mb as usize;
    mb > 0 && len > mb * 1024 * 1024
}
