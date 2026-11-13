//! 自動インデント: Enter で改行したときに、新しい行の先頭へ入れる字下げを決める
//!
//! 決め方は設定 `editor.auto_indent` ([`AutoIndent`]) で選ぶ (形式ごとに上書きできる)。
//! 決め方を足すときは、[`AutoIndent`] に種類を足して [`for_new_line`] の `match` を埋める。
//! ブロックに合わせて字下げを増減する決め方 (`{` の後ろで 1 段深くする、など) を足すときは、
//! ここに「キャレットより前の行」「タブ幅」「タブ文字を使うか」を渡して決める形にする
//! (今の決め方は、キャレットのある行の先頭だけ見れば足りる)。

use crate::settings::AutoIndent;

/// 新しい行の先頭に入れる字下げ。`before` は、改行する位置のある行の、行頭からキャレットまで
pub fn for_new_line(mode: AutoIndent, before: &str) -> String {
    match mode {
        AutoIndent::None => String::new(),
        AutoIndent::Keep => leading_whitespace(before).to_string(),
    }
}

/// 行頭の空白 (半角スペースとタブ)。全角空白は字下げとして扱わない: 日本語の文章では段落の頭の 1 字下げに
/// 使うので、次の行にも入れると、段落の途中の行まで字下げされてしまう
fn leading_whitespace(line: &str) -> &str {
    let end = line
        .find(|c: char| c != ' ' && c != '\t')
        .unwrap_or(line.len());
    &line[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keep_copies_the_indent_of_the_current_line() {
        assert_eq!(for_new_line(AutoIndent::Keep, "    let x = 1;"), "    ");
        assert_eq!(for_new_line(AutoIndent::Keep, "\t  - item"), "\t  ");
        assert_eq!(for_new_line(AutoIndent::Keep, "no indent"), "");
        // 字下げの途中で改行したら、キャレットまでの分だけ (残りは元の行の頭に付いたまま、新しい行へ行く)
        assert_eq!(for_new_line(AutoIndent::Keep, "  "), "  ");
        // 全角空白は字下げに数えない
        assert_eq!(for_new_line(AutoIndent::Keep, "　段落の頭"), "");
        assert_eq!(for_new_line(AutoIndent::Keep, "  　混ざった行"), "  ");
    }

    #[test]
    fn none_never_indents() {
        assert_eq!(for_new_line(AutoIndent::None, "    indented"), "");
    }
}
