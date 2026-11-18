//! Web プレビュー (wxWebView): Markdown を HTML にして表示する「Markdown モード」と、
//! 静的サイトジェネレーターの開発サーバーを表示する「サイトモード」
//!
//! 配信の仕組み: wxDragon の `register_handler` で独自スキーム `typball` を登録すると、wx (Edge) は
//! それを仮想ホスト `https://typball.wxsite/` として扱い、そのホストへの要求を Rust のクロージャに
//! 回してくる。ページ本体 (`/<文書番号>/index.html`)、同梱の CSS / JS (`/assets/...`)、文書の
//! フォルダにある画像 (`/<文書番号>/<相対パス>`) をすべてここから返すので、一時ファイルも HTTP
//! サーバーも要らず、相対パスの画像はブラウザがページの URL 基準で解決してくれる。
//!
//! 更新はページを読み直さず、スクリプトで本文 (`<article>`) だけ差し替える。スクロール位置が
//! 残り、画像はブラウザのキャッシュが効く。スクリプトは [`run_script_async`] で結果を待たずに流す:
//! wx の同期 `RunScript` (と、それを使う `CanCopy` / `Copy` などの編集系) は結果が来るまで
//! `while (...) wxYield();` で回り続け、WebView2 が結果を返せない状況 (読み込み中のページに対して
//! メニューを開いたときなど) ではアプリ全体が固まる。ページ側の状態 (選択の有無など) はページから
//! メッセージで知らせてもらう。
//!
//! 文書の情報 (フォルダ・最新の本文) は番号をキーにしたプロセス全体の表 [`DOCS`] に置く。
//! ハンドラのクロージャは WebView ごとに登録するが、どの文書の要求かは URL の番号で分かる。

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{c_void, CString};
use std::path::{Component, Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use wxdragon::event::WebViewEvents;
use wxdragon::ffi;
use wxdragon::prelude::*;
use wxdragon::widgets::webview::{WebView, WebViewBackend, WebViewHandlerResponse};

use crate::editor::rgb;
use crate::filetype::FileType;
use crate::i18n::{tr, tr_args};
use crate::mdhtml;
use crate::previewbar::{Glyph, IconButton};
use crate::settings::{self, MarkdownMode};
use crate::site::{self, PageUrl, Project, Server, StartError};
use crate::theme;

/// 独自スキーム名。wx はこれを `https://<スキーム>.wxsite/` の仮想ホストにする
pub(crate) const SCHEME: &str = "typball";
const HOST: &str = "typball.wxsite";
/// ページ → Rust のメッセージ用オブジェクト名 (`window.typball_host.postMessage`)
const MESSAGE_HANDLER: &str = "typball_host";
const CSS: &str = include_str!("../assets/preview.css");
const JS: &str = include_str!("../assets/preview.js");
/// スクロール同期の計算 (Markdown モードとサイトモードで共通)。Markdown モードのページは `/assets/` から読み、
/// サイトモードでは開発サーバーのページに流し込む
const SCROLL_SYNC_JS: &str = include_str!("../assets/scrollsync.js");
/// サイトモードのスクロール同期 (ページの要素との突き合わせ)。`SCROLL_SYNC_JS` に続けて、開発サーバーの
/// ページに流し込む
const SITE_SYNC_JS: &str = include_str!("../assets/sitesync.js");
/// スクロール同期の計算結果を記録させる (`TYPBALL_SYNC_LOG` を付けて起動したとき。`sync_log`)
const SYNC_DEBUG_JS: &str = "window.typballSyncDebug = true;";
/// Mermaid 12.3.0 (MIT)。1 ファイル版をそのまま置いている。Markdown のプレビューは Mermaid のコードブロックが
/// あるときだけ読み込み、Typst の文書の中の Mermaid は mermaid.rs が見えない WebView で描くのに使う
const MERMAID: &str = include_str!("../assets/mermaid/mermaid.min.js");
/// サーバーの起動待ちの間隔と上限 (250 ms × 1200 = 5 分。大きなサイトは組むのに分単位でかかることがある)
const READY_POLL_MS: i32 = 250;
const READY_MAX_TICKS: u32 = 1200;
/// 表示した後、サーバーが落ちていないかを見る間隔
const WATCH_POLL_MS: i32 = 1000;
/// サーバーが応答してから、開いているファイルのページの URL が分かるまで待つ上限 (250 ms × 60 = 15 秒)。
/// 過ぎたらサイトのトップを出し、分かった時点でそのページへ移る
const PAGE_WAIT_TICKS: u32 = 60;
/// プレビュー側のスクロールをエディタに反映した直後は、エディタ → プレビューの同期を止める時間
const SYNC_GUARD: Duration = Duration::from_millis(400);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Markdown,
    Site,
}

/// 文書ごとの配信情報
struct DocState {
    base_dir: Option<PathBuf>,
    /// ルート相対の URL を探すフォルダ (サイトの static/ と assets/)
    root_dirs: Vec<PathBuf>,
    body: String,
    title: String,
    /// 書き出し (PDF) 用のページ: 常にライトの配色で、末尾の余白を詰める
    export: bool,
}

fn docs() -> &'static Mutex<HashMap<u64, DocState>> {
    static DOCS: OnceLock<Mutex<HashMap<u64, DocState>>> = OnceLock::new();
    DOCS.get_or_init(|| Mutex::new(HashMap::new()))
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// プレビューの上のバー: スクロールの同期の入・切と、モードの切り替え。どちらも無ければバーは出さない
struct ModeBar {
    panel: Panel,
    /// スクロールの同期の入・切 (エディタ → プレビュー, プレビュー → エディタ)。エディタの付いたタブだけ
    /// (プレビューウィンドウにはエディタが無いので出さない)
    sync: Option<(IconButton, IconButton)>,
    /// モードの一覧と、その右の知らせ (サイトのプロジェクトが見つかったときだけ)
    site: Option<(Choice, StaticText)>,
}

/// スクロールの同期の設定 (`[filetypes.md] sync_scroll_to_preview` / `sync_scroll_to_editor`) を反転して保存する。
/// 「設定 → ファイル → Markdown」のチェックボックスと同じ項目
fn toggle_sync_setting(to_editor: bool) {
    let mut s = settings::get();
    let now = FileType::Md.panes(&s);
    let entry = FileType::Md.overrides_mut(&mut s);
    if to_editor {
        entry.sync_scroll_to_editor = Some(!now.sync_scroll_to_editor);
    } else {
        entry.sync_scroll_to_preview = Some(!now.sync_scroll_to_preview);
    }
    settings::set(s.clone());
    let _ = s.save();
}

