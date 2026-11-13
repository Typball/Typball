//! Typst の文書の中の Mermaid
//!
//! Typst の文書に書いた Mermaid のコードブロック (```` ```mermaid ````) を、描いた図に置き換える。
//! Typst は Mermaid を知らないので、Typball が橋渡しをする:
//!
//! 1. コンパイルの前に本文から Mermaid のコードを探す ([`blocks`]。Typst と同じ `typst-syntax` で読む)。
//! 2. まだ描いていないコードは、見えない WebView に読み込んだ Mermaid (`assets/mermaid/`) に描かせる
//!    ([`render_pending`])。結果の SVG はコードごとに覚える。
//! 3. 描けたコードについて、「このコードのコードブロックはこの SVG の画像にする」という表示ルールを
//!    本文の先頭に差し込む ([`prelude`]。`typball_core::set_prelude_hook`)。プレビュー・書き出し・
//!    プレビューウィンドウのどのコンパイルにも同じように効く。
//! 4. 新しく描けたら知らせて ([`on_rendered`])、開いている Typst のプレビューを組み直す。
//!
//! 描けるまでの間と、書き間違いで描けないコードは、コードブロックのまま出る (理由はコンソールに出す)。
//! SVG は Typst (resvg) が描けるものにしてある: 文字は `<text>`、フォントは同梱の Noto Sans JP の別名
//! (`typball_core::DIAGRAM_FONT_FAMILY`。「Noto Sans JP」のままだと、可変フォント版のある PC で極細になる)。
//!
//! Markdown のプレビューは、プレビューのページ自身が Mermaid を描く (`assets/preview.js`)。こちらは関わらない。

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};

use typst_syntax::ast;
use typst_syntax::SyntaxNode;
use wxdragon::event::WebViewEvents;
use wxdragon::prelude::*;
use wxdragon::widgets::webview::{WebView, WebViewHandlerResponse};

use crate::i18n::tr;
use crate::webpreview::{self, run_script_async};

const HTML: &str = include_str!("../assets/mermaid/render.html");
const JS: &str = include_str!("../assets/mermaid/render.js");
const PAGE_URL: &str = "https://typball.wxsite/mermaid/render.html";
const MESSAGE_HANDLER: &str = "typball_host";
/// 覚えておく図の数。超えたら全部忘れる (開いている文書の分は、次のコンパイルで描き直される)
const CACHE_LIMIT: usize = 300;

/// 描いた結果: SVG か、描けなかった理由
type Outcome = Result<String, String>;

/// コード → 描いた結果。コンパイルは GUI 以外のスレッドでも走るので、スレッドをまたいで読めるようにしておく
fn cache() -> &'static Mutex<HashMap<String, Outcome>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Outcome>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// まだ描いていないコード (コンパイルのときに見つけ、GUI スレッドの [`render_pending`] が描かせる)
fn pending() -> &'static Mutex<Vec<String>> {
    static PENDING: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(Vec::new()))
}

/// 起動時に 1 回呼ぶ: Typst のコンパイルに Mermaid の表示ルールを差し込む
pub fn install() {
    typball_core::set_prelude_hook(prelude);
}

/// Typst の本文の中の Mermaid のコード (出てくる順、同じものは 1 つ)
pub fn blocks(source: &str) -> Vec<String> {
    // ほとんどの文書には無いので、構文を読む前に文字だけで確かめる
    if !source.contains("mermaid") {
        return Vec::new();
    }
    let mut out = Vec::new();
    collect(&typst_syntax::parse(source), &mut out);
    out
}

fn collect(node: &SyntaxNode, out: &mut Vec<String>) {
    if let Some(raw) = node.cast::<ast::Raw>() {
        if raw.lang().is_some_and(|lang| lang.get() == "mermaid") {
            // Typst の `raw.text` と同じ作り方 (行を "\n" でつなぐ)。表示ルールがこの文字列で引く
            let text = raw
                .lines()
                .map(|line| line.get().to_string())
                .collect::<Vec<_>>()
                .join("\n");
            if !text.trim().is_empty() && !out.contains(&text) {
                out.push(text);
            }
        }
        return;
    }
    for child in node.children() {
        collect(child, out);
    }
}

