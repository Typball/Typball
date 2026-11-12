//! Typball のコア: Typst ソース文字列をコンパイルし、ページを RGBA ビットマップ (または PDF) として返す。
//!
//! 本文はエディタの内容をそのまま受け取る。本文が参照するファイル (`#image("fig.png")` や
//! `#import "lib.typ"`) は、文書を保存してあればそのフォルダをプロジェクトのルートとしてディスクから読む
//! (Typst の CLI で `--root` を付けないときと同じ。ルートの外は読めない)

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::SystemTime;

use parking_lot::Mutex;
use typst::diag::{FileError, FileResult, PackageError, SourceDiagnostic};
use typst::foundations::{Bytes, Datetime, Duration};
use typst::layout::Abs;
use typst::syntax::{DiagSpan, FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{
    Font, FontBook, FontFamily as TypstFontFamily, FontFlags, FontInfo, FontList, TextElem,
    TextSize,
};
use typst::utils::LazyHash;
use typst::{Library, World};
use typst::{LibraryExt, WorldExt};
use typst_kit::fonts::FontStore;
use typst_layout::PagedDocument;

/// 1ページ分のラスタ画像 (RGBA8, premultiplied ではない)
#[derive(Clone, Debug)]
pub struct PageImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// コンパイル結果
#[derive(Clone, Debug, Default)]
pub struct CompileOutput {
    pub pages: Vec<PageImage>,
    /// エラー・警告メッセージ (行番号付き)
    pub diagnostics: Vec<String>,
    pub ok: bool,
}

/// 初期サンプル文書
pub const SAMPLE_SOURCE: &str = r#"#set page(width: 120mm, height: auto, margin: 8mm)
#set text(font: ("Yu Gothic UI", "Meiryo", "Hiragino Sans", "Noto Sans JP"), lang: "ja", size: 11pt)

= Typball へようこそ

左のエディタを編集すると、右のプレビューが即座に更新されます。
日本語の入力（変換中の文字列）もそのまま扱えます。

- 箇条書き *太字* _斜体_
- 数式: $ sum_(i=1)^n i = (n(n+1)) / 2 $
- 半角と全角の混在: Typst 0.15 と 日本語

#table(
  columns: 2,
  [項目], [値],
  [エディタ], [wxStyledTextCtrl],
  [組版], [Typst 0.15],
)
"#;

/// システムにあるフォントファミリー 1 つ分 (設定画面の一覧用)
#[derive(Clone, Debug)]
pub struct FontFamily {
    pub name: String,
    /// 等幅の書体を 1 つでも含むか
    pub monospace: bool,
}

struct Engine {
    /// 標準ライブラリ。既定のテキストフォント / サイズ (`set_default_text`) を変えると作り直す
    library: Mutex<Arc<LazyHash<Library>>>,
    /// 今のライブラリに入れた既定 (フォント名, pt)。変わっていなければ作り直さない
    text_default: Mutex<(String, f64)>,
    fonts: FontStore,
    /// システムフォントのファミリー名 (名前順)
    system_families: Vec<FontFamily>,
    /// 前回コンパイルした本文。同じ文書 (同じ FileId) なら書き換えて使い回す (差分で解析し直せる)
    main: Mutex<Source>,
    /// ディスクから読んだファイル (画像・取り込んだ .typ など)
    files: Mutex<FileCache>,
}

/// 1 回のコンパイルに使う World。Engine のフォントとファイルを借り、ライブラリはそのときの既定を
/// 反映したものを握る (World::library が参照を返すので、Engine の Mutex の中身を直接は返せない)
struct Session<'a> {
    eng: &'a Engine,
    library: Arc<LazyHash<Library>>,
    /// 本文 (エディタの内容)
    main: Source,
    /// プロジェクトのルート (文書のあるフォルダ)。未保存の文書には無い
    root: Option<PathBuf>,
}

