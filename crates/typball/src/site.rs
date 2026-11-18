//! 静的サイトジェネレーター (SSG) のプロジェクト検出と開発サーバーの管理
//!
//! ジェネレーターは「定義」([`Generator`]) の表で扱う。定義は、プロジェクトのルートを見分ける条件 (`detect`)、
//! 開発サーバーのコマンド、表示する URL、ルート相対の URL を探すフォルダからなる。Hugo・Jekyll などの定義を
//! 内蔵し ([`BUILTINS`])、設定 `[preview.generators.<ID>]` で内蔵の定義の一部を上書きしたり、内蔵に無い
//! ジェネレーターを足したりできる ([`generators`])。ジェネレーターごとのコードは無い。
//!
//! `.md` のあるフォルダから上へ辿り、どれかの定義の条件に当てはまる最初のフォルダをプロジェクトのルートとする
//! ([`detect`])。そのフォルダを作業フォルダにして定義のコマンドを子プロセスとして起動し、WebView に URL を出す。
//! サーバーはプロジェクトのルートごとに 1 つで、タブをまたいで共有し、アプリの終了時に止める。
//!
//! WebView には、サイトのトップではなく、開いているファイルのページを出す ([`Server::page_url`]。求め方は
//! `sitepages.rs`: ジェネレーターに出させた一覧か、URL の規則)。保存の後は、開発サーバーのライブリロードで
//! そのページが更新される (Hugo は `-N` = navigateToChanged で、保存したページへ自分でも移動する)。

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::settings::{GeneratorOverride, Preview};
use crate::sitepages;

// ---------------------------------------------------------------------------
// ジェネレーターの定義
// ---------------------------------------------------------------------------

/// ジェネレーターの定義 (内蔵の定義に設定の上書きを重ねたもの、または設定で足したもの)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Generator {
    /// 設定ファイルのテーブル名 (`[preview.generators.hugo]`)
    pub id: String,
    /// 画面に出す名前
    pub name: String,
    /// 検出に使うか (切ると、このジェネレーターのプロジェクトは素の Markdown として扱う)
    pub enabled: bool,
    /// プロジェクトのルートを見分ける条件。どれか 1 つに当てはまればよい ([`rule_matches`])
    pub detect: Vec<String>,
    /// 開発サーバーのコマンド。`{port}` `{port2}` `{root}` が置き換わる
    pub command: String,
    /// WebView に表示する URL。`{port}` が置き換わる
    pub url: String,
    /// ルート相対の URL (`/images/x.png`) を探すフォルダ (ルートからの相対。`.` はルートそのもの)
    pub static_dirs: Vec<String>,
    /// 「ファイル → URL」の一覧を出すコマンド (空なら使わない)。プロジェクトのフォルダで実行し、出力は CSV
    /// (`sitepages::parse_listing`)。`{root}` が置き換わる
    pub pages_command: String,
    /// ページの URL の規則 (`フォルダ => ひな形`。`sitepages::candidates`)。一覧に無いとき、候補を開発サーバーに
    /// 問い合わせて確かめる
    pub page_urls: Vec<String>,
    /// 内蔵の定義か (設定で足したものは false)
    pub builtin: bool,
}

struct Builtin {
    id: &'static str,
    name: &'static str,
    detect: &'static [&'static str],
    command: &'static str,
    url: &'static str,
    static_dirs: &'static [&'static str],
    pages_command: &'static str,
    page_urls: &'static [&'static str],
}

/// Jekyll に「ファイル → URL」の一覧を出させる Ruby の 1 行スクリプト (`$1` は先頭に付けるコマンド)。
/// サイトを読み込むだけで組みはしない (`site.read`)。URL は Jekyll 自身が計算するので、`_config.yml` の permalink の
/// 形式や front matter の `permalink`、カテゴリー、下書きの日付もそのとおりになる。
/// スクリプトの中に `"` `%` `^` を書かないこと: Bundler 経由では `bundle.bat` (cmd.exe) を通る
macro_rules! jekyll_pages {
    ($prefix:literal) => {
        concat!(
            $prefix,
            "ruby -rjekyll -rcsv -e \"$stdout.set_encoding('UTF-8'); ",
            "s = Jekyll::Site.new(Jekyll.configuration('show_drafts' => true, 'quiet' => true)); s.read; ",
            "puts 'path,permalink'; ",
            "(s.pages + s.collections.values.flat_map { |c| c.docs }).each ",
            "{ |d| print [d.relative_path.delete_prefix('/'), d.url].to_csv }\""
        )
    };
}

/// Jekyll の一覧が取れなかったとき (Ruby が無いなど) の規則: 既定の permalink の形式だけを当てる
const JEKYLL_PAGE_URLS: &[&str] = &[
    "_posts => /{year}/{month}/{day}/{title}.html",
    ". => /{path}.html",
    ". => /{path}/",
];

