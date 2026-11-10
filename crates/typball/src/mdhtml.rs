//! Markdown → HTML (プレビュー用)
//!
//! pulldown-cmark のイベント列を加工してから HTML にする。加工の内容:
//! - 最上位のブロックを `<div class="tb-block" data-line="N">` で包み、リストの項目には
//!   `data-line` 付きの印を入れる (エディタとプレビューのスクロール同期に使う)
//! - コードブロックは syntect で色付けする (Typst の依存に入っている two-face の構文定義を使う)
//! - 画像の URL をプレビュー用の仮想ホストの形に書き換える (`webpreview.rs` の配信と対になる)。
//!   相対パスはページの URL 基準でブラウザが解決するのでそのまま、ルート相対 (`/images/x.png`) は
//!   `~root/`、絶対パスと `..` を含む相対パスは `~abs/` の形にする
//! - `$...$` / `$$...$$` は KaTeX が後で描けるよう `<span class="math">` にする (KaTeX が無いときは
//!   等幅のまま見える)
//! - front matter (`---` YAML / `+++` TOML) は読み飛ばし、`title` だけ見出しとして出す
//! - Hugo のショートコード (`{{< ... >}}` / `{{% ... %}}`) は灰色の枠でそのまま見せる
//!
//! この加工は純粋な文字列処理なので単体テストできる (GUI 不要)。

use std::ops::Range;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use pulldown_cmark::{html, CodeBlockKind, CowStr, Event, Options, Parser, Tag, TagEnd};
use syntect::easy::HighlightLines;
use syntect::highlighting::Theme;
use syntect::html::{styled_line_to_highlighted_html, IncludeBackground};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;
use two_face::theme::{EmbeddedLazyThemeSet, EmbeddedThemeName};

/// 変換に必要な文書側の情報
pub struct Context<'a> {
    /// 文書の番号 (仮想ホストのパスの先頭)。`webpreview.rs` が振る
    pub doc_id: u64,
    /// `.md` のあるフォルダ (無題なら None)
    pub base_dir: Option<&'a Path>,
    /// 保存のたびに増える番号。画像 URL のクエリに付けて WebView のキャッシュを更新させる
    pub version: u64,
    /// ダーク配色 (コードの色付けテーマを選ぶ)
    pub dark: bool,
    /// 書き出し (HTML / PDF) 用に変換するときの画像 URL の扱い。None ならプレビュー用の仮想ホストの URL
    pub export: Option<ExportUrls>,
}

/// 書き出したファイルの画像の参照のしかた
pub struct ExportUrls {
    /// 画像を data: URL として HTML に埋め込む (1 ファイルで完結する)
    pub embed: bool,
    /// 書き出し先のフォルダ。`.md` と同じフォルダなら相対パスをそのまま残す (違えば file: URL にする)
    pub out_dir: Option<PathBuf>,
}

/// 変換結果
pub struct Output {
    /// `<article>` の中身
    pub body: String,
    /// front matter の `title`
    pub title: Option<String>,
}

fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_SMART_PUNCTUATION
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS
        | Options::ENABLE_MATH
        | Options::ENABLE_GFM
        | Options::ENABLE_DEFINITION_LIST
        | Options::ENABLE_SUPERSCRIPT
        | Options::ENABLE_SUBSCRIPT
}