impl Engine {
    /// `source` を本文にした World を作る。`path` は文書の保存先 (未保存なら None)
    fn session(&self, source: &str, path: Option<&Path>) -> Session<'_> {
        let (root, id) = locate(path);
        let main = {
            let mut main = self.main.lock();
            if main.id() == id {
                main.replace(source);
            } else {
                *main = Source::new(id, source.to_string());
            }
            main.clone()
        };
        self.resume(main, root)
    }

    /// コンパイル済みの本文とルートから World を作り直す (書き出しのときの診断用)
    fn resume(&self, main: Source, root: Option<PathBuf>) -> Session<'_> {
        self.files.lock().begin();
        Session {
            eng: self,
            library: self.library.lock().clone(),
            main,
            root,
        }
    }
}

/// 未保存の文書の本文の名前
const UNTITLED_MAIN: &str = "/main.typ";

/// 文書の保存先から、プロジェクトのルート (そのフォルダ) と本文の FileId を決める。
/// 本文はルート直下の実際のファイル名にする (`#import "main.typ"` が別のファイルを指せるように)
fn locate(path: Option<&Path>) -> (Option<PathBuf>, FileId) {
    let untitled = || VirtualPath::new(UNTITLED_MAIN).expect("valid path");
    let path = path.and_then(|p| std::path::absolute(p).ok());
    let root = path
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf);
    let vpath = match (&root, &path) {
        (Some(root), Some(path)) => {
            VirtualPath::virtualize(root, path).unwrap_or_else(|_| untitled())
        }
        _ => untitled(),
    };
    (root, RootedPath::new(VirtualRoot::Project, vpath).intern())
}

/// ディスクから読んだファイル 1 つ
struct CachedFile {
    /// 読んだときの更新日時と大きさ。変わっていたら読み直す
    stamp: (Option<SystemTime>, u64),
    bytes: Bytes,
    /// Typst のソースとして使ったときの解析結果
    source: Option<Source>,
    /// 最後に使ったコンパイルの番号
    used: u64,
}

/// ディスクから読んだファイルの控え。キー入力のたびにコンパイルし直すので、画像などを毎回読み直さない。
/// 同じ `Bytes` を返せば Typst 側の画像のデコード結果も使い回される
#[derive(Default)]
struct FileCache {
    entries: HashMap<PathBuf, CachedFile>,
    /// 今のコンパイルの番号
    generation: u64,
}

/// 控えの合計がこれを超えたら、今のコンパイルで使っていないものを捨てる
const FILE_CACHE_LIMIT: u64 = 512 * 1024 * 1024;

/// UTF-8 の BOM。ソースとして読むときは外す
const UTF8_BOM: &[u8] = &[0xef, 0xbb, 0xbf];

impl FileCache {
    /// コンパイルを 1 回始める
    fn begin(&mut self) {
        self.generation += 1;
    }

    /// `path` の中身。更新日時か大きさが変わっていたら読み直す
    fn load(&mut self, path: &Path) -> FileResult<&mut CachedFile> {
        let io = |e| FileError::from_io(e, path);
        let meta = std::fs::metadata(path).map_err(io)?;
        if meta.is_dir() {
            return Err(FileError::IsDirectory);
        }
        let stamp = (meta.modified().ok(), meta.len());
        let fresh = self.entries.get(path).is_some_and(|f| f.stamp == stamp);
        if !fresh {
            let bytes = Bytes::new(std::fs::read(path).map_err(io)?);
            // 前のソースは残す (中身を書き換えて差分で解析し直す)
            let source = self.entries.remove(path).and_then(|f| f.source);
            self.evict();
            self.entries.insert(
                path.to_path_buf(),
                CachedFile {
                    stamp,
                    bytes,
                    source,
                    used: 0,
                },
            );
        }
        let generation = self.generation;
        let file = self.entries.get_mut(path).expect("just inserted");
        file.used = generation;
        Ok(file)
    }

    fn evict(&mut self) {
        let total: u64 = self.entries.values().map(|f| f.stamp.1).sum();
        if total > FILE_CACHE_LIMIT {
            let generation = self.generation;
            self.entries.retain(|_, f| f.used == generation);
        }
    }
}