/// Typst の文字列にする (1 行に収める: 前置きは改行を含められない)
fn typst_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// 本文の先頭に差し込む表示ルール。描けている Mermaid のコードブロックを SVG の画像に置き換える。
/// まだ描いていないコードは、描く待ちの一覧に入れる
fn prelude(source: &str) -> String {
    let codes = blocks(source);
    if codes.is_empty() {
        return String::new();
    }
    let mut pairs = Vec::new();
    let mut missing = Vec::new();
    {
        let cache = cache().lock().unwrap();
        for code in codes {
            match cache.get(&code) {
                Some(Ok(svg)) => {
                    pairs.push(format!("{}: {}", typst_string(&code), typst_string(svg)))
                }
                Some(Err(_)) => {}
                None => missing.push(code),
            }
        }
    }
    if !missing.is_empty() {
        let mut pending = pending().lock().unwrap();
        for code in missing {
            if !pending.contains(&code) {
                pending.push(code);
            }
        }
    }
    if pairs.is_empty() {
        return String::new();
    }
    format!(
        concat!(
            "#show raw.where(lang: \"mermaid\"): it => {{ let m = ({},); ",
            "let s = m.at(it.text, default: none); ",
            "if s == none {{ it }} else {{ block(image(bytes(s), format: \"svg\")) }} }};"
        ),
        pairs.join(", ")
    )
}