/// Markdown を HTML (`<article>` の中身) にする
pub fn render(markdown: &str, ctx: &Context) -> Output {
    let lines = LineIndex::new(markdown);
    let events = coalesce_text(Parser::new_ext(markdown, options()).into_offset_iter());

    let mut out: Vec<Event> = Vec::with_capacity(events.len() + 64);
    let mut depth = 0usize;
    let mut code: Option<(String, String)> = None;
    let mut meta: Option<String> = None;
    let mut title = None;

    for (event, range) in events {
        // front matter: 本文には出さず title だけ拾う
        if let Some(buf) = meta.as_mut() {
            match event {
                Event::End(TagEnd::MetadataBlock(_)) => {
                    title = front_matter_title(buf);
                    meta = None;
                    if let Some(t) = &title {
                        out.push(Event::Html(
                            format!(
                                "<div class=\"tb-block tb-frontmatter\" data-line=\"1\"><h1 class=\"tb-title\">{}</h1></div>",
                                escape_html(t)
                            )
                            .into(),
                        ));
                    }
                }
                Event::Text(t) => buf.push_str(&t),
                _ => {}
            }
            continue;
        }
        // コードブロック: 中身を集めて最後にまとめて色付けする
        if let Some((lang, text)) = code.as_mut() {
            match event {
                Event::End(TagEnd::CodeBlock) => {
                    let html = highlight_block(lang, text, ctx.dark);
                    out.push(Event::Html(html.into()));
                    code = None;
                    depth -= 1;
                    if depth == 0 {
                        out.push(Event::Html("</div>".into()));
                    }
                }
                Event::Text(t) => text.push_str(&t),
                _ => {}
            }
            continue;
        }

        match event {
            Event::Start(Tag::MetadataBlock(_)) => meta = Some(String::new()),
            Event::Start(Tag::CodeBlock(kind)) => {
                if depth == 0 {
                    out.push(block_open(lines.line_of(range.start)));
                }
                depth += 1;
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split(|c: char| c.is_whitespace() || c == ',' || c == '{')
                        .next()
                        .unwrap_or("")
                        .to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                code = Some((lang, String::new()));
            }
            Event::Start(tag) if is_block(&tag) => {
                if depth == 0 {
                    out.push(block_open(lines.line_of(range.start)));
                }
                depth += 1;
                out.push(Event::Start(tag));
            }
            Event::End(end) if is_block_end(&end) => {
                out.push(Event::End(end));
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    out.push(Event::Html("</div>".into()));
                }
            }
            Event::Start(Tag::Item) => {
                out.push(Event::Start(Tag::Item));
                out.push(Event::Html(
                    format!(
                        "<span class=\"tb-mark\" data-line=\"{}\"></span>",
                        lines.line_of(range.start)
                    )
                    .into(),
                ));
            }
            Event::Rule => {
                if depth == 0 {
                    out.push(block_open(lines.line_of(range.start)));
                    out.push(Event::Rule);
                    out.push(Event::Html("</div>".into()));
                } else {
                    out.push(Event::Rule);
                }
            }
            Event::Start(Tag::Image {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                let dest = rewrite_url(&dest_url, ctx);
                out.push(Event::Start(Tag::Image {
                    link_type,
                    dest_url: dest.into(),
                    title,
                    id,
                }));
            }
            Event::InlineMath(src) => out.push(Event::Html(
                format!(
                    "<span class=\"math math-inline\">{}</span>",
                    escape_html(&src)
                )
                .into(),
            )),
            Event::DisplayMath(src) => out.push(Event::Html(
                format!(
                    "<span class=\"math math-display\">{}</span>",
                    escape_html(&src)
                )
                .into(),
            )),
            Event::Text(text) if is_shortcode(&text) => out.push(Event::Html(
                format!(
                    "<span class=\"tb-shortcode\">{}</span>",
                    escape_html(text.trim())
                )
                .into(),
            )),
            other => out.push(other),
        }
    }
    // 閉じ忘れ (壊れた入力) の保険
    while depth > 0 {
        depth -= 1;
        if depth == 0 {
            out.push(Event::Html("</div>".into()));
        }
    }

    let mut body = String::with_capacity(markdown.len() * 2);
    html::push_html(&mut body, out.into_iter());
    Output { body, title }
}

fn block_open(line: usize) -> Event<'static> {
    Event::Html(format!("<div class=\"tb-block\" data-line=\"{line}\">").into())
}

/// 最上位に来うるブロック (入れ子の深さを数える対象)
fn is_block(tag: &Tag) -> bool {
    matches!(
        tag,
        Tag::Paragraph
            | Tag::Heading { .. }
            | Tag::BlockQuote(_)
            | Tag::CodeBlock(_)
            | Tag::HtmlBlock
            | Tag::List(_)
            | Tag::FootnoteDefinition(_)
            | Tag::DefinitionList
            | Tag::Table(_)
    )
}

fn is_block_end(end: &TagEnd) -> bool {
    matches!(
        end,
        TagEnd::Paragraph
            | TagEnd::Heading(_)
            | TagEnd::BlockQuote(_)
            | TagEnd::CodeBlock
            | TagEnd::HtmlBlock
            | TagEnd::List(_)
            | TagEnd::FootnoteDefinition
            | TagEnd::DefinitionList
            | TagEnd::Table
    )
}

