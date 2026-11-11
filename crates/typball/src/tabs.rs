//! タブとスペースの相互変換 (編集 > タブをスペースに変換 / スペースをタブに変換)
//!
//! 桁は「半角 1 文字の幅」を 1 として数える。文字の幅は呼ぶ側が教える (`width`): 全角文字は 2 など。
//! エディタはタブを「タブ幅 × 半角の幅」ごとの位置にそろえて描くので、同じ数え方で変換すれば、
//! 変換の前後で文字の位置が変わらない。改行 (CR / LF) で桁は 0 に戻り、改行そのものは変えない

/// `col` の次のタブ位置
fn next_stop(col: usize, tab: usize) -> usize {
    (col / tab + 1) * tab
}

/// `text` (1 行の先頭からの部分) の終わりの桁
pub fn column(text: &str, tab: usize, mut width: impl FnMut(char) -> usize) -> usize {
    let tab = tab.max(1);
    let mut col = 0;
    for c in text.chars() {
        match c {
            '\t' => col = next_stop(col, tab),
            '\r' | '\n' => col = 0,
            _ => col += width(c),
        }
    }
    col
}

/// `col` から次のタブ位置までの桁数 (1..=`tab`)。Tab キーで入れるスペースの数
pub fn to_next_stop(col: usize, tab: usize) -> usize {
    next_stop(col, tab.max(1)) - col
}

/// Shift+Tab でキャレットを戻す先: `text` (行頭からキャレットまで) の中の位置 (バイト数)。
/// 前のタブ位置に一番近い、それ以下の桁の位置 (全角文字の途中にタブ位置が来るときは、その文字の前)
pub fn previous_stop_index(text: &str, tab: usize, mut width: impl FnMut(char) -> usize) -> usize {
    let tab = tab.max(1);
    // 各文字の後ろの (位置, 桁)
    let mut positions = vec![(0, 0)];
    let mut col = 0;
    for (i, c) in text.char_indices() {
        col = match c {
            '\t' => next_stop(col, tab),
            '\r' | '\n' => 0,
            _ => col + width(c),
        };
        positions.push((i + c.len_utf8(), col));
    }
    if col == 0 {
        return text.len();
    }
    let target = (col - 1) / tab * tab;
    positions
        .iter()
        .rev()
        .find(|(_, c)| *c <= target)
        .map_or(0, |(i, _)| *i)
}

/// タブを、次のタブ位置までのスペースに置き換える
pub fn expand(text: &str, tab: usize, mut width: impl FnMut(char) -> usize) -> String {
    let tab = tab.max(1);
    let mut out = String::with_capacity(text.len() + text.len() / 8);
    let mut col = 0;
    for c in text.chars() {
        match c {
            '\t' => {
                let stop = next_stop(col, tab);
                out.extend(std::iter::repeat(' ').take(stop - col));
                col = stop;
            }
            '\r' | '\n' => {
                out.push(c);
                col = 0;
            }
            _ => {
                out.push(c);
                col += width(c);
            }
        }
    }
    out
}