/// 本文の中の、描けなかった Mermaid についての知らせ (コンソールに出す行)
pub fn diagnostics(source: &str) -> Vec<String> {
    let codes = blocks(source);
    if codes.is_empty() {
        return Vec::new();
    }
    let cache = cache().lock().unwrap();
    codes
        .iter()
        .filter_map(|code| match cache.get(code) {
            Some(Err(message)) => {
                let first = code.lines().next().unwrap_or_default();
                Some(format!("warning: Mermaid ({first}): {message}"))
            }
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 描く (見えない WebView)
// ---------------------------------------------------------------------------

struct Renderer {
    webview: WebView,
    /// WebView を置いたパネル (そのウィンドウが閉じるときに手放す。[`release`])
    parent: usize,
    /// ページが "ready" を知らせてきた
    ready: Cell<bool>,
    /// 描かせている最中のコード (番号 → コード)
    in_flight: RefCell<HashMap<u64, String>>,
    next_id: Cell<u64>,
}

thread_local! {
    static RENDERER: RefCell<Option<Rc<Renderer>>> = const { RefCell::new(None) };
    static LISTENERS: RefCell<Vec<Rc<dyn Fn()>>> = const { RefCell::new(Vec::new()) };
}

/// 新しく描けた (描けないと分かった) ときに呼ぶ処理を登録する。開いている Typst のプレビューを組み直すのに使う
pub fn on_rendered(f: impl Fn() + 'static) {
    LISTENERS.with(|l| l.borrow_mut().push(Rc::new(f)));
}

fn notify() {
    let listeners: Vec<Rc<dyn Fn()>> = LISTENERS.with(|l| l.borrow().clone());
    for f in listeners {
        f();
    }
}

fn store(code: String, outcome: Outcome) {
    let mut cache = cache().lock().unwrap();
    if cache.len() >= CACHE_LIMIT {
        cache.clear();
    }
    cache.insert(code, outcome);
}

/// 描く待ちのコードがあれば描かせる (GUI スレッドで、Typst をコンパイルした後に呼ぶ)。
/// `parent` は見えない WebView を置くパネル (そのウィンドウの中のどこか)
pub fn render_pending(parent: &Panel) {
    if pending().lock().unwrap().is_empty() {
        return;
    }
    if !webpreview::available() {
        // WebView2 が無い: 描けないことを覚えて、何度も試さない
        let codes: Vec<String> = pending().lock().unwrap().drain(..).collect();
        for code in codes {
            store(code, Err(tr("preview.webview_missing")));
        }
        notify();
        return;
    }
    let renderer = RENDERER.with(|r| r.borrow().clone());
    let renderer = match renderer {
        Some(renderer) => renderer,
        None => {
            let renderer = Renderer::new(parent);
            RENDERER.with(|r| *r.borrow_mut() = Some(renderer.clone()));
            renderer
        }
    };
    if renderer.ready.get() {
        renderer.flush();
    }
}

/// ウィンドウを閉じるとき: 見えない WebView がそのウィンドウ (のパネル) にあるなら手放す。
/// 描いている途中だったコードは待ちに戻す (次に描くときに、別のウィンドウで作り直す)
pub fn release(parent: &Panel) {
    let key = parent.handle_ptr() as usize;
    let renderer = RENDERER.with(|r| {
        let hosted = r.borrow().as_ref().is_some_and(|x| x.parent == key);
        if hosted {
            r.borrow_mut().take()
        } else {
            None
        }
    });
    if let Some(renderer) = renderer {
        let mut pending = pending().lock().unwrap();
        for (_, code) in renderer.in_flight.borrow_mut().drain() {
            if !pending.contains(&code) {
                pending.push(code);
            }
        }
    }
}

impl Renderer {
    fn new(parent: &Panel) -> Rc<Renderer> {
        let webview = WebView::builder(parent)
            .with_backend(webpreview::backend())
            .with_size(Size::new(800, 600))
            .build();
        webview.register_handler(webpreview::SCHEME, serve);
        webview.add_script_message_handler(MESSAGE_HANDLER);
        webview.enable_context_menu(false);
        webview.show(false);
        let renderer = Rc::new(Renderer {
            webview,
            parent: parent.handle_ptr() as usize,
            ready: Cell::new(false),
            in_flight: RefCell::new(HashMap::new()),
            next_id: Cell::new(1),
        });
        let weak = Rc::downgrade(&renderer);
        webview.on_script_message_received(move |event| {
            let (Some(this), Some(message)) = (weak.upgrade(), event.get_string()) else {
                return;
            };
            this.handle_message(&message);
        });
        webview.load_url(PAGE_URL);
        renderer
    }

    /// 待ちのコードをページに渡す
    fn flush(&self) {
        let codes: Vec<String> = pending().lock().unwrap().drain(..).collect();
        for code in codes {
            if self.in_flight.borrow().values().any(|c| *c == code) {
                continue;
            }
            let id = self.next_id.get();
            self.next_id.set(id + 1);
            let json = serde_json::to_string(&code).unwrap_or_else(|_| "\"\"".into());
            self.in_flight.borrow_mut().insert(id, code);
            run_script_async(
                &self.webview,
                &format!("window.typballMermaid && typballMermaid.render({id},{json});"),
            );
        }
    }

    fn handle_message(&self, message: &str) {
        if message == "ready" {
            self.ready.set(true);
            self.flush();
            return;
        }
        // "mermaid:<番号>:ok:<SVG>" / "mermaid:<番号>:err:<理由>"
        let Some(rest) = message.strip_prefix("mermaid:") else {
            return;
        };
        let mut parts = rest.splitn(3, ':');
        let (Some(id), Some(kind), Some(body)) = (parts.next(), parts.next(), parts.next()) else {
            return;
        };
        let Some(code) = id
            .parse::<u64>()
            .ok()
            .and_then(|id| self.in_flight.borrow_mut().remove(&id))
        else {
            return;
        };
        let outcome = if kind == "ok" {
            Ok(body.to_string())
        } else {
            Err(body.to_string())
        };
        store(code, outcome);
        // まとめて描かせた分が全部戻ってから知らせる (1 つごとにプレビューを組み直さない)
        if self.in_flight.borrow().is_empty() {
            notify();
        }
    }
}

/// Mermaid を描くページの要求に応える
fn serve(uri: &str) -> Option<WebViewHandlerResponse> {
    let path = webpreview::request_path(uri)?;
    let path = path.trim_start_matches('/');
    let (data, mime): (&[u8], &str) = match path {
        "mermaid/render.html" => (HTML.as_bytes(), "text/html"),
        "mermaid/render.js" => (JS.as_bytes(), "text/javascript"),
        _ => return webpreview::embedded_asset(path.strip_prefix("assets/")?),
    };
    Some(WebViewHandlerResponse {
        data: data.to_vec(),
        mime_type: Some(mime.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_mermaid_blocks_like_typst_does() {
        let source = "= 見出し\n\n```mermaid\nflowchart TD\n    a --> b\n```\n\n```rust\nfn main() {}\n```\n\n```mermaid\nflowchart TD\n    a --> b\n```\n";
        // 同じコードは 1 つ。中身は Typst の raw.text と同じ (囲みと言語名を除き、行を \n でつなぐ)
        assert_eq!(
            blocks(source),
            vec!["flowchart TD\n    a --> b".to_string()]
        );
        assert!(blocks("```rust\nlet mermaid = 1;\n```").is_empty());
        assert!(blocks("本文だけ").is_empty());
    }

    #[test]
    fn the_show_rule_replaces_a_rendered_block_and_keeps_line_numbers() {
        let code = "flowchart LR\n    x --> y".to_string();
        let svg = concat!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"40\" height=\"20\" viewBox=\"0 0 40 20\">",
            "<rect width=\"40\" height=\"20\" fill=\"#eee\"/><text x=\"4\" y=\"14\">a \"b\" \\ c</text></svg>"
        );
        store(code.clone(), Ok(svg.to_string()));
        let source = format!("```mermaid\n{code}\n```\n\n#unknown-function()\n");
        let prelude = prelude(&source);
        assert!(prelude.starts_with("#show raw.where(lang: \"mermaid\")"));
        assert!(!prelude.contains('\n'), "the prelude must stay on one line");
        // 前置きを付けた本文がそのまま組める (図になったコードブロックは画像)
        let ok = format!("{prelude}```mermaid\n{code}\n```\n");
        let (doc, diagnostics) = typball_core::compile_doc(&ok, None);
        assert!(doc.is_some(), "{diagnostics:?}");
    }

    #[test]
    fn page_and_script_are_served() {
        assert!(serve("https://typball.wxsite/mermaid/render.html").is_some());
        assert!(serve("https://typball.wxsite/mermaid/render.js").is_some());
        assert!(serve("https://typball.wxsite/assets/mermaid.min.js").is_some());
        assert!(serve("https://typball.wxsite/mermaid/nope.js").is_none());
    }
}