/// 内蔵の定義。並びは設定画面の並びで、検出の優先順でもある (同じフォルダが複数に当てはまるときは先のもの)。
/// 同じ名前の設定ファイルを使うもの (Hugo の旧 config.toml と Zola、Jekyll と Hexo) は、条件の側で見分けている
const BUILTINS: &[Builtin] = &[
    Builtin {
        id: "hugo",
        name: "Hugo",
        detect: &[
            "hugo.toml|hugo.yaml|hugo.yml|hugo.json",
            "config/_default/hugo.*|config/_default/config.*",
            // 旧来の config.toml。Zola も同じ名前を使うが、Zola は base_url (Hugo は baseURL) を必ず書く
            "config.toml|config.yaml|config.yml|config.json + content|layouts|themes|archetypes + !config.toml:base_url",
        ],
        // -D: 下書きも出す、-N: 保存したページへ移動、-M: 出力をディスクに書かない
        command: "hugo server -D -N -M --port {port} -s {root}",
        url: "http://localhost:{port}/",
        static_dirs: &["static", "assets"],
        pages_command: "hugo list all",
        page_urls: &["content => /{path}/"],
    },
    Builtin {
        id: "jekyll-bundler",
        name: "Jekyll (Bundler)",
        detect: &["_config.yml|_config.yaml|_config.toml + Gemfile|gems.rb + !package.json:hexo"],
        // --baseurl "": _config.yml の baseurl (/blog など) を外して、サイトのトップを / に出す
        command: "bundle exec jekyll serve --drafts --livereload --livereload-port {port2} --host 127.0.0.1 --port {port} --baseurl \"\"",
        url: "http://127.0.0.1:{port}/",
        static_dirs: &["."],
        pages_command: jekyll_pages!("bundle exec "),
        page_urls: JEKYLL_PAGE_URLS,
    },
    Builtin {
        id: "jekyll",
        name: "Jekyll",
        detect: &["_config.yml|_config.yaml|_config.toml + !package.json:hexo"],
        command: "jekyll serve --drafts --livereload --livereload-port {port2} --host 127.0.0.1 --port {port} --baseurl \"\"",
        url: "http://127.0.0.1:{port}/",
        static_dirs: &["."],
        pages_command: jekyll_pages!(""),
        page_urls: JEKYLL_PAGE_URLS,
    },
    Builtin {
        id: "zola",
        name: "Zola",
        detect: &["config.toml:base_url"],
        command: "zola serve --drafts --interface 127.0.0.1 --port {port}",
        url: "http://127.0.0.1:{port}/",
        static_dirs: &["static"],
        pages_command: "",
        page_urls: &["content => /{path}/"],
    },
    Builtin {
        id: "eleventy",
        name: "Eleventy",
        detect: &[".eleventy.js|.eleventy.cjs|eleventy.config.*"],
        command: "npx @11ty/eleventy --serve --port={port}",
        url: "http://localhost:{port}/",
        static_dirs: &["public", "."],
        pages_command: "",
        page_urls: &["src => /{path}/", ". => /{path}/"],
    },
    Builtin {
        id: "astro",
        name: "Astro",
        detect: &["astro.config.*"],
        command: "npx astro dev --port {port}",
        url: "http://localhost:{port}/",
        static_dirs: &["public"],
        pages_command: "",
        page_urls: &[],
    },
    Builtin {
        id: "hexo",
        name: "Hexo",
        detect: &["_config.yml + package.json:hexo"],
        command: "npx hexo server --draft -p {port}",
        url: "http://localhost:{port}/",
        static_dirs: &["source"],
        pages_command: "",
        page_urls: &[],
    },
    Builtin {
        id: "mkdocs",
        name: "MkDocs",
        detect: &["mkdocs.yml|mkdocs.yaml"],
        command: "mkdocs serve -a 127.0.0.1:{port}",
        url: "http://127.0.0.1:{port}/",
        static_dirs: &["docs"],
        pages_command: "",
        page_urls: &["docs => /{path}/"],
    },
    Builtin {
        id: "docusaurus",
        name: "Docusaurus",
        detect: &["docusaurus.config.*"],
        command: "npx docusaurus start --no-open --port {port}",
        url: "http://localhost:{port}/",
        static_dirs: &["static"],
        pages_command: "",
        page_urls: &[],
    },
    Builtin {
        id: "vitepress",
        name: "VitePress",
        detect: &[".vitepress/config.*"],
        command: "npx vitepress dev --port {port}",
        url: "http://localhost:{port}/",
        static_dirs: &["public"],
        pages_command: "",
        page_urls: &[],
    },
    Builtin {
        id: "mdbook",
        name: "mdBook",
        detect: &["book.toml"],
        command: "mdbook serve -n 127.0.0.1 -p {port}",
        url: "http://127.0.0.1:{port}/",
        static_dirs: &["src"],
        pages_command: "",
        page_urls: &["src => /{path}.html", "src => /{path}/"],
    },
    Builtin {
        id: "pelican",
        name: "Pelican",
        detect: &["pelicanconf.py"],
        command: "pelican --autoreload --listen --port {port}",
        url: "http://127.0.0.1:{port}/",
        static_dirs: &["content"],
        pages_command: "",
        page_urls: &[],
    },
];

/// 設定で足した定義で、URL を書かなかったときの値
pub const DEFAULT_URL: &str = "http://localhost:{port}/";

impl Builtin {
    fn generator(&self) -> Generator {
        Generator {
            id: self.id.to_string(),
            name: self.name.to_string(),
            enabled: true,
            detect: self.detect.iter().map(|s| s.to_string()).collect(),
            command: self.command.to_string(),
            url: self.url.to_string(),
            static_dirs: self.static_dirs.iter().map(|s| s.to_string()).collect(),
            pages_command: self.pages_command.to_string(),
            page_urls: self.page_urls.iter().map(|s| s.to_string()).collect(),
            builtin: true,
        }
    }
}

/// 内蔵の定義 (上書きの無い既定の値)。`id` が内蔵に無ければ None
pub fn builtin(id: &str) -> Option<Generator> {
    BUILTINS.iter().find(|b| b.id == id).map(Builtin::generator)
}

impl Generator {
    /// 設定の上書きを重ねる (書かれた項目だけ)
    fn apply(&mut self, over: &GeneratorOverride) {
        if let Some(name) = over.name.as_ref().filter(|n| !n.trim().is_empty()) {
            self.name = name.trim().to_string();
        }
        if let Some(enabled) = over.enabled {
            self.enabled = enabled;
        }
        if let Some(detect) = &over.detect {
            self.detect = clean_list(detect);
        }
        if let Some(command) = over.command.as_ref().filter(|c| !c.trim().is_empty()) {
            self.command = command.trim().to_string();
        }
        if let Some(url) = over.url.as_ref().filter(|u| !u.trim().is_empty()) {
            self.url = url.trim().to_string();
        }
        if let Some(dirs) = &over.static_dirs {
            self.static_dirs = clean_list(dirs);
        }
        // この 2 つは、空を書くと「使わない」(内蔵の一覧のコマンドや規則を止められる)
        if let Some(command) = &over.pages_command {
            self.pages_command = command.trim().to_string();
        }
        if let Some(rules) = &over.page_urls {
            self.page_urls = clean_list(rules);
        }
    }

    /// 設定ファイルに書く形にする: 内蔵の定義は既定と違う項目だけ、設定で足した定義は全部。
    /// 何も書くことが無ければ None (内蔵の定義を既定のまま使っている)
    pub fn to_override(&self) -> Option<GeneratorOverride> {
        let Some(base) = builtin(&self.id) else {
            return Some(GeneratorOverride {
                name: Some(self.name.clone()),
                enabled: (!self.enabled).then_some(false),
                detect: Some(self.detect.clone()),
                command: Some(self.command.clone()),
                url: (self.url != DEFAULT_URL).then(|| self.url.clone()),
                static_dirs: (!self.static_dirs.is_empty()).then(|| self.static_dirs.clone()),
                pages_command: (!self.pages_command.is_empty()).then(|| self.pages_command.clone()),
                page_urls: (!self.page_urls.is_empty()).then(|| self.page_urls.clone()),
            });
        };
        let over = GeneratorOverride {
            name: None,
            enabled: (self.enabled != base.enabled).then_some(self.enabled),
            detect: (self.detect != base.detect).then(|| self.detect.clone()),
            command: (self.command != base.command).then(|| self.command.clone()),
            url: (self.url != base.url).then(|| self.url.clone()),
            static_dirs: (self.static_dirs != base.static_dirs).then(|| self.static_dirs.clone()),
            pages_command: (self.pages_command != base.pages_command)
                .then(|| self.pages_command.clone()),
            page_urls: (self.page_urls != base.page_urls).then(|| self.page_urls.clone()),
        };
        (over != GeneratorOverride::default()).then_some(over)
    }
}