impl CachedFile {
    /// Typst のソースとして読む。`id` と中身が前と同じなら解析結果をそのまま返す
    fn source(&mut self, id: FileId) -> FileResult<Source> {
        let bytes = self.bytes.as_slice();
        let text = std::str::from_utf8(bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes))?;
        let source = match self.source.take() {
            Some(source) if source.id() == id && source.text() == text => source,
            Some(mut source) if source.id() == id => {
                source.replace(text);
                source
            }
            _ => Source::new(id, text.to_string()),
        };
        self.source = Some(source.clone());
        Ok(source)
    }
}

impl Session<'_> {
    /// `id` のディスク上の場所
    fn real_path(&self, id: FileId) -> FileResult<PathBuf> {
        match id.root() {
            VirtualRoot::Project => match &self.root {
                Some(root) => Ok(id.vpath().realize(root)?),
                // 未保存の文書には基準のフォルダが無い
                None => Err(FileError::Other(Some(
                    "the document has not been saved, so there is no folder to load files from"
                        .into(),
                ))),
            },
            VirtualRoot::Package(_) => Err(FileError::Package(PackageError::Other(Some(
                "packages are not supported".into(),
            )))),
        }
    }
}

/// 同梱フォントのファミリー名 (プレビューのフォント設定の既定)
pub const EMBEDDED_FONT_FAMILY: &str = "Noto Sans JP";

/// 文書が `#set text(font: ...)` で指定しなかったときに使う既定のフォントとサイズを入れた標準ライブラリ。
/// 空のフォント名 / 0 以下のサイズは Typst の既定 (Libertinus Serif / 11pt) のまま
fn build_library(font: &str, size_pt: f64) -> Library {
    let mut lib = Library::default();
    if !font.trim().is_empty() {
        lib.styles.set(
            TextElem::font,
            FontList(vec![TypstFontFamily::new(font.trim())]),
        );
    }
    if size_pt > 0.0 {
        lib.styles
            .set(TextElem::size, TextSize(Abs::pt(size_pt).into()));
    }
    lib
}

/// プレビュー (と PDF 書き出し) の既定のテキストフォントとサイズを設定する。文書側の `#set text` が優先する。
/// 設定が変わったときだけライブラリを作り直す
pub fn set_default_text(font: &str, size_pt: f64) {
    let eng = engine();
    let mut current = eng.text_default.lock();
    if current.0 == font && current.1 == size_pt {
        return;
    }
    *current = (font.to_string(), size_pt);
    *eng.library.lock() = Arc::new(LazyHash::new(build_library(font, size_pt)));
}

impl World for Session<'_> {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }
    fn book(&self) -> &LazyHash<FontBook> {
        self.eng.fonts.book()
    }
    fn main(&self) -> FileId {
        self.main.id()
    }
    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main.id() {
            return Ok(self.main.clone());
        }
        let path = self.real_path(id)?;
        self.eng.files.lock().load(&path)?.source(id)
    }
    fn file(&self, id: FileId) -> FileResult<Bytes> {
        // 本文そのものを読むとき (`read("自分.typ")`) も、ディスクではなくエディタの内容を返す
        if id == self.main.id() {
            return Ok(Bytes::from_string(self.main.text().to_string()));
        }
        let path = self.real_path(id)?;
        Ok(self.eng.files.lock().load(&path)?.bytes.clone())
    }
    fn font(&self, index: usize) -> Option<Font> {
        self.eng.fonts.font(index)
    }
    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        None
    }
}

static ENGINE: OnceLock<Engine> = OnceLock::new();

/// 実行ファイルに埋め込む CJK フォント (Noto Sans JP, SIL OFL 1.1)。
/// Typst 同梱のフォントに CJK が無いので、どの OS でも日本語の字形が同じになるよう
/// フォールバック用に持つ。可変フォントは Typst が既定インスタンスしか使えないので
/// Regular と Bold を別ファイルで持つ
const EMBEDDED_CJK_FONTS: &[&[u8]] = &[
    include_bytes!("../fonts/NotoSansJP-Regular.otf"),
    include_bytes!("../fonts/NotoSansJP-Bold.otf"),
];

