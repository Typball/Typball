//! アウトライン (見出し一覧)
//!
//! 左ペインに表示する見出しを種別ごとに抜き出す。Typst は構文木の `Heading` ノード、
//! Markdown は行頭の `#`。行番号はエディタで飛ぶために 0 始まりで持つ。

use typst_syntax::{LinkedNode, SyntaxKind};

use crate::filetype::FileType;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineItem {
    /// 1 = 最上位
    pub level: usize,
    pub text: String,
    /// 0 始まりの行番号
    pub line: usize,
}

impl OutlineItem {
    /// リストに出す 1 行 (階層をインデントで表す)
    pub fn label(&self) -> String {
        format!(
            "{}{}",
            "    ".repeat(self.level.saturating_sub(1)),
            self.text
        )
    }
}

pub fn outline(text: &str, filetype: FileType) -> Vec<OutlineItem> {
    match filetype {
        FileType::Typ => typst_outline(text),
        FileType::Md => markdown_outline(text),
        FileType::Txt => Vec::new(),
    }
}

fn typst_outline(text: &str) -> Vec<OutlineItem> {
    let root = typst_syntax::parse(text);
    let mut out = Vec::new();
    collect_headings(&LinkedNode::new(&root), text, &mut out);
    out
}

fn collect_headings(node: &LinkedNode, text: &str, out: &mut Vec<OutlineItem>) {
    if node.kind() == SyntaxKind::Heading {
        let marker_len = node
            .children()
            .find(|c| c.kind() == SyntaxKind::HeadingMarker)
            .map(|m| m.get().len())
            .unwrap_or(1);
        let start = node.offset();
        let end = (start + node.get().len()).min(text.len());
        let body = &text[start.min(end)..end];
        let title = body[marker_len.min(body.len())..].trim().to_string();
        out.push(OutlineItem {
            level: marker_len.max(1),
            text: title,
            line: line_of(text, node.offset()),
        });
        return;
    }
    for child in node.children() {
        collect_headings(&child, text, out);
    }
}

fn markdown_outline(text: &str) -> Vec<OutlineItem> {
    let mut out = Vec::new();
    let mut in_fence = false;
    // 冒頭のメタ情報 (front matter) の中の `# コメント` は見出しではない
    let skip = front_matter_lines(text);
    for (line, raw) in text.lines().enumerate().skip(skip) {
        let s = raw.trim_start();
        if s.starts_with("```") || s.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let level = s.bytes().take_while(|b| *b == b'#').count();
        if level == 0 || level > 6 {
            continue;
        }
        let rest = &s[level..];
        if !rest.starts_with(' ') && !rest.starts_with('\t') {
            continue;
        }
        let title = rest.trim().trim_end_matches('#').trim();
        out.push(OutlineItem {
            level,
            text: title.to_string(),
            line,
        });
    }
    out
}

/// 冒頭のメタ情報 (`---` で囲んだ YAML、`+++` で囲んだ TOML) の行数。無ければ 0
fn front_matter_lines(text: &str) -> usize {
    let mut lines = text.lines();
    let closes: &[&str] = match lines.next().map(str::trim_end) {
        Some("---") => &["---", "..."],
        Some("+++") => &["+++"],
        _ => return 0,
    };
    lines
        .position(|line| closes.contains(&line.trim_end()))
        .map_or(0, |i| i + 2)
}

/// バイトオフセットが属する 0 始まりの行番号
fn line_of(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())]
        .bytes()
        .filter(|b| *b == b'\n')
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typst_headings_with_levels_and_lines() {
        let src = "#set page(width: 10cm)\n= Title\ntext\n== Sub *bold*\n\n=== Deep\n";
        let items = typst_outline(src);
        assert_eq!(items.len(), 3);
        assert_eq!(
            (items[0].level, items[0].line, items[0].text.as_str()),
            (1, 1, "Title")
        );
        assert_eq!((items[1].level, items[1].line), (2, 3));
        assert_eq!(items[1].text, "Sub *bold*");
        assert_eq!(
            (items[2].level, items[2].line, items[2].text.as_str()),
            (3, 5, "Deep")
        );
    }

    #[test]
    fn markdown_headings_skip_front_matter() {
        // 冒頭のメタ情報の中の `# コメント` は見出しにしない。行番号はずれない
        let src = "---\ntitle: x\n# comment\n---\n\n# A\n";
        let items = markdown_outline(src);
        assert_eq!(items.len(), 1);
        assert_eq!((items[0].line, items[0].text.as_str()), (5, "A"));
        let src = "+++\n# comment\n+++\n# B\n";
        let items = markdown_outline(src);
        assert_eq!(items.len(), 1);
        assert_eq!((items[0].line, items[0].text.as_str()), (3, "B"));
        // 閉じていなければメタ情報ではない (横線と見出し)
        let src = "---\n# C\n";
        assert_eq!(markdown_outline(src).len(), 1);
    }

    #[test]
    fn markdown_headings_skip_fences_and_tags() {
        let src = "# A\n```\n# not heading\n```\n#notag\n## B ##\n";
        let items = markdown_outline(src);
        assert_eq!(items.len(), 2);
        assert_eq!(
            (items[0].level, items[0].line, items[0].text.as_str()),
            (1, 0, "A")
        );
        assert_eq!(
            (items[1].level, items[1].line, items[1].text.as_str()),
            (2, 5, "B")
        );
    }

    #[test]
    fn label_indents_by_level() {
        let item = OutlineItem {
            level: 3,
            text: "x".into(),
            line: 0,
        };
        assert_eq!(item.label(), "        x");
    }
}