fn clean_list(items: &[String]) -> Vec<String> {
    items
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// 有効な定義の一覧 (検出の優先順): 設定で足したもの (ID の順)、内蔵のもの (上書きを重ねて) の順。
/// 設定で足したものを先にするのは、内蔵と同じ設定ファイル名を使う自作の定義が負けないように
pub fn generators(preview: &Preview) -> Vec<Generator> {
    let mut out: Vec<Generator> = preview
        .generators
        .iter()
        .filter(|(id, _)| builtin(id).is_none())
        .map(|(id, over)| {
            let mut g = Generator {
                id: id.clone(),
                name: id.clone(),
                enabled: true,
                detect: Vec::new(),
                command: String::new(),
                url: DEFAULT_URL.to_string(),
                static_dirs: Vec::new(),
                pages_command: String::new(),
                page_urls: Vec::new(),
                builtin: false,
            };
            g.apply(over);
            g
        })
        .collect();
    out.extend(BUILTINS.iter().map(|b| {
        let mut g = b.generator();
        if let Some(over) = preview.generators.get(b.id) {
            g.apply(over);
        }
        g
    }));
    out
}

/// 定義の一覧を設定ファイルに書く形にする ([`generators`] の逆)
pub fn to_overrides(list: &[Generator]) -> BTreeMap<String, GeneratorOverride> {
    list.iter()
        .filter_map(|g| Some((g.id.clone(), g.to_override()?)))
        .collect()
}

// ---------------------------------------------------------------------------
// プロジェクトの検出
// ---------------------------------------------------------------------------

/// 上へ辿るフォルダの数の上限
const MAX_DEPTH: usize = 32;

/// 見つかったプロジェクト
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    /// 設定ファイルのあるフォルダ (サーバーの作業フォルダ、`{root}`)
    pub root: PathBuf,
    pub generator: Generator,
}

impl Project {
    /// ルート相対の URL (`/images/x.png`) を探すフォルダ
    pub fn static_dirs(&self) -> Vec<PathBuf> {
        self.generator
            .static_dirs
            .iter()
            .filter(|d| is_plain_relative(Path::new(d)))
            .map(|d| self.root.join(d))
            .collect()
    }
}

/// `path` (ファイルかフォルダ) から上へ辿ってプロジェクトを探す。近いフォルダが優先で、同じフォルダが
/// 複数の定義に当てはまるときは `generators` の並びで先のもの
pub fn detect(path: &Path, generators: &[Generator]) -> Option<Project> {
    let mut dir = if path.is_dir() {
        Some(path)
    } else {
        path.parent()
    };
    for _ in 0..MAX_DEPTH {
        let d = dir?;
        let found = generators.iter().find(|g| {
            g.enabled
                && !g.command.trim().is_empty()
                && g.detect.iter().any(|rule| rule_matches(d, rule))
        });
        if let Some(generator) = found {
            return Some(Project {
                root: d.to_path_buf(),
                generator: generator.clone(),
            });
        }
        dir = d.parent();
    }
    None
}

/// 条件 1 つが `dir` に当てはまるか。
///
/// 条件は `+` で区切った項目の並びで、すべての項目が成り立つとき当てはまる。項目は `|` で区切った名前の並びで、
/// どれか 1 つがあれば成り立つ。先頭に `!` を付けた項目は、どれも無いときに成り立つ。名前はフォルダからの
/// 相対パス (ファイルでもフォルダでも) で、最後の部分に `*` を使える。`名前:文字列` は「そのファイルがあり、
/// 中にその文字列がある」。`!` の項目だけの条件は当てはまらない (どのフォルダにも当てはまってしまうので)。
///
/// 例: `config.toml|config.yaml + content|layouts + !config.toml:base_url`
pub fn rule_matches(dir: &Path, rule: &str) -> bool {
    let mut positive = false;
    for term in rule.split('+') {
        let term = term.trim();
        if term.is_empty() {
            continue;
        }
        let (negate, names) = match term.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, term),
        };
        let hit = names.split('|').any(|name| name_matches(dir, name.trim()));
        if hit == negate {
            return false;
        }
        positive |= !negate;
    }
    positive
}

fn name_matches(dir: &Path, name: &str) -> bool {
    let (file, needle) = match name.split_once(':') {
        Some((file, needle)) => (file.trim(), Some(needle.trim())),
        None => (name, None),
    };
    let rel = Path::new(file);
    if file.is_empty() || !is_plain_relative(rel) {
        return false;
    }
    let accept = |path: &Path| match needle {
        None => path.exists(),
        Some(needle) => path.is_file() && file_contains(path, needle),
    };
    let last = rel
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !last.contains('*') {
        return accept(&dir.join(rel));
    }
    // 最後の部分に * がある: そのフォルダの中から名前の合うものを探す
    let parent = match rel.parent() {
        Some(p) => dir.join(p),
        None => dir.to_path_buf(),
    };
    let Ok(entries) = std::fs::read_dir(parent) else {
        return false;
    };
    entries
        .flatten()
        .any(|e| wildcard_matches(&last, &e.file_name().to_string_lossy()) && accept(&e.path()))
}

/// `*` (0 文字以上) だけのワイルドカード。大文字と小文字は区別しない
fn wildcard_matches(pattern: &str, name: &str) -> bool {
    let pattern = pattern.to_lowercase();
    let name = name.to_lowercase();
    let parts: Vec<&str> = pattern.split('*').collect();
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if parts.len() == 1 {
        return name == first;
    }
    if name.len() < first.len() + last.len() || !name.starts_with(first) || !name.ends_with(last) {
        return false;
    }
    let mut rest = &name[first.len()..name.len() - last.len()];
    for part in &parts[1..parts.len() - 1] {
        match rest.find(part) {
            Some(i) => rest = &rest[i + part.len()..],
            None => return false,
        }
    }
    true
}

/// ファイルの先頭 256 KB に `needle` があるか
fn file_contains(path: &Path, needle: &str) -> bool {
    const LIMIT: u64 = 256 * 1024;
    if needle.is_empty() {
        return true;
    }
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut data = Vec::new();
    if file.take(LIMIT).read_to_end(&mut data).is_err() {
        return false;
    }
    data.windows(needle.len()).any(|w| w == needle.as_bytes())
}

