//! サイトモード: 開いている Markdown のファイルから、開発サーバーの上の「そのページの URL」を求める部品
//!
//! ファイルと URL の対応はジェネレーターごとに違い、設定 (permalink の形式、front matter の `url` / `slug` /
//! `permalink`、多言語) にも左右されるので、Typball が自分で計算はしない。定義 (`site::Generator`) に書いた
//! 2 つの方法で求める (どちらも `site::Server::page_url` が別スレッドで行う):
//!
//! 1. **ページ一覧のコマンド** (`pages_command`): ジェネレーター自身に「ファイル → URL」の一覧を出させる。
//!    出力は CSV で、見出しの行に `path` と `permalink` (または `url`) の列があればよい ([`parse_listing`])。
//!    Hugo の `hugo list all` がこの形で、Jekyll は Ruby の 1 行スクリプトで同じ形を出す。正確。
//! 2. **ページの URL の規則** (`page_urls`): 一覧を出せないジェネレーター向け。`フォルダ => URL のひな形` の並びから
//!    候補を作り ([`candidates`])、開発サーバーに実際に問い合わせて、応答のあった最初のものを使う。
//!    外れていても「見つからない」になるだけで、間違ったページは出ない。
//!
//! どちらでも見つからなければ、サイトのトップを表示する。

use std::collections::HashMap;
use std::path::Path;

/// `file` を、プロジェクトのルートからの相対パス (`/` 区切り) にする。ルートの外なら None
pub fn relative(root: &Path, file: &Path) -> Option<String> {
    let rel = match file.strip_prefix(root) {
        Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
        // Windows では大文字と小文字の違い (ドライブ文字など) で外れることがあるので、文字列でも比べる
        Err(_) if cfg!(windows) => {
            let root = root.to_string_lossy().replace('\\', "/");
            let root = root.trim_end_matches('/');
            let file = file.to_string_lossy().replace('\\', "/");
            let inside = file.len() > root.len() + 1
                && file.is_char_boundary(root.len())
                && file[..root.len()].eq_ignore_ascii_case(root)
                && file.as_bytes()[root.len()] == b'/';
            if !inside {
                return None;
            }
            file[root.len() + 1..].to_string()
        }
        Err(_) => return None,
    };
    let rel = rel.trim_matches('/');
    (!rel.is_empty() && rel.split('/').all(|part| part != ".." && part != "."))
        .then(|| rel.to_string())
}

/// 相対パスを、一覧を引くときのキーにする (`/` 区切り、先頭の `./` と `/` を外す。Windows では小文字)
pub fn key(rel: &str) -> String {
    let rel = rel.trim().replace('\\', "/");
    let rel = rel.trim_start_matches("./").trim_start_matches('/');
    if cfg!(windows) {
        rel.to_lowercase()
    } else {
        rel.to_string()
    }
}

/// 一覧のコマンドの出力 (CSV) を「キー → URL のパス」にする。
///
/// 見出しの行 (`path` と、`permalink` か `url` の列がある最初の行) より前は読み飛ばす (ジェネレーターの知らせなど)。
/// URL が `https://example.org/a/b/` のような絶対 URL なら、パスの部分 (`/a/b/`) だけを使う
pub fn parse_listing(output: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut columns: Option<(usize, usize)> = None;
    for line in output.lines() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let fields = parse_csv_line(line);
        let Some((path_col, url_col)) = columns else {
            let find = |name: &str| {
                fields
                    .iter()
                    .position(|f| f.trim().eq_ignore_ascii_case(name))
            };
            if let (Some(path), Some(url)) =
                (find("path"), find("permalink").or_else(|| find("url")))
            {
                columns = Some((path, url));
            }
            continue;
        };
        let (Some(path), Some(url)) = (fields.get(path_col), fields.get(url_col)) else {
            continue;
        };
        if path.trim().is_empty() || url.trim().is_empty() {
            continue;
        }
        map.insert(key(path), url_path(url));
    }
    map
}

/// CSV の 1 行を欄に分ける (`"` で囲んだ欄の中の `,` と `""` に対応)
fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => fields.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    fields.push(cur);
    fields
}

/// URL のパス以降 (`/a/b/?x=1`)。絶対 URL なら `scheme://host` を外す
pub fn url_path(url: &str) -> String {
    let url = url.trim();
    let rest = match url.find("://") {
        Some(i) => {
            let after = &url[i + 3..];
            after.find('/').map(|j| &after[j..]).unwrap_or("/")
        }
        None => url,
    };
    if rest.starts_with('/') {
        rest.to_string()
    } else {
        format!("/{rest}")
    }
}

/// URL の `scheme://host:port` の部分 (開発サーバーの URL から、ページの URL を組むのに使う)
pub fn origin(url: &str) -> &str {
    let Some(i) = url.find("://") else {
        return url.trim_end_matches('/');
    };
    match url[i + 3..].find('/') {
        Some(j) => &url[..i + 3 + j],
        None => url,
    }
}