/// スペースの並びを、タブ位置をまたぐところでタブに置き換える (またがない残りはスペースのまま)。
///
/// - 単語の間の 1 個だけのスペースは、たまたまタブ位置で終わっていてもそのままにする
///   (文章のスペースが 1 つおきにタブになってしまう)。
/// - スペースとタブが混ざった並び (`"  \t"` など) は、同じ位置まで進む一番短い形に直る。
pub fn tabify(text: &str, tab: usize, mut width: impl FnMut(char) -> usize) -> String {
    let tab = tab.max(1);
    let mut out = String::with_capacity(text.len());
    let mut col = 0;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' => {
                // 空白の並びの終わりの桁を求める
                let start = col;
                let mut only_one_space = c == ' ';
                col = if c == '\t' {
                    next_stop(col, tab)
                } else {
                    col + 1
                };
                while let Some(&next) = chars.peek() {
                    match next {
                        ' ' => col += 1,
                        '\t' => col = next_stop(col, tab),
                        _ => break,
                    }
                    only_one_space = false;
                    chars.next();
                }
                if only_one_space {
                    out.push(' ');
                    continue;
                }
                let mut at = start;
                while next_stop(at, tab) <= col {
                    out.push('\t');
                    at = next_stop(at, tab);
                }
                out.extend(std::iter::repeat(' ').take(col - at));
            }
            '\r' | '\n' => {
                out.push(c);
                col = 0;
            }
            _ => {
                out.push(c);
                col += width(c);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 全角 (ひらがな・カタカナ・漢字・全角英数) を 2、ほかを 1 と数える
    fn width(c: char) -> usize {
        if ('\u{3000}'..='\u{9fff}').contains(&c) || ('\u{ff00}'..='\u{ff60}').contains(&c) {
            2
        } else {
            1
        }
    }

    #[test]
    fn tabs_expand_to_the_next_stop() {
        assert_eq!(expand("\ta", 4, width), "    a");
        assert_eq!(expand("ab\tc", 4, width), "ab  c");
        assert_eq!(expand("abcd\te", 4, width), "abcd    e");
        assert_eq!(expand("a\t\tb", 4, width), "a       b");
        // 改行で桁が戻る。改行そのものは変えない
        assert_eq!(
            expand("ab\tc\r\n\td\ne\tf\rg\th", 4, width),
            "ab  c\r\n    d\ne   f\rg   h"
        );
        // 全角文字は 2 桁
        assert_eq!(expand("あ\tb", 4, width), "あ  b");
        assert_eq!(expand("あい\tb", 4, width), "あい    b");
        // タブ幅 1・タブが無い文書・空の文書
        assert_eq!(expand("a\tb", 1, width), "a b");
        assert_eq!(expand("no tabs here", 4, width), "no tabs here");
        assert_eq!(expand("", 4, width), "");
    }

    #[test]
    fn columns_count_tabs_and_wide_characters() {
        assert_eq!(column("", 4, width), 0);
        assert_eq!(column("abc", 4, width), 3);
        assert_eq!(column("a\tb", 4, width), 5);
        assert_eq!(column("\t\t", 4, width), 8);
        assert_eq!(column("あい  ", 4, width), 6);
        assert_eq!(column("    ", 2, width), 4);
    }

    #[test]
    fn the_tab_key_fills_up_to_the_next_stop() {
        // (行頭からキャレットまで, タブ幅 4 で入れるスペースの数)
        for (before, spaces) in [
            ("", 4),
            ("a", 3),
            ("abc", 1),
            ("abcd", 4),
            // 全角文字は 2 桁: 文字数で数えると 3 個・2 個・3 個になってしまう
            ("あ", 2),
            ("あい", 4),
            ("aあ", 1),
            ("あいう", 2),
            ("\t", 4),
            ("a\tb", 3),
        ] {
            let col = column(before, 4, width);
            assert_eq!(to_next_stop(col, 4), spaces, "{before:?}");
        }
        assert_eq!(to_next_stop(3, 1), 1);
    }

    #[test]
    fn shift_tab_goes_back_to_the_previous_stop() {
        // (行頭からキャレットまで, 戻った後に残る部分)
        for (before, kept) in [
            ("abcdef", "abcd"),
            ("abcd", ""),
            ("abcdefgh", "abcd"),
            ("abc", ""),
            // 全角文字: 「あい」で 4 桁
            ("あいう", "あい"),
            ("あい", ""),
            // タブ位置 (4 桁目) が全角文字の途中に来る: その文字の前 (3 桁目) まで戻る
            ("abcあd", "abc"),
            // タブとスペース
            ("a\tb", "a\t"),
            ("ab  cd", "ab  "),
            ("", ""),
        ] {
            let index = previous_stop_index(before, 4, width);
            assert_eq!(&before[..index], kept, "{before:?}");
        }
    }

    #[test]
    fn spaces_become_tabs_where_they_cross_a_stop() {
        assert_eq!(tabify("    a", 4, width), "\ta");
        assert_eq!(tabify("        a", 4, width), "\t\ta");
        // タブ位置に届かない残りはスペースのまま
        assert_eq!(tabify("      a", 4, width), "\t  a");
        assert_eq!(tabify("  a", 4, width), "  a");
        // 行の途中: 桁 2 から 4 まで (2 個) はタブ位置で終わるのでタブになる
        assert_eq!(tabify("ab  c", 4, width), "ab\tc");
        assert_eq!(tabify("ab   c", 4, width), "ab\t c");
        // 1 個だけのスペースは、タブ位置で終わっていてもそのまま
        assert_eq!(tabify("abc d", 4, width), "abc d");
        assert_eq!(tabify("a b c d e", 4, width), "a b c d e");
        // スペースとタブが混ざった並びは一番短い形に
        assert_eq!(tabify("  \ta", 4, width), "\ta");
        assert_eq!(tabify("\t    a", 4, width), "\t\ta");
        assert_eq!(tabify("a \tb", 4, width), "a\tb");
        // 改行で桁が戻る
        assert_eq!(tabify("    a\r\n    b\n  c", 4, width), "\ta\r\n\tb\n  c");
        // 全角文字は 2 桁: 「あ」の後の 2 個は桁 4 で終わる
        assert_eq!(tabify("あ  b", 4, width), "あ\tb");
    }

    #[test]
    fn expanding_what_was_tabified_gives_the_same_layout() {
        let tab = 4;
        for text in [
            "    fn main() {\n        let x = 1;  // note\n    }\n",
            "名前    値      説明\r\nあいう  12      ここ\r\n",
            "a b  c   d    e     f\n",
            "\t mixed \t  runs\t\n",
        ] {
            let spaces = expand(text, tab, width);
            let tabs = tabify(text, tab, width);
            // タブにしても、スペースに戻せば同じ配置
            assert_eq!(expand(&tabs, tab, width), spaces, "{text:?}");
            // もう一度かけても変わらない
            assert_eq!(tabify(&tabs, tab, width), tabs, "{text:?}");
            assert_eq!(expand(&spaces, tab, width), spaces, "{text:?}");
        }
    }
}