/// `..` や絶対パス、ドライブ文字を含まない相対パスか
fn is_plain_relative(path: &Path) -> bool {
    path.components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

// ---------------------------------------------------------------------------
// 開発サーバー
// ---------------------------------------------------------------------------

/// 残しておく出力の量 (これを超えたら古い行から捨てる)
const LOG_LIMIT: usize = 16 * 1024;

/// 起動した開発サーバー (プロセスは drop で止まる)
pub struct Server {
    child: RefCell<Option<Child>>,
    /// 子プロセスとその子孫を入れたジョブ (Windows)。閉じると全部止まる
    #[cfg(windows)]
    job: RefCell<Option<Job>>,
    pub url: String,
    /// 起動時に使ったコマンドと URL (設定が変わったら作り直すための比較用)
    key: String,
    /// 起動した時刻 (落ちたサーバーを、すぐには起動し直さないため)
    started: Instant,
    /// HTTP の要求に応答した (ページを出せる)。別スレッド ([`watch_ready`]) が立てる
    ready: Arc<AtomicBool>,
    /// 止めた・終わった (応答を待つスレッドを終わらせる)
    stopped: Arc<AtomicBool>,
    /// stderr の先頭 (起動に失敗した理由を出すため)
    stderr: Arc<Mutex<String>>,
    /// stdout と stderr の末尾 (コンソールペイン用)
    log: Arc<Mutex<String>>,
    /// プロジェクトのルート (ページの URL を求めるとき、ファイルの相対パスの基準)
    root: PathBuf,
    port: u16,
    /// 定義の「ページ一覧のコマンド」と「ページの URL の規則」(起動したときの値)
    pages_command: String,
    page_urls: Arc<Vec<String>>,
    /// ファイル → URL の対応 (別スレッドが埋める)
    pages: Arc<Mutex<Pages>>,
}

/// ファイル → URL の対応の、今分かっていること
#[derive(Default)]
struct Pages {
    /// 一覧のコマンドの結果 (`sitepages::key` → URL のパス)。まだ取れていなければ None
    listing: Option<HashMap<String, String>>,
    listing_running: bool,
    /// 最後に一覧のコマンドが終わった時刻 (失敗も含む)
    listed_at: Option<Instant>,
    /// 規則で探した結果 (キー → 見つかった URL のパス。None は「どの候補にも応答が無かった」)
    probed: HashMap<String, Option<String>>,
    probing: HashSet<String>,
}

/// ファイルのページの URL ([`Server::page_url`])
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageUrl {
    /// 調べている途中 (少し後でもう一度聞く)
    Pending,
    Found(String),
    /// 分からない (サイトのトップを出す)
    Unknown,
}

/// 一覧に無いファイルを聞かれたとき、一覧を取り直してよい間隔 (新しく作ったファイルのため)
const RELIST_AFTER: Duration = Duration::from_secs(10);

impl Server {
    /// `file` のページの URL。待たずに、今分かっていることを返す: まだ調べていなければ別スレッドで調べ始めて
    /// [`PageUrl::Pending`] を返すので、呼び出し側はタイマーでもう一度聞く。
    ///
    /// 一覧のコマンドの結果にあればそれ、無ければ規則の候補を開発サーバーに問い合わせて確かめる (`sitepages.rs`)。
    /// `refresh` は、一覧に無いときに一覧を取り直してよいか (プレビューを開いたときの最初の 1 回だけ true にする)
    pub fn page_url(&self, file: &Path, refresh: bool) -> PageUrl {
        let Some(rel) = sitepages::relative(&self.root, file) else {
            return PageUrl::Unknown;
        };
        let key = sitepages::key(&rel);
        let found = |path: &str| PageUrl::Found(format!("{}{path}", sitepages::origin(&self.url)));
        {
            let Ok(mut pages) = self.pages.lock() else {
                return PageUrl::Unknown;
            };
            let mut list_now = false;
            if !self.pages_command.is_empty() {
                if let Some(path) = pages.listing.as_ref().and_then(|m| m.get(&key)) {
                    return found(path);
                }
                if pages.listing_running {
                    return PageUrl::Pending;
                }
                list_now = match pages.listed_at {
                    None => true,
                    Some(at) => refresh && at.elapsed() > RELIST_AFTER,
                };
            }
            if !list_now {
                if self.page_urls.is_empty() {
                    return PageUrl::Unknown;
                }
                match pages.probed.get(&key) {
                    Some(Some(path)) => return found(path),
                    Some(None) => return PageUrl::Unknown,
                    None => {}
                }
                if pages.probing.insert(key.clone()) {
                    self.probe(rel, key);
                }
                return PageUrl::Pending;
            }
            pages.listing_running = true;
        }
        self.start_listing();
        PageUrl::Pending
    }

    /// `file` の URL が変わったかもしれない (保存で front matter が変わった): 調べ直す。調べ直している間、
    /// [`Server::page_url`] は前の URL を返し、結果が出たら新しい URL を返す
    pub fn refresh_page(&self, file: &Path) {
        let Some(rel) = sitepages::relative(&self.root, file) else {
            return;
        };
        let list_now = {
            let Ok(mut pages) = self.pages.lock() else {
                return;
            };
            pages.probed.remove(&sitepages::key(&rel));
            let list_now = !self.pages_command.is_empty()
                && !pages.listing_running
                && pages.listed_at.is_some();
            if list_now {
                pages.listing_running = true;
            }
            list_now
        };
        if list_now {
            self.start_listing();
        }
    }

    /// 一覧のコマンドを実行し、終わったら別スレッドで結果を読む (`listing_running` は呼び出し側が立てておく)
    fn start_listing(&self) {
        let spawned =
            build_command(&self.root, &self.pages_command, &[]).and_then(|(mut cmd, program)| {
                cmd.stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|e| StartError::Other(format!("{program}: {e}")))
            });
        let child = match spawned {
            Ok(child) => child,
            Err(_) => {
                // 実行できない (Ruby が無いなど): 一覧は無いものとして、規則で探す
                if let Ok(mut pages) = self.pages.lock() {
                    pages.listing_running = false;
                    pages.listed_at = Some(Instant::now());
                }
                return;
            }
        };
        // サーバーを止めたとき、終わっていない一覧のコマンドも一緒に止まるように
        #[cfg(windows)]
        if let Some(job) = self.job.borrow().as_ref() {
            job.assign(&child);
        }
        let pages = self.pages.clone();
        std::thread::spawn(move || {
            let listing = child
                .wait_with_output()
                .ok()
                .map(|out| sitepages::parse_listing(&String::from_utf8_lossy(&out.stdout)))
                .filter(|map| !map.is_empty());
            if let Ok(mut pages) = pages.lock() {
                // 取れなかったとき (エラーで終わった) は、前の一覧を残す
                if listing.is_some() {
                    pages.listing = listing;
                }
                pages.listing_running = false;
                pages.listed_at = Some(Instant::now());
            }
        });
    }

    /// 規則の候補を開発サーバーに問い合わせ、応答のあった最初のものを覚える (別スレッド)
    fn probe(&self, rel: String, key: String) {
        let rules = self.page_urls.clone();
        let pages = self.pages.clone();
        let port = self.port;
        std::thread::spawn(move || {
            let hit = sitepages::candidates(&rules, &rel)
                .into_iter()
                .find(|path| {
                    // 2xx と 3xx (末尾の / を足すリダイレクトなど) を「ある」とみなす
                    matches!(
                        http_status(port, &sitepages::encode_path(path)),
                        Some(200..=399)
                    )
                });
            if let Ok(mut pages) = pages.lock() {
                pages.probing.remove(&key);
                pages.probed.insert(key, hit);
            }
        });
    }

    /// サーバーがページを出せるようになったか (HTTP の要求に応答した)。
    ///
    /// 「ポートに接続できる」では判定しない: Hugo は起動してすぐポートを開き、サイトを組み終えてから応答を始めるので、
    /// 接続できた時点で表示すると、組んでいる間は何も出ず、組むのに失敗して終了すると「接続できません」になる。
    /// 応答の確認は別スレッドがしているので、ここは待たない (画面のスレッドから何度呼んでもよい)
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }

    /// プロセスが終わっていればその説明 (終了コードと、出力の中のエラーの行)
    pub fn exit_error(&self) -> Option<String> {
        let mut child = self.child.borrow_mut();
        let status = child.as_mut()?.try_wait().ok().flatten()?;
        self.stopped.store(true, Ordering::Relaxed);
        let stderr = self.stderr.lock().map(|s| s.clone()).unwrap_or_default();
        let log = self.log.lock().map(|s| s.clone()).unwrap_or_default();
        let summary = error_summary(&stderr, &log);
        // 理由を先に (プレビューのバーは狭く、後ろは切れる)。`status` は "exit code: 1" の形
        Some(if summary.is_empty() {
            status.to_string()
        } else {
            format!("{summary} ({status})")
        })
    }

    /// 出力 (stdout と stderr) の末尾 `lines` 行 (コンソールペイン用)
    pub fn log_tail(&self, lines: usize) -> String {
        let all = self.log.lock().map(|s| s.clone()).unwrap_or_default();
        let v: Vec<&str> = all.lines().filter(|l| !l.trim().is_empty()).collect();
        let start = v.len().saturating_sub(lines);
        v[start..].join("\n")
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        // 先にジョブを閉じて子孫 (bundle.bat → ruby、npx → node) ごと止める
        #[cfg(windows)]
        drop(self.job.borrow_mut().take());
        if let Some(mut child) = self.child.borrow_mut().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

thread_local! {
    static SERVERS: RefCell<HashMap<PathBuf, Rc<Server>>> = RefCell::new(HashMap::new());
}

/// 落ちたサーバーを、次に頼まれたとき自動では起動し直さない時間 (起動してから)
const RESTART_COOLDOWN: Duration = Duration::from_secs(30);

/// プロジェクトのサーバーを返す (無ければ起動する)。サーバーはプロジェクトのルートごとに 1 つで、同じプロジェクトの
/// タブやプレビューウィンドウは同じものを使う。コマンドか URL が変わっていたら起動し直す。
///
/// 落ちているサーバーは起動し直すが、起動してすぐ (30 秒以内) に落ちたものは、落ちたまま返す (呼び出し側は
/// [`Server::exit_error`] で分かる): サイトを組めずに終了するプロジェクトのファイルをたくさん開いたときに、
/// タブの数だけ起動して失敗するのを繰り返さないように。`retry` (利用者がバーで「サイト」を選び直した) なら、すぐ起動し直す
pub fn server_for(project: &Project, retry: bool) -> Result<Rc<Server>, StartError> {
    let root = project.root.clone();
    let key = server_key(&project.generator);
    let existing = SERVERS.with(|s| s.borrow().get(&root).cloned());
    if let Some(server) = existing {
        if server.key == key {
            let dead = server.exit_error().is_some();
            if !dead || (!retry && server.started.elapsed() < RESTART_COOLDOWN) {
                return Ok(server);
            }
        }
        server.stop();
        SERVERS.with(|s| s.borrow_mut().remove(&root));
    }
    let server = Rc::new(start(&root, &project.generator)?);
    SERVERS.with(|s| s.borrow_mut().insert(root, server.clone()));
    Ok(server)
}

fn server_key(generator: &Generator) -> String {
    format!(
        "{}\n{}\n{}\n{}",
        generator.command,
        generator.url,
        generator.pages_command,
        generator.page_urls.join("\n")
    )
}

/// プレビューがサーバーを使い終えた (タブを閉じた): ほかに使っているプレビューが無ければ止める
pub fn release(server: Rc<Server>) {
    let root = server.root.clone();
    drop(server);
    SERVERS.with(|s| {
        let mut servers = s.borrow_mut();
        // 表にだけ残っている (参照が 1 つ) なら、表から外す。Server の drop でプロセスが止まる
        if servers
            .get(&root)
            .is_some_and(|server| Rc::strong_count(server) == 1)
        {
            servers.remove(&root);
        }
    });
}

/// すべてのサーバーを止める (アプリの終了時)
pub fn stop_all() {
    SERVERS.with(|s| s.borrow_mut().clear());
}

/// サーバーを起動できなかった理由
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartError {
    /// コマンドのプログラムが見つからない (PATH に無い)
    NotFound(String),
    Other(String),
}

fn start(root: &Path, generator: &Generator) -> Result<Server, StartError> {
    let port = free_port().ok_or_else(|| StartError::Other("no free port".to_string()))?;
    // {port2}: ライブリロードなど、サーバーがもう 1 つ使うポート (コマンドに書かれているときだけ取る)
    let port2 = if generator.command.contains("{port2}") {
        free_port().ok_or_else(|| StartError::Other("no free port".to_string()))?
    } else {
        0
    };
    let (mut cmd, program) = build_command(
        root,
        &generator.command,
        &[("{port2}", port2.to_string()), ("{port}", port.to_string())],
    )?;
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            StartError::NotFound(program.clone())
        } else {
            StartError::Other(format!("{program}: {e}"))
        }
    })?;
    #[cfg(windows)]
    let job = Job::new();
    #[cfg(windows)]
    if let Some(job) = &job {
        job.assign(&child);
    }
    let stderr = Arc::new(Mutex::new(String::new()));
    let log = Arc::new(Mutex::new(String::new()));
    if let Some(pipe) = child.stdout.take() {
        collect_output(pipe, None, log.clone());
    }
    if let Some(pipe) = child.stderr.take() {
        collect_output(pipe, Some(stderr.clone()), log.clone());
    }
    let ready = Arc::new(AtomicBool::new(false));
    let stopped = Arc::new(AtomicBool::new(false));
    watch_ready(port, ready.clone(), stopped.clone());
    Ok(Server {
        child: RefCell::new(Some(child)),
        #[cfg(windows)]
        job: RefCell::new(job),
        url: generator.url.replace("{port}", &port.to_string()),
        key: server_key(generator),
        started: Instant::now(),
        ready,
        stopped,
        stderr,
        log,
        root: root.to_path_buf(),
        port,
        pages_command: generator.pages_command.trim().to_string(),
        page_urls: Arc::new(generator.page_urls.clone()),
        pages: Arc::new(Mutex::new(Pages::default())),
    })
}

