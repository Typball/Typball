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

use crate::i18n::{tr, tr_args};
use crate::mdhtml;
use crate::settings::{self, MarkdownMode};
use crate::site::{self, Project, Server};

/// 独自スキーム名。wx はこれを `https://<スキーム>.wxsite/` の仮想ホストにする
const SCHEME: &str = "typball";
const HOST: &str = "typball.wxsite";
/// ページ → Rust のメッセージ用オブジェクト名 (`window.typball_host.postMessage`)
const MESSAGE_HANDLER: &str = "typball_host";
const CSS: &str = include_str!("../assets/preview.css");
const JS: &str = include_str!("../assets/preview.js");
/// サーバーの起動待ちの間隔と上限 (250 ms × 240 = 60 秒)
const READY_POLL_MS: i32 = 250;
const READY_MAX_TICKS: u32 = 240;
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

/// モード切り替えのバー (サイトのプロジェクトが見つかったときだけ出す)
struct ModeBar {
    choice: Choice,
    label: StaticText,
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
    ready_timer: Timer<Panel>,
    ready_ticks: Cell<u32>,
    /// 保存のたびに増やす (画像のキャッシュ更新用)
    version: Cell<u64>,
    /// 最後にプレビューへ送った (行, 基準点 × 1000)。同じなら送り直さない
    last_anchor: Cell<(i32, i32)>,
    sync_guard_until: Cell<Option<Instant>>,
}

/// この環境で WebView が使えるか (Windows は WebView2 ランタイムの有無)
pub fn available() -> bool {
    WebView::is_backend_available(backend())
}