pub struct WebPreview {
    /// ペインの中身として置くパネル (バー + WebView)
    pub container: Panel,
    webview: Option<WebView>,
    pub id: u64,
    frame: Frame,
    mode: Cell<Mode>,
    project: Option<Project>,
    bar: Option<ModeBar>,
    /// 殻ページが読み込まれて `typball.update` を受け付ける状態
    loaded: Cell<bool>,
    /// ページで文字列が選択されているか (ページからの "sel:" メッセージで更新。編集メニュー用)
    has_selection: Rc<Cell<bool>>,
    server: RefCell<Option<Rc<Server>>>,
    /// 開発サーバーを起動できなかったときの説明と出力の末尾 (コンソールペイン用)。素の Markdown に切り替えた後も残す
    failed_log: RefCell<Option<String>>,
    /// 開発サーバーの状態が変わったときに呼ぶ処理 ([`WebPreview::on_server_change`])
    server_changed: RefCell<Option<Box<dyn Fn()>>>,
    /// 開いているファイル (サイトモードで、そのページの URL を求めるのに使う。名前を付けて保存すると変わる)
    path: RefCell<Option<PathBuf>>,
    /// サイトモードで最後に表示させた URL (ページの URL が変わったかを見る)
    shown_url: RefCell<Option<String>>,
    /// サーバーが応答してから、ページの URL が分かるのを待った回数
    page_wait: Cell<u32>,
    /// ページの URL を 1 回でも聞いたか
    page_asked: Cell<bool>,
    /// 文書の行数 (サイトモードのスクロール同期で、最後の対応点から末尾までの配分に使う)
    source_lines: Cell<usize>,
    /// サイトモードのスクロール同期のために、最後にページへ渡した本文のハッシュ
    site_source: Cell<Option<u64>>,
    /// 保存された (次の set_markdown で front matter を比べる)
    saved: Cell<bool>,
    /// 開いたとき・最後に保存したときの front matter のハッシュ
    front_matter: Cell<Option<u64>>,
    ready_timer: Timer<Panel>,
    ready_ticks: Cell<u32>,
    /// 保存のたびに増やす (画像のキャッシュ更新用)
    version: Cell<u64>,
    /// 最後にプレビューへ送った (行 × 100, 基準点 × 1000)。同じなら送り直さない
    last_anchor: Cell<(i32, i32)>,
    sync_guard_until: Cell<Option<Instant>>,
}

/// この環境で WebView が使えるか (Windows は WebView2 ランタイムの有無)
pub fn available() -> bool {
    WebView::is_backend_available(backend())
}

pub(crate) fn backend() -> WebViewBackend {
    if cfg!(windows) {
        WebViewBackend::Edge
    } else {
        WebViewBackend::WebKit
    }
}

impl WebPreview {
    /// 中の WebView (作れなかったときは None)。編集メニューの振り分け用
    pub fn webview(&self) -> Option<WebView> {
        self.webview
    }

    /// サイトモードのときの開発サーバーの URL と出力の末尾 (コンソールペイン用)。
    /// 起動できずに素の Markdown へ切り替えたときは、その理由と出力
    pub fn server_log(&self) -> Option<String> {
        if let Some(log) = self.failed_log.borrow().as_ref() {
            return Some(log.clone());
        }
        if self.mode.get() != Mode::Site {
            return None;
        }
        let server = self.server.borrow();
        let server = server.as_ref()?;
        let name = self
            .project
            .as_ref()
            .map(|p| p.generator.name.as_str())
            .unwrap_or_default();
        // 開いているファイルのページを出しているなら、その URL (どのページに解決されたかが分かる)
        let url = self
            .shown_url
            .borrow()
            .clone()
            .unwrap_or_else(|| server.url.clone());
        let mut out = tr_args("console.site_server", &[("name", name), ("url", &url)]);
        let tail = server.log_tail(20);
        if !tail.is_empty() {
            out.push('\n');
            out.push_str(&tail);
        }
        Some(out)
    }

    /// `parent` の中にプレビューを作る。`path` は文書のパス (サイトのプロジェクト検出に使う)。
    /// `sync_changed` を渡すと、バーにスクロールの同期の入・切のボタンを出す (エディタの付いたタブ)。ボタンで
    /// 設定を変えた後に呼ぶので、開いているすべてのプレビューの表示を合わせ直す処理を渡す
    /// ([`WebPreview::refresh_sync_buttons`] が呼ばれるようにする)
    pub fn new(
        parent: &Panel,
        frame: Frame,
        path: Option<&Path>,
        sync_changed: Option<Rc<dyn Fn()>>,
    ) -> Rc<WebPreview> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let settings = settings::get();
        let project = path.and_then(|p| site::detect(p, &site::generators(&settings.preview)));
        let container = Panel::builder(parent).build();
        let sizer = BoxSizer::builder(Orientation::Vertical).build();

        let bar = (project.is_some() || sync_changed.is_some()).then(|| {
            let panel = Panel::builder(&container).build();
            panel.set_background_color(rgb(theme::ui().bar_bg));
            let px = |v: i32| panel.from_dip_int(v);
            let hbox = BoxSizer::builder(Orientation::Horizontal).build();
            let center = SizerFlag::AlignCenterVertical;
            let sync = sync_changed.map(|changed| {
                let to_preview =
                    IconButton::new(&panel, Glyph::SyncToPreview, &tr("preview.sync_to_preview"));
                let to_editor =
                    IconButton::new(&panel, Glyph::SyncToEditor, &tr("preview.sync_to_editor"));
                for (button, is_to_editor) in [(&to_preview, false), (&to_editor, true)] {
                    let changed = changed.clone();
                    button.on_click(move || {
                        toggle_sync_setting(is_to_editor);
                        changed();
                    });
                }
                hbox.add(
                    &to_preview.panel,
                    0,
                    center | SizerFlag::Left | SizerFlag::Top | SizerFlag::Bottom,
                    px(4),
                );
                hbox.add(&to_editor.panel, 0, center | SizerFlag::Left, px(2));
                (to_preview, to_editor)
            });
            let site = project.as_ref().map(|project| {
                let choice = Choice::builder(&panel).build();
                choice.freeze();
                choice.append(&tr("preview.mode_markdown"));
                choice.append(&tr_args(
                    "preview.mode_site",
                    &[("name", &project.generator.name)],
                ));
                choice.thaw();
                let label = StaticText::builder(&panel).with_label("").build();
                hbox.add(&choice, 0, center | SizerFlag::All, px(4));
                hbox.add(&label, 1, center | SizerFlag::All, px(4));
                (choice, label)
            });
            panel.set_sizer(hbox, true);
            sizer.add(&panel, 0, SizerFlag::Expand, 0);
            ModeBar { panel, sync, site }
        });