/// コマンド行から `Command` を作る: 引数に分け、`{root}` と `replace` の組を置き換え、プログラムを探し
/// ([`resolve_program`])、プロジェクトのフォルダで、コンソールの窓を出さずに実行するようにする。
/// 戻り値の文字列は、コマンド行に書かれたプログラム名 (エラーの文面用)
fn build_command(
    root: &Path,
    line: &str,
    replace: &[(&str, String)],
) -> Result<(Command, String), StartError> {
    let root_str = root.to_string_lossy().into_owned();
    let args: Vec<String> = split_command(line)
        .into_iter()
        .map(|arg| {
            replace
                .iter()
                .fold(arg, |arg, (from, to)| arg.replace(from, to))
                .replace("{root}", &root_str)
        })
        .collect();
    let (program, rest) = args
        .split_first()
        .ok_or_else(|| StartError::Other("empty command".to_string()))?;
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    let resolved = resolve_program(program, root, &path_var);
    let mut cmd = Command::new(resolved.as_deref().unwrap_or(Path::new(program)));
    cmd.args(rest).current_dir(root).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    Ok((cmd, program.clone()))
}

/// サーバーが HTTP の要求に応答するまで、別スレッドで試し続ける (応答したら `ready` を立てて終わる)。
/// 画面のスレッドで試すと、つながらない間は接続の待ち時間ぶん画面が止まる
fn watch_ready(port: u16, ready: Arc<AtomicBool>, stopped: Arc<AtomicBool>) {
    /// あきらめるまでの時間 (プロセスが残ったまま応答しないサーバーを、いつまでも試し続けない)
    const GIVE_UP: Duration = Duration::from_secs(600);
    std::thread::spawn(move || {
        let started = Instant::now();
        while !stopped.load(Ordering::Relaxed) && started.elapsed() < GIVE_UP {
            if responds(port) {
                ready.store(true, Ordering::Relaxed);
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    });
}

/// `GET /` に HTTP の応答が返るか (状態コードは問わない)
fn responds(port: u16) -> bool {
    http_status(port, "/").is_some()
}

/// 開発サーバーに `GET <path>` を送り、応答の状態コードを返す (応答が無ければ None)。
/// Node 系は localhost を IPv6 だけで待つことがあるので、IPv4 と IPv6 の両方を試す
fn http_status(port: u16, path: &str) -> Option<u16> {
    let v4 = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let v6 = std::net::SocketAddr::from((std::net::Ipv6Addr::LOCALHOST, port));
    [v4, v6].iter().find_map(|addr| {
        let mut stream = TcpStream::connect_timeout(addr, Duration::from_millis(300)).ok()?;
        // サイトを組んでいる間の Hugo は、接続は受けるが応答しない。読み取りを待ち切って次の回へ
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
        let request = format!("GET {path} HTTP/1.0\r\nHost: localhost:{port}\r\n\r\n");
        stream.write_all(request.as_bytes()).ok()?;
        // "HTTP/1.1 200"
        let mut head = [0u8; 12];
        stream.read_exact(&mut head).ok()?;
        let head = std::str::from_utf8(&head).ok()?;
        head.starts_with("HTTP/")
            .then(|| head[9..12].parse().ok())
            .flatten()
    })
}

/// 端末用の色などのエスケープ列 (`ESC [ ... 文字`) を取り除く。Jekyll は端末でなくてもエラーに色を付ける
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        if chars.peek() == Some(&'[') {
            chars.next();
            // 引数 (数字と ;) の後の、最後の 1 文字 (@ から ~) まで
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
        }
    }
    out
}

