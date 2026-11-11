//! Markdown の構文の色分け
//!
//! プレビューと同じ pulldown-cmark (同じオプション) で全文を解析し、要素ごとにスタイルを決める。
//! 色は Typst の色分けと同じもの (`typst_syntax::Tag` ごとの配色。`editor::tag_appearance`) を使う:
//! 見出しは見出し、強調は強調、コードは raw、リンクはリンク、という対応にして、2 つの形式で見た目を
//! そろえる。
//!
//! pulldown-cmark は要素ごとに元のテキストの範囲 (バイト位置) を返す。外側の要素から順に来るので、
//! 来た順に範囲を塗れば、内側の要素 (見出しの中のコードなど) が上書きする。記号 (引用の `>`、
//! 表の `|`) は範囲として返ってこないので、最後に自分で探して塗る
//!
//! # コードブロックの中
//!
//! 言語を書いたコードブロック (```` ```rust ```` など) の中は、プレビューと同じ syntect の構文定義で字句に
//! 分けて色を付ける ([`Style::Code`])。syntect が返すのは字句の種類の名前 (`keyword.control.rust` のような
//! スコープ) なので、それを Typst のタグに対応させて ([`scope_rules`])、エディタの配色で描く。
//! 言語が無い・分からないブロックは今までどおり 1 色。
//!
//! syntect の解析は pulldown-cmark より 1 桁遅い。色分けは 1 文字打つたびに全文をやり直すので、
//! ブロックごとの結果を覚えておき ([`CACHE`])、中身が変わったブロックだけ解析し直す

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;
use std::sync::OnceLock;

use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag as Md, TagEnd};
use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference};
use syntect::util::LinesWithEndings;
use typst_syntax::Tag;

/// 文字のスタイル
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    /// 本文
    Plain,
    /// Markdown の要素。Typst の同じ役割のタグの色で描く
    Tag(Tag),
    /// 言語の分かるコードブロックの中のコード。地はコードの背景色で、字はその字句の種類のタグの色
    /// (どの種類でもない字句は本文の色)
    Code(Option<Tag>),
}

/// これより大きいコードブロックは字句に分けない (1 色のまま)
const CODE_LIMIT: usize = 256 * 1024;