        let webview = if available() {
            let wv = WebView::builder(&container).with_backend(backend()).build();
            wv.register_handler(SCHEME, serve);
            wv.add_script_message_handler(MESSAGE_HANDLER);
            wv.enable_context_menu(false);
            sizer.add(&wv, 1, SizerFlag::Expand, 0);
            Some(wv)
        } else {
            let text = StaticText::builder(&container)
                .with_label(&tr("preview.webview_missing"))
                .build();
            text.wrap(container.from_dip_int(360));
            sizer.add(
                &text,
                1,
                SizerFlag::Expand | SizerFlag::All,
                container.from_dip_int(12),
            );
            None
        };
        container.set_sizer(sizer, true);

        let ready_timer = Timer::new(&container);
        let mode = match settings.preview.markdown_mode {
            MarkdownMode::Markdown => Mode::Markdown,
            MarkdownMode::Auto | MarkdownMode::Site if project.is_some() => Mode::Site,
            _ => Mode::Markdown,
        };
        let this = Rc::new(WebPreview {
            container,
            webview,
            id,
            frame,
            mode: Cell::new(mode),
            project,
            bar,
            loaded: Cell::new(false),
            has_selection: Rc::new(Cell::new(false)),
            server: RefCell::new(None),
            failed_log: RefCell::new(None),
            server_changed: RefCell::new(None),
            path: RefCell::new(path.map(Path::to_path_buf)),
            shown_url: RefCell::new(None),
            page_wait: Cell::new(0),
            page_asked: Cell::new(false),
            source_lines: Cell::new(1),
            site_source: Cell::new(None),
            saved: Cell::new(false),
            front_matter: Cell::new(None),
            ready_timer,
            ready_ticks: Cell::new(0),
            version: Cell::new(1),
            last_anchor: Cell::new((-1, 0)),
            sync_guard_until: Cell::new(None),
        });
        docs().lock().unwrap().insert(
            id,
            DocState {
                base_dir: path.and_then(Path::parent).map(Path::to_path_buf),
                root_dirs: this
                    .project
                    .as_ref()
                    .map(Project::static_dirs)
                    .unwrap_or_default(),
                body: String::new(),
                title: String::new(),
                export: false,
            },
        );
        this.bind_events();
        if let Some((choice, _)) = this.bar.as_ref().and_then(|b| b.site.as_ref()) {
            choice.set_selection(if mode == Mode::Site { 1 } else { 0 });
        }
        this.refresh_sync_buttons();
        this.apply_mode(false);
        this
    }

    fn bind_events(self: &Rc<Self>) {
        let weak: Weak<Self> = Rc::downgrade(self);
        if let Some(wv) = self.webview {
            // 殻ページが読み終わったら最新の本文を流し込む
            wv.on_loaded({
                let weak = weak.clone();
                move |event| {
                    let Some(this) = weak.upgrade() else {
                        return;
                    };
                    let url = event.get_url().unwrap_or_default();
                    if url.starts_with(&this.page_url()) {
                        this.loaded.set(true);
                        this.last_anchor.set((-1, 0));
                        if let (Some(wv), true) = (this.webview, sync_log_path().is_some()) {
                            run_script_async(&wv, SYNC_DEBUG_JS);
                        }
                        this.flush();
                    } else if this.mode.get() == Mode::Site && this.loaded.get() {
                        // 開発サーバーのページ: スクロール同期のスクリプトを流し込む。ページを読み直すたび
                        // (保存の後のライブリロードなど) に消えるので、読み込みのたびに入れ直す
                        if let Some(wv) = this.webview {
                            // 共通の計算を先に、突き合わせを後に (スクリプトは送った順に実行される)
                            run_script_async(&wv, SCROLL_SYNC_JS);
                            run_script_async(&wv, SITE_SYNC_JS);
                            if sync_log_path().is_some() {
                                run_script_async(&wv, SYNC_DEBUG_JS);
                            }
                        }
                        this.last_anchor.set((-1, 0));
                        this.push_site_source(true);
                    }
                }
            });
        }
        if let Some(choice) = self.site_choice() {
            choice.on_selection_changed({
                let weak = weak.clone();
                move |_| {
                    let Some(this) = weak.upgrade() else {
                        return;
                    };
                    let mode = match this.site_choice().and_then(|c| c.get_selection()) {
                        Some(1) => Mode::Site,
                        _ => Mode::Markdown,
                    };
                    if mode != this.mode.get() {
                        this.mode.set(mode);
                        this.apply_mode(true);
                    }
                }
            });
        }
        self.ready_timer.on_tick({
            let weak = weak.clone();
            move |_| {
                if let Some(this) = weak.upgrade() {
                    this.check_server();
                }
            }
        });
        // パネルが破棄されるとき (タブを閉じた、ペインを作り直した、プレビューウィンドウを閉じた) にタイマーを止める。
        // タイマーはパネルを宛先にして動くので、止めないまま破棄されると、次の時刻に破棄済みのパネルへ知らせが飛んで
        // アプリごと落ちる (サイトモードは表示した後もタイマーを回すので、タブを閉じた 1 秒後に必ず落ちた)。
        // この WebPreview 自体は、閉じたタブを持っている処理が残っていると、すぐには drop されない
        self.container.on_destroy({
            let weak = weak.clone();
            move |event| {
                if let Some(this) = weak.upgrade() {
                    this.shutdown();
                }
                event.skip(true);
            }
        });
    }

    /// タイマーを止める (パネルが破棄されるとき)
    fn shutdown(&self) {
        self.ready_timer.stop();
    }

    /// 使い終わった (タブやプレビューウィンドウを閉じる): タイマーを止め、開発サーバーを手放す。
    /// 同じプロジェクトをほかのタブが使っていなければ、サーバーはここで止まる (大きなサイトの Hugo は
    /// メモリを GB 単位で使うので、タブを閉じた後まで残さない)。パネルを破棄する前に呼ぶ
    pub fn close(&self) {
        self.shutdown();
        let server = self.server.borrow_mut().take();
        if let Some(server) = server {
            site::release(server);
        }
    }

    /// ページ → Rust のメッセージ ("open:<url>", "file:<相対パス>", "line:<行>", "ready")。
    /// 選択の有無 ("sel:0/1") とコピー ("copy:<文字列>") はここで処理して `f` には渡さない
    pub fn on_message(&self, f: impl Fn(&str) + 'static) {
        if let Some(wv) = self.webview {
            let has_selection = self.has_selection.clone();
            wv.on_script_message_received(move |event| {
                let Some(msg) = event.get_string() else {
                    return;
                };
                if let Some(flag) = msg.strip_prefix("sel:") {
                    has_selection.set(flag == "1");
                } else if let Some(text) = msg.strip_prefix("copy:") {
                    set_clipboard_text(text);
                } else if let Some(text) = msg.strip_prefix("dbg:") {
                    sync_log(text);
                } else {
                    f(&msg);
                }
            });
        }
    }

    /// ページで文字列が選択されているか (編集メニューの「コピー」の有効 / 無効)
    pub fn has_selection(&self) -> bool {
        self.has_selection.get()
    }

    /// 選択範囲をクリップボードへ (ページが "copy:" で文字列を返してくる)
    pub fn copy(&self) {
        if let Some(wv) = self.webview {
            run_script_async(&wv, "window.typball && typball.copySelection();");
        }
    }

    /// 本文をすべて選択する
    pub fn select_all(&self) {
        if let Some(wv) = self.webview {
            run_script_async(&wv, "window.typball && typball.selectAll();");
        }
    }

    fn page_url(&self) -> String {
        format!("https://{HOST}/{}/index.html", self.id)
    }

    /// バーのモードの一覧 (サイトのプロジェクトが見つかったときだけある)
    fn site_choice(&self) -> Option<Choice> {
        self.bar
            .as_ref()
            .and_then(|bar| bar.site.as_ref())
            .map(|(choice, _)| *choice)
    }

    fn set_label(&self, text: &str) {
        if let Some((_, label)) = self.bar.as_ref().and_then(|bar| bar.site.as_ref()) {
            label.set_label(text);
            // 長い文 (起動できなかった理由) はペインの幅で切れるので、全文をツールチップでも読めるようにする
            label.set_tooltip(text);
            self.container.layout();
        }
    }

    /// スクロールの同期のボタンの入・切を、今の設定に合わせる (ボタンや設定画面で設定が変わった後に呼ぶ)
    pub fn refresh_sync_buttons(&self) {
        if let Some((to_preview, to_editor)) = self.bar.as_ref().and_then(|bar| bar.sync.as_ref()) {
            let panes = FileType::Md.panes(&settings::get());
            to_preview.set_checked(panes.sync_scroll_to_preview);
            to_editor.set_checked(panes.sync_scroll_to_editor);
        }
    }

    /// 配色が変わったときに、バーの背景を合わせる
    pub fn apply_theme(&self) {
        if let Some(bar) = &self.bar {
            bar.panel.set_background_color(rgb(theme::ui().bar_bg));
            bar.panel.refresh(true, None);
        }
    }

    /// ページを読み直す (プレビューのフォント設定が変わったとき)。本文は殻ページに埋め込まれるので失われない
    pub fn reload_page(&self) {
        if self.mode.get() == Mode::Markdown {
            self.apply_mode(false);
        }
    }

    /// 開発サーバーの状態が変わったとき (表示できた・落ちた) に呼ぶ処理を登録する (コンソールペインの更新用)。
    /// `f` にこのプレビューの持ち主 (タブ) を持たせないこと: 互いに持ち合って、どちらも解放されなくなる
    pub fn on_server_change(&self, f: impl Fn() + 'static) {
        *self.server_changed.borrow_mut() = Some(Box::new(f));
    }

    fn notify_server_change(&self) {
        if let Some(f) = self.server_changed.borrow().as_ref() {
            f();
        }
    }

    /// 今のモードの表示を始める。`retry` は利用者がバーで選び直したとき (落ちたサーバーをすぐ起動し直す)
    fn apply_mode(&self, retry: bool) {
        let Some(wv) = self.webview else {
            return;
        };
        self.ready_timer.stop();
        match self.mode.get() {
            Mode::Markdown => {
                self.loaded.set(false);
                *self.failed_log.borrow_mut() = None;
                self.set_label("");
                wv.load_url(&self.page_url());
            }
            Mode::Site => {
                self.loaded.set(false);
                self.start_site(retry);
            }
        }
    }

    fn start_site(&self, retry: bool) {
        let Some(project) = &self.project else {
            return;
        };
        *self.failed_log.borrow_mut() = None;
        match site::server_for(project, retry) {
            Ok(server) => {
                *self.server.borrow_mut() = Some(server);
                self.ready_ticks.set(0);
                self.page_wait.set(0);
                self.page_asked.set(false);
                *self.shown_url.borrow_mut() = None;
                self.set_label(&tr_args(
                    "preview.site_starting",
                    &[("name", &project.generator.name)],
                ));
                // 既に動いているサーバーならすぐ表示、起動中ならタイマーで待つ。表示した後もタイマーは止めず、
                // サーバーが落ちていないかを見張る (check_server)
                self.check_server();
                if self.server.borrow().is_some() && !self.loaded.get() {
                    self.ready_timer.start(READY_POLL_MS, false);
                }
            }
            Err(StartError::NotFound(program)) => {
                self.site_failed(&tr_args("preview.site_not_found", &[("command", &program)]))
            }
            Err(StartError::Other(e)) => self.site_failed(&e),
        }
    }

    /// 開発サーバーを出せなかった: 理由をバーとステータスバーに出し、素の Markdown の表示に切り替える
    /// (コマンドが入っていない機械でも中身は見られるように)。バーで「サイト」を選び直せばもう一度試す
    fn site_failed(&self, error: &str) {
        self.ready_timer.stop();
        let server = self.server.borrow_mut().take();
        // サイトを表示した後で落ちたのか、起動できなかったのか (サイトモードの `loaded` は「表示した」印)
        let key = if self.loaded.get() {
            "preview.site_exited"
        } else {
            "preview.site_error"
        };
        let msg = tr_args(key, &[("error", error)]);
        let mut log = msg.clone();
        if let Some(server) = server {
            let tail = server.log_tail(20);
            if !tail.is_empty() {
                log.push('\n');
                log.push_str(&tail);
            }
        }
        *self.failed_log.borrow_mut() = Some(log);
        self.frame.set_status_text(&msg, 0);
        self.mode.set(Mode::Markdown);
        if let Some(choice) = self.site_choice() {
            choice.set_selection(0);
        }
        self.loaded.set(false);
        if let Some(wv) = self.webview {
            wv.load_url(&self.page_url());
        }
        self.set_label(&msg);
        self.notify_server_change();
    }

    /// サーバーの起動待ちと、表示した後の見張り (タイマーから)
    fn check_server(&self) {
        let server = self.server.borrow().clone();
        let Some(server) = server else {
            self.ready_timer.stop();
            return;
        };
        // 起動の途中でも、表示した後でも、落ちたら理由を出して素の Markdown に切り替える
        if let Some(err) = server.exit_error() {
            self.site_failed(&err);
            return;
        }
        // サイトモードでは `loaded` を「サーバーのページを出した」印に使う
        if self.loaded.get() {
            // 表示した後で、開いているファイルのページの URL が分かった (調べるのに時間がかかった) か、変わった
            // (保存で front matter が変わった、名前を付けて保存した) ら、そのページへ移る。利用者がプレビューの中で
            // リンクを辿っただけなら URL は変わっていないので、何もしない
            if let PageUrl::Found(url) = self.page_target(&server) {
                if self.shown_url.borrow().as_deref() != Some(url.as_str()) {
                    self.show_site(&url);
                    // コンソールの URL も新しくする
                    self.notify_server_change();
                }
            }
            return;
        }
        if server.is_ready() {
            // 開いているファイルのページを出す。調べている間 (ページ一覧のコマンドが動いている) は少し待ち、
            // 分からなければサイトのトップを出す
            let url = match self.page_target(&server) {
                PageUrl::Found(url) => url,
                PageUrl::Unknown => server.url.clone(),
                PageUrl::Pending => {
                    let waited = self.page_wait.get() + 1;
                    self.page_wait.set(waited);
                    if waited <= PAGE_WAIT_TICKS {
                        return;
                    }
                    server.url.clone()
                }
            };
            self.loaded.set(true);
            self.show_site(&url);
            self.set_label(&tr("preview.site_hint"));
            // ここからは落ちていないかを見るだけなので、間隔を空ける
            self.ready_timer.start(WATCH_POLL_MS, false);
            self.notify_server_change();
            return;
        }
        let ticks = self.ready_ticks.get() + 1;
        self.ready_ticks.set(ticks);
        if ticks > READY_MAX_TICKS {
            self.site_failed(&tr("preview.site_timeout"));
        }
    }

    /// 開いているファイルの、開発サーバーの上のページの URL (`site::Server::page_url`)
    fn page_target(&self, server: &Server) -> PageUrl {
        let path = self.path.borrow().clone();
        match path {
            // 最初の 1 回だけ、一覧に無ければ一覧を取り直してよい (サーバーの起動後に作ったファイル)
            Some(path) => server.page_url(&path, !self.page_asked.replace(true)),
            None => PageUrl::Unknown,
        }
    }

    /// 開発サーバーのページを表示する
    fn show_site(&self, url: &str) {
        if let Some(wv) = self.webview {
            wv.load_url(url);
        }
        *self.shown_url.borrow_mut() = Some(url.to_string());
    }

    /// Markdown を変換して表示を更新する (Markdown モードのとき)
    pub fn set_markdown(&self, text: &str, path: Option<&Path>) {
        // サイトモード: 保存で front matter が変わっていたら、ページの URL が変わったかもしれないので調べ直す
        // (url / slug / permalink / date など)。変わっていなければ調べ直さない: 一覧のコマンドは軽くない
        *self.path.borrow_mut() = path.map(Path::to_path_buf);
        let front_matter = front_matter_hash(text);
        let saved = self.saved.replace(false);
        match self.front_matter.get() {
            None => self.front_matter.set(Some(front_matter)),
            Some(before) if saved && before != front_matter => {
                self.front_matter.set(Some(front_matter));
                if let (Some(server), Some(path)) = (self.server.borrow().as_ref(), path) {
                    server.refresh_page(path);
                }
            }
            Some(_) => {}
        }
        let base_dir = path.and_then(Path::parent).map(Path::to_path_buf);
        let ctx = mdhtml::Context {
            doc_id: self.id,
            base_dir: base_dir.as_deref(),
            version: self.version.get(),
            dark: crate::theme::preview_dark(),
            export: None,
        };
        let out = mdhtml::render(text, &ctx);
        {
            let mut docs = docs().lock().unwrap();
            if let Some(doc) = docs.get_mut(&self.id) {
                doc.base_dir = base_dir;
                doc.body = out.body;
                doc.title = out.title.unwrap_or_default();
            }
        }
        self.source_lines.set(text.lines().count().max(1));
        if self.mode.get() == Mode::Markdown && self.loaded.get() {
            self.flush();
        } else if self.mode.get() == Mode::Site && self.loaded.get() {
            // 行がずれたかもしれないので、スクロール同期の対応点を作り直させる
            self.push_site_source(false);
        }
    }

    /// サイトモードのスクロール同期: Markdown から作った HTML (行番号付き) を、開発サーバーのページに入れた
    /// スクリプト (sitesync.js) に渡す。スクリプトがページの要素と突き合わせて、行との対応点を作る。
    /// `force` でなければ、前に渡したものと同じなら渡さない
    fn push_site_source(&self, force: bool) {
        use std::hash::{Hash, Hasher};
        let Some(wv) = self.webview else {
            return;
        };
        let body = docs()
            .lock()
            .unwrap()
            .get(&self.id)
            .map(|d| d.body.clone())
            .unwrap_or_default();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        body.hash(&mut hasher);
        let hash = hasher.finish();
        if !force && self.site_source.get() == Some(hash) {
            return;
        }
        self.site_source.set(Some(hash));
        let json = serde_json::to_string(&body).unwrap_or_else(|_| "\"\"".to_string());
        let lines = self.source_lines.get();
        run_script_async(
            &wv,
            &format!("window.typballSite && typballSite.setSource({json}, {lines});"),
        );
    }

    /// 最新の本文をページに流し込む (結果は待たない。スクリプトは送った順に実行される)
    fn flush(&self) {
        let Some(wv) = self.webview else {
            return;
        };
        let body = docs()
            .lock()
            .unwrap()
            .get(&self.id)
            .map(|d| d.body.clone())
            .unwrap_or_default();
        let json = serde_json::to_string(&body).unwrap_or_else(|_| "\"\"".to_string());
        // 行数も渡す (スクロール同期で、最後のブロックから文書の末尾までの配分に使う)
        let lines = self.source_lines.get();
        run_script_async(
            &wv,
            &format!("window.typball && typball.update({json}, {lines});"),
        );
    }

    /// 保存した: 画像のキャッシュを更新させる (呼び出し側が続けて set_markdown する)
    pub fn on_saved(&self) {
        self.saved.set(true);
        self.version.set(self.version.get() + 1);
    }

    /// エディタの行 `line` (1 始まり) に対応する要素を、プレビューの画面の `anchor` (0 = 上端, 1 = 下端)
    /// の位置に来るようにスクロールする。`line` の小数部は、折り返された行の途中の位置 (表示上の何行目か)。
    /// 計算は Markdown モードもサイトモードも同じ (assets/scrollsync.js)
    pub fn scroll_to_line(&self, line: f64, anchor: f64) {
        let Some(wv) = self.webview else {
            return;
        };
        if !self.loaded.get() {
            return;
        }
        // Markdown モードは preview.js、サイトモードは開発サーバーのページに流し込んだ sitesync.js が受ける
        let target = match self.mode.get() {
            Mode::Markdown => "typball",
            Mode::Site => "typballSite",
        };
        if let Some(until) = self.sync_guard_until.get() {
            if Instant::now() < until {
                return;
            }
        }
        let key = sync_key(line, anchor);
        if self.last_anchor.get() == key {
            return;
        }
        self.last_anchor.set(key);
        run_script_async(
            &wv,
            &format!("window.{target} && {target}.scrollToLine({line:.2}, {anchor:.3});"),
        );
    }

    /// プレビュー側のスクロールをエディタに反映した直後に呼ぶ (往復の揺れを止める)
    pub fn note_preview_scroll(&self, line: f64, anchor: f64) {
        self.last_anchor.set(sync_key(line, anchor));
        self.sync_guard_until.set(Some(Instant::now() + SYNC_GUARD));
    }
}

