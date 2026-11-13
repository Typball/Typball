//! 図エディタ (.tbdg): maxGraph を載せた WebView
//!
//! `.tbdg` の中身は SVG で、Typst からは `#image("図.tbdg")` でそのまま読める。再編集用のデータ
//! (maxGraph のモデルの XML) は SVG の `<metadata id="typball-diagram">` に入っている。SVG を組み立てる
//! のはページ (`assets/diagram/diagram.js`) で、こちらは「今のファイルの中身」を文字列として持つ。
//! タブ (`tab.rs`) はテキストの文書と同じように、その文字列を保存・バックアップ・タブの移動に使う。
//!
//! ページとのやり取り (詳細は diagram.js の先頭):
//! - ページの準備ができると "ready" が来るので、中身を `typballDiagram.load(...)` で流し込む
//! - 図が変わるたびに "svg:<全文>" が来る。保存はそのとき持っている文字列を書くだけ
//! - 編集メニューの有効 / 無効に使う状態は "state:<JSON>" で知らせてもらう (WebView に同期で問い合わせると
//!   固まることがある。webpreview.rs)
//! - Mermaid モード (Markdown に貼る Mermaid のフローチャートを作るモード。図のデータの中の印で決まり、
//!   制限も記述の生成もページ側) の「コピー」は "copytext:<文字列>" で来るので、OS のクリップボードに入れる
//! - コピーした図形は "clip:<XML>" で受け取ってプロセス全体で覚え、ほかの図のタブにも貼れるようにする
//!   (OS のクリップボードは使わない)
//! - WebView にフォーカスがあるとメニューのショートカット (Ctrl+S など) がフレームに届かないので、
//!   ページが "key:ctrl+s" のように知らせてくる
//!
//! 文字の編集中 (図形のラベルを入力している途中) の内容はまだ図に入っていない。保存やタブを閉じる
//! 前に [`DiagramEditor::commit_then`] で確定させ、確定の知らせが来てから続きを実行する。

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use wxdragon::event::WebViewEvents;
use wxdragon::prelude::*;
use wxdragon::widgets::webview::{WebView, WebViewHandlerResponse};

use crate::editmenu::EditItem;
use crate::editor::hash_of;
use crate::i18n::tr;
use crate::webpreview::{self, run_script_async};

const HTML: &str = include_str!("../assets/diagram/diagram.html");
const CSS: &str = include_str!("../assets/diagram/diagram.css");
const JS: &str = include_str!("../assets/diagram/diagram.js");
/// maxGraph 0.25.1 (Apache-2.0)。jsDelivr が 1 ファイルにまとめたものをそのまま置いている
const MAXGRAPH: &str = include_str!("../assets/diagram/maxgraph.esm.min.js");
const PAGE_URL: &str = "https://typball.wxsite/diagram/index.html";
/// ネットワーク図のアイコンの表示名と並び順 (セットと種類)。アイコンの中身は [`icons::ICONS`]
const ICON_CATALOG: &str = include_str!("../assets/diagram/icons/catalog.json");
/// アイコンを配る URL の先頭 (`<セット>/<種類>.svg`、`catalog.json`、`files.json` が続く)
const ICONS_PREFIX: &str = "assets/diagram/icons/";

/// 同梱のアイコン: build.rs が assets/diagram/icons/<セット>/<種類>.svg を全部並べた表
mod icons {
    include!(concat!(env!("OUT_DIR"), "/diagram_icons.rs"));
}
/// ページ → Rust のメッセージ用オブジェクト名 (`window.typball_host.postMessage`)
const MESSAGE_HANDLER: &str = "typball_host";
/// 文字の編集の確定を待つ上限。過ぎたら確定を待たずに続きを実行する (ページが応答しないとき保存できなく
/// ならないように)
const COMMIT_TIMEOUT_MS: i32 = 500;

/// 新しい図の中身: 何も描かれていない SVG に、空の編集用データを付けたもの
pub const EMPTY: &str = concat!(
    "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"100\" viewBox=\"0 0 100 100\">",
    "<metadata id=\"typball-diagram\" data-format=\"maxgraph-model-xml\" data-version=\"1\"></metadata>",
    "</svg>\n"
);

