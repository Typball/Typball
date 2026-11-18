//! Hugo のショートコードのうち、Markdown として読むと困るものの扱い
//!
//! Hugo ではコードを ```` ``` ```` の囲みの代わりに、ショートコードで書ける:
//!
//! ```text
//! {{< highlight bash >}}
//! # これはシェルのコメント
//! $ ls
//! {{< /highlight >}}
//! ```
//!
//! 中身は Markdown ではないのに、そのまま Markdown として読むと `#` で始まる行が見出しになる (アウトラインに
//! 並び、エディタで見出しの色になり、プレビューで見出しとして描かれる)。そこで、解析の前に、対になった
//! `highlight` の開きと閉じの行を**同じ長さ**のコードの囲みに置き換える ([`fence_highlight`])。長さが同じなので、
//! 解析結果の位置 (バイト位置・行番号) は元のテキストにそのまま当てはまる。

use std::borrow::Cow;
use std::ops::Range;

/// 置き換えに使う囲み。中身に ```` ``` ```` の行があっても閉じないように、4 つにする
const FENCE: &str = "````";

#[derive(Debug, PartialEq, Eq)]
enum Tag {
    /// `{{< highlight 言語 ... >}}` (言語は無ければ空)
    Open(String),
    /// `{{< /highlight >}}`
    Close,
}

/// 対になった `{{< highlight 言語 >}}` … `{{< /highlight >}}` の 2 行を、同じ長さのコードの囲み
/// (```` ````言語 ```` と ```` ```` ````。足りないぶんは空白) に置き換える。無ければ元のまま返す
pub fn fence_highlight(text: &str) -> Cow<'_, str> {
    if !text.contains("highlight") {
        return Cow::Borrowed(text);
    }
    let mut out: Option<String> = None;
    let mut open: Option<(Range<usize>, String)> = None;
    let mut pos = 0;
    for line in text.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        // 字下げは残し、その後ろの部分だけを置き換える
        let start = pos + (content.len() - content.trim_start().len());
        let range = start..start + content.trim().len();
        pos += line.len();
        match tag(content) {
            // 開きの中の開きは、中身 (コード) の一部
            Some(Tag::Open(lang)) if open.is_none() => open = Some((range, lang)),
            Some(Tag::Close) => {
                // 閉じが見つかったときだけ、開きと一緒に置き換える (閉じていない開き、開きの無い閉じはそのまま)
                if let Some((open_range, lang)) = open.take() {
                    let out = out.get_or_insert_with(|| text.to_string());
                    out.replace_range(open_range.clone(), &fence(&lang, open_range.len()));
                    out.replace_range(range.clone(), &fence("", range.len()));
                }
            }
            _ => {}
        }
    }
    match out {
        Some(out) => Cow::Owned(out),
        None => Cow::Borrowed(text),
    }
}

/// 長さ `len` バイトの囲みの行 (囲み + 言語 + 空白)
fn fence(lang: &str, len: usize) -> String {
    let mut s = String::with_capacity(len);
    s.push_str(FENCE);
    if FENCE.len() + lang.len() <= len {
        s.push_str(lang);
    }
    while s.len() < len {
        s.push(' ');
    }
    s.truncate(len);
    s
}

/// 行が `highlight` のショートコードの開きか閉じか (その行にショートコードだけがあるとき)
fn tag(line: &str) -> Option<Tag> {
    let t = line.trim();
    let inner = t
        .strip_prefix("{{<")
        .and_then(|s| s.strip_suffix(">}}"))
        .or_else(|| t.strip_prefix("{{%").and_then(|s| s.strip_suffix("%}}")))?
        .trim();
    if let Some(name) = inner.strip_prefix('/') {
        return (name.trim() == "highlight").then_some(Tag::Close);
    }
    // 自分で閉じる形 (`{{< highlight ... />}}`) は対にならない
    if inner.ends_with('/') {
        return None;
    }
    let mut words = inner.split_whitespace();
    if words.next()? != "highlight" {
        return None;
    }
    // 言語: 最初の引数。`"bash"` の引用符は外す。言語の名前らしくないもの (`linenos=table` など) は使わない
    let lang = words
        .next()
        .map(|w| w.trim_matches(['"', '\'', '`']))
        .filter(|w| {
            !w.is_empty()
                && w.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "+-#_.".contains(c))
        })
        .unwrap_or("");
    Some(Tag::Open(lang.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_pairs_become_fences_of_the_same_length() {
        let src = "前\n{{< highlight bash>}}\n# コメント\n$ ls\n{{< / highlight >}}\n# 見出し\n";
        let out = fence_highlight(src);
        assert_eq!(out.len(), src.len());
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[1].trim_end(), "````bash");
        assert_eq!(lines[1].len(), "{{< highlight bash>}}".len());
        assert_eq!(lines[2], "# コメント");
        assert_eq!(lines[4].trim_end(), "````");
        assert_eq!(lines[5], "# 見出し");
        // 行の位置も変わらない
        assert_eq!(out.find("# 見出し"), src.find("# 見出し"));
    }

    #[test]
    fn crlf_indent_and_arguments_are_kept() {
        let src = "  {{< highlight go \"linenos=table\" >}}\r\nx\r\n  {{< /highlight >}}\r\n";
        let out = fence_highlight(src);
        assert_eq!(out.len(), src.len());
        assert!(out.starts_with("  ````go "), "{out:?}");
        assert!(out.contains("\r\n  ````"), "{out:?}");
        assert_eq!(out.matches("\r\n").count(), 3);
        // `{{% %}}` の形と、言語の無い形
        let out = fence_highlight("{{% highlight %}}\n# x\n{{% /highlight %}}\n");
        assert!(out.starts_with("````  "), "{out:?}");
        // 言語らしくない最初の引数は言語にしない
        let out = fence_highlight("{{< highlight linenos=table >}}\nx\n{{< /highlight >}}\n");
        assert!(out.starts_with("```` "), "{out:?}");
    }

    #[test]
    fn unpaired_tags_and_other_shortcodes_are_left_alone() {
        for src in [
            "{{< highlight bash >}}\n# 閉じていない\n",
            "{{< /highlight >}}\n# 開きが無い\n",
            "{{< figure src=\"a.png\" >}}\n{{< toc >}}\n",
            "本文に highlight という語があるだけ\n",
            "文の途中の {{< highlight bash >}} は対象外\nx\n{{< /highlight >}}\n",
        ] {
            assert!(matches!(fence_highlight(src), Cow::Borrowed(_)), "{src:?}");
        }
        // 2 つ続いても、それぞれが対になる
        let src = "{{< highlight a >}}\n1\n{{< /highlight >}}\n# H\n{{< highlight b >}}\n2\n{{< /highlight >}}\n";
        let out = fence_highlight(src);
        assert_eq!(out.matches("````").count(), 4);
        assert!(out.contains("\n# H\n"));
    }
}