/// 規則から、`rel` (ルートからの相対パス、`/` 区切り) のページの URL の候補を作る (規則の並びの順)。
///
/// 規則は `フォルダ => ひな形`。フォルダはルートからの相対で、その中のファイルにだけ当てはまる (`.` はルート。
/// `=>` を省くとルート)。ひな形の中で置き換わるもの:
/// - `{path}`: フォルダからの相対パス (拡張子なし)。`index` / `_index` / `README` はそのフォルダになる
///   (`docs/a/index.md` → `a`)
/// - `{dir}` / `{name}`: 同じく、フォルダの部分とファイル名 (拡張子なし)
/// - `{year}` `{month}` `{day}` `{title}`: ファイル名が `2026-11-14-題名` の形のときの各部分。この形でなければ、
///   これらを使う規則は当てはまらない
///
/// 例: `content => /{path}/`、`src => /{path}.html`、`_posts => /{year}/{month}/{day}/{title}.html`
pub fn candidates(rules: &[String], rel: &str) -> Vec<String> {
    let rel = rel.replace('\\', "/");
    let mut out: Vec<String> = Vec::new();
    for rule in rules {
        let (folder, template) = match rule.split_once("=>") {
            Some((folder, template)) => (folder.trim(), template.trim()),
            None => (".", rule.trim()),
        };
        if template.is_empty() {
            continue;
        }
        let folder = folder.replace('\\', "/");
        let folder = folder.trim_matches('/');
        let inner = if folder.is_empty() || folder == "." {
            rel.as_str()
        } else {
            let prefix_len = folder.len() + 1;
            let inside = rel.len() > prefix_len
                && rel.is_char_boundary(folder.len())
                && rel.as_bytes()[folder.len()] == b'/'
                && if cfg!(windows) {
                    rel[..folder.len()].eq_ignore_ascii_case(folder)
                } else {
                    &rel[..folder.len()] == folder
                };
            if !inside {
                continue;
            }
            &rel[prefix_len..]
        };
        let (dir, file) = inner.rsplit_once('/').unwrap_or(("", inner));
        let stem = file.rsplit_once('.').map(|(s, _)| s).unwrap_or(file);
        let is_index = ["index", "_index", "readme"]
            .iter()
            .any(|n| stem.eq_ignore_ascii_case(n));
        let path = if is_index {
            dir.to_string()
        } else if dir.is_empty() {
            stem.to_string()
        } else {
            format!("{dir}/{stem}")
        };
        let mut url = template.to_string();
        if ["{year}", "{month}", "{day}", "{title}"]
            .iter()
            .any(|p| url.contains(p))
        {
            let Some((year, month, day, title)) = dated(stem) else {
                continue;
            };
            url = url
                .replace("{year}", year)
                .replace("{month}", month)
                .replace("{day}", day)
                .replace("{title}", title);
        }
        let url = url
            .replace("{path}", &path)
            .replace("{dir}", dir)
            .replace("{name}", stem);
        let url = tidy_path(&url);
        if !out.contains(&url) {
            out.push(url);
        }
    }
    out
}

/// `2026-11-14-題名` → (年, 月, 日, 題名)
fn dated(stem: &str) -> Option<(&str, &str, &str, &str)> {
    let b = stem.as_bytes();
    let digits = |range: std::ops::Range<usize>| b[range].iter().all(u8::is_ascii_digit);
    (b.len() > 11
        && digits(0..4)
        && b[4] == b'-'
        && digits(5..7)
        && b[7] == b'-'
        && digits(8..10)
        && b[10] == b'-')
        .then(|| (&stem[0..4], &stem[5..7], &stem[8..10], &stem[11..]))
}

/// 先頭に `/` を付け、重なった `/` を 1 つにする (`{path}` が空のときの `//` など)
fn tidy_path(url: &str) -> String {
    let mut out = String::with_capacity(url.len() + 1);
    out.push('/');
    for c in url.chars() {
        if c == '/' && out.ends_with('/') {
            continue;
        }
        out.push(c);
    }
    out
}