/// 終了したサーバーの出力から、理由を 1 行にまとめる: stderr (無ければ出力全体) の中の「error」を含む最初の行、
/// それも無ければ最後の行。長い行は切る (全文はコンソールペインに出る)
fn error_summary(stderr: &str, log: &str) -> String {
    const MAX_CHARS: usize = 240;
    let source = if stderr.trim().is_empty() {
        log
    } else {
        stderr
    };
    let lines: Vec<&str> = source
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let line = lines
        .iter()
        .find(|l| {
            let lower = l.to_lowercase();
            lower.contains("error") || lower.contains("fatal")
        })
        .or(lines.last())
        .copied()
        .unwrap_or_default();
    if line.chars().count() > MAX_CHARS {
        let cut: String = line.chars().take(MAX_CHARS).collect();
        format!("{cut}...")
    } else {
        line.to_string()
    }
}

/// パイプを読み続けて `log` (末尾を残す) と `head` (先頭を残す) に溜める。読まないとパイプが詰まってサーバーが止まる
fn collect_output(
    mut pipe: impl Read + Send + 'static,
    head: Option<Arc<Mutex<String>>>,
    log: Arc<Mutex<String>>,
) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 1024];
        while let Ok(n) = pipe.read(&mut buf) {
            if n == 0 {
                break;
            }
            let text = strip_ansi(&String::from_utf8_lossy(&buf[..n]));
            if let Some(head) = &head {
                if let Ok(mut s) = head.lock() {
                    if s.len() < 8192 {
                        s.push_str(&text);
                    }
                }
            }
            if let Ok(mut s) = log.lock() {
                s.push_str(&text);
                if s.len() > LOG_LIMIT {
                    // 半分より後ろの最初の行頭から残す
                    let mut cut = s.len() - LOG_LIMIT / 2;
                    while !s.is_char_boundary(cut) {
                        cut += 1;
                    }
                    let cut = s[cut..].find('\n').map(|i| cut + i + 1).unwrap_or(cut);
                    s.drain(..cut);
                }
            }
        }
    });
}

/// コマンドのプログラム名を、実行するファイルのパスにする。見つからなければ None (名前のまま `Command` に任せる)。
///
/// 名前だけのプログラムは、コマンドプロンプトと同じく、先にプロジェクトのフォルダ、次に PATH から探す。プロジェクトに
/// 置いた実行ファイル (サイトに合わせた版の `hugo.exe` など) が、PATH にある別の版より優先される。
///
/// Windows では PATHEXT の拡張子を補う: Rust の `Command` は名前に `.exe` しか補わないので、`bundle` (bundle.bat) や
/// `npx` (npx.cmd) をそのまま渡すと「見つからない」になる
#[cfg(windows)]
fn resolve_program(program: &str, cwd: &Path, path_var: &std::ffi::OsStr) -> Option<PathBuf> {
    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
    let exts: Vec<&str> = pathext.split(';').filter(|e| !e.is_empty()).collect();
    let given = Path::new(program);
    // 拡張子が書かれていれば、まずその名前のまま。無ければ PATHEXT の順 (拡張子の無い同名のファイルは実行できない)
    let candidates = |base: &Path| -> Option<PathBuf> {
        if given.extension().is_some() && base.is_file() {
            return Some(base.to_path_buf());
        }
        exts.iter()
            .map(|ext| {
                // PATHEXT は大文字 (.EXE) なので小文字にして足す (見つかるファイルは同じ。表示が hugo.EXE にならないように)
                let mut name = base.as_os_str().to_os_string();
                name.push(ext.to_ascii_lowercase());
                PathBuf::from(name)
            })
            .find(|p| p.is_file())
    };
    if given.components().count() > 1 || given.is_absolute() {
        // フォルダ付き: 作業フォルダからの相対か絶対パス
        return candidates(&cwd.join(given));
    }
    candidates(&cwd.join(given)).or_else(|| {
        std::env::split_paths(path_var)
            .filter(|dir| !dir.as_os_str().is_empty())
            .find_map(|dir| candidates(&dir.join(given)))
    })
}