/// 隣り合う Text イベントを 1 つにまとめる (pulldown-cmark は `<` などで Text を分割するので、
/// ショートコードの判定を行単位でできるようにする)
fn coalesce_text<'a>(
    iter: impl Iterator<Item = (Event<'a>, Range<usize>)>,
) -> Vec<(Event<'a>, Range<usize>)> {
    let mut out: Vec<(Event<'a>, Range<usize>)> = Vec::new();
    for (event, range) in iter {
        if let (Event::Text(t), Some((Event::Text(prev), prev_range))) = (&event, out.last_mut()) {
            if prev_range.end == range.start {
                let mut s = String::with_capacity(prev.len() + t.len());
                s.push_str(prev);
                s.push_str(t);
                *prev = CowStr::from(s);
                prev_range.end = range.end;
                continue;
            }
        }
        out.push((event, range));
    }
    out
}

fn is_shortcode(text: &str) -> bool {
    let t = text.trim();
    (t.starts_with("{{<") && t.ends_with(">}}")) || (t.starts_with("{{%") && t.ends_with("%}}"))
}

/// front matter から title を取り出す (YAML `title: x` / TOML `title = "x"`)
fn front_matter_title(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line
            .strip_prefix("title")
            .map(str::trim_start)
            .and_then(|r| r.strip_prefix(':').or_else(|| r.strip_prefix('=')))
        else {
            continue;
        };
        let rest = rest.trim();
        let value = rest
            .strip_prefix('"')
            .and_then(|r| r.strip_suffix('"'))
            .or_else(|| rest.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')))
            .unwrap_or(rest);
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// URL の書き換え
// ---------------------------------------------------------------------------

/// 画像の URL をプレビュー用に書き換える
fn rewrite_url(dest: &str, ctx: &Context) -> String {
    let dest = dest.trim();
    if dest.is_empty() || dest.starts_with('#') {
        return dest.to_string();
    }
    if let Some(export) = &ctx.export {
        return export_url(dest, ctx, export);
    }
    if let Some(scheme) = url_scheme(dest) {
        if scheme.eq_ignore_ascii_case("file") {
            let path = dest[scheme.len() + 1..].trim_start_matches("//");
            // file:///C:/x → C:/x、file:///home/x → /home/x
            let path = percent_decode(path);
            let path = match path.as_bytes() {
                [b'/', d, b':', ..] if d.is_ascii_alphabetic() => path[1..].to_string(),
                _ => path,
            };
            return abs_url(Path::new(&path), ctx);
        }
        return dest.to_string();
    }
    let normalized = dest.replace('\\', "/");
    // Windows の絶対パス (C:/...)
    if is_drive_path(&normalized) {
        return abs_url(Path::new(&normalized), ctx);
    }
    // ルート相対 (サイトの static/ など)
    if let Some(rest) = normalized.strip_prefix('/') {
        return format!(
            "/{}/~root/{}?v={}",
            ctx.doc_id,
            percent_encode_path(&percent_decode(strip_query(rest))),
            ctx.version
        );
    }
    // 親フォルダを跨ぐ相対パスは、ページの URL 基準では解決できないので絶対パスにする
    if normalized.split('/').any(|seg| seg == "..") {
        if let Some(base) = ctx.base_dir {
            let joined = normalize_path(&base.join(percent_decode(strip_query(&normalized))));
            return abs_url(&joined, ctx);
        }
    }
    // 相対パス: ページ (/<id>/index.html) 基準でブラウザが解決する
    if normalized.contains('?') {
        normalized
    } else {
        format!("{normalized}?v={}", ctx.version)
    }
}

/// 書き出し用の画像 URL: 埋め込むなら data: URL、そうでなければ相対パスのまま (書き出し先が `.md` と
/// 同じフォルダのとき) か file: URL。読めないファイルや Web の URL はそのまま残す
fn export_url(dest: &str, ctx: &Context, export: &ExportUrls) -> String {
    let normalized = dest.replace('\\', "/");
    let path: Option<PathBuf> = match url_scheme(dest) {
        Some(scheme) if scheme.eq_ignore_ascii_case("file") => {
            let path = percent_decode(dest[scheme.len() + 1..].trim_start_matches("//"));
            Some(PathBuf::from(match path.as_bytes() {
                [b'/', d, b':', ..] if d.is_ascii_alphabetic() => path[1..].to_string(),
                _ => path,
            }))
        }
        Some(_) => None,
        None if is_drive_path(&normalized) => {
            Some(PathBuf::from(percent_decode(strip_query(&normalized))))
        }
        // ルート相対 (サイトの static/) は書き出し先では解決できないのでそのまま
        None if normalized.starts_with('/') => None,
        None => ctx
            .base_dir
            .map(|base| normalize_path(&base.join(percent_decode(strip_query(&normalized))))),
    };
    let Some(path) = path else {
        return dest.to_string();
    };
    if export.embed {
        if let Some(uri) = data_uri(&path) {
            return uri;
        }
    }
    let relative = url_scheme(dest).is_none() && !is_drive_path(&normalized);
    let same_dir = match (&export.out_dir, ctx.base_dir) {
        (Some(out), Some(base)) => normalize_path(out) == normalize_path(base),
        _ => false,
    };
    if relative && same_dir {
        return dest.to_string();
    }
    let text = path.to_string_lossy().replace('\\', "/");
    format!(
        "file:///{}",
        percent_encode_path(text.trim_start_matches('/'))
    )
}

/// 画像ファイルを data: URL にする (画像以外・大きすぎるファイル・読めないファイルは None)
fn data_uri(path: &Path) -> Option<String> {
    let mime = match path
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
        Some("bmp") => "image/bmp",
        Some("ico") => "image/x-icon",
        Some("avif") => "image/avif",
        _ => return None,
    };
    const MAX: u64 = 32 * 1024 * 1024;
    if std::fs::metadata(path).ok()?.len() > MAX {
        return None;
    }
    let data = std::fs::read(path).ok()?;
    Some(format!("data:{mime};base64,{}", base64(&data)))
}

/// Base64 (標準のアルファベット、パディング付き)
pub fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// 絶対パスのファイルを指す URL (`/<id>/~abs/C:/dir/a.png`)
fn abs_url(path: &Path, ctx: &Context) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    format!(
        "/{}/~abs/{}?v={}",
        ctx.doc_id,
        percent_encode_path(text.trim_start_matches('/')),
        ctx.version
    )
}

/// `scheme:` で始まるなら scheme を返す (Windows のドライブ文字は除く)
fn url_scheme(s: &str) -> Option<&str> {
    let colon = s.find(':')?;
    let scheme = &s[..colon];
    if scheme.len() < 2
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return None;
    }
    Some(scheme)
}