/// 同梱フォントの生データ。GUI 側がエディタ用に OS へプロセス専用フォントとして登録するのに使う
pub fn embedded_font_data() -> &'static [&'static [u8]] {
    EMBEDDED_CJK_FONTS
}

fn embedded_cjk() -> impl Iterator<Item = (Font, FontInfo)> {
    EMBEDDED_CJK_FONTS.iter().flat_map(|data| {
        Font::iter(Bytes::new(*data)).map(|font| {
            let info = font.info().clone();
            (font, info)
        })
    })
}

fn engine() -> &'static Engine {
    ENGINE.get_or_init(|| {
        let mut fonts = FontStore::new();
        fonts.extend(typst_kit::fonts::embedded());
        fonts.extend(embedded_cjk());
        let system: Vec<_> = typst_kit::fonts::system().collect();
        let mut families: BTreeMap<String, bool> = BTreeMap::new();
        for (_, info) in &system {
            let mono = families.entry(info.family.clone()).or_insert(false);
            *mono |= info.flags.contains(FontFlags::MONOSPACE);
        }
        let system_families = families
            .into_iter()
            .map(|(name, monospace)| FontFamily { name, monospace })
            .collect();
        fonts.extend(system);
        let (_, main_id) = locate(None);
        Engine {
            library: Mutex::new(Arc::new(LazyHash::new(build_library("", 0.0)))),
            text_default: Mutex::new((String::new(), 0.0)),
            fonts,
            system_families,
            main: Mutex::new(Source::new(main_id, String::new())),
            files: Mutex::new(FileCache::default()),
        }
    })
}

/// フォント検索などの初期化を先に済ませる (任意)。
pub fn warm_up() {
    let _ = engine();
}

/// システムにあるフォントファミリー (名前順)。最初の呼び出しでは検索が終わるまで待つ
pub fn system_font_families() -> &'static [FontFamily] {
    &engine().system_families
}

/// コンパイルだけ行う (ラスタライズはしない)。失敗したら文書は None。
/// `path` は文書の保存先 (未保存なら None)。そのフォルダを基準に画像などを読む
fn compile_document(source: &str, path: Option<&Path>) -> (Option<Document>, Vec<String>) {
    // Typst は組版の途中結果や、表示用に拡大縮小した画像を覚えておく (comemo)。捨てないと増え続けるので、
    // コンパイルのたびに、ここ 10 回使われなかったものを捨てる (Typst の CLI の監視モードと同じ回数)
    comemo::evict(10);
    let world = engine().session(source, path);
    let warned = typst::compile::<PagedDocument>(&world);
    let mut diagnostics: Vec<String> = warned
        .warnings
        .iter()
        .map(|d| format_diag(&world, d, "warning"))
        .collect();
    match warned.output {
        Ok(doc) => {
            let doc = Document {
                doc,
                main: world.main,
                root: world.root,
            };
            (Some(doc), diagnostics)
        }
        Err(errors) => {
            diagnostics.extend(errors.iter().map(|d| format_diag(&world, d, "error")));
            (None, diagnostics)
        }
    }
}

/// 各ページを `pixels_per_pt` の解像度でラスタライズする
fn rasterize(doc: &PagedDocument, pixels_per_pt: f32) -> Vec<PageImage> {
    doc.pages()
        .iter()
        .map(|page| rasterize_page(page, pixels_per_pt))
        .collect()
}