impl Drop for WebPreview {
    fn drop(&mut self) {
        self.ready_timer.stop();
        docs().lock().unwrap().remove(&self.id);
    }
}

/// 最後にプレビューへ送った位置を比べるためのキー: (行 × 100, 基準点 × 1000)
fn sync_key(line: f64, anchor: f64) -> (i32, i32) {
    (
        (line * 100.0).round() as i32,
        (anchor * 1000.0).round() as i32,
    )
}

/// スクロール同期の調査用: 環境変数 `TYPBALL_SYNC_LOG` にファイルのパスを入れて起動すると、サイトモードの
/// ページ側の計算結果 (対応点の数、行 → スクロール位置) をそのファイルに書き足す。ふだんは何もしない。
/// WebView の中のスクロール位置は外から読めないので、同期の不具合はこの記録で確かめる (docs/testing.md)
fn sync_log_path() -> Option<&'static PathBuf> {
    static PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
    PATH.get_or_init(|| {
        std::env::var_os("TYPBALL_SYNC_LOG")
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
    })
    .as_ref()
}

fn sync_log(text: &str) {
    use std::io::Write;
    let Some(path) = sync_log_path() else {
        return;
    };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{text}");
    }
}

/// front matter (先頭の `---` か `+++` で囲んだ部分) のハッシュ。無ければ、空のもののハッシュ
fn front_matter_hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut lines = text.lines();
    let mut block = String::new();
    if let Some(open) = lines.next().map(str::trim_end) {
        if open == "---" || open == "+++" {
            for line in lines {
                if line.trim_end() == open {
                    break;
                }
                block.push_str(line);
                block.push('\n');
            }
        }
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    block.hash(&mut hasher);
    hasher.finish()
}