/// Unix では PATH からの検索は OS (`execvp`) がするので、プロジェクトのフォルダにある実行ファイルだけを見る
#[cfg(not(windows))]
fn resolve_program(program: &str, cwd: &Path, _path_var: &std::ffi::OsStr) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let given = Path::new(program);
    if given.components().count() != 1 {
        return None;
    }
    let local = cwd.join(given);
    let executable = std::fs::metadata(&local)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false);
    executable.then_some(local)
}

/// 「閉じると中のプロセスが全部止まる」ジョブオブジェクト (Windows)。
///
/// 開発サーバーは `bundle.bat` → `ruby`、`npx.cmd` → `node` のように孫プロセスで動くことが多く、子を止めるだけでは
/// サーバーが残る。子をジョブに入れておけば孫もジョブに入り、ハンドルを閉じたときにまとめて止まる。Typball が
/// 異常終了したときも、OS がハンドルを閉じるので残らない
#[cfg(windows)]
struct Job(usize);

#[cfg(windows)]
impl Job {
    fn new() -> Option<Job> {
        use windows_sys::Win32::System::JobObjects::{
            CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        // SAFETY: Win32 のジョブオブジェクト API を正しい大きさの構造体で呼ぶ
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return None;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const std::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            Some(Job(job as usize))
        }
    }

    fn assign(&self, child: &Child) {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::HANDLE;
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
        // SAFETY: 有効なジョブとプロセスのハンドル
        unsafe {
            AssignProcessToJobObject(self.0 as HANDLE, child.as_raw_handle() as HANDLE);
        }
    }
}

#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;
        // SAFETY: 自分で作ったジョブのハンドル。閉じるのは 1 回だけ
        unsafe {
            TerminateJobObject(self.0 as HANDLE, 1);
            CloseHandle(self.0 as HANDLE);
        }
    }
}

/// 空いているポートを OS に選ばせる
fn free_port() -> Option<u16> {
    TcpListener::bind(("127.0.0.1", 0))
        .ok()?
        .local_addr()
        .ok()
        .map(|a| a.port())
}