/// HTTP の要求の行に書けるように、パスの中の ASCII 以外と空白などを `%XX` にする (`/` `?` `=` `&` `%` などは残す)
pub fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        let keep = b.is_ascii_alphanumeric() || b"-._~/?=&%+:@,;!$'()*".contains(&b);
        if keep {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn hugo_listing_maps_files_to_url_paths() {
        let csv = concat!(
            "WARN something before the header\n",
            "path,slug,title,date,expiryDate,publishDate,draft,permalink,kind,section\n",
            "content/a/b.md,,\"Title, with comma\",2026-01-01T00:00:00Z,,,false,https://example.org/a/b/,page,a\r\n",
            "content\\blog\\x.md,,\"He said \"\"hi\"\"\",,,,true,https://example.org/blog/2013/x/,page,blog\n",
            "content/_index.md,,Top,,,,false,https://example.org/,home,\n",
            "broken line\n",
        );
        let map = parse_listing(csv);
        assert_eq!(map.len(), 3);
        assert_eq!(map[&key("content/a/b.md")], "/a/b/");
        assert_eq!(map[&key("content/blog/x.md")], "/blog/2013/x/");
        assert_eq!(map[&key("content/_index.md")], "/");
        // 見出しの行が無い出力からは何も取らない
        assert!(parse_listing("content/a.md,/a/\n").is_empty());
    }

    #[test]
    fn jekyll_listing_uses_relative_urls() {
        let csv = "path,permalink\n_posts/2026-11-14-hello.md,/2026/11/14/hello.html\nindex.md,/\n";
        let map = parse_listing(csv);
        assert_eq!(
            map[&key("_posts/2026-11-14-hello.md")],
            "/2026/11/14/hello.html"
        );
        assert_eq!(map[&key("index.md")], "/");
        // `url` という列名でもよい
        assert_eq!(parse_listing("url,path\n/x/,x.md\n")[&key("x.md")], "/x/");
    }

    #[test]
    fn urls_split_into_origin_and_path() {
        assert_eq!(url_path("https://example.org/a/b/?p=1"), "/a/b/?p=1");
        assert_eq!(url_path("https://example.org"), "/");
        assert_eq!(url_path("/a/"), "/a/");
        assert_eq!(url_path("a/b"), "/a/b");
        assert_eq!(origin("http://localhost:1313/"), "http://localhost:1313");
        assert_eq!(
            origin("http://127.0.0.1:4000/blog/"),
            "http://127.0.0.1:4000"
        );
        assert_eq!(origin("http://localhost:1313"), "http://localhost:1313");
    }

    #[test]
    fn rules_make_candidates_in_order() {
        // フォルダの中だけに当てはまり、index はフォルダになる
        let zola = rules(&["content => /{path}/"]);
        assert_eq!(candidates(&zola, "content/blog/a.md"), vec!["/blog/a/"]);
        assert_eq!(candidates(&zola, "content/blog/_index.md"), vec!["/blog/"]);
        assert_eq!(candidates(&zola, "content/index.md"), vec!["/"]);
        assert!(candidates(&zola, "README.md").is_empty());
        assert!(candidates(&zola, "contents/a.md").is_empty());

        // 複数の候補 (mdBook)
        let mdbook = rules(&["src => /{path}.html", "src => /{path}/"]);
        assert_eq!(
            candidates(&mdbook, "src/guide/intro.md"),
            vec!["/guide/intro.html", "/guide/intro/"]
        );
        assert_eq!(
            candidates(&mdbook, "src/guide/README.md"),
            vec!["/guide.html", "/guide/"]
        );

        // 日付の付いたファイル名。形が違えば当てはまらない
        let jekyll = rules(&[
            "_posts => /{year}/{month}/{day}/{title}.html",
            ". => /{path}.html",
            "/{path}/",
        ]);
        assert_eq!(
            candidates(&jekyll, "_posts/2026-11-14-hello-world.md"),
            vec![
                "/2026/11/14/hello-world.html",
                "/_posts/2026-11-14-hello-world.html",
                "/_posts/2026-11-14-hello-world/"
            ]
        );
        assert_eq!(
            candidates(&jekyll, "about.md"),
            vec!["/about.html", "/about/"]
        );
        // {dir} と {name}
        assert_eq!(
            candidates(&rules(&["docs => /{dir}/x-{name}"]), "docs/a/b.md"),
            vec!["/a/x-b"]
        );
    }

    #[test]
    fn paths_are_relative_to_the_project() {
        let root = Path::new("/site");
        assert_eq!(
            relative(root, Path::new("/site/content/a.md")).as_deref(),
            Some("content/a.md")
        );
        assert_eq!(relative(root, Path::new("/other/a.md")), None);
        assert_eq!(relative(root, Path::new("/site")), None);
        assert_eq!(key("./content\\a.md"), "content/a.md");
        assert_eq!(key("/content/a.md"), "content/a.md");
        if cfg!(windows) {
            // Windows のファイル名は大文字と小文字を区別しない
            assert_eq!(key("Content/A.md"), "content/a.md");
            assert_eq!(
                relative(Path::new("D:\\Site"), Path::new("d:\\site\\Content\\A.md")).as_deref(),
                Some("Content/A.md")
            );
        }
        assert_eq!(
            encode_path("/a b/日本.html?x=1"),
            "/a%20b/%E6%97%A5%E6%9C%AC.html?x=1"
        );
    }
}