fn is_drive_path(s: &str) -> bool {
    matches!(s.as_bytes(), [d, b':', b'/', ..] if d.is_ascii_alphabetic())
}

fn strip_query(s: &str) -> &str {
    s.split(['?', '#']).next().unwrap_or(s)
}

/// `.` と `..` を字句的に解決する (存在しないパスでもよい)
fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// パス用のパーセントエンコード (`/` と `:` は残す)
pub fn percent_encode_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/' | b':') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// パーセントエンコードを戻す (UTF-8 として解釈、壊れていれば U+FFFD)
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 行番号
// ---------------------------------------------------------------------------

struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    fn new(text: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        Self { starts }
    }

    /// バイト位置 → 1 始まりの行番号
    fn line_of(&self, offset: usize) -> usize {
        match self.starts.binary_search(&offset) {
            Ok(i) => i + 1,
            Err(i) => i,
        }
    }
}

// ---------------------------------------------------------------------------
// コードの色付け
// ---------------------------------------------------------------------------

fn syntax_set() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(two_face::syntax::extra_newlines)
}

fn theme(dark: bool) -> &'static Theme {
    static THEMES: OnceLock<EmbeddedLazyThemeSet> = OnceLock::new();
    let set = THEMES.get_or_init(two_face::theme::extra);
    set.get(if dark {
        EmbeddedThemeName::Base16OceanDark
    } else {
        EmbeddedThemeName::InspiredGithub
    })
}

/// 色付けの準備 (構文定義の読み込みは 100 ms 程度かかるので起動時に別スレッドで済ませてもよい)
pub fn warm_up() {
    let _ = syntax_set();
    let _ = theme(false);
}