/// 全文のスタイル: (長さ (バイト), スタイル) の並び
pub fn runs(text: &str) -> Vec<(usize, Style)> {
    let mut styles: Vec<Option<Tag>> = vec![None; text.len()];
    // コードブロックの中の字句 (範囲, タグ)。最後に `styles` の上にかぶせる
    let mut tokens: Vec<(Range<usize>, Option<Tag>)> = Vec::new();
    // 今いるコードブロックの構文定義と、中身の断片 (引用や箇条書きの中では、行頭の字下げや `>` を
    // 除いた部分が 1 行ずつ来る)
    let mut code: Option<(&'static SyntaxReference, Vec<Range<usize>>)> = None;
    CACHE.with(|cache| cache.borrow_mut().pass += 1);
    let bytes = text.as_bytes();
    // 後で記号を塗る範囲
    let mut quotes: Vec<Range<usize>> = Vec::new();
    let mut tables: Vec<Range<usize>> = Vec::new();
    // 表の見出し行の先頭 (その次の行が `|---|` の区切り)
    let mut table_heads: Vec<usize> = Vec::new();

    let parser = Parser::new_ext(text, crate::mdhtml::options());
    for (event, range) in parser.into_offset_iter() {
        let range = range.start.min(text.len())..range.end.min(text.len());
        match event {
            Event::Start(tag) => match tag {
                Md::Heading { .. } => paint(&mut styles, range, Tag::Heading),
                Md::Strong => paint(&mut styles, range, Tag::Strong),
                Md::Emphasis => paint(&mut styles, range, Tag::Emph),
                // 取り消し線: 消した文字らしく薄い色に
                Md::Strikethrough => paint(&mut styles, range, Tag::Punctuation),
                Md::CodeBlock(kind) => {
                    paint(&mut styles, range, Tag::Raw);
                    if let CodeBlockKind::Fenced(info) = &kind {
                        code = crate::mdhtml::find_syntax(crate::mdhtml::code_lang(info))
                            .map(|syntax| (syntax, Vec::new()));
                    }
                }
                Md::Link { .. } | Md::Image { .. } => paint(&mut styles, range, Tag::Link),
                Md::HtmlBlock => html(&mut styles, bytes, range),
                Md::FootnoteDefinition(_) => {
                    // `[^1]:` の部分だけ
                    let end = bytes[range.clone()]
                        .iter()
                        .position(|b| *b == b':')
                        .map_or(range.start, |i| range.start + i + 1);
                    paint(&mut styles, range.start..end, Tag::Ref);
                }
                Md::Item => {
                    let end = list_marker_end(bytes, range.clone());
                    paint(&mut styles, range.start..end, Tag::ListMarker);
                }
                Md::DefinitionListTitle => paint(&mut styles, range, Tag::ListTerm),
                Md::BlockQuote(_) => quotes.push(range),
                Md::Table(_) => tables.push(range),
                Md::TableHead => {
                    paint(&mut styles, range.clone(), Tag::Strong);
                    table_heads.push(range.start);
                }
                Md::MetadataBlock(_) => front_matter(&mut styles, text, range),
                _ => {}
            },
            Event::Text(_) => {
                if let Some((_, pieces)) = &mut code {
                    pieces.push(range);
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((syntax, pieces)) = code.take() {
                    code_block(&mut tokens, text, syntax, &pieces);
                }
            }
            Event::Code(_) => paint(&mut styles, range, Tag::Raw),
            Event::InlineMath(_) | Event::DisplayMath(_) => {
                paint(&mut styles, range, Tag::MathDelimiter)
            }
            // ブロックの HTML は上の HtmlBlock でまとめて塗る (行ごとの Event::Html は見ない)
            Event::InlineHtml(_) => html(&mut styles, bytes, range),
            Event::FootnoteReference(_) => paint(&mut styles, range, Tag::Ref),
            Event::Rule => paint(&mut styles, range, Tag::Punctuation),
            Event::TaskListMarker(_) => paint(&mut styles, range, Tag::ListMarker),
            _ => {}
        }
    }

    // 表: 見出し行の次の区切りの行 (`|---|:--:|`) と、縦線
    for head_start in table_heads {
        let start = next_line(bytes, head_start);
        let end = line_end(bytes, start);
        paint(&mut styles, start..end, Tag::Punctuation);
    }
    for table in tables {
        for i in table {
            // コードの中の縦線と、`\|` と書いた縦線は表の区切りではない
            if bytes[i] == b'|' && styles[i] != Some(Tag::Raw) && (i == 0 || bytes[i - 1] != b'\\')
            {
                styles[i] = Some(Tag::Punctuation);
            }
        }
    }
    // 引用: 行頭の `>` (入れ子なら続けて並ぶ)
    for quote in quotes {
        let mut line = quote.start;
        while line < quote.end {
            let mut i = line;
            loop {
                while i < quote.end && bytes[i] == b' ' {
                    i += 1;
                }
                if i < quote.end && bytes[i] == b'>' {
                    styles[i] = Some(Tag::ListMarker);
                    i += 1;
                } else {
                    break;
                }
            }
            line = next_line(bytes, line);
        }
    }

    // コードの字句をかぶせる
    let mut styles: Vec<Style> = styles
        .into_iter()
        .map(|tag| tag.map_or(Style::Plain, Style::Tag))
        .collect();
    for (range, tag) in tokens {
        if let Some(part) = styles.get_mut(range) {
            part.fill(Style::Code(tag));
        }
    }
    CACHE.with(|cache| cache.borrow_mut().trim());

    // 同じスタイルの続きをまとめる
    let mut runs: Vec<(usize, Style)> = Vec::new();
    for style in styles {
        match runs.last_mut() {
            Some(last) if last.1 == style => last.0 += 1,
            _ => runs.push((1, style)),
        }
    }
    runs
}

/// コードブロック 1 つ: 中身 (`pieces` をつないだもの) を字句に分けて、元のテキストでの範囲を `out` に足す
fn code_block(
    out: &mut Vec<(Range<usize>, Option<Tag>)>,
    text: &str,
    syntax: &'static SyntaxReference,
    pieces: &[Range<usize>],
) {
    let len: usize = pieces.iter().map(|piece| piece.len()).sum();
    if len == 0 || len > CODE_LIMIT {
        return;
    }
    let mut code = String::with_capacity(len);
    for piece in pieces {
        code.push_str(&text[piece.clone()]);
    }
    let lexed = CACHE.with(|cache| cache.borrow_mut().lex(syntax, &code));
    // 字句はつないだ中身の上での長さ。断片に順に当てはめて元の位置に戻す
    let mut pieces = pieces.iter().filter(|piece| !piece.is_empty());
    let Some(mut piece) = pieces.next().cloned() else {
        return;
    };
    for &(mut left, tag) in lexed.iter() {
        while left > 0 {
            if piece.is_empty() {
                match pieces.next() {
                    Some(next) => piece = next.clone(),
                    None => return,
                }
            }
            let take = left.min(piece.len());
            out.push((piece.start..piece.start + take, tag));
            piece.start += take;
            left -= take;
        }
    }
}

/// 字句の並び: (長さ (バイト), タグ)
type Lexed = Rc<Vec<(usize, Option<Tag>)>>;

/// コードブロックごとの字句の並びの控え。キーは (構文定義, 中身のハッシュ, 中身の長さ)
#[derive(Default)]
struct Cache {
    /// 何回目の色分けか ([`runs`] のたびに進む)
    pass: u64,
    /// 値は (最後に使った回, 字句の並び)
    blocks: HashMap<(usize, u64, usize), (u64, Lexed)>,
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

impl Cache {
    /// 控えがこの数を超えたら、今回使わなかったものを捨てる
    const KEEP: usize = 64;

    fn lex(&mut self, syntax: &'static SyntaxReference, code: &str) -> Lexed {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        code.hash(&mut hasher);
        let key = (
            syntax as *const SyntaxReference as usize,
            hasher.finish(),
            code.len(),
        );
        let pass = self.pass;
        let entry = self
            .blocks
            .entry(key)
            .or_insert_with(|| (pass, Rc::new(lex(syntax, code))));
        entry.0 = pass;
        entry.1.clone()
    }

    fn trim(&mut self) {
        if self.blocks.len() > Self::KEEP {
            let pass = self.pass;
            self.blocks.retain(|_, (used, _)| *used == pass);
        }
    }
}

/// コードを syntect で字句に分ける。同じタグの続きはまとめる
fn lex(syntax: &SyntaxReference, code: &str) -> Vec<(usize, Option<Tag>)> {
    let set = crate::mdhtml::syntax_set();
    let mut state = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut out: Vec<(usize, Option<Tag>)> = Vec::new();
    let mut push = |len: usize, tag: Option<Tag>| match out.last_mut() {
        Some(last) if last.1 == tag => last.0 += len,
        _ => out.push((len, tag)),
    };
    for line in LinesWithEndings::from(code) {
        // 解析できない行は色無し
        let ops = state.parse_line(line, set).unwrap_or_default();
        let mut pos = 0;
        for (at, op) in ops {
            let at = at.min(line.len());
            if at > pos {
                push(at - pos, tag_of(&stack));
                pos = at;
            }
            let _ = stack.apply(&op);
        }
        if line.len() > pos {
            push(line.len() - pos, tag_of(&stack));
        }
    }
    out
}

/// スコープの先頭がこれに合ったらどうするか
#[derive(Clone, Copy)]
enum Rule {
    /// このタグの色
    Is(Tag),
    /// 色無し (外側のスコープも見ない)
    Plain,
}

/// syntect のスコープ (字句の種類の名前) → タグ。上から順に見て、先頭が合った最初のもの。
///
/// 字句には内側から外側へスコープが重なっている (`source.rust` > `string.quoted` > `constant.character.escape`)。
/// 内側から順にこの表を引き、どれにも合わないスコープ (`punctuation` など) は飛ばして外側を見る:
/// 文字列の引用符は文字列の色、コメントの `//` はコメントの色になる。`source` (言語そのもの・文字列に
/// 埋め込んだ式) まで来たら色無し
fn scope_rules() -> &'static [(Scope, Rule)] {
    static RULES: OnceLock<Vec<(Scope, Rule)>> = OnceLock::new();
    RULES.get_or_init(|| {
        use Rule::{Is, Plain};
        [
            ("comment", Is(Tag::Comment)),
            ("string", Is(Tag::String)),
            ("constant.character.escape", Is(Tag::Escape)),
            ("constant.numeric", Is(Tag::Number)),
            ("constant.language", Is(Tag::Keyword)),
            ("constant", Is(Tag::Number)),
            // `and` / `not` / `new` のような、単語の演算子
            ("keyword.operator.word", Is(Tag::Keyword)),
            ("keyword.operator", Is(Tag::Operator)),
            ("keyword", Is(Tag::Keyword)),
            ("storage", Is(Tag::Keyword)),
            ("entity.name.function", Is(Tag::Function)),
            ("support.function", Is(Tag::Function)),
            ("variable.function", Is(Tag::Function)),
            ("variable.language", Is(Tag::Keyword)),
            // 変数は色無し (文字列の中に埋め込んだ変数も、文字列の色にしない)
            ("variable", Plain),
            // HTML / XML のタグと属性、YAML のキー
            ("entity.name.tag", Is(Tag::Keyword)),
            ("entity.other.attribute-name", Is(Tag::Label)),
            ("entity.name.section", Is(Tag::Heading)),
            // 型・クラス・名前空間
            ("entity.other.inherited-class", Is(Tag::MathDelimiter)),
            ("entity.name", Is(Tag::MathDelimiter)),
            ("support.type", Is(Tag::MathDelimiter)),
            ("support.class", Is(Tag::MathDelimiter)),
            ("support.constant", Is(Tag::Number)),
            // diff
            ("markup.deleted", Is(Tag::Escape)),
            ("markup.inserted", Is(Tag::Number)),
            ("meta.diff", Is(Tag::Keyword)),
            // Markdown などのマークアップ
            ("markup.heading", Is(Tag::Heading)),
            ("markup.bold", Is(Tag::Strong)),
            ("markup.italic", Is(Tag::Emph)),
            ("markup.underline.link", Is(Tag::Link)),
            ("markup.raw", Is(Tag::Raw)),
            ("meta.interpolation", Plain),
            ("meta.embedded", Plain),
            ("source", Plain),
            ("text", Plain),
        ]
        .into_iter()
        .filter_map(|(prefix, rule)| Some((Scope::new(prefix).ok()?, rule)))
        .collect()
    })
}

fn tag_of(stack: &ScopeStack) -> Option<Tag> {
    for scope in stack.as_slice().iter().rev() {
        for (prefix, rule) in scope_rules() {
            if prefix.is_prefix_of(*scope) {
                return match rule {
                    Rule::Is(tag) => Some(*tag),
                    Rule::Plain => None,
                };
            }
        }
    }
    None
}

fn paint(styles: &mut [Option<Tag>], range: Range<usize>, tag: Tag) {
    if let Some(part) = styles.get_mut(range) {
        part.fill(Some(tag));
    }
}

/// HTML: タグ (`<...>`) とコメント (`<!-- ... -->`) だけに色を付ける。タグの間の文は本文の色のまま
fn html(styles: &mut [Option<Tag>], bytes: &[u8], range: Range<usize>) {
    let find = |from: usize, what: &[u8]| {
        bytes[from..range.end]
            .windows(what.len())
            .position(|w| w == what)
            .map_or(range.end, |i| from + i + what.len())
    };
    let mut i = range.start;
    while i < range.end {
        if bytes[i..range.end].starts_with(b"<!--") {
            let end = find(i + 4, b"-->");
            paint(styles, i..end, Tag::Comment);
            i = end;
        } else if bytes[i] == b'<' {
            let end = find(i + 1, b">");
            paint(styles, i..end, Tag::Keyword);
            i = end;
        } else {
            i += 1;
        }
    }
}

/// `from` 以降の最初の改行の位置 (無ければ末尾)
fn line_end(bytes: &[u8], from: usize) -> usize {
    bytes[from.min(bytes.len())..]
        .iter()
        .position(|b| *b == b'\n' || *b == b'\r')
        .map_or(bytes.len(), |i| from + i)
}

/// `from` のある行の次の行の先頭 (無ければ末尾)
fn next_line(bytes: &[u8], from: usize) -> usize {
    let mut i = line_end(bytes, from);
    if bytes.get(i) == Some(&b'\r') {
        i += 1;
    }
    if bytes.get(i) == Some(&b'\n') {
        i += 1;
    }
    i.min(bytes.len())
}

/// 箇条書きの項目 (`- ` / `* ` / `+ ` / `1. ` / `1) `) の記号の終わり
fn list_marker_end(bytes: &[u8], item: Range<usize>) -> usize {
    let mut i = item.start;
    match bytes.get(i) {
        Some(b'-' | b'*' | b'+') => i + 1,
        Some(b) if b.is_ascii_digit() => {
            while i < item.end && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if matches!(bytes.get(i), Some(b'.' | b')')) {
                i + 1
            } else {
                item.start
            }
        }
        _ => item.start,
    }
}

/// 冒頭のメタ情報 (`---` で囲んだ YAML、`+++` で囲んだ TOML): 区切りの行は薄く、`名前:` / `名前 =` の
/// 名前とコメントに色を付ける (中身を YAML / TOML として解析はしない)
fn front_matter(styles: &mut [Option<Tag>], text: &str, block: Range<usize>) {
    let bytes = text.as_bytes();
    let mut line = block.start;
    while line < block.end {
        let end = line_end(bytes, line).min(block.end);
        let content = &text[line..end];
        let trimmed = content.trim_start();
        let indent = content.len() - trimmed.len();
        if trimmed.starts_with("---") || trimmed.starts_with("+++") {
            paint(styles, line..end, Tag::Punctuation);
        } else if trimmed.starts_with('#') {
            paint(styles, line..end, Tag::Comment);
        } else if let Some(key) = trimmed.find([':', '=']) {
            let name = trimmed[..key].trim_end();
            // 名前らしいもの (空白や引用符を含まない) だけ
            if !name.is_empty() && !name.contains([' ', '"', '\'', '[', '{']) {
                paint(
                    styles,
                    line + indent..line + indent + name.len(),
                    Tag::Label,
                );
            }
        }
        let next = next_line(bytes, line);
        if next <= line {
            break;
        }
        line = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `needle` (最初に出てくる場所) の先頭のスタイル
    fn style_at(text: &str, needle: &str) -> Option<Tag> {
        match full_style_at(text, needle) {
            Style::Plain => None,
            Style::Tag(tag) => Some(tag),
            Style::Code(tag) => tag,
        }
    }

    fn full_style_at(text: &str, needle: &str) -> Style {
        let at = text
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} not in the text"));
        let mut pos = 0;
        for (len, style) in runs(text) {
            if at < pos + len {
                return style;
            }
            pos += len;
        }
        panic!("the runs end before {needle:?}");
    }

    #[test]
    fn code_blocks_are_lexed() {
        use Style::Code;
        let text = "前\n\n```rust\n// 注\nfn main() {\n    let s = \"a\\n\";\n    let n = 42;\n}\n```\n\n```\nfn plain() {}\n```\n\n```nosuchlang\nlet x = 1;\n```\n";
        assert_eq!(full_style_at(text, "前"), Style::Plain);
        // 囲みの行は今までどおり
        assert_eq!(full_style_at(text, "```rust"), Style::Tag(Tag::Raw));
        assert_eq!(full_style_at(text, "// 注"), Code(Some(Tag::Comment)));
        assert_eq!(full_style_at(text, "注"), Code(Some(Tag::Comment)));
        assert_eq!(full_style_at(text, "fn main"), Code(Some(Tag::Keyword)));
        assert_eq!(full_style_at(text, "main"), Code(Some(Tag::Function)));
        assert_eq!(full_style_at(text, "let s"), Code(Some(Tag::Keyword)));
        assert_eq!(full_style_at(text, " s ="), Code(None));
        assert_eq!(full_style_at(text, "\"a"), Code(Some(Tag::String)));
        assert_eq!(full_style_at(text, "\\n\""), Code(Some(Tag::Escape)));
        assert_eq!(full_style_at(text, "42"), Code(Some(Tag::Number)));
        // 言語の無いブロック・知らない言語は 1 色
        assert_eq!(full_style_at(text, "fn plain"), Style::Tag(Tag::Raw));
        assert_eq!(full_style_at(text, "let x"), Style::Tag(Tag::Raw));
    }

    #[test]
    fn code_blocks_keep_their_place() {
        use Style::Code;
        // CRLF、引用と箇条書きの中 (行頭の `>` や字下げは字句に入らない)
        let text = "```js\r\nconst a = 1;\r\nlet b = 'x';\r\n```\r\n\r\n> ```py\r\n> def f():\r\n>     return 1\r\n> ```\r\n\r\n- 項目\r\n\r\n  ```json\r\n  {\"k\": true}\r\n  ```\r\n";
        assert_eq!(full_style_at(text, "const"), Code(Some(Tag::Keyword)));
        assert_eq!(full_style_at(text, "1;"), Code(Some(Tag::Number)));
        assert_eq!(full_style_at(text, "let b"), Code(Some(Tag::Keyword)));
        assert_eq!(full_style_at(text, "'x'"), Code(Some(Tag::String)));
        assert_eq!(full_style_at(text, "> def"), Style::Tag(Tag::ListMarker));
        assert_eq!(full_style_at(text, "def f"), Code(Some(Tag::Keyword)));
        assert_eq!(full_style_at(text, "f():"), Code(Some(Tag::Function)));
        assert_eq!(full_style_at(text, "return"), Code(Some(Tag::Keyword)));
        assert_eq!(full_style_at(text, "\"k\""), Code(Some(Tag::String)));
        assert_eq!(full_style_at(text, "true"), Code(Some(Tag::Keyword)));
        let total: usize = runs(text).iter().map(|(len, _)| len).sum();
        assert_eq!(total, text.len());
        // 2 回目 (控えから) も同じ
        assert_eq!(runs(text), runs(text));
    }

    #[test]
    fn runs_cover_the_whole_text() {
        for text in [
            "",
            "plain",
            "# 見出し\r\n\r\n本文 **太字** と `code`\r\n",
            "- a\n- b\n\n> quote\n",
            "| a | b |\n|---|---|\n| 1 | 2 |",
            "---\ntitle: x\n---\n\nbody\n",
            "末尾が全角 **あ",
        ] {
            let total: usize = runs(text).iter().map(|(len, _)| len).sum();
            assert_eq!(total, text.len(), "{text:?}");
        }
    }

    #[test]
    fn inline_elements_are_coloured() {
        let text =
            "普通の文 **太字** と *斜体* と `code` と [リンク](https://example.com) と ~~消す~~\n";
        assert_eq!(style_at(text, "普通"), None);
        assert_eq!(style_at(text, "**太字"), Some(Tag::Strong));
        assert_eq!(style_at(text, "太字"), Some(Tag::Strong));
        assert_eq!(style_at(text, "*斜体"), Some(Tag::Emph));
        assert_eq!(style_at(text, "`code"), Some(Tag::Raw));
        assert_eq!(style_at(text, "[リンク"), Some(Tag::Link));
        assert_eq!(style_at(text, "https"), Some(Tag::Link));
        assert_eq!(style_at(text, "~~消す"), Some(Tag::Punctuation));
        assert_eq!(style_at(text, " と *"), None);
    }

    #[test]
    fn html_tags_are_coloured() {
        let text = "文 <span>中</span> <!-- 注 <b> --> 後\n\n<div class=\"x\">\n本文\n</div>\n\n<!--\n複数行 <i>\n-->\n";
        assert_eq!(style_at(text, "<span>"), Some(Tag::Keyword));
        // タグの間の文は本文の色のまま
        assert_eq!(style_at(text, "中"), None);
        assert_eq!(style_at(text, "</span>"), Some(Tag::Keyword));
        assert_eq!(style_at(text, "<!-- 注"), Some(Tag::Comment));
        assert_eq!(style_at(text, "<b>"), Some(Tag::Comment));
        assert_eq!(style_at(text, " 後"), None);
        assert_eq!(style_at(text, "<div"), Some(Tag::Keyword));
        assert_eq!(style_at(text, "本文"), None);
        assert_eq!(style_at(text, "</div>"), Some(Tag::Keyword));
        assert_eq!(style_at(text, "複数行"), Some(Tag::Comment));
        assert_eq!(style_at(text, "<i>"), Some(Tag::Comment));
    }

    /// 大きな文書の解析にかかる時間 (`cargo test --release -- --ignored md_speed --nocapture`)
    #[test]
    #[ignore]
    fn md_speed() {
        let part = "# 見出し\n\n本文に **太字** と *斜体* と `code` と [リンク](https://example.com)。\n\n- 項目\n- 項目\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n```rust\nlet a = 1;\n```\n\n";
        for mb in [1usize, 4, 16] {
            let text = part.repeat(mb * 1024 * 1024 / part.len());
            let start = std::time::Instant::now();
            let runs = runs(&text);
            println!("{} MB: {} runs, {:?}", mb, runs.len(), start.elapsed());
        }
        // コードの多い文書: 1 回目は全部のブロックを字句に分ける。2 回目は控えから。3 回目は 1 つだけ変えたとき
        let code = "/// 説明\nfn add(a: i32, b: i32) -> i32 {\n    let s = \"text\\n\";\n    a + b * 42 // 注\n}\n\n"
            .repeat(50);
        let block = |n: usize| format!("## 節 {n}\n\n```rust\n// {n}\n{code}```\n\n");
        let mut text: String = (0..20).map(block).collect();
        for label in ["first", "cached", "one block edited"] {
            let start = std::time::Instant::now();
            let runs = runs(&text);
            println!(
                "code {} KB ({label}): {} runs, {:?}",
                text.len() / 1024,
                runs.len(),
                start.elapsed()
            );
            if label == "cached" {
                text = text.replacen("// 3\n", "// 3!\n", 1);
            }
        }
    }

    #[test]
    fn blocks_are_coloured() {
        let text = "# 見出し `x`\n\n本文\n\n```rust\nlet a = 1;\n```\n\n- 項目\n1. 番号\n- [ ] 仕事\n\n> 引用\n> > 入れ子\n\n---\n";
        assert_eq!(style_at(text, "# 見出し"), Some(Tag::Heading));
        assert_eq!(style_at(text, "見出し"), Some(Tag::Heading));
        // 見出しの中のコードは、内側の要素が勝つ
        assert_eq!(style_at(text, "`x`"), Some(Tag::Raw));
        assert_eq!(style_at(text, "本文"), None);
        assert_eq!(style_at(text, "```rust"), Some(Tag::Raw));
        assert_eq!(style_at(text, "let a"), Some(Tag::Keyword));
        assert_eq!(style_at(text, "- 項目"), Some(Tag::ListMarker));
        assert_eq!(style_at(text, "項目"), None);
        assert_eq!(style_at(text, "1. 番号"), Some(Tag::ListMarker));
        assert_eq!(style_at(text, "番号"), None);
        assert_eq!(style_at(text, "[ ] 仕事"), Some(Tag::ListMarker));
        assert_eq!(style_at(text, "> 引用"), Some(Tag::ListMarker));
        assert_eq!(style_at(text, "引用"), None);
        assert_eq!(style_at(text, "> > 入れ子"), Some(Tag::ListMarker));
        assert_eq!(style_at(text, "> 入れ子"), Some(Tag::ListMarker));
        assert_eq!(style_at(text, "入れ子"), None);
        assert_eq!(style_at(text, "---"), Some(Tag::Punctuation));
    }

    #[test]
    fn tables_are_coloured() {
        let text = "| 名前 | 値 |\r\n|------|---:|\r\n| a `b\\|c` | 1 |\r\n\r\n後ろの文 | 縦線\r\n";
        assert_eq!(style_at(text, "| 名前"), Some(Tag::Punctuation));
        assert_eq!(style_at(text, "名前"), Some(Tag::Strong));
        assert_eq!(style_at(text, "|------"), Some(Tag::Punctuation));
        assert_eq!(style_at(text, "------"), Some(Tag::Punctuation));
        assert_eq!(style_at(text, "| a"), Some(Tag::Punctuation));
        assert_eq!(style_at(text, "a `"), None);
        // コードの中の、バックスラッシュを付けた縦線は区切りではない
        assert_eq!(style_at(text, "|c`"), Some(Tag::Raw));
        // 表の外の縦線はそのまま
        assert_eq!(style_at(text, "| 縦線"), None);
    }

    #[test]
    fn front_matter_is_coloured() {
        let yaml = "---\ntitle: \"記事\"\n# メモ\ntags:\n  - a\n---\n\n# 本文\n";
        assert_eq!(style_at(yaml, "---"), Some(Tag::Punctuation));
        assert_eq!(style_at(yaml, "title"), Some(Tag::Label));
        assert_eq!(style_at(yaml, ": \"記事"), None);
        assert_eq!(style_at(yaml, "# メモ"), Some(Tag::Comment));
        assert_eq!(style_at(yaml, "tags"), Some(Tag::Label));
        assert_eq!(style_at(yaml, "# 本文"), Some(Tag::Heading));
        let toml = "+++\ntitle = \"記事\"\ndraft = true\n+++\n\n本文\n";
        assert_eq!(style_at(toml, "+++"), Some(Tag::Punctuation));
        assert_eq!(style_at(toml, "draft"), Some(Tag::Label));
        assert_eq!(style_at(toml, "本文"), None);
    }
}