/// コマンド行を引数に分ける (二重引用符でまとめられる。`""` は空の引数)
pub fn split_command(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut has_token = false;
    for c in s.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                has_token = true;
            }
            c if c.is_whitespace() && !quoted => {
                if has_token {
                    out.push(std::mem::take(&mut cur));
                    has_token = false;
                }
            }
            c => {
                cur.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 試験用のフォルダ (drop で消す)
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            let dir =
                std::env::temp_dir().join(format!("typball-site-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn file(&self, rel: &str, text: &str) -> PathBuf {
            let path = self.0.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, text).unwrap();
            path
        }

        fn dir(&self, rel: &str) {
            std::fs::create_dir_all(self.0.join(rel)).unwrap();
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn defaults() -> Vec<Generator> {
        generators(&Preview::default())
    }

    fn detected(path: &Path) -> Option<(String, PathBuf)> {
        detect(path, &defaults()).map(|p| (p.generator.id, p.root))
    }

    #[test]
    fn split_command_handles_quotes() {
        assert_eq!(
            split_command("hugo server -s \"C:\\My Site\" --port {port}"),
            vec!["hugo", "server", "-s", "C:\\My Site", "--port", "{port}"]
        );
        assert_eq!(split_command("  "), Vec::<String>::new());
        // "" は空の引数 (jekyll serve --baseurl "")
        assert_eq!(
            split_command("a --baseurl \"\""),
            vec!["a", "--baseurl", ""]
        );
    }

    #[test]
    fn detect_hugo_project() {
        let t = TempDir::new("hugo");
        t.file("hugo.toml", "baseURL = 'https://example.org/'\n");
        let md = t.file("content/posts/hello.md", "# hi\n");
        let project = detect(&md, &defaults()).unwrap();
        assert_eq!(project.generator.id, "hugo");
        assert_eq!(project.root, t.0);
        assert!(project.static_dirs()[0].ends_with("static"));
        assert!(detect(Path::new("C:\\nonexistent\\x.md"), &defaults()).is_none());

        // config/_default/ に置く形
        let t = TempDir::new("hugo-default");
        t.file("config/_default/hugo.yaml", "title: x\n");
        let md = t.file("content/a.md", "");
        assert_eq!(detected(&md), Some(("hugo".to_string(), t.0.clone())));
    }

    #[test]
    fn config_toml_is_hugo_or_zola_by_its_contents() {
        let hugo = TempDir::new("legacy-hugo");
        hugo.file("config.toml", "baseURL = \"https://example.org/\"\n");
        let md = hugo.file("content/a.md", "");
        assert_eq!(detected(&md), Some(("hugo".to_string(), hugo.0.clone())));

        let zola = TempDir::new("zola");
        zola.file("config.toml", "base_url = \"https://example.org\"\n");
        zola.dir("templates");
        let md = zola.file("content/blog/a.md", "");
        assert_eq!(detected(&md), Some(("zola".to_string(), zola.0.clone())));

        // 設定ファイルだけで、サイトらしいフォルダが無いものは拾わない
        let plain = TempDir::new("plain-config");
        plain.file("config.toml", "name = \"app\"\n");
        let md = plain.file("notes/a.md", "");
        assert_eq!(detected(&md), None);
    }

    #[test]
    fn detect_jekyll_with_and_without_bundler_and_hexo() {
        let t = TempDir::new("jekyll");
        t.file("_config.yml", "title: x\n");
        let md = t.file("_posts/2026-11-14-hello.md", "");
        assert_eq!(detected(&md), Some(("jekyll".to_string(), t.0.clone())));
        // ルート相対の画像はプロジェクトのフォルダから探す
        let project = detect(&md, &defaults()).unwrap();
        assert_eq!(project.static_dirs(), vec![t.0.join(".")]);

        t.file("Gemfile", "gem \"jekyll\"\n");
        assert_eq!(
            detected(&md),
            Some(("jekyll-bundler".to_string(), t.0.clone()))
        );

        let hexo = TempDir::new("hexo");
        hexo.file("_config.yml", "title: x\n");
        hexo.file("package.json", "{\"dependencies\":{\"hexo\":\"^7.0.0\"}}\n");
        let md = hexo.file("source/_posts/a.md", "");
        assert_eq!(detected(&md), Some(("hexo".to_string(), hexo.0.clone())));
    }

    #[test]
    fn the_nearest_project_wins_in_nested_sites() {
        // Hugo のサイトの中に Jekyll のサイト (docs/) がある
        let t = TempDir::new("nested");
        t.file("hugo.toml", "");
        t.file("docs/_config.yml", "");
        let inner = t.file("docs/guide/deep/a.md", "");
        let outer = t.file("content/a.md", "");
        assert_eq!(
            detected(&inner),
            Some(("jekyll".to_string(), t.0.join("docs")))
        );
        assert_eq!(detected(&outer), Some(("hugo".to_string(), t.0.clone())));
    }

    #[test]
    fn detect_by_wildcard_names() {
        let t = TempDir::new("astro");
        t.file("astro.config.mjs", "export default {}\n");
        let md = t.file("src/content/blog/a.md", "");
        assert_eq!(detected(&md), Some(("astro".to_string(), t.0.clone())));

        let v = TempDir::new("vitepress");
        v.file("docs/.vitepress/config.ts", "");
        let md = v.file("docs/guide/a.md", "");
        assert_eq!(
            detected(&md),
            Some(("vitepress".to_string(), v.0.join("docs")))
        );

        assert!(wildcard_matches("astro.config.*", "astro.config.TS"));
        assert!(wildcard_matches("*.config.*", "a.config.b"));
        assert!(wildcard_matches("a*b*c", "aXbYc"));
        assert!(!wildcard_matches("a*b*c", "aXcYb"));
        assert!(!wildcard_matches("astro.config.*", "astro.config"));
        assert!(wildcard_matches("book.toml", "Book.toml"));
    }

    #[test]
    fn rules_combine_names() {
        let t = TempDir::new("rules");
        t.file("a.txt", "hello world\n");
        t.dir("sub");
        let d = &t.0;
        assert!(rule_matches(d, "a.txt"));
        assert!(rule_matches(d, "nope|a.txt"));
        assert!(rule_matches(d, "a.txt + sub"));
        assert!(!rule_matches(d, "a.txt + nope"));
        assert!(rule_matches(d, "a.txt + !nope"));
        assert!(!rule_matches(d, "a.txt + !sub"));
        assert!(rule_matches(d, "a.txt:world"));
        assert!(!rule_matches(d, "a.txt:mars"));
        assert!(rule_matches(d, "a.txt + !a.txt:mars"));
        // フォルダは「中に文字列がある」に当てはまらない
        assert!(!rule_matches(d, "sub:x"));
        // 否定だけ・空・フォルダの外は当てはまらない
        assert!(!rule_matches(d, "!nope"));
        assert!(!rule_matches(d, ""));
        assert!(!rule_matches(&d.join("sub"), "../a.txt"));
    }

    #[test]
    fn settings_override_and_add_generators() {
        let preview: Preview = toml::from_str(concat!(
            "[generators.hugo]\ncommand = \"hugo server --port {port}\"\n",
            "[generators.jekyll]\nenabled = false\n",
            "[generators.mysite]\nname = \"My SSG\"\ndetect = [\"mysite.json\"]\n",
            "command = \"mysite serve {port}\"\n",
        ))
        .unwrap();
        let list = generators(&preview);
        // 設定で足したものが先
        assert_eq!(list[0].id, "mysite");
        assert_eq!(list[0].name, "My SSG");
        assert_eq!(list[0].url, DEFAULT_URL);
        assert!(!list[0].builtin);
        let hugo = list.iter().find(|g| g.id == "hugo").unwrap();
        assert_eq!(hugo.command, "hugo server --port {port}");
        // 書いていない項目は内蔵のまま
        assert_eq!(hugo.url, builtin("hugo").unwrap().url);
        assert_eq!(hugo.detect, builtin("hugo").unwrap().detect);
        assert!(!list.iter().find(|g| g.id == "jekyll").unwrap().enabled);

        // 設定ファイルに書く形に戻すと、変えた項目だけになる
        assert_eq!(to_overrides(&list), preview.generators);
        assert!(to_overrides(&defaults()).is_empty());

        // 切ったジェネレーターのプロジェクトは見つからない (素の Markdown になる)
        let t = TempDir::new("disabled");
        t.file("_config.yml", "");
        let md = t.file("a.md", "");
        assert!(detect(&md, &list).is_none());
        // 自作の定義で見つかる
        t.file("mysite.json", "{}");
        assert_eq!(detect(&md, &list).unwrap().generator.id, "mysite");
    }

    #[test]
    fn builtin_ids_are_unique_and_complete() {
        for (i, b) in BUILTINS.iter().enumerate() {
            assert!(BUILTINS[i + 1..].iter().all(|o| o.id != b.id), "{}", b.id);
            assert!(!b.detect.is_empty() && b.command.contains("{port}"));
            assert!(b.url.contains("{port}"));
        }
    }

    #[test]
    fn exit_reason_is_the_error_line() {
        let stderr = "WARN  deprecated: languageCode\nERROR error building site: duplicate key\n   4 | type: post\n";
        assert_eq!(
            error_summary(stderr, "ignored"),
            "ERROR error building site: duplicate key"
        );
        // エラーらしい行が無ければ最後の行。stderr が空なら出力全体から
        assert_eq!(error_summary("a\nb\n\n", ""), "b");
        assert_eq!(error_summary("  ", "x\nFatal: boom\ny"), "Fatal: boom");
        assert_eq!(error_summary("", ""), "");
        // 色のエスケープ列は取り除いて溜める
        assert_eq!(
            strip_ansi("\u{1b}[31m  Dependency Error:\u{1b}[0m Yikes!"),
            "  Dependency Error: Yikes!"
        );
        let long = format!("error {}", "x".repeat(400));
        assert_eq!(error_summary(&long, "").chars().count(), 243);
    }

    #[test]
    fn ready_means_an_http_response_not_an_open_port() {
        // 接続は受けるが応答しないサーバー (サイトを組んでいる間の Hugo)
        let silent = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = silent.local_addr().unwrap().port();
        assert!(TcpStream::connect(("127.0.0.1", port)).is_ok());
        assert!(!responds(port));
        drop(silent);
        assert!(!responds(port));

        // 応答するサーバー (状態コードは問わない)
        let server = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = server.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = server.accept().unwrap();
            let mut buf = [0u8; 256];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(b"HTTP/1.0 404 Not Found\r\n\r\n");
        });
        assert!(responds(port));
        handle.join().unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn programs_are_found_with_pathext() {
        let t = TempDir::new("pathext");
        // 拡張子の無い同名のファイル (Ruby の bin にある jekyll と jekyll.bat) は選ばない
        t.file("bin/tool", "#!ruby\n");
        let bat = t.file("bin/tool.bat", "@echo off\n");
        let path_var = std::env::join_paths([t.0.join("empty"), t.0.join("bin")]).unwrap();
        let found = resolve_program("tool", &t.0, &path_var).unwrap();
        let same = |a: &Path, b: &Path| {
            let norm = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
            norm(a) == norm(b)
        };
        assert!(same(&found, &bat), "{}", found.display());
        assert_eq!(resolve_program("tool.bat", &t.0, &path_var), Some(bat));
        assert_eq!(resolve_program("missing", &t.0, &path_var), None);
        // プロジェクトのフォルダに置いた実行ファイルは PATH より先 (サイトに合わせた版の hugo.exe)
        let local = t.file("site/tool.exe", "");
        let found = resolve_program("tool", &t.0.join("site"), &path_var).unwrap();
        assert!(same(&found, &local), "{}", found.display());
        // フォルダ付きは作業フォルダから
        assert!(resolve_program("bin/tool", &t.0, &path_var).is_some());
    }
}