thread_local! {
    /// コピーした図形 (maxGraph のモデルの XML)。どの図のタブ・ウィンドウからも貼れる
    static CLIPBOARD: RefCell<String> = const { RefCell::new(String::new()) };
}

/// ページが知らせてきた状態
#[derive(Debug, Clone, Copy, Default)]
struct PageState {
    undo: bool,
    redo: bool,
    selection: u64,
    /// 図形の文字を入力している途中
    editing: bool,
    /// 読めないファイルを開いていて、編集できない
    readonly: bool,
}

pub struct DiagramEditor {
    /// タブのエディタ部分に置くパネル (中身は WebView だけ)
    pub container: Panel,
    webview: Option<WebView>,
    /// 今のファイルの中身 (SVG)
    text: RefCell<String>,
    /// 保存済み (読み込んだとき) の中身のハッシュ。None は「未保存の変更あり」として入れられた
    saved: Cell<Option<u64>>,
    /// ページが "ready" を知らせてきた (load を受け付ける)
    ready: Cell<bool>,
    state: Cell<PageState>,
    on_change: RefCell<Option<Rc<dyn Fn()>>>,
    on_key: RefCell<Option<Rc<dyn Fn(&str)>>>,
    /// 文字の編集が確定したら実行する処理
    after_commit: RefCell<Vec<Box<dyn FnOnce()>>>,
    commit_timer: Timer<Panel>,
}