fn rasterize_page(page: &typst_layout::Page, pixels_per_pt: f32) -> PageImage {
    let opts = typst_render::RenderOptions {
        pixel_per_pt: typst::utils::Scalar::new(pixels_per_pt as f64),
        ..Default::default()
    };
    let pixmap = typst_render::render(page, &opts);
    let mut rgba = pixmap.data().to_vec();
    // premultiplied -> straight alpha
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a != 0 && a != 255 {
            for c in &mut px[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    PageImage {
        width: pixmap.width(),
        height: pixmap.height(),
        rgba,
    }
}

/// ソースをコンパイルして各ページを `pixels_per_pt` の解像度でラスタライズする。
pub fn compile(source: &str, path: Option<&Path>, pixels_per_pt: f32) -> CompileOutput {
    let (doc, diagnostics) = compile_document(source, path);
    match doc {
        Some(doc) => CompileOutput {
            pages: doc.rasterize(pixels_per_pt),
            diagnostics,
            ok: true,
        },
        None => CompileOutput {
            pages: Vec::new(),
            diagnostics,
            ok: false,
        },
    }
}

/// コンパイルして診断メッセージだけ返す (ラスタライズしない。プレビュー無しでコンソールだけ出すとき用)。
/// 戻り値の bool はエラー無しで文書ができたか
pub fn check(source: &str, path: Option<&Path>) -> (bool, Vec<String>) {
    let (doc, diagnostics) = compile_document(source, path);
    (doc.is_some(), diagnostics)
}

/// PDF にエクスポートする。
pub fn export_pdf(source: &str, path: Option<&Path>) -> Result<Vec<u8>, String> {
    let world = engine().session(source, path);
    let doc = typst::compile::<PagedDocument>(&world)
        .output
        .map_err(|errs| {
            errs.iter()
                .map(|d| format_diag(&world, d, "error"))
                .collect::<Vec<_>>()
                .join("\n")
        })?;
    typst_pdf::pdf(&doc, &typst_pdf::PdfOptions::default()).map_err(|errs| {
        errs.iter()
            .map(|d| format_diag(&world, d, "error"))
            .collect::<Vec<_>>()
            .join("\n")
    })
}

// ---------------------------------------------------------------------------
// コンパイル済みの文書 (プレビューの拡大縮小・ページ送り、書き出し)
// ---------------------------------------------------------------------------

/// コンパイル済みの文書。拡大率を変えたときはコンパイルし直さずにラスタライズだけやり直す
pub struct Document {
    doc: PagedDocument,
    /// コンパイルした本文とルート (書き出しで出た診断の位置を求めるのに使う)
    main: Source,
    root: Option<PathBuf>,
}

/// PDF の書き出し設定
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PdfSettings {
    /// PDF のバージョン ("1.4" / "1.5" / "1.6" / "1.7" / "2.0")。空なら Typst の既定 (1.7)
    pub version: String,
    /// PDF/A ("a-1b" / "a-2u" / "a-4f" など)。空なら無し
    pub pdfa: String,
    /// PDF/UA-1 に準拠させる
    pub pdfua: bool,
    /// タグ付き PDF (アクセシビリティ用の構造) を書く
    pub tagged: bool,
}

impl Default for PdfSettings {
    fn default() -> Self {
        Self {
            version: "1.7".to_string(),
            pdfa: String::new(),
            pdfua: false,
            tagged: true,
        }
    }
}

/// 選べる PDF のバージョン
pub const PDF_VERSIONS: &[&str] = &["1.4", "1.5", "1.6", "1.7", "2.0"];
/// 選べる PDF/A (空 = 無し)
pub const PDF_A_STANDARDS: &[&str] = &[
    "", "a-1b", "a-1a", "a-2b", "a-2u", "a-2a", "a-3b", "a-3u", "a-3a", "a-4", "a-4f", "a-4e",
];

/// PDF/A が求める PDF のバージョン (PDF/A を選んだときにバージョンを合わせるため)
pub fn pdfa_version(pdfa: &str) -> Option<&'static str> {
    match pdfa.get(..3)? {
        "a-1" => Some("1.4"),
        "a-2" | "a-3" => Some("1.7"),
        "a-4" => Some("2.0"),
        _ => None,
    }
}

fn pdf_standard(name: &str) -> Option<typst_pdf::PdfStandard> {
    use typst_pdf::PdfStandard as S;
    Some(match name {
        "1.4" => S::V_1_4,
        "1.5" => S::V_1_5,
        "1.6" => S::V_1_6,
        "1.7" => S::V_1_7,
        "2.0" => S::V_2_0,
        "a-1b" => S::A_1b,
        "a-1a" => S::A_1a,
        "a-2b" => S::A_2b,
        "a-2u" => S::A_2u,
        "a-2a" => S::A_2a,
        "a-3b" => S::A_3b,
        "a-3u" => S::A_3u,
        "a-3a" => S::A_3a,
        "a-4" => S::A_4,
        "a-4f" => S::A_4f,
        "a-4e" => S::A_4e,
        "ua-1" => S::Ua_1,
        _ => return None,
    })
}

/// ソースをコンパイルする。失敗したら文書は None (診断メッセージは両方で返る)。
/// `path` は文書の保存先 (未保存なら None)。そのフォルダを基準に画像などを読む
pub fn compile_doc(source: &str, path: Option<&Path>) -> (Option<Document>, Vec<String>) {
    compile_document(source, path)
}

/// 1 pt あたりのピクセル数 (dpi / 72)
pub fn ppp_for_dpi(dpi: f32) -> f32 {
    dpi / 72.0
}

impl Document {
    pub fn page_count(&self) -> usize {
        self.doc.pages().len()
    }

    /// 各ページの大きさ (pt、塗り足しを含む)
    pub fn page_sizes_pt(&self) -> Vec<(f64, f64)> {
        self.doc
            .pages()
            .iter()
            .map(|p| {
                (
                    (p.frame.width() + p.bleed.left + p.bleed.right).to_pt(),
                    (p.frame.height() + p.bleed.top + p.bleed.bottom).to_pt(),
                )
            })
            .collect()
    }

    /// 各ページを `pixels_per_pt` の解像度でラスタライズする
    pub fn rasterize(&self, pixels_per_pt: f32) -> Vec<PageImage> {
        rasterize(&self.doc, pixels_per_pt)
    }

    /// 1 ページだけラスタライズする (`index` は 0 始まり)
    pub fn rasterize_page(&self, index: usize, pixels_per_pt: f32) -> Option<PageImage> {
        self.doc
            .pages()
            .get(index)
            .map(|page| rasterize_page(page, pixels_per_pt))
    }

    /// PDF にする
    pub fn pdf(&self, settings: &PdfSettings) -> Result<Vec<u8>, String> {
        let mut list = Vec::new();
        for name in [settings.version.as_str(), settings.pdfa.as_str()] {
            if !name.is_empty() {
                list.push(
                    pdf_standard(name).ok_or_else(|| format!("unknown PDF standard: {name}"))?,
                );
            }
        }
        if settings.pdfua {
            list.extend(pdf_standard("ua-1"));
        }
        let standards = typst_pdf::PdfStandards::new(&list).map_err(|e| {
            let mut msg = e.message().to_string();
            for hint in e.hints() {
                msg.push_str(&format!("\nhint: {hint}"));
            }
            msg
        })?;
        let options = typst_pdf::PdfOptions {
            standards,
            tagged: settings.tagged || settings.pdfua,
            ..Default::default()
        };
        let world = engine().resume(self.main.clone(), self.root.clone());
        typst_pdf::pdf(&self.doc, &options).map_err(|errs| {
            errs.iter()
                .map(|d| format_diag(&world, d, "error"))
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    /// 各ページの PNG (`dpi` の解像度)
    pub fn png_pages(&self, dpi: f32) -> Result<Vec<Vec<u8>>, String> {
        let opts = render_options(dpi);
        self.doc
            .pages()
            .iter()
            .map(|page| {
                typst_render::render(page, &opts)
                    .encode_png()
                    .map_err(|e| e.to_string())
            })
            .collect()
    }

    /// 全ページを縦に並べた 1 枚の PNG (ページ間に `gap_pt` のすき間、すき間は白)
    pub fn png_merged(&self, dpi: f32, gap_pt: f64) -> Result<Vec<u8>, String> {
        typst_render::render_merged(
            &self.doc,
            &render_options(dpi),
            Abs::pt(gap_pt),
            Some(typst::visualize::Color::WHITE),
        )
        .encode_png()
        .map_err(|e| e.to_string())
    }

    /// 各ページの SVG
    pub fn svg_pages(&self) -> Vec<String> {
        let opts = typst_svg::SvgOptions::default();
        self.doc
            .pages()
            .iter()
            .map(|page| typst_svg::svg(page, &opts))
            .collect()
    }

    /// 全ページを縦に並べた 1 枚の SVG
    pub fn svg_merged(&self, gap_pt: f64) -> String {
        typst_svg::svg_merged(
            &self.doc,
            &typst_svg::SvgOptions::default(),
            Abs::pt(gap_pt),
        )
    }
}

fn render_options(dpi: f32) -> typst_render::RenderOptions {
    typst_render::RenderOptions {
        pixel_per_pt: typst::utils::Scalar::new(ppp_for_dpi(dpi.clamp(10.0, 2400.0)) as f64),
        ..Default::default()
    }
}

fn format_diag(world: &Session, d: &SourceDiagnostic, kind: &str) -> String {
    let loc = diag_location(world, d.span).unwrap_or_default();
    format!("{kind}: {loc}{}", d.message)
}

/// 診断の位置。本文なら `行:桁: `、ほかのファイル (取り込んだ .typ や読み込んだデータ) なら
/// `ファイル名:行:桁: `
fn diag_location(world: &Session, span: DiagSpan) -> Option<String> {
    let id = span.id()?;
    let start = world.range(span)?.start;
    if id == world.main.id() {
        let (line, col) = world.main.lines().byte_to_line_column(start)?;
        return Some(format!("{}:{}: ", line + 1, col + 1));
    }
    let name = id.vpath().get_without_slash();
    let bytes = world.file(id).ok()?;
    let before = bytes.as_slice().get(..start)?;
    let line = before.iter().filter(|b| **b == b'\n').count() + 1;
    let line_start = before
        .iter()
        .rposition(|b| *b == b'\n')
        .map_or(0, |i| i + 1);
    let col = String::from_utf8_lossy(&before[line_start..])
        .chars()
        .count()
        + 1;
    Some(format!("{name}:{line}:{col}: "))
}

/// 幅 `max_width_px` に収まるように解像度を自動調整してコンパイルする。
/// コンパイルは 1 回だけで、ページ幅 (pt) から解像度を決めてからラスタライズする
pub fn compile_fit(
    source: &str,
    path: Option<&Path>,
    max_width_px: u32,
    max_ppp: f32,
) -> CompileOutput {
    let (doc, diagnostics) = compile_document(source, path);
    let Some(doc) = doc else {
        return CompileOutput {
            pages: Vec::new(),
            diagnostics,
            ok: false,
        };
    };
    let widest_pt = doc
        .doc
        .pages()
        .iter()
        .map(|p| (p.frame.width() + p.bleed.left + p.bleed.right).to_pt())
        .fold(0.0_f64, f64::max);
    let mut ppp = max_ppp as f64;
    if max_width_px > 0 && widest_pt * ppp > max_width_px as f64 {
        ppp = max_width_px as f64 / widest_pt;
    }
    CompileOutput {
        pages: doc.rasterize(ppp.max(0.2) as f32),
        diagnostics,
        ok: true,
    }
}

/// 複数ページを縦に 1 枚の画像へ結合する (ページ間に `gap` px の隙間、背景は灰色)。
pub fn merge_pages(pages: &[PageImage], gap: u32) -> PageImage {
    let width = pages.iter().map(|p| p.width).max().unwrap_or(1).max(1);
    let height = pages.iter().map(|p| p.height + gap).sum::<u32>().max(1);
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for px in rgba.chunks_exact_mut(4) {
        px.copy_from_slice(&[230, 230, 230, 255]);
    }
    let mut y0 = 0u32;
    for p in pages {
        for y in 0..p.height {
            let src = &p.rgba[(y * p.width * 4) as usize..((y + 1) * p.width * 4) as usize];
            let dst_start = (((y0 + y) * width) * 4) as usize;
            rgba[dst_start..dst_start + src.len()].copy_from_slice(src);
        }
        y0 += p.height + gap;
    }
    PageImage {
        width,
        height,
        rgba,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_cjk_font_is_available() {
        let book = engine().fonts.book();
        assert!(book.contains_family("noto sans jp"));
        // Regular と Bold の両方 (システムに同名のフォントがあればさらに増える)
        assert!(book.select_family("noto sans jp").count() >= 2);
    }

    #[test]
    fn japanese_compiles_with_embedded_font() {
        let out = compile(
            "#set text(font: \"Noto Sans JP\")\n日本語の文章。",
            None,
            1.0,
        );
        assert!(out.ok, "{:?}", out.diagnostics);
        assert!(
            out.diagnostics.is_empty(),
            "フォントが見つからない警告が出た: {:?}",
            out.diagnostics
        );
    }

    /// 試験用のフォルダ (終わったら消す)
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("typball-core-test-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn write(&self, relative: &str, data: &[u8]) -> PathBuf {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, data).unwrap();
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const SVG: &[u8] =
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10"/></svg>"#;

    #[test]
    fn image_next_to_the_document_is_found() {
        let dir = TempDir::new("image");
        dir.write("Schematic.svg", SVG);
        dir.write("assets/sub.svg", SVG);
        let main = dir.0.join("Manual.typ");
        // 本文はディスクに無くてもよい (エディタの内容を使う)
        for path in [
            "./Schematic.svg",
            "Schematic.svg",
            "/Schematic.svg",
            "assets/sub.svg",
        ] {
            let (ok, diagnostics) = check(&format!("#image(\"{path}\")"), Some(&main));
            assert!(ok, "{path}: {diagnostics:?}");
        }
    }

    #[test]
    fn missing_file_reports_the_real_path() {
        let dir = TempDir::new("missing");
        let main = dir.0.join("Manual.typ");
        let (ok, diagnostics) = check("#image(\"nothing.png\")", Some(&main));
        assert!(!ok);
        let expected = dir.0.join("nothing.png").display().to_string();
        assert!(
            diagnostics.iter().any(|d| d.contains(&expected)),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn files_outside_the_folder_are_refused() {
        let dir = TempDir::new("outside");
        dir.write("secret.svg", SVG);
        let main = dir.0.join("docs").join("Manual.typ");
        let (ok, diagnostics) = check("#image(\"../secret.svg\")", Some(&main));
        assert!(!ok, "{diagnostics:?}");
    }

    #[test]
    fn untitled_document_explains_why_files_are_missing() {
        let (ok, diagnostics) = check("#image(\"Schematic.png\")", None);
        assert!(!ok);
        assert!(
            diagnostics.iter().any(|d| d.contains("has not been saved")),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn imported_source_is_read_and_reloaded() {
        let dir = TempDir::new("import");
        let lib = dir.write("lib.typ", b"#let answer = 1\n");
        let main = dir.0.join("Manual.typ");
        let source = "#import \"lib.typ\": answer\n#assert(answer == 1)";
        let (ok, diagnostics) = check(source, Some(&main));
        assert!(ok, "{diagnostics:?}");
        // 取り込んだファイルが変わったら読み直す (大きさが変わるので更新日時によらず気づく)
        std::fs::write(&lib, "#let answer = 22\n").unwrap();
        let (ok, diagnostics) = check(source, Some(&main));
        assert!(!ok, "古い中身のまま: {diagnostics:?}");
    }

    #[test]
    fn error_in_imported_file_names_the_file() {
        let dir = TempDir::new("import-error");
        dir.write("lib.typ", b"#let a = 1\n#unknown()\n");
        let main = dir.0.join("Manual.typ");
        let (ok, diagnostics) = check("#import \"lib.typ\": a", Some(&main));
        assert!(!ok);
        assert!(
            diagnostics.iter().any(|d| d.contains("lib.typ:2:2: ")),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn document_named_like_another_file_does_not_shadow_it() {
        // 編集中の本文は実際のファイル名で扱うので、同じフォルダの main.typ を取り込める
        let dir = TempDir::new("main-name");
        dir.write("main.typ", b"#let from_main = 5\n");
        let chapter = dir.0.join("chapter.typ");
        let (ok, diagnostics) = check(
            "#import \"main.typ\": from_main\n#assert(from_main == 5)",
            Some(&chapter),
        );
        assert!(ok, "{diagnostics:?}");
    }
}