fn backend() -> WebViewBackend {
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

    /// サイトモードのときの開発サーバーの URL と stderr の末尾 (コンソールペイン用)
    pub fn server_log(&self) -> Option<String> {
        if self.mode.get() != Mode::Site {
            return None;
        }
        let server = self.server.borrow();
        let server = server.as_ref()?;
        let mut out = tr_args("console.site_server", &[("url", &server.url)]);
        let tail = server.stderr_tail(20);
        if !tail.is_empty() {
            out.push('\n');
            out.push_str(&tail);
        }
        Some(out)
    }

    /// `parent` の中にプレビューを作る。`path` は文書のパス (サイトのプロジェクト検出に使う)
    pub fn new(parent: &Panel, frame: Frame, path: Option<&Path>) -> Rc<WebPreview> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let project = path.and_then(site::detect);
        let container = Panel::builder(parent).build();
        let sizer = BoxSizer::builder(Orientation::Vertical).build();

        let bar = project.as_ref().map(|_| {
            let panel = Panel::builder(&container).build();
            let choice = Choice::builder(&panel).build();
            choice.freeze();
            choice.append(&tr("preview.mode_markdown"));
            choice.append(&tr("preview.mode_site"));
            choice.thaw();
            let label = StaticText::builder(&panel).with_label("").build();
            let hbox = BoxSizer::builder(Orientation::Horizontal).build();
            hbox.add(
                &choice,
                0,
                SizerFlag::AlignCenterVertical | SizerFlag::All,
                panel.from_dip_int(4),
            );
            hbox.add(
                &label,
                1,
                SizerFlag::AlignCenterVertical | SizerFlag::All,
                panel.from_dip_int(4),
            );
            panel.set_sizer(hbox, true);
            sizer.add(&panel, 0, SizerFlag::Expand, 0);
            ModeBar { choice, label }
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
        let settings = settings::get();
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
        if let Some(bar) = &this.bar {
            bar.choice
                .set_selection(if mode == Mode::Site { 1 } else { 0 });
        }
        this.apply_mode();
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
                        this.flush();
                    }
                }
            });
        }
        if let Some(bar) = &self.bar {
            bar.choice.on_selection_changed({
                let weak = weak.clone();
                move |_| {
                    let Some(this) = weak.upgrade() else {
                        return;
                    };
                    let mode = match this.bar.as_ref().and_then(|b| b.choice.get_selection()) {
                        Some(1) => Mode::Site,
                        _ => Mode::Markdown,
                    };
                    if mode != this.mode.get() {
                        this.mode.set(mode);
                        this.apply_mode();
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

    fn set_label(&self, text: &str) {
        if let Some(bar) = &self.bar {
            bar.label.set_label(text);
            self.container.layout();
        }
    }

    /// ページを読み直す (プレビューのフォント設定が変わったとき)。本文は殻ページに埋め込まれるので失われない
    pub fn reload_page(&self) {
        if self.mode.get() == Mode::Markdown {
            self.apply_mode();
        }
    }

    /// 今のモードの表示を始める
    fn apply_mode(&self) {
        let Some(wv) = self.webview else {
            return;
        };
        self.ready_timer.stop();
        match self.mode.get() {
            Mode::Markdown => {
                self.loaded.set(false);
                self.set_label("");
                wv.load_url(&self.page_url());
            }
            Mode::Site => {
                self.loaded.set(false);
                self.start_site();
            }
        }
    }

    fn start_site(&self) {
        let Some(project) = &self.project else {
            return;
        };
        let settings = settings::get();
        match site::server_for(
            &project.root,
            &settings.preview.site_command,
            &settings.preview.site_url,
        ) {
            Ok(server) => {
                *self.server.borrow_mut() = Some(server);
                self.ready_ticks.set(0);
                self.set_label(&tr("preview.site_starting"));
                // 既に動いているサーバーならすぐ表示、起動中ならタイマーで待つ
                self.check_server();
                if self.server.borrow().is_some() && !self.loaded.get() {
                    self.ready_timer.start(READY_POLL_MS, false);
                }
            }
            Err(e) => self.site_failed(&e),
        }
    }

    fn site_failed(&self, error: &str) {
        self.ready_timer.stop();
        *self.server.borrow_mut() = None;
        let msg = tr_args("preview.site_error", &[("error", error)]);
        self.set_label(&msg);
        self.frame.set_status_text(&msg, 0);
    }

    /// サーバーの起動待ち (タイマーから)
    fn check_server(&self) {
        let server = self.server.borrow().clone();
        let Some(server) = server else {
            self.ready_timer.stop();
            return;
        };
        if let Some(err) = server.exit_error() {
            self.site_failed(&err);
            return;
        }
        if server.is_ready() {
            self.ready_timer.stop();
            // サイトモードでは `loaded` を「サーバーのページを出した」印に使う
            self.loaded.set(true);
            if let Some(wv) = self.webview {
                wv.load_url(&server.url);
            }
            self.set_label(&tr("preview.site_hint"));
            return;
        }
        let ticks = self.ready_ticks.get() + 1;
        self.ready_ticks.set(ticks);
        if ticks > READY_MAX_TICKS {
            self.site_failed(&tr("preview.site_timeout"));
        }
    }

    /// Markdown を変換して表示を更新する (Markdown モードのとき)
    pub fn set_markdown(&self, text: &str, path: Option<&Path>) {
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
        if self.mode.get() == Mode::Markdown && self.loaded.get() {
            self.flush();
        }
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
        run_script_async(&wv, &format!("window.typball && typball.update({json});"));
    }

    /// 保存した: 画像のキャッシュを更新させる (呼び出し側が続けて set_markdown する)
    pub fn on_saved(&self) {
        self.version.set(self.version.get() + 1);
    }

    /// エディタの行 `line` (1 始まり) に対応する要素を、プレビューの画面の `anchor` (0 = 上端, 1 = 下端)
    /// の位置に来るようにスクロールする
    pub fn scroll_to_line(&self, line: i32, anchor: f64) {
        let Some(wv) = self.webview else {
            return;
        };
        if self.mode.get() != Mode::Markdown || !self.loaded.get() {
            return;
        }
        if let Some(until) = self.sync_guard_until.get() {
            if Instant::now() < until {
                return;
            }
        }
        let key = (line, (anchor * 1000.0).round() as i32);
        if self.last_anchor.get() == key {
            return;
        }
        self.last_anchor.set(key);
        run_script_async(
            &wv,
            &format!("window.typball && typball.scrollToLine({line}, {anchor:.3});"),
        );
    }

    /// プレビュー側のスクロールをエディタに反映した直後に呼ぶ (往復の揺れを止める)
    pub fn note_preview_scroll(&self, line: i32, anchor: f64) {
        self.last_anchor
            .set((line, (anchor * 1000.0).round() as i32));
        self.sync_guard_until.set(Some(Instant::now() + SYNC_GUARD));
    }
}

impl Drop for WebPreview {
    fn drop(&mut self) {
        self.ready_timer.stop();
        docs().lock().unwrap().remove(&self.id);
    }
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

fn set_clipboard_text(text: &str) {
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
        webview.on_loaded({
            let url = url.clone();
            move |event| {
                if started.get() || !event.get_url().unwrap_or_default().starts_with(&url) {
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
fn request_path(uri: &str) -> Option<&str> {
    let path = if let Some(i) = uri.find(HOST) {
        &uri[i + HOST.len()..]
    } else if let Some(rest) = uri.strip_prefix(&format!("{SCHEME}:")) {
        rest.trim_start_matches('/')
    } else {
        return None;
    };
    Some(path.split(['?', '#']).next().unwrap_or(path))
}

fn embedded_asset(name: &str) -> Option<WebViewHandlerResponse> {
    let fonts = typball_core::embedded_font_data();
    let (data, mime): (&[u8], &str) = match name {
        "preview.css" => (CSS.as_bytes(), "text/css"),
        "preview.js" => (JS.as_bytes(), "application/javascript"),
        // 同梱の Noto Sans JP (インストールされていなくても @font-face で使えるように)
        "NotoSansJP-Regular.otf" => (fonts.first()?, "font/otf"),
        "NotoSansJP-Bold.otf" => (fonts.get(1)?, "font/otf"),
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
    const ALLOWED: [&str; 22] = [
        "png", "jpg", "jpeg", "gif", "svg", "webp", "avif", "bmp", "ico", "apng", "css", "woff",
        "woff2", "ttf", "otf", "pdf", "mp4", "webm", "mp3", "ogg", "wav", "txt",
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
        Some("svg") => "image/svg+xml",
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

/// 設定のプレビューフォントを本文に当てる CSS。同梱の Noto Sans JP は `/assets/` から @font-face で読む
/// (インストールされていない機械でも同じ見た目になる)。後ろに preview.css と同じフォールバックを並べる
fn font_css(settings: &settings::Settings) -> String {
    let mut css = String::from(concat!(
        "@font-face{font-family:\"Noto Sans JP\";font-weight:400;",
        "src:url(/assets/NotoSansJP-Regular.otf) format(\"opentype\")}",
        "@font-face{font-family:\"Noto Sans JP\";font-weight:700;",
        "src:url(/assets/NotoSansJP-Bold.otf) format(\"opentype\")}",
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
    css.push_str(&format!("font-size:{size}pt}}"));
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
        assert!(embedded_asset("preview.css").is_some());
        assert!(embedded_asset("nope.css").is_none());
    }
}