impl DiagramEditor {
    /// `parent` の中に図エディタを作る。中身は [`DiagramEditor::set_text`] で入れる
    pub fn new(parent: &Panel) -> Rc<DiagramEditor> {
        let container = Panel::builder(parent).build();
        let sizer = BoxSizer::builder(Orientation::Vertical).build();
        let webview = if webpreview::available() {
            let wv = WebView::builder(&container)
                .with_backend(webpreview::backend())
                .build();
            wv.register_handler(webpreview::SCHEME, serve);
            wv.add_script_message_handler(MESSAGE_HANDLER);
            wv.enable_context_menu(false);
            sizer.add(&wv, 1, SizerFlag::Expand, 0);
            Some(wv)
        } else {
            let text = StaticText::builder(&container)
                .with_label(&tr("diagram.webview_missing"))
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
        let commit_timer = Timer::new(&container);
        let this = Rc::new(DiagramEditor {
            container,
            webview,
            text: RefCell::new(String::new()),
            saved: Cell::new(Some(hash_of(""))),
            ready: Cell::new(false),
            state: Cell::new(PageState::default()),
            on_change: RefCell::new(None),
            on_key: RefCell::new(None),
            after_commit: RefCell::new(Vec::new()),
            commit_timer,
        });
        this.bind_events();
        if let Some(wv) = this.webview {
            wv.load_url(PAGE_URL);
        }
        this
    }

    fn bind_events(self: &Rc<Self>) {
        let weak: Weak<Self> = Rc::downgrade(self);
        if let Some(wv) = self.webview {
            wv.on_script_message_received({
                let weak = weak.clone();
                move |event| {
                    let (Some(this), Some(message)) = (weak.upgrade(), event.get_string()) else {
                        return;
                    };
                    this.handle_message(&message);
                }
            });
        }
        self.commit_timer.on_tick(move |_| {
            if let Some(this) = weak.upgrade() {
                this.run_after_commit();
            }
        });
    }

    fn handle_message(&self, message: &str) {
        if message == "ready" {
            self.ready.set(true);
            self.push_text();
        } else if let Some(text) = message.strip_prefix("svg:") {
            if *self.text.borrow() != text {
                *self.text.borrow_mut() = text.to_string();
                self.notify_change();
            }
        } else if let Some(json) = message.strip_prefix("state:") {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(json) {
                let flag = |key: &str| value.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
                self.state.set(PageState {
                    undo: flag("undo"),
                    redo: flag("redo"),
                    selection: value.get("sel").and_then(|v| v.as_u64()).unwrap_or(0),
                    editing: flag("editing"),
                    readonly: flag("readonly"),
                });
            }
        } else if message == "committed" {
            // 文字の確定と、確定した図の受け渡し ("svg:") が済んだ。状態の知らせ ("state:" の editing) は
            // 図より先に届くので、続きの処理はこの知らせを待ってから実行する
            self.run_after_commit();
        } else if let Some(xml) = message.strip_prefix("clip:") {
            CLIPBOARD.with(|c| *c.borrow_mut() = xml.to_string());
        } else if message == "paste" {
            self.paste();
        } else if let Some(text) = message.strip_prefix("copytext:") {
            // Mermaid モードの「コピー」: Markdown に貼る Mermaid の記述を OS のクリップボードへ
            webpreview::set_clipboard_text(text);
        } else if let Some(key) = message.strip_prefix("key:") {
            // 呼び先はタブを閉じてこの WebView を破棄することがある (Ctrl+W)。WebView のイベント処理の
            // 中で破棄しないよう、処理を抜けてから実行する
            let handler = self.on_key.borrow().clone();
            if let Some(handler) = handler {
                let key = key.to_string();
                defer(move || handler(&key));
            }
        }
    }

    fn notify_change(&self) {
        let handler = self.on_change.borrow().clone();
        if let Some(handler) = handler {
            handler();
        }
    }

    /// 図が変わったとき (中身の文字列が入れ替わった後) に呼ぶ処理
    pub fn on_change(&self, f: impl Fn() + 'static) {
        *self.on_change.borrow_mut() = Some(Rc::new(f));
    }

    /// ページが使わなかったショートカット ("ctrl+s" など) を受け取る処理
    pub fn on_key(&self, f: impl Fn(&str) + 'static) {
        *self.on_key.borrow_mut() = Some(Rc::new(f));
    }

    /// 今のファイルの中身 (SVG)
    pub fn text(&self) -> String {
        self.text.borrow().clone()
    }

    /// ファイルの中身を入れる (開く・読み直し・タブの移動)。`modified` なら未保存の変更ありの扱い
    pub fn set_text(&self, text: &str, modified: bool) {
        *self.text.borrow_mut() = text.to_string();
        self.saved
            .set(if modified { None } else { Some(hash_of(text)) });
        self.state.set(PageState::default());
        self.push_text();
    }

    /// 未保存の変更があるか。元に戻して保存時と同じ図になれば、変更なしに戻る
    pub fn is_modified(&self) -> bool {
        self.saved.get() != Some(hash_of(self.text.borrow().as_str()))
    }

    /// 保存した: 今の中身を「保存済み」にする
    pub fn mark_saved(&self) {
        self.saved.set(Some(hash_of(self.text.borrow().as_str())));
    }

    /// 中身をページに流し込む (ページの準備ができていれば。まだなら "ready" が来たときに流す)
    fn push_text(&self) {
        let Some(wv) = self.webview else {
            return;
        };
        if !self.ready.get() {
            return;
        }
        let text =
            serde_json::to_string(self.text.borrow().as_str()).unwrap_or_else(|_| "\"\"".into());
        run_script_async(
            &wv,
            &format!(
                "window.typballDiagram && typballDiagram.load({text},{});",
                options()
            ),
        );
    }

    /// 配色か言語が変わった: ページの表示を合わせる (図の中身はそのまま)
    pub fn apply_options(&self) {
        if let Some(wv) = self.webview {
            run_script_async(
                &wv,
                &format!(
                    "window.typballDiagram && typballDiagram.setOptions({});",
                    options()
                ),
            );
        }
    }

    pub fn focus(&self) {
        match self.webview {
            Some(wv) => wv.set_focus(),
            None => self.container.set_focus(),
        }
    }

    /// 編集メニューの項目がいま実行できるか
    pub fn can(&self, item: EditItem) -> bool {
        let state = self.state.get();
        if self.webview.is_none() || state.readonly {
            return false;
        }
        match item {
            EditItem::Undo => state.undo || state.editing,
            EditItem::Redo => state.redo || state.editing,
            EditItem::Cut | EditItem::Copy | EditItem::Delete => {
                state.selection > 0 || state.editing
            }
            // 文字の編集中の貼り付けは OS のクリップボードの文字 (キー操作の Ctrl+V はページが直接受ける)
            EditItem::Paste => !state.editing && CLIPBOARD.with(|c| !c.borrow().is_empty()),
            EditItem::SelectAll => true,
        }
    }

    /// 編集メニューの項目を実行する
    pub fn apply(&self, item: EditItem) {
        match item {
            EditItem::Undo => self.command("undo"),
            EditItem::Redo => self.command("redo"),
            EditItem::Cut => self.command("cut"),
            EditItem::Copy => self.command("copy"),
            EditItem::Paste => self.paste(),
            EditItem::Delete => self.command("delete"),
            EditItem::SelectAll => self.command("selectAll"),
        }
    }

    fn command(&self, name: &str) {
        if let Some(wv) = self.webview {
            run_script_async(
                &wv,
                &format!("window.typballDiagram && typballDiagram.command(\"{name}\");"),
            );
        }
    }

    /// 覚えている図形を貼り付ける
    fn paste(&self) {
        let Some(wv) = self.webview else {
            return;
        };
        let xml = CLIPBOARD.with(|c| c.borrow().clone());
        if xml.is_empty() {
            return;
        }
        let xml = serde_json::to_string(&xml).unwrap_or_else(|_| "\"\"".into());
        run_script_async(
            &wv,
            &format!("window.typballDiagram && typballDiagram.command(\"paste\",{xml});"),
        );
    }

    /// 図形の文字を入力している途中なら、確定させてから `f` を実行して true を返す (`f` は後で呼ばれる)。
    /// 途中でなければ何もせず false を返す (呼び出し側がそのまま続ける)
    pub fn commit_then(&self, f: impl FnOnce() + 'static) -> bool {
        if !self.state.get().editing || self.webview.is_none() {
            return false;
        }
        self.after_commit.borrow_mut().push(Box::new(f));
        self.command("commit");
        self.commit_timer.start(COMMIT_TIMEOUT_MS, true);
        true
    }

    /// 確定を待っていた処理を実行する (確定の知らせが来たとき、または待ち時間が過ぎたとき)
    fn run_after_commit(&self) {
        let pending: Vec<Box<dyn FnOnce()>> = self.after_commit.borrow_mut().drain(..).collect();
        if pending.is_empty() {
            return;
        }
        self.commit_timer.stop();
        // 確定できなかった (ページが応答しない) ときも、次の保存は待たずに進める
        let mut state = self.state.get();
        state.editing = false;
        self.state.set(state);
        // 続きの処理 (保存・タブを閉じる) も、WebView やタイマーのイベント処理を抜けてから実行する
        defer(move || {
            for f in pending {
                f();
            }
        });
    }

    /// タブを閉じるとき: タブを捕まえている処理を手放す (タブ ⇔ 図エディタの循環参照を切る)
    pub fn clear_handlers(&self) {
        self.commit_timer.stop();
        self.on_change.borrow_mut().take();
        self.on_key.borrow_mut().take();
        self.after_commit.borrow_mut().clear();
    }
}

impl Drop for DiagramEditor {
    fn drop(&mut self) {
        self.commit_timer.stop();
    }
}

thread_local! {
    /// [`defer`] で後回しにした処理
    static DEFERRED: RefCell<Vec<Box<dyn FnOnce()>>> = const { RefCell::new(Vec::new()) };
}

/// 今のイベント処理が終わった後 (アイドル時) に GUI スレッドで実行する。
/// `wxdragon::call_after` は別スレッドからも呼べるように `Send` な処理しか受け取らないので、
/// 処理そのものはこのスレッドの置き場に入れ、取り出して実行するだけの処理を渡す
fn defer(f: impl FnOnce() + 'static) {
    DEFERRED.with(|q| q.borrow_mut().push(Box::new(f)));
    wxdragon::call_after(Box::new(|| {
        let pending: Vec<Box<dyn FnOnce()>> = DEFERRED.with(|q| q.borrow_mut().drain(..).collect());
        for f in pending {
            f();
        }
    }));
    // call_after の中身はアイドル時に実行されるので、何も起きていないアプリを起こす
    wxdragon::wake_up_idle();
}

/// 図を 1 ページに置いただけの Typst の文書 (プレビューウィンドウ用)。SVG は文字列として埋め込むので、
/// ファイルを読みに行かない。ページは A4 の幅にして図を中央に置く: 図の大きさのページにすると、
/// 小さな図が「幅に合わせる」表示で何倍にも拡大されてしまう。幅に収まらない図は、幅に合わせて縮む
pub fn typst_source(svg: &str) -> String {
    let mut escaped = String::with_capacity(svg.len() + 16);
    for c in svg.chars() {
        match c {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            _ => escaped.push(c),
        }
    }
    format!(
        "#set page(width: 21cm, height: auto, margin: 1cm)\n#align(center, image(bytes(\"{escaped}\"), format: \"svg\"))\n"
    )
}

/// ページに渡す設定 (配色と言語) の JSON
fn options() -> String {
    format!(
        "{{\"theme\":\"{}\",\"lang\":\"{}\"}}",
        if crate::theme::ui_dark() {
            "dark"
        } else {
            "light"
        },
        crate::i18n::current_language().replace(['"', '\\'], "")
    )
}

/// 図エディタの WebView からの要求に応える (ページ本体と、同梱のスクリプト・スタイル・フォント)
fn serve(uri: &str) -> Option<WebViewHandlerResponse> {
    let path = webpreview::request_path(uri)?;
    let path = path.trim_start_matches('/');
    let (data, mime): (&[u8], &str) = match path {
        "diagram/index.html" => (HTML.as_bytes(), "text/html"),
        "assets/diagram/diagram.css" => (CSS.as_bytes(), "text/css"),
        "assets/diagram/diagram.js" => (JS.as_bytes(), "text/javascript"),
        "assets/diagram/maxgraph.esm.min.js" => (MAXGRAPH.as_bytes(), "text/javascript"),
        _ => {
            if let Some(name) = path.strip_prefix(ICONS_PREFIX) {
                return serve_icon(name);
            }
            return webpreview::embedded_asset(path.strip_prefix("assets/")?);
        }
    };
    Some(WebViewHandlerResponse {
        data: data.to_vec(),
        mime_type: Some(mime.to_string()),
    })
}

/// アイコンまわりの要求に応える: 一覧 (`catalog.json` = 表示名、`files.json` = 実際にあるアイコン) と中身
fn serve_icon(name: &str) -> Option<WebViewHandlerResponse> {
    let (data, mime): (Vec<u8>, &str) = match name {
        "catalog.json" => (ICON_CATALOG.as_bytes().to_vec(), "application/json"),
        "files.json" => (icon_files_json().into_bytes(), "application/json"),
        _ => {
            let key = name.strip_suffix(".svg")?;
            let (_, data) = icons::ICONS.iter().find(|(n, _)| *n == key)?;
            (data.to_vec(), "image/svg+xml")
        }
    };
    Some(WebViewHandlerResponse {
        data,
        mime_type: Some(mime.to_string()),
    })
}

/// 同梱しているアイコンの名前の一覧 (`["セット/種類", ...]`)。名前は英小文字・数字・ハイフンと `/` だけ
/// (build.rs が確かめている) なので、そのまま JSON の文字列にできる
fn icon_files_json() -> String {
    let names: Vec<String> = icons::ICONS
        .iter()
        .map(|(name, _)| format!("\"{name}\""))
        .collect();
    format!("[{}]", names.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// アイコンの一覧 (catalog.json) と、同梱しているファイルが食い違っていない
    #[test]
    fn icon_catalog_matches_the_bundled_files() {
        let catalog: serde_json::Value = serde_json::from_str(ICON_CATALOG).unwrap();
        let sets = catalog["sets"].as_array().unwrap();
        let files: Vec<String> = serde_json::from_str(&icon_files_json()).unwrap();
        assert!(!files.is_empty());
        // どのセットにもアイコンがあり、色を変えられるセットは、どのアイコンにも置き換える色の文字列がある
        for set in sets {
            let id = set["id"].as_str().unwrap();
            let own: Vec<&(&str, &[u8])> = icons::ICONS
                .iter()
                .filter(|(name, _)| name.split('/').next() == Some(id))
                .collect();
            assert!(!own.is_empty(), "{id} has no icons");
            if let Some(tint) = set["tint"].as_array() {
                for (name, data) in own {
                    let text = String::from_utf8_lossy(data);
                    assert!(
                        tint.iter().any(|t| text.contains(t.as_str().unwrap())),
                        "{name} uses none of {tint:?}"
                    );
                }
            }
        }
        // 同梱しているアイコンは、どれも一覧にあるセットのもの
        for name in &files {
            let set = name.split('/').next().unwrap();
            assert!(
                sets.iter().any(|s| s["id"].as_str() == Some(set)),
                "{name} is in a set that catalog.json does not list"
            );
        }
        // 表示名のある種類は、どこかのセットに実物がある (書き間違いの検出)
        for kind in catalog["kinds"].as_array().unwrap() {
            let id = kind["id"].as_str().unwrap();
            assert!(
                files.iter().any(|f| f.split('/').nth(1) == Some(id)),
                "catalog.json names the kind {id}, but no set has it"
            );
        }
        let uri = "https://typball.wxsite/assets/diagram/icons/affinity-circle/router.svg";
        assert!(serve(uri).is_some());
        assert!(serve("https://typball.wxsite/assets/diagram/icons/catalog.json").is_some());
        assert!(serve("https://typball.wxsite/assets/diagram/icons/nope/router.svg").is_none());
    }

    #[test]
    fn page_and_assets_are_served() {
        for path in [
            "diagram/index.html",
            "assets/diagram/diagram.css",
            "assets/diagram/diagram.js",
            "assets/diagram/maxgraph.esm.min.js",
            "assets/NotoSansJP-Regular.otf",
            "assets/NotoSansJP-Bold.otf",
        ] {
            let uri = format!("https://typball.wxsite/{path}");
            assert!(serve(&uri).is_some(), "{path}");
        }
        assert!(serve("https://typball.wxsite/assets/diagram/nope.js").is_none());
        assert!(serve("https://typball.wxsite/1/index.html").is_none());
    }

    /// ページが読むファイルは、どれも HTML が指している名前で配っている
    #[test]
    fn page_references_only_served_files() {
        assert!(HTML.contains("/assets/diagram/diagram.css"));
        assert!(HTML.contains("/assets/diagram/diagram.js"));
        assert!(JS.contains("'./maxgraph.esm.min.js'"));
        assert!(CSS.contains("/assets/NotoSansJP-Regular.otf"));
        // 図の文字のフォント名は typball-core の別名と同じ
        assert!(JS.contains(&format!(
            "const FONT = '{}';",
            typball_core::DIAGRAM_FONT_FAMILY
        )));
        assert!(CSS.contains(&format!(
            "font-family: \"{}\";",
            typball_core::DIAGRAM_FONT_FAMILY
        )));
    }

    /// 図を Typst の文書に包むと、そのまま組める (文字列の中の引用符と円記号は逃がす)
    #[test]
    fn typst_source_compiles() {
        let svg = concat!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"40\" height=\"20\" viewBox=\"0 0 40 20\">",
            "<metadata id=\"typball-diagram\">a\\b &quot;c&quot;</metadata>",
            "<rect x=\"1\" y=\"1\" width=\"38\" height=\"18\" fill=\"#fff\" stroke=\"#000\"/></svg>"
        );
        let (doc, diagnostics) = typball_core::compile_doc(&typst_source(svg), None);
        assert!(doc.is_some(), "{diagnostics:?}");
        // 新しい図 (何も描かれていない) も組める
        let (doc, diagnostics) = typball_core::compile_doc(&typst_source(EMPTY), None);
        assert!(doc.is_some(), "{diagnostics:?}");
    }

    /// 新しい図の中身は SVG として先頭から始まり (Typst は先頭付近で形式を判定する)、編集用データの印を持つ
    #[test]
    fn empty_diagram_is_an_svg_with_the_marker() {
        assert!(EMPTY.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
        assert!(EMPTY.contains("<metadata id=\"typball-diagram\""));
    }
}