/// URL を既定のブラウザで開く
pub fn open_external(url: &str) {
    if !(url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:")) {
        return;
    }
    let result = if cfg!(windows) {
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
    let _ = result;
}

/// スクリプトを実行し、結果は待たない。wx の `RunScript` (wxDragon の `run_script`) は使わないこと:
/// 結果を `while (...) wxYield();` で待つので、WebView2 が結果を返せないと抜けられずに固まる
pub fn run_script_async(wv: &WebView, js: &str) {
    let ptr = wv.handle_ptr() as *mut ffi::wxd_WebView_t;
    if ptr.is_null() {
        return;
    }
    let Ok(script) = CString::new(js) else {
        return;
    };
    unsafe { ffi::wxd_WebView_RunScriptAsync(ptr, script.as_ptr()) }
}

pub(crate) fn set_clipboard_text(text: &str) {
    let clipboard = Clipboard::get();
    if clipboard.open() {
        clipboard.set_text(text);
        clipboard.close();
    }
}

// ---------------------------------------------------------------------------
// 書き出し (HTML / PDF)
// ---------------------------------------------------------------------------

/// PDF の用紙 (wxd_WebView_PrintToPdf の番号と同じ順)
pub const PAPERS: [&str; 6] = ["A4", "A3", "A5", "B5 (JIS)", "Letter", "Legal"];

/// Markdown を 1 ファイルで完結する HTML にする (CSS は埋め込み、画像は `embed` なら data: URL)
pub fn export_html(
    markdown: &str,
    md_path: Option<&Path>,
    out: &Path,
    embed_images: bool,
) -> String {
    let ctx = mdhtml::Context {
        doc_id: 0,
        base_dir: md_path.and_then(Path::parent),
        version: 0,
        dark: false,
        export: Some(mdhtml::ExportUrls {
            embed: embed_images,
            out_dir: out.parent().map(Path::to_path_buf),
        }),
    };
    let rendered = mdhtml::render(markdown, &ctx);
    let title = rendered
        .title
        .or_else(|| {
            md_path
                .and_then(Path::file_stem)
                .map(|s| s.to_string_lossy().into_owned())
        })
        .unwrap_or_default();
    format!(
        concat!(
            "<!DOCTYPE html>\n<html data-theme=\"light\"><head><meta charset=\"utf-8\">\n",
            "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n",
            "<title>{title}</title>\n<style>\n{css}\n{font_css}\n</style>\n</head>\n",
            "<body class=\"export\"><article id=\"content\" class=\"markdown-body\">\n{body}\n</article></body></html>\n"
        ),
        title = mdhtml::escape_html(&title),
        css = CSS,
        font_css = body_font_css(&settings::get()),
        body = rendered.body,
    )
}

/// 書き出し中の PDF。PDF は WebView2 の印刷で作るので、見えない WebView に書き出し用のページ
/// (ライトの配色) を読み込み、読み終わったら印刷する。WebView は `parent` (書き出しダイアログ) の
/// 子なので、ダイアログを閉じれば一緒に消える
pub struct PdfExport {
    id: u64,
}

impl PdfExport {
    /// 書き出しを始める。`done` は書けたら Ok、失敗したら Err (メッセージ) で 1 回だけ呼ばれる
    pub fn start(
        parent: &dyn WxWidget,
        markdown: &str,
        md_path: Option<&Path>,
        out: &Path,
        paper: usize,
        landscape: bool,
        done: impl FnOnce(Result<(), String>) + 'static,
    ) -> Result<Rc<PdfExport>, String> {
        if !available() {
            return Err(tr("preview.webview_missing"));
        }
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let base_dir = md_path.and_then(Path::parent);
        let ctx = mdhtml::Context {
            doc_id: id,
            base_dir,
            version: 1,
            dark: false,
            export: None,
        };
        let rendered = mdhtml::render(markdown, &ctx);
        docs().lock().unwrap().insert(
            id,
            DocState {
                base_dir: base_dir.map(Path::to_path_buf),
                root_dirs: Vec::new(),
                body: rendered.body,
                title: rendered.title.unwrap_or_default(),
                export: true,
            },
        );
        let webview = WebView::builder(parent)
            .with_backend(backend())
            .with_size(Size::new(800, 600))
            .build();
        webview.register_handler(SCHEME, serve);
        webview.enable_context_menu(false);
        webview.show(false);
        let job = Rc::new(PdfExport { id });
        let out = out.display().to_string();
        let done = Rc::new(RefCell::new(Some(done)));
        let url = format!("https://{HOST}/{id}/index.html");
        let started = Cell::new(false);
        // 印刷は、ページが「描き終えた」("rendered") と知らせてきてから: Mermaid の図は読み込みの後で描かれる。
        // 知らせが来ないとき (ページのスクリプトが動かなかったとき) に待ち続けないよう、読み込みの 10 秒後にも
        // 同じ知らせを出させる
        webview.add_script_message_handler(MESSAGE_HANDLER);
        webview.on_loaded({
            let url = url.clone();
            move |event| {
                if event.get_url().unwrap_or_default().starts_with(&url) {
                    run_script_async(
                        &webview,
                        "setTimeout(function(){try{window.typball_host.postMessage('rendered')}catch(e){}},10000);",
                    );
                }
            }
        });
        webview.on_script_message_received({
            move |event| {
                if started.get() || event.get_string().as_deref() != Some("rendered") {
                    return;
                }
                started.set(true);
                let finish = {
                    let done = done.clone();
                    move |result: Result<(), String>| {
                        if let Some(f) = done.borrow_mut().take() {
                            f(result);
                        }
                    }
                };
                let on_saved = {
                    let finish = finish.clone();
                    move |ok: bool| {
                        finish(if ok {
                            Ok(())
                        } else {
                            Err(tr("export.pdf_failed"))
                        })
                    }
                };
                if !print_to_pdf(&webview, &out, paper, landscape, on_saved) {
                    finish(Err(tr("export.pdf_unsupported")));
                }
            }
        });
        webview.load_url(&url);
        Ok(job)
    }
}

impl Drop for PdfExport {
    fn drop(&mut self) {
        docs().lock().unwrap().remove(&self.id);
    }
}

type PdfCallback = Box<dyn FnOnce(bool)>;

extern "C" fn pdf_saved(user_data: *mut c_void, ok: bool) {
    if user_data.is_null() {
        return;
    }
    let f: Box<PdfCallback> = unsafe { Box::from_raw(user_data as *mut PdfCallback) };
    f(ok);
}

/// 今のページを PDF に印刷する (WebView2)。印刷を始められなければ false (`f` は呼ばれない)
fn print_to_pdf(
    wv: &WebView,
    path: &str,
    paper: usize,
    landscape: bool,
    f: impl FnOnce(bool) + 'static,
) -> bool {
    let ptr = wv.handle_ptr() as *mut ffi::wxd_WebView_t;
    let Ok(c_path) = CString::new(path) else {
        return false;
    };
    if ptr.is_null() {
        return false;
    }
    let user_data = Box::into_raw(Box::new(Box::new(f) as PdfCallback)) as *mut c_void;
    let ok = unsafe {
        ffi::wxd_WebView_PrintToPdf(
            ptr,
            c_path.as_ptr(),
            paper as i32,
            landscape,
            Some(pdf_saved),
            user_data,
        )
    };
    if !ok {
        drop(unsafe { Box::from_raw(user_data as *mut PdfCallback) });
    }
    ok
}

// ---------------------------------------------------------------------------
// 配信
// ---------------------------------------------------------------------------

/// 仮想ホストへの要求に応える (wx のハンドラから。`uri` は `https://typball.wxsite/...` か `typball:...`)
fn serve(uri: &str) -> Option<WebViewHandlerResponse> {
    let path = request_path(uri)?;
    let (first, rest) = match path.trim_start_matches('/').split_once('/') {
        Some((a, b)) => (a, b),
        None => (path.trim_start_matches('/'), ""),
    };
    if first == "assets" {
        return embedded_asset(rest);
    }
    let id: u64 = first.parse().ok()?;
    let docs = docs().lock().unwrap();
    let doc = docs.get(&id)?;
    if rest.is_empty() || rest == "index.html" {
        return Some(WebViewHandlerResponse {
            data: page_html(doc).into_bytes(),
            mime_type: Some("text/html".to_string()),
        });
    }
    let rest = mdhtml::percent_decode(rest);
    let file = resolve_file(doc, &rest)?;
    let data = std::fs::read(&file).ok()?;
    Some(WebViewHandlerResponse {
        data,
        mime_type: Some(mime_for(&file).to_string()),
    })
}

/// URL からパス部分 (`/7/img/a.png`) を取り出す (クエリとフラグメントは捨てる)
pub(crate) fn request_path(uri: &str) -> Option<&str> {
    let path = if let Some(i) = uri.find(HOST) {
        &uri[i + HOST.len()..]
    } else if let Some(rest) = uri.strip_prefix(&format!("{SCHEME}:")) {
        rest.trim_start_matches('/')
    } else {
        return None;
    };
    Some(path.split(['?', '#']).next().unwrap_or(path))
}

pub(crate) fn embedded_asset(name: &str) -> Option<WebViewHandlerResponse> {
    let fonts = typball_core::embedded_font_data();
    let (data, mime): (&[u8], &str) = match name {
        "preview.css" => (CSS.as_bytes(), "text/css"),
        "preview.js" => (JS.as_bytes(), "application/javascript"),
        "scrollsync.js" => (SCROLL_SYNC_JS.as_bytes(), "application/javascript"),
        "mermaid.min.js" => (MERMAID.as_bytes(), "application/javascript"),
        // 同梱の Noto Sans JP (インストールされていなくても @font-face で使えるように)
        "NotoSansJP-Regular.otf" => (fonts.first()?, "font/otf"),
        "NotoSansJP-Bold.otf" => (fonts.get(1)?, "font/otf"),
        // 同梱の UDEV Gothic (プレビューのフォントに選んだとき)
        "UDEVGothic-Regular.ttf" | "UDEVGothic-Bold.ttf" => {
            (crate::fonts::editor_font_file(name)?, "font/ttf")
        }
        _ => return None,
    };
    Some(WebViewHandlerResponse {
        data: data.to_vec(),
        mime_type: Some(mime.to_string()),
    })
}

/// `~abs/<絶対パス>`、`~root/<サイトのルート相対>`、それ以外は文書のフォルダからの相対
fn resolve_file(doc: &DocState, rest: &str) -> Option<PathBuf> {
    if let Some(p) = rest.strip_prefix("~abs/") {
        let path = if matches!(p.as_bytes(), [d, b':', ..] if d.is_ascii_alphabetic()) {
            PathBuf::from(p)
        } else {
            PathBuf::from(format!("/{p}"))
        };
        return (is_safe_extension(&path) && path.is_file()).then_some(path);
    }
    if let Some(p) = rest.strip_prefix("~root/") {
        let rel = Path::new(p);
        if !is_plain_relative(rel) {
            return None;
        }
        return doc
            .root_dirs
            .iter()
            .map(|d| d.join(rel))
            .find(|f| f.is_file());
    }
    let rel = Path::new(rest);
    if !is_plain_relative(rel) {
        return None;
    }
    let file = doc.base_dir.as_ref()?.join(rel);
    file.is_file().then_some(file)
}

/// `..` や絶対パス、ドライブ文字を含まない相対パスか
fn is_plain_relative(path: &Path) -> bool {
    path.components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// 絶対パスで読めるのは画像・フォント・メディアなど表示用のファイルだけ
fn is_safe_extension(path: &Path) -> bool {
    const ALLOWED: [&str; 23] = [
        "png", "jpg", "jpeg", "gif", "svg", "tbdg", "webp", "avif", "bmp", "ico", "apng", "css",
        "woff", "woff2", "ttf", "otf", "pdf", "mp4", "webm", "mp3", "ogg", "wav", "txt",
    ];
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| ALLOWED.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn mime_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        // 図 (.tbdg) の中身は SVG
        Some("svg" | "tbdg") => "image/svg+xml",
        Some("webp") => "image/webp",
        Some("avif") => "image/avif",
        Some("bmp") => "image/bmp",
        Some("ico") => "image/x-icon",
        Some("apng") => "image/apng",
        Some("css") => "text/css",
        Some("js") => "application/javascript",
        Some("html" | "htm") => "text/html",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("pdf") => "application/pdf",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mp3") => "audio/mpeg",
        Some("ogg") => "audio/ogg",
        Some("wav") => "audio/wav",
        Some("txt" | "md") => "text/plain; charset=utf-8",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    }
}

/// 殻ページ。本文は最初から埋め込み、以降は `typball.update` で差し替える。
/// CSP で外部スクリプトとインラインスクリプトを止める (Markdown に書かれた `<script>` は動かない)
fn page_html(doc: &DocState) -> String {
    format!(
        concat!(
            "<!DOCTYPE html><html data-theme=\"{theme}\"><head><meta charset=\"utf-8\">",
            "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'; ",
            "img-src 'self' data: https: http:; media-src 'self' data: https: http:; ",
            "style-src 'self' 'unsafe-inline'; font-src 'self' data:; script-src 'self'; ",
            "connect-src 'none'; form-action 'none'\">",
            "<title>{title}</title>",
            "<link rel=\"stylesheet\" href=\"/assets/preview.css\">",
            "<style>{font_css}</style>",
            // スクロール同期の計算を先に読む (preview.js が使う)
            "<script src=\"/assets/scrollsync.js\"></script>",
            "<script src=\"/assets/preview.js\"></script>",
            "</head><body{body_class}><article id=\"content\" class=\"markdown-body\">{body}</article></body></html>"
        ),
        theme = if crate::theme::preview_dark() && !doc.export {
            "dark"
        } else {
            "light"
        },
        body_class = if doc.export { " class=\"export\"" } else { "" },
        title = mdhtml::escape_html(&doc.title),
        font_css = font_css(&settings::get()),
        body = doc.body
    )
}

/// 設定のプレビューフォントを本文に当てる CSS。同梱の Noto Sans JP と UDEV Gothic は `/assets/` から @font-face で読む
/// (インストールされていない機械でも同じ見た目になる)。後ろに preview.css と同じフォールバックを並べる
fn font_css(settings: &settings::Settings) -> String {
    let mut css = String::from(concat!(
        "@font-face{font-family:\"Noto Sans JP\";font-weight:400;",
        "src:url(/assets/NotoSansJP-Regular.otf) format(\"opentype\")}",
        "@font-face{font-family:\"Noto Sans JP\";font-weight:700;",
        "src:url(/assets/NotoSansJP-Bold.otf) format(\"opentype\")}",
        // UDEV Gothic も同じ (使われたときにだけ読み込まれる)
        "@font-face{font-family:\"UDEV Gothic\";font-weight:400;",
        "src:url(/assets/UDEVGothic-Regular.ttf) format(\"truetype\")}",
        "@font-face{font-family:\"UDEV Gothic\";font-weight:700;",
        "src:url(/assets/UDEVGothic-Bold.ttf) format(\"truetype\")}",
    ));
    css.push_str(&body_font_css(settings));
    css
}

/// 本文のフォント指定 (@font-face 無し。書き出した HTML は見る側のフォントを使う)
fn body_font_css(settings: &settings::Settings) -> String {
    let face = settings.preview.font_face.trim().replace(['"', '\\'], "");
    let size = settings.preview.font_size.clamp(6, 72);
    let mut css = String::from("body{font-family:");
    if !face.is_empty() {
        css.push_str(&format!("\"{face}\","));
    }
    css.push_str(concat!(
        "-apple-system,\"Segoe UI\",\"Yu Gothic UI\",\"Meiryo\",\"Hiragino Sans\",",
        "\"Noto Sans JP\",sans-serif;"
    ));
    // コードの中のタブの幅 (pre / code に受け継がれる)
    let tab = settings.preview.tab_width.clamp(1, 16);
    css.push_str(&format!("font-size:{size}pt;tab-size:{tab}}}"));
    css
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(base: &Path) -> DocState {
        DocState {
            base_dir: Some(base.to_path_buf()),
            root_dirs: vec![base.join("static")],
            body: String::new(),
            title: String::new(),
            export: false,
        }
    }

    #[test]
    fn request_paths() {
        assert_eq!(
            request_path("https://typball.wxsite/7/img/a.png?v=3"),
            Some("/7/img/a.png")
        );
        assert_eq!(request_path("typball:7/index.html"), Some("7/index.html"));
        assert_eq!(request_path("https://example.com/x"), None);
    }

    #[test]
    fn resolve_rejects_escapes() {
        let dir = std::env::temp_dir().join(format!("typball-wp-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("static/images")).unwrap();
        std::fs::write(dir.join("a.png"), b"x").unwrap();
        std::fs::write(dir.join("static/images/b.png"), b"y").unwrap();
        let d = doc(&dir);
        assert_eq!(resolve_file(&d, "a.png"), Some(dir.join("a.png")));
        assert_eq!(
            resolve_file(&d, "~root/images/b.png"),
            Some(dir.join("static/images/b.png"))
        );
        assert_eq!(resolve_file(&d, "../a.png"), None);
        assert_eq!(resolve_file(&d, "~root/../a.png"), None);
        let abs = dir.join("a.png").to_string_lossy().replace('\\', "/");
        assert_eq!(
            resolve_file(&d, &format!("~abs/{abs}")),
            Some(PathBuf::from(
                dir.join("a.png").to_string_lossy().replace('\\', "/")
            ))
        );
        std::fs::write(dir.join("secret.toml"), b"z").unwrap();
        let toml = dir.join("secret.toml").to_string_lossy().replace('\\', "/");
        assert_eq!(resolve_file(&d, &format!("~abs/{toml}")), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn page_has_csp_and_assets() {
        let d = DocState {
            base_dir: None,
            root_dirs: vec![],
            body: "<p>hi</p>".into(),
            title: "T <1>".into(),
            export: false,
        };
        let html = page_html(&d);
        assert!(html.contains("Content-Security-Policy"));
        assert!(html.contains("<title>T &lt;1&gt;</title>"));
        assert!(html.contains("/assets/preview.js"));
        // スクロール同期の計算は preview.js より先に読む
        let sync = html.find("/assets/scrollsync.js").unwrap();
        assert!(sync < html.find("/assets/preview.js").unwrap());
        assert!(embedded_asset("scrollsync.js").is_some());
        assert!(embedded_asset("preview.css").is_some());
        assert!(embedded_asset("nope.css").is_none());
    }
}