fn highlight_block(lang: &str, code: &str, dark: bool) -> String {
    let lang_attr = if lang.is_empty() {
        String::new()
    } else {
        format!(" class=\"language-{}\"", escape_html(lang))
    };
    let mut out = format!("<pre class=\"tb-code\"><code{lang_attr}>");
    let set = syntax_set();
    let syntax = (!lang.is_empty())
        .then(|| {
            set.find_syntax_by_token(lang)
                .or_else(|| set.find_syntax_by_extension(lang))
        })
        .flatten();
    match syntax {
        Some(syntax) => {
            let mut highlighter = HighlightLines::new(syntax, theme(dark));
            for line in LinesWithEndings::from(code) {
                let html = highlighter
                    .highlight_line(line, set)
                    .ok()
                    .and_then(|ranges| {
                        styled_line_to_highlighted_html(&ranges, IncludeBackground::No).ok()
                    });
                match html {
                    Some(h) => out.push_str(&h),
                    None => out.push_str(&escape_html(line)),
                }
            }
        }
        None => out.push_str(&escape_html(code)),
    }
    out.push_str("</code></pre>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(base: Option<&Path>) -> Context<'_> {
        Context {
            doc_id: 7,
            base_dir: base,
            version: 3,
            dark: false,
            export: None,
        }
    }

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn export_urls_keep_relative_paths_next_to_the_document() {
        let base = Path::new("C:/docs");
        let mut c = ctx(Some(base));
        c.export = Some(ExportUrls {
            embed: false,
            out_dir: Some(PathBuf::from("C:/docs")),
        });
        assert_eq!(rewrite_url("img/a.png", &c), "img/a.png");
        assert_eq!(
            rewrite_url("https://example.com/a.png", &c),
            "https://example.com/a.png"
        );
        c.export = Some(ExportUrls {
            embed: false,
            out_dir: Some(PathBuf::from("D:/out")),
        });
        assert_eq!(
            rewrite_url("img/a b.png", &c),
            "file:///C:/docs/img/a%20b.png"
        );
        assert_eq!(rewrite_url("../x.png", &c), "file:///C:/x.png");
    }

    #[test]
    fn blocks_carry_line_numbers() {
        let out = render("# Title\n\npara\n\n- a\n- b\n", &ctx(None));
        let body = out.body.replace('\n', "");
        assert!(
            body.contains("<div class=\"tb-block\" data-line=\"1\"><h1>Title</h1></div>"),
            "{body}"
        );
        assert!(
            body.contains("<div class=\"tb-block\" data-line=\"3\"><p>para</p></div>"),
            "{body}"
        );
        assert!(
            body.contains("<li><span class=\"tb-mark\" data-line=\"6\"></span>b</li>"),
            "{body}"
        );
        assert_eq!(body.matches("<div class=\"tb-block\"").count(), 3);
    }

    #[test]
    fn image_urls() {
        let base = Path::new("C:\\docs\\post");
        let c = ctx(Some(base));
        let out = render(
            "![a](img/a.png) ![b](/images/b.png) ![c](../c.png) ![d](https://x/y.png) ![e](C:\\e.png) ![f](<C:\\e f.png>)",
            &c,
        );
        let body = &out.body;
        assert!(body.contains("src=\"img/a.png?v=3\""), "{body}");
        assert!(body.contains("src=\"/7/~root/images/b.png?v=3\""), "{body}");
        assert!(body.contains("src=\"/7/~abs/C:/docs/c.png?v=3\""), "{body}");
        assert!(body.contains("src=\"https://x/y.png\""), "{body}");
        assert!(body.contains("src=\"/7/~abs/C:/e.png?v=3\""), "{body}");
        assert!(body.contains("src=\"/7/~abs/C:/e%20f.png?v=3\""), "{body}");
    }

    #[test]
    fn front_matter_title_and_math_and_shortcode() {
        let md = "---\ntitle: \"Hello\"\ndate: 2026-01-01\n---\n\nText $x^2$\n\n{{< figure src=\"a.png\" >}}\n";
        let out = render(md, &ctx(None));
        assert_eq!(out.title.as_deref(), Some("Hello"));
        assert!(out.body.contains("<h1 class=\"tb-title\">Hello</h1>"));
        assert!(out
            .body
            .contains("<span class=\"math math-inline\">x^2</span>"));
        assert!(out
            .body
            .contains("<span class=\"tb-shortcode\">{{&lt; figure src="));
        assert!(!out.body.contains("date:"));
    }

    #[test]
    fn code_is_highlighted() {
        let out = render("```rust\nfn main() {}\n```\n", &ctx(None));
        assert!(out
            .body
            .contains("<pre class=\"tb-code\"><code class=\"language-rust\">"));
        assert!(out.body.contains("<span style="));
        let plain = render("```\nx < y\n```\n", &ctx(None));
        assert!(plain.body.contains("x &lt; y"));
    }

    #[test]
    fn percent_roundtrip() {
        let s = "a b/日本語.png";
        assert_eq!(percent_decode(&percent_encode_path(s)), s);
        assert_eq!(percent_decode("%41%zz"), "A%zz");
    }

    #[test]
    fn gfm_alert_and_tasklist() {
        let out = render("> [!NOTE]\n> hi\n\n- [x] done\n", &ctx(None));
        assert!(out.body.contains("markdown-alert-note"));
        assert!(out.body.contains("type=\"checkbox\""));
    }
}
