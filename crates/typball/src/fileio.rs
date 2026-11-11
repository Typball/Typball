//! ファイルのバイト列とテキストの変換 (文字コード・BOM) と、壊れにくい書き込み
//!
//! GUI には依存しない純粋な処理だけを置く。別スレッドから呼べるように `settings::get()`
//! (スレッドローカル) も使わない。
//!
//! 文字コードの判定は Notepad++ / VSCode と同じ順: BOM → UTF-8 として正しいか → UTF-16 らしさ →
//! 設定の既定 (無ければ chardetng の推定)。保存はもとの文字コードに戻す (round-trip)。
//! 表せない文字があれば [`encode`] がエラーを返すので、呼ぶ側が UTF-8 への切り替えを聞く。
//!
//! 書き込みは同じフォルダの一時ファイルに書いて fsync してから元のファイルと入れ替える
//! (Vim の `backupcopy=auto`、Sublime の `atomic_save` と同じ)。途中でデバイスが外れても
//! 元のファイルは残る。シンボリックリンクやハードリンクは入れ替えると壊れるので直接書く。
//! 保存先のフォルダに一時ファイルを作りたくない人向けに、設定 `files.save_method` で直接書き込みも
//! 選べる ([`write_with`] の `atomic` に false)。

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use encoding_rs::{Encoding as RsEncoding, UTF_16BE, UTF_16LE, UTF_8};

/// 文書の文字コード
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8 {
        bom: bool,
    },
    Utf16 {
        little_endian: bool,
        bom: bool,
    },
    /// encoding_rs のエンコーディング (Shift_JIS など。UTF-8 / UTF-16 以外)
    Legacy(&'static RsEncoding),
}

impl Default for Encoding {
    fn default() -> Self {
        Self::Utf8 { bom: false }
    }
}

impl Encoding {
    pub const UTF8: Encoding = Encoding::Utf8 { bom: false };

    /// メニューと設定に出す一覧。先頭が既定
    pub fn all() -> Vec<Encoding> {
        let mut v = vec![
            Encoding::Utf8 { bom: false },
            Encoding::Utf8 { bom: true },
            Encoding::Utf16 {
                little_endian: true,
                bom: true,
            },
            Encoding::Utf16 {
                little_endian: false,
                bom: true,
            },
        ];
        for label in [
            "shift_jis",
            "euc-jp",
            "iso-2022-jp",
            "windows-1252",
            "iso-8859-2",
            "windows-1251",
            "koi8-r",
            "gbk",
            "gb18030",
            "big5",
            "euc-kr",
        ] {
            if let Some(e) = RsEncoding::for_label(label.as_bytes()) {
                v.push(Encoding::Legacy(e));
            }
        }
        v
    }

    /// 設定ファイル用の名前 ("utf-8" / "utf-8-bom" / "utf-16le" / "utf-16be" / "shift_jis" ...)
    pub fn id(&self) -> String {
        match self {
            Encoding::Utf8 { bom: false } => "utf-8".into(),
            Encoding::Utf8 { bom: true } => "utf-8-bom".into(),
            Encoding::Utf16 {
                little_endian: true,
                ..
            } => "utf-16le".into(),
            Encoding::Utf16 {
                little_endian: false,
                ..
            } => "utf-16be".into(),
            Encoding::Legacy(e) => e.name().to_ascii_lowercase(),
        }
    }

    /// [`id`] から戻す。知らない名前なら None
    pub fn parse(id: &str) -> Option<Encoding> {
        let id = id.trim().to_ascii_lowercase();
        match id.as_str() {
            "utf-8" | "utf8" => Some(Encoding::Utf8 { bom: false }),
            "utf-8-bom" | "utf8-bom" => Some(Encoding::Utf8 { bom: true }),
            "utf-16le" | "utf-16" | "ucs-2le" => Some(Encoding::Utf16 {
                little_endian: true,
                bom: true,
            }),
            "utf-16be" | "ucs-2be" => Some(Encoding::Utf16 {
                little_endian: false,
                bom: true,
            }),
            _ => {
                let e = RsEncoding::for_label(id.as_bytes())?;
                if e == UTF_8 {
                    Some(Encoding::Utf8 { bom: false })
                } else if e == UTF_16LE || e == UTF_16BE {
                    Some(Encoding::Utf16 {
                        little_endian: e == UTF_16LE,
                        bom: true,
                    })
                } else {
                    Some(Encoding::Legacy(e))
                }
            }
        }
    }

    /// ステータスバーとメニューに出す名前
    pub fn label(&self) -> String {
        match self {
            Encoding::Utf8 { bom: false } => "UTF-8".into(),
            Encoding::Utf8 { bom: true } => "UTF-8 (BOM)".into(),
            Encoding::Utf16 { little_endian, bom } => {
                let base = if *little_endian {
                    "UTF-16 LE"
                } else {
                    "UTF-16 BE"
                };
                if *bom {
                    format!("{base} (BOM)")
                } else {
                    base.into()
                }
            }
            Encoding::Legacy(e) => match e.name() {
                "Shift_JIS" => "Shift_JIS".into(),
                "EUC-JP" => "EUC-JP".into(),
                "ISO-2022-JP" => "ISO-2022-JP (JIS)".into(),
                "windows-1252" => "Windows-1252 (Latin-1)".into(),
                "ISO-8859-2" => "ISO-8859-2".into(),
                "windows-1251" => "Windows-1251 (Cyrillic)".into(),
                "KOI8-R" => "KOI8-R".into(),
                "GBK" => "GBK".into(),
                "gb18030" => "GB18030".into(),
                "Big5" => "Big5".into(),
                "EUC-KR" => "EUC-KR".into(),
                other => other.into(),
            },
        }
    }

    /// 同じ文字コードか (BOM の有無は無視)
    pub fn same_family(&self, other: &Encoding) -> bool {
        match (self, other) {
            (Encoding::Utf8 { .. }, Encoding::Utf8 { .. }) => true,
            (
                Encoding::Utf16 {
                    little_endian: a, ..
                },
                Encoding::Utf16 {
                    little_endian: b, ..
                },
            ) => a == b,
            (Encoding::Legacy(a), Encoding::Legacy(b)) => a == b,
            _ => false,
        }
    }
}

/// 環境変数 TYPBALL_IO_DEBUG があるときだけ、読み書きの経路 (入れ替えの失敗やフォールバック、裏で続く保存の
/// 受け取り) を記録する。値がパスならそのファイルに追記し (GUI では stderr が見えない)、それ以外なら stderr に出す
pub(crate) fn debug_log(message: impl FnOnce() -> String) {
    let Some(value) = std::env::var_os("TYPBALL_IO_DEBUG") else {
        return;
    };
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let elapsed = START.get_or_init(std::time::Instant::now).elapsed();
    let line = format!("[typball io {:8.3}s] {}", elapsed.as_secs_f64(), message());
    let path = Path::new(&value);
    if path.is_absolute() {
        // 複数のスレッドから書くので、1 行を 1 回の write で出す
        if let Ok(mut f) = OpenOptions::new().append(true).create(true).open(path) {
            let _ = f.write_all(
                format!(
                    "{line}
"
                )
                .as_bytes(),
            );
        }
    } else {
        eprintln!("{line}");
    }
}

/// 読み込みの指示
#[derive(Debug, Clone, Copy, Default)]
pub struct DecodeOptions {
    /// この文字コードで読む (「指定して開き直す」)。BOM があっても従う
    pub forced: Option<Encoding>,
    /// UTF-8 として読めなかったときの既定。None なら内容から推定する
    pub fallback: Option<Encoding>,
    /// 推定のヒント (言語のトップレベルドメイン: 日本語 UI なら b"jp")
    pub hint: Option<&'static [u8]>,
}

/// 読み込んだ結果
#[derive(Debug)]
pub struct Decoded {
    pub text: String,
    pub encoding: Encoding,
    /// 読めないバイトや NUL を U+FFFD に置き換えた (このまま保存すると元のバイト列は失われる)
    pub lossy: bool,
    /// NUL バイトがあった (バイナリらしい)
    pub binary: bool,
}

/// 判定に使う先頭のバイト数 (推定・UTF-16 らしさ)
const SNIFF_LEN: usize = 4 * 1024 * 1024;

/// バイト列をテキストにする
pub fn decode(bytes: Vec<u8>, options: DecodeOptions) -> Decoded {
    let (text, encoding, lossy) = decode_inner(bytes, options);
    // wxDragon の set_text は内部 NUL のある文字列を空にしてしまう (CString) ので、NUL は
    // 置き換え文字にして「失われる」扱いにする
    let binary = text.contains('\0');
    let text = if binary {
        text.replace('\0', "\u{FFFD}")
    } else {
        text
    };
    Decoded {
        text,
        encoding,
        lossy: lossy || binary,
        binary,
    }
}

fn decode_inner(bytes: Vec<u8>, options: DecodeOptions) -> (String, Encoding, bool) {
    let bom = sniff_bom(&bytes);
    if let Some(forced) = options.forced {
        // 指定された文字コードと同じ系統の BOM だけ取り除く
        let skip = match (bom, forced) {
            (Some((b, n)), f) if b.same_family(&f) => n,
            _ => 0,
        };
        let forced = match (forced, bom) {
            (Encoding::Utf8 { .. }, Some((Encoding::Utf8 { .. }, _))) => {
                Encoding::Utf8 { bom: true }
            }
            (Encoding::Utf8 { .. }, None) => Encoding::Utf8 { bom: false },
            (Encoding::Utf16 { little_endian, .. }, Some((Encoding::Utf16 { .. }, _))) => {
                Encoding::Utf16 {
                    little_endian,
                    bom: true,
                }
            }
            (Encoding::Utf16 { little_endian, .. }, _) => Encoding::Utf16 {
                little_endian,
                bom: false,
            },
            (f, _) => f,
        };
        let (text, lossy) = decode_with(&bytes[skip..], forced);
        return (text, forced, lossy);
    }
    if let Some((encoding, skip)) = bom {
        let (text, lossy) = decode_with(&bytes[skip..], encoding);
        return (text, encoding, lossy);
    }
    // BOM の無い UTF-16: ASCII 主体なら "h\0e\0..." で UTF-8 としても正しいので、先に見る
    if bytes.contains(&0) {
        if let Some(little_endian) = looks_like_utf16(&bytes) {
            let encoding = Encoding::Utf16 {
                little_endian,
                bom: false,
            };
            let (text, lossy) = decode_with(&bytes, encoding);
            return (text, encoding, lossy);
        }
    }
    // UTF-8 として正しければそのまま (コピーしない)
    match String::from_utf8(bytes) {
        Ok(text) => (text, Encoding::Utf8 { bom: false }, false),
        Err(e) => {
            let bytes = e.into_bytes();
            let encoding = options
                .fallback
                .unwrap_or_else(|| guess(&bytes, options.hint));
            let (text, lossy) = decode_with(&bytes, encoding);
            (text, encoding, lossy)
        }
    }
}

/// 先頭の BOM と、その長さ
fn sniff_bom(bytes: &[u8]) -> Option<(Encoding, usize)> {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        Some((Encoding::Utf8 { bom: true }, 3))
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        Some((
            Encoding::Utf16 {
                little_endian: true,
                bom: true,
            },
            2,
        ))
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        Some((
            Encoding::Utf16 {
                little_endian: false,
                bom: true,
            },
            2,
        ))
    } else {
        None
    }
}

/// BOM の無い UTF-16 らしいか (偶数長で、片側のバイトの多くが 0)。Some(リトルエンディアンか)
fn looks_like_utf16(bytes: &[u8]) -> Option<bool> {
    if bytes.len() < 4 || bytes.len() % 2 != 0 {
        return None;
    }
    let sample = &bytes[..bytes.len().min(SNIFF_LEN)];
    let pairs = sample.len() / 2;
    let (mut even_zero, mut odd_zero) = (0usize, 0usize);
    for pair in sample.chunks_exact(2) {
        if pair[0] == 0 {
            even_zero += 1;
        }
        if pair[1] == 0 {
            odd_zero += 1;
        }
    }
    // ASCII 主体の UTF-16 LE なら奇数位置 (上位バイト) がほとんど 0 で、偶数位置はほぼ非 0
    if odd_zero * 10 >= pairs * 3 && even_zero * 20 <= pairs {
        Some(true)
    } else if even_zero * 10 >= pairs * 3 && odd_zero * 20 <= pairs {
        Some(false)
    } else {
        None
    }
}

/// 内容から文字コードを推定する (UTF-8 として正しくないバイト列向け)
fn guess(bytes: &[u8], hint: Option<&[u8]>) -> Encoding {
    let mut detector = chardetng::EncodingDetector::new();
    let sample = &bytes[..bytes.len().min(SNIFF_LEN)];
    detector.feed(sample, sample.len() == bytes.len());
    let e = detector.guess(hint, true);
    if e == UTF_8 {
        Encoding::Utf8 { bom: false }
    } else if e == UTF_16LE || e == UTF_16BE {
        Encoding::Utf16 {
            little_endian: e == UTF_16LE,
            bom: false,
        }
    } else {
        Encoding::Legacy(e)
    }
}

/// 指定の文字コードで読む。(テキスト, 置き換えがあったか)
fn decode_with(bytes: &[u8], encoding: Encoding) -> (String, bool) {
    let rs: &'static RsEncoding = match encoding {
        Encoding::Utf8 { .. } => UTF_8,
        Encoding::Utf16 {
            little_endian: true,
            ..
        } => UTF_16LE,
        Encoding::Utf16 {
            little_endian: false,
            ..
        } => UTF_16BE,
        Encoding::Legacy(e) => e,
    };
    let (text, lossy) = rs.decode_without_bom_handling(bytes);
    (text.into_owned(), lossy)
}

/// その文字コードで表せない文字があった (最初の 1 つ)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unencodable {
    /// テキスト (UTF-8) の中のバイト位置。Scintilla の文書位置と同じ
    pub offset: usize,
    pub ch: char,
}

/// テキストをその文字コードのバイト列にする (BOM 付き)。表せない文字は置き換えずにエラーにする
/// (「?」や数値参照に化けたまま保存されるのを防ぐ)
pub fn encode(text: &str, encoding: Encoding) -> Result<Vec<u8>, Unencodable> {
    match encoding {
        Encoding::Utf8 { bom } => {
            let mut out = Vec::with_capacity(text.len() + 3);
            if bom {
                out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
            }
            out.extend_from_slice(text.as_bytes());
            Ok(out)
        }
        Encoding::Utf16 { little_endian, bom } => {
            let mut out = Vec::with_capacity(text.len() * 2 + 2);
            let units = text.encode_utf16();
            if bom {
                push_u16(&mut out, 0xFEFF, little_endian);
            }
            for u in units {
                push_u16(&mut out, u, little_endian);
            }
            Ok(out)
        }
        Encoding::Legacy(e) => encode_legacy(text, e),
    }
}

/// encoding_rs で変換する。`Encoding::encode` は表せない文字を数値参照 (`&#128512;`) に置き換えて
/// しまうので、置き換えない版を使って最初の 1 文字の位置を返す
fn encode_legacy(text: &str, e: &'static RsEncoding) -> Result<Vec<u8>, Unencodable> {
    use encoding_rs::EncoderResult;
    let mut encoder = e.new_encoder();
    // 2 バイト系の文字コードは UTF-8 より長くならないことが多い。足りなければ下で広げる
    let mut out = Vec::with_capacity(text.len() + 16);
    let mut read = 0;
    loop {
        let (result, n) =
            encoder.encode_from_utf8_to_vec_without_replacement(&text[read..], &mut out, true);
        read += n;
        match result {
            EncoderResult::InputEmpty => return Ok(out),
            EncoderResult::OutputFull => out.reserve((text.len() - read).max(4096)),
            // 読んだ量は表せなかった文字を含む
            EncoderResult::Unmappable(ch) => {
                return Err(Unencodable {
                    offset: read.saturating_sub(ch.len_utf8()),
                    ch,
                })
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 改行コード
// ---------------------------------------------------------------------------

/// 改行コード。ファイルのものをそのまま保ち、新しく入力する改行だけこれに合わせる
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Crlf,
    Lf,
    Cr,
}

impl LineEnding {
    /// メニューと保存ダイアログに出す並び
    pub const ALL: [LineEnding; 3] = [LineEnding::Crlf, LineEnding::Lf, LineEnding::Cr];

    /// 改行そのもの
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Crlf => "\r\n",
            LineEnding::Lf => "\n",
            LineEnding::Cr => "\r",
        }
    }

    /// 設定ファイルに書く名前 ("crlf" / "lf" / "cr")
    pub fn id(self) -> &'static str {
        match self {
            LineEnding::Crlf => "crlf",
            LineEnding::Lf => "lf",
            LineEnding::Cr => "cr",
        }
    }

    /// 設定ファイルの名前から (大文字小文字は区別しない)。"auto" や知らない名前は None
    pub fn parse(id: &str) -> Option<LineEnding> {
        LineEnding::ALL
            .into_iter()
            .find(|eol| eol.id().eq_ignore_ascii_case(id.trim()))
    }

    /// 短い名前 ("CRLF" など。混在の表示や説明に使う)
    pub fn short_label(self) -> &'static str {
        match self {
            LineEnding::Crlf => "CRLF",
            LineEnding::Lf => "LF",
            LineEnding::Cr => "CR",
        }
    }

    /// 使う OS を添えた名前 ("CRLF (Windows)" など) の言語ファイルのキー
    pub fn label_key(self) -> &'static str {
        match self {
            LineEnding::Crlf => "eol.crlf",
            LineEnding::Lf => "eol.lf",
            LineEnding::Cr => "eol.cr",
        }
    }

    /// すべての改行をこの改行コードにそろえたテキスト (Scintilla の SCI_CONVERTEOLS と同じ結果になる:
    /// CR LF・CR・LF のどれも 1 つの改行として数える)
    pub fn convert(self, text: &str) -> String {
        let eol = self.as_str();
        let mut out = String::with_capacity(text.len() + text.len() / 32);
        let mut rest = text;
        while let Some(i) = rest.find(['\r', '\n']) {
            out.push_str(&rest[..i]);
            out.push_str(eol);
            let len = if rest[i..].starts_with("\r\n") { 2 } else { 1 };
            rest = &rest[i + len..];
        }
        out.push_str(rest);
        out
    }

    /// テキストで一番多い改行コード (先頭 1 MiB を見る)。改行が無ければ None
    pub fn detect(text: &str) -> Option<LineEnding> {
        EolCounts::of(&text.as_bytes()[..text.len().min(1024 * 1024)]).majority()
    }

    /// この OS の改行コード (設定 `files.default_line_ending` が "auto" のときの新規文書の改行)
    pub fn platform() -> LineEnding {
        if cfg!(windows) {
            LineEnding::Crlf
        } else {
            LineEnding::Lf
        }
    }
}

/// テキストの中の改行コードごとの数
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EolCounts {
    pub crlf: usize,
    pub lf: usize,
    pub cr: usize,
}

impl EolCounts {
    /// `bytes` (UTF-8 のテキスト) の改行を数える。CR LF は 1 つの改行
    pub fn of(bytes: &[u8]) -> EolCounts {
        let mut counts = EolCounts::default();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'\r' if bytes.get(i + 1) == Some(&b'\n') => {
                    counts.crlf += 1;
                    i += 1;
                }
                b'\r' => counts.cr += 1,
                b'\n' => counts.lf += 1,
                _ => {}
            }
            i += 1;
        }
        counts
    }

    /// 一番多い改行コード。改行が無ければ None
    pub fn majority(&self) -> Option<LineEnding> {
        if self.crlf == 0 && self.lf == 0 && self.cr == 0 {
            None
        } else if self.crlf >= self.lf && self.crlf >= self.cr {
            Some(LineEnding::Crlf)
        } else if self.lf >= self.cr {
            Some(LineEnding::Lf)
        } else {
            Some(LineEnding::Cr)
        }
    }

    /// 使われている改行コード (`LineEnding::ALL` の順)
    pub fn kinds(&self) -> Vec<LineEnding> {
        LineEnding::ALL
            .into_iter()
            .filter(|eol| match eol {
                LineEnding::Crlf => self.crlf > 0,
                LineEnding::Lf => self.lf > 0,
                LineEnding::Cr => self.cr > 0,
            })
            .collect()
    }

    /// 2 種類以上の改行コードが混ざっている
    pub fn mixed(&self) -> bool {
        self.kinds().len() > 1
    }
}

fn push_u16(out: &mut Vec<u8>, u: u16, little_endian: bool) {
    if little_endian {
        out.extend_from_slice(&u.to_le_bytes());
    } else {
        out.extend_from_slice(&u.to_be_bytes());
    }
}

// ---------------------------------------------------------------------------
// 書き込み
// ---------------------------------------------------------------------------

/// `path` に `bytes` を書く。できるだけ「全部書けたか、元のまま」のどちらかになるようにする:
/// 同じフォルダの一時ファイルに書いて fsync し、成功したら元のファイルと入れ替える。
/// 一時ファイルを作れない (フォルダに書けない) ときと、リンクのときは直接書く。
/// 設定ファイルや復元用のスナップショットなど、利用者の設定に関係なく安全に書きたいものはこれを使う
pub fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_with(path, bytes, true)
}

/// `atomic` が true なら [`write`] と同じ。false なら一時ファイルを作らず、元のファイルに直接書き込む
/// (途中で失敗すると元の内容は失われるが、保存先のフォルダに別のファイルが現れない)
pub fn write_with(path: &Path, bytes: &[u8], atomic: bool) -> io::Result<()> {
    // 動作確認用: 遅いデバイスの代わりに書き込みを遅らせる (進捗ダイアログと「待つのをやめる」の確認)
    if let Some(ms) = std::env::var("TYPBALL_SLOW_IO_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
    if !atomic {
        debug_log(|| format!("writing {} in place (save_method = direct)", path.display()));
        return write_in_place(path, bytes);
    }
    // 読み取り専用のファイルは入れ替えでも書き換えない。Unix の rename はフォルダに書ければ通るので、
    // 確かめないと読み取り専用の印が意味を持たなくなる (直接書き込みなら OS が断る)
    if fs::metadata(path).is_ok_and(|m| m.permissions().readonly()) {
        return Err(io::Error::from_raw_os_error(ACCESS_DENIED));
    }
    if !replaceable(path) {
        return write_in_place(path, bytes);
    }
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => Path::new(".").to_path_buf(),
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let temp = dir.join(format!(
        ".{name}.typball-{}-{nanos:08x}.tmp",
        std::process::id()
    ));
    let Ok(mut file) = OpenOptions::new().write(true).create_new(true).open(&temp) else {
        // フォルダに新しいファイルを作れない (権限など): 従来どおり直接書く
        return write_in_place(path, bytes);
    };
    if !same_owner(path, &temp) {
        // 他のユーザーのファイル (グループで共有しているなど): 入れ替えると持ち主が自分に変わってしまう
        drop(file);
        let _ = fs::remove_file(&temp);
        return write_in_place(path, bytes);
    }
    let written = (|| {
        file.write_all(bytes)?;
        file.sync_all()
    })();
    drop(file);
    if let Err(e) = written {
        // デバイスに書けていないので元のファイルには触らない
        let _ = fs::remove_file(&temp);
        return Err(e);
    }
    copy_permissions(path, &temp);
    match replace(&temp, path) {
        Ok(()) => {
            sync_dir(&dir);
            Ok(())
        }
        Err(e) => {
            // 入れ替えられない (他のプロセスが開いている・読み取り専用など)。デバイスには書けて
            // いるので、従来どおり直接書く
            debug_log(|| {
                format!(
                    "replace failed for {}: {e}; writing in place",
                    path.display()
                )
            });
            let _ = fs::remove_file(&temp);
            write_in_place(path, bytes)
        }
    }
}

/// 「アクセスが拒否されました」の OS のエラー番号 (メッセージは OS の言語で出る)
#[cfg(windows)]
const ACCESS_DENIED: i32 = windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED as i32;
/// EACCES (Linux / macOS / BSD で共通)
#[cfg(not(windows))]
const ACCESS_DENIED: i32 = 13;

/// 元のファイルと一時ファイルの持ち主 (ユーザーとグループ) が同じか。元が無ければ true
#[cfg(unix)]
fn same_owner(target: &Path, temp: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (fs::metadata(target), fs::metadata(temp)) {
        (Ok(a), Ok(b)) => a.uid() == b.uid() && a.gid() == b.gid(),
        _ => true,
    }
}

/// Windows の持ち主と ACL は ReplaceFile が元のファイルから引き継ぐ
#[cfg(not(unix))]
fn same_owner(_target: &Path, _temp: &Path) -> bool {
    true
}

/// 入れ替えで壊れないファイルか: シンボリックリンクとハードリンク (他の名前からも見える) は直接書く
fn replaceable(path: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        // まだ無い (新規) なら入れ替えでよい
        return true;
    };
    if meta.file_type().is_symlink() {
        return false;
    }
    link_count(path) <= 1
}

#[cfg(unix)]
fn link_count(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).map(|m| m.nlink()).unwrap_or(1)
}

#[cfg(windows)]
fn link_count(path: &Path) -> u64 {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    // std の number_of_links はまだ unstable なので Win32 で取る
    let Ok(file) = File::open(path) else {
        return 1;
    };
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: 開いているファイルのハンドルと、書き込み先の構造体を渡している
    let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) };
    if ok != 0 {
        u64::from(info.nNumberOfLinks)
    } else {
        1
    }
}

#[cfg(not(any(unix, windows)))]
fn link_count(_path: &Path) -> u64 {
    1
}

/// 元のファイルのパーミッションを一時ファイルに写す (Windows の属性は ReplaceFile が引き継ぐ)
#[cfg(unix)]
fn copy_permissions(from: &Path, to: &Path) {
    if let Ok(meta) = fs::metadata(from) {
        let _ = fs::set_permissions(to, meta.permissions());
    }
}

#[cfg(not(unix))]
fn copy_permissions(_from: &Path, _to: &Path) {}

/// 一時ファイルを `target` の場所へ移す。`target` があれば置き換える
#[cfg(windows)]
fn replace(temp: &Path, target: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND};
    use windows_sys::Win32::Storage::FileSystem::{
        ReplaceFileW, REPLACEFILE_IGNORE_ACL_ERRORS, REPLACEFILE_IGNORE_MERGE_ERRORS,
    };
    let wide = |p: &Path| -> Vec<u16> {
        p.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    };
    // ReplaceFile は「元を backup の名前に変える → 一時ファイルを元の名前にする」の 2 段階で、
    // 2 段目で失敗すると (ERROR_UNABLE_TO_MOVE_REPLACEMENT_2) 元のファイルが無くなる。backup の名前を
    // 渡しておけば元は消されずに残るので、失敗したら戻せる。成功したら backup は消す
    let backup = temp.with_extension("bak");
    let (t, r, b) = (wide(target), wide(temp), wide(&backup));
    // ReplaceFile は属性・ACL・作成日時・代替ストリームを元のファイルから引き継ぐ。
    // 元のファイルが無ければ (新規) 普通に移動する
    // SAFETY: いずれも NUL 終端の UTF-16 文字列で、呼び出し中は生きている
    let ok = unsafe {
        ReplaceFileW(
            t.as_ptr(),
            r.as_ptr(),
            b.as_ptr(),
            REPLACEFILE_IGNORE_MERGE_ERRORS | REPLACEFILE_IGNORE_ACL_ERRORS,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if ok != 0 {
        let _ = fs::remove_file(&backup);
        return Ok(());
    }
    let err = io::Error::last_os_error();
    debug_log(|| format!("ReplaceFileW({}) failed: {err}", target.display()));
    let not_found = matches!(err.raw_os_error(), Some(code)
        if code == ERROR_FILE_NOT_FOUND as i32 || code == ERROR_PATH_NOT_FOUND as i32);
    if not_found {
        // 元のファイルが無い (新規)
        return fs::rename(temp, target);
    }
    // 入れ替えを MoveFileEx で試す。それでもだめなら、backup に残っている元のファイルを戻す
    match fs::rename(temp, target) {
        Ok(()) => {
            let _ = fs::remove_file(&backup);
            Ok(())
        }
        Err(_) => {
            if backup.exists() && !target.exists() {
                let _ = fs::rename(&backup, target);
            }
            let _ = fs::remove_file(&backup);
            Err(err)
        }
    }
}

#[cfg(not(windows))]
fn replace(temp: &Path, target: &Path) -> io::Result<()> {
    fs::rename(temp, target)
}

/// 名前の付け替えをディスクに反映させる (Unix)。Windows では不要
#[cfg(unix)]
fn sync_dir(dir: &Path) {
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) {}

/// 元のファイルをそのまま上書きする (直接書き込み / フォールバック)。書き終えたら fsync する。
/// 既存のファイルは TRUNCATE_EXISTING で開く: `File::create` (CREATE_ALWAYS) は Windows で隠し属性・
/// システム属性のファイルに ERROR_ACCESS_DENIED になる (属性を指定し直さないと置き換えを拒む仕様)
fn write_in_place(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = match OpenOptions::new().write(true).truncate(true).open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => File::create(path)?,
        Err(e) => return Err(e),
    };
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("typball-fileio-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn utf8_without_bom_is_kept_as_is() {
        let d = decode("こんにちは\n".as_bytes().to_vec(), DecodeOptions::default());
        assert_eq!(d.encoding, Encoding::Utf8 { bom: false });
        assert_eq!(d.text, "こんにちは\n");
        assert!(!d.lossy && !d.binary);
    }

    #[test]
    fn utf8_bom_is_detected_and_restored() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("a".as_bytes());
        let d = decode(bytes.clone(), DecodeOptions::default());
        assert_eq!(d.encoding, Encoding::Utf8 { bom: true });
        assert_eq!(d.text, "a");
        assert_eq!(encode(&d.text, d.encoding).unwrap(), bytes);
    }

    #[test]
    fn shift_jis_round_trips_with_a_japanese_hint() {
        let sjis = vec![0x93, 0xFA, 0x96, 0x7B, 0x8C, 0xEA]; // 日本語 (短いので推定にはヒントが要る)
        let d = decode(
            sjis.clone(),
            DecodeOptions {
                hint: Some(b"jp"),
                ..Default::default()
            },
        );
        assert_eq!(d.text, "日本語");
        assert_eq!(d.encoding.id(), "shift_jis");
        assert!(!d.lossy);
        assert_eq!(encode(&d.text, d.encoding).unwrap(), sjis);
    }

    #[test]
    fn shift_jis_is_detected_without_a_hint_when_there_is_enough_text() {
        let text = "吾輩は猫である。名前はまだ無い。どこで生れたかとんと見当がつかぬ。\n\
                    何でも薄暗いじめじめした所でニャーニャー泣いていた事だけは記憶している。\n";
        let sjis = encode(text, Encoding::parse("shift_jis").unwrap()).unwrap();
        let d = decode(sjis.clone(), DecodeOptions::default());
        assert_eq!(d.encoding.id(), "shift_jis");
        assert_eq!(d.text, text);
        assert_eq!(encode(&d.text, d.encoding).unwrap(), sjis);
    }

    #[test]
    fn euc_jp_round_trips() {
        let text = "吾輩は猫である。名前はまだ無い。どこで生れたかとんと見当がつかぬ。\n";
        let euc = encode(text, Encoding::parse("euc-jp").unwrap()).unwrap();
        let d = decode(
            euc.clone(),
            DecodeOptions {
                hint: Some(b"jp"),
                ..Default::default()
            },
        );
        assert_eq!(d.encoding.id(), "euc-jp");
        assert_eq!(d.text, text);
    }

    #[test]
    fn fallback_encoding_overrides_detection() {
        let sjis = vec![0x93, 0xFA, 0x96, 0x7B, 0x8C, 0xEA];
        let d = decode(
            sjis,
            DecodeOptions {
                fallback: Encoding::parse("euc-jp"),
                ..Default::default()
            },
        );
        assert_eq!(d.encoding.id(), "euc-jp");
    }

    #[test]
    fn forced_encoding_is_used_even_for_valid_utf8() {
        let d = decode(
            b"abc".to_vec(),
            DecodeOptions {
                forced: Encoding::parse("shift_jis"),
                ..Default::default()
            },
        );
        assert_eq!(d.encoding.id(), "shift_jis");
        assert_eq!(d.text, "abc");
    }

    #[test]
    fn utf16_le_with_and_without_bom() {
        let with_bom = encode(
            "hi",
            Encoding::Utf16 {
                little_endian: true,
                bom: true,
            },
        )
        .unwrap();
        assert_eq!(with_bom, vec![0xFF, 0xFE, b'h', 0, b'i', 0]);
        let d = decode(with_bom.clone(), DecodeOptions::default());
        assert_eq!(d.text, "hi");
        assert_eq!(encode(&d.text, d.encoding).unwrap(), with_bom);

        let no_bom = vec![b'h', 0, b'e', 0, b'l', 0, b'l', 0, b'o', 0, b'\n', 0];
        let d = decode(no_bom.clone(), DecodeOptions::default());
        assert_eq!(
            d.encoding,
            Encoding::Utf16 {
                little_endian: true,
                bom: false
            }
        );
        assert_eq!(d.text, "hello\n");
        assert_eq!(encode(&d.text, d.encoding).unwrap(), no_bom);
    }

    #[test]
    fn nul_bytes_are_replaced_and_flagged() {
        let d = decode(b"a\0b".to_vec(), DecodeOptions::default());
        assert!(d.binary && d.lossy);
        assert_eq!(d.text, "a\u{FFFD}b");
    }

    #[test]
    fn invalid_utf8_is_lossy_when_nothing_else_fits() {
        let d = decode(
            vec![b'a', 0xFF, b'b'],
            DecodeOptions {
                forced: Some(Encoding::UTF8),
                ..Default::default()
            },
        );
        assert!(d.lossy);
        assert_eq!(d.text, "a\u{FFFD}b");
    }

    #[test]
    fn unencodable_characters_are_reported() {
        let sjis = Encoding::parse("shift_jis").unwrap();
        assert!(encode("日本語", sjis).is_ok());
        // 最初の 1 文字の位置 (UTF-8 のバイト位置) と文字が分かる
        let text = "絵文字 😀 と €";
        let err = encode(text, sjis).unwrap_err();
        assert_eq!(err.ch, '😀');
        assert_eq!(&text[err.offset..err.offset + 4], "😀");
        // 数値参照などに置き換えて通したりしない
        let latin1 = Encoding::parse("windows-1252").unwrap();
        assert_eq!(
            encode("aあ", latin1),
            Err(Unencodable {
                offset: 1,
                ch: 'あ'
            })
        );
    }

    #[test]
    fn every_listed_encoding_round_trips_plain_text() {
        // 大きめの入力で出力バッファの継ぎ足し (OutputFull) も通す
        let ascii = "line of plain text\r\n".repeat(5000);
        for e in Encoding::all() {
            let bytes = encode(&ascii, e).unwrap();
            let d = decode(
                bytes,
                DecodeOptions {
                    forced: Some(e),
                    ..Default::default()
                },
            );
            assert_eq!(d.text, ascii, "{}", e.id());
            assert!(!d.lossy, "{}", e.id());
        }
        // 状態を持つ ISO-2022-JP: 末尾で ASCII に戻すエスケープまで書けていること
        let jis = Encoding::parse("iso-2022-jp").unwrap();
        let bytes = encode("日本語 abc 日本語", jis).unwrap();
        assert!(bytes.ends_with(&[0x1B, b'(', b'B']));
        let d = decode(
            bytes,
            DecodeOptions {
                forced: Some(jis),
                ..Default::default()
            },
        );
        assert_eq!(d.text, "日本語 abc 日本語");
    }

    #[test]
    fn line_endings_are_detected_by_majority() {
        assert_eq!(LineEnding::detect("no newline"), None);
        assert_eq!(LineEnding::detect("a\nb\n"), Some(LineEnding::Lf));
        assert_eq!(LineEnding::detect("a\r\nb\r\n"), Some(LineEnding::Crlf));
        assert_eq!(LineEnding::detect("a\rb\r"), Some(LineEnding::Cr));
        assert_eq!(LineEnding::detect("a\nb\nc\r\n"), Some(LineEnding::Lf));
    }

    #[test]
    fn line_ending_ids_round_trip() {
        for eol in LineEnding::ALL {
            assert_eq!(LineEnding::parse(eol.id()), Some(eol));
        }
        assert_eq!(LineEnding::parse(" CRLF "), Some(LineEnding::Crlf));
        assert_eq!(LineEnding::parse("auto"), None);
        assert_eq!(LineEnding::parse(""), None);
    }

    #[test]
    fn mixed_line_endings_are_counted() {
        let counts = EolCounts::of(b"a\r\nb\nc\r\nd\re\r\n");
        assert_eq!(
            counts,
            EolCounts {
                crlf: 3,
                lf: 1,
                cr: 1
            }
        );
        assert!(counts.mixed());
        assert_eq!(counts.majority(), Some(LineEnding::Crlf));
        assert_eq!(
            counts.kinds(),
            vec![LineEnding::Crlf, LineEnding::Lf, LineEnding::Cr]
        );
        // 1 種類だけ・改行なしは混在ではない。CR LF を CR と LF の 2 つに数えない
        assert!(!EolCounts::of(b"a\r\nb\r\n").mixed());
        assert_eq!(EolCounts::of(b"a\r\nb\r\n").kinds(), vec![LineEnding::Crlf]);
        assert!(!EolCounts::of(b"no newline").mixed());
        assert_eq!(EolCounts::of(b"").majority(), None);
        // 文末が CR で終わっていても次の文字を読みにいかない
        assert_eq!(EolCounts::of(b"a\r").cr, 1);
    }

    #[test]
    fn line_endings_are_converted_whatever_they_were() {
        // 混ざっていても、どの改行も 1 つとして数えてそろえる (CR LF を 2 つにしない)
        let mixed = "a\r\nb\nc\rd\r\n\r\ne\n\nf\r\rg";
        assert_eq!(LineEnding::Lf.convert(mixed), "a\nb\nc\nd\n\ne\n\nf\n\ng");
        assert_eq!(
            LineEnding::Crlf.convert(mixed),
            "a\r\nb\r\nc\r\nd\r\n\r\ne\r\n\r\nf\r\n\r\ng"
        );
        assert_eq!(LineEnding::Cr.convert(mixed), "a\rb\rc\rd\r\re\r\rf\r\rg");
        // 改行が無い・末尾の改行・日本語
        assert_eq!(LineEnding::Crlf.convert("改行なし"), "改行なし");
        assert_eq!(LineEnding::Crlf.convert("日本語\n"), "日本語\r\n");
        assert_eq!(LineEnding::Lf.convert(""), "");
        for eol in LineEnding::ALL {
            // そろえた後はその改行コードだけになり、もう一度かけても変わらない
            let once = eol.convert(mixed);
            assert_eq!(LineEnding::detect(&once), Some(eol));
            assert_eq!(eol.convert(&once), once);
        }
    }

    #[test]
    fn read_only_files_are_not_replaced() {
        let dir = temp_dir("readonly");
        let path = dir.join("r.txt");
        fs::write(&path, b"keep").unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&path, perms.clone()).unwrap();
        for atomic in [true, false] {
            let err = write_with(&path, b"overwritten", atomic).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::PermissionDenied, "{atomic}");
            assert_eq!(fs::read(&path).unwrap(), b"keep");
        }
        // 一時ファイルも残さない
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        fs::set_permissions(&path, perms).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ids_round_trip() {
        for e in Encoding::all() {
            assert_eq!(Encoding::parse(&e.id()), Some(e), "{}", e.id());
        }
        assert_eq!(Encoding::parse("nonsense"), None);
    }

    /// 実フォルダでの繰り返し書き込み (ウイルス対策ソフトなどとの競合を見る)。
    /// TYPBALL_STRESS_DIR にフォルダを指定して `cargo test -- --ignored --nocapture` で実行する
    #[test]
    #[ignore]
    fn stress_write_keeps_the_file_present() {
        let Ok(dir) = std::env::var("TYPBALL_STRESS_DIR") else {
            return;
        };
        let path = Path::new(&dir).join("stress.txt");
        let mut missing = 0;
        for i in 0..300 {
            let body = format!("iteration {i}\n").repeat(50);
            write(&path, body.as_bytes()).unwrap();
            if !path.exists() {
                missing += 1;
            }
            assert_eq!(fs::read(&path).unwrap(), body.as_bytes());
        }
        eprintln!("missing after write: {missing}");
        assert_eq!(missing, 0);
    }

    #[test]
    fn write_replaces_existing_file_and_creates_new_one() {
        let dir = temp_dir("write");
        let path = dir.join("a.txt");
        write(&path, b"first").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"first");
        write(&path, b"second, longer").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second, longer");
        // 一時ファイルは残らない
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn direct_write_creates_no_other_files() {
        let dir = temp_dir("direct");
        let path = dir.join("a.txt");
        write_with(&path, b"first, longer", false).unwrap();
        write_with(&path, b"second", false).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("a.txt")]);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Windows: 隠し属性のファイルは CREATE_ALWAYS で開けない。どちらの書き方でも書けること
    #[cfg(windows)]
    #[test]
    fn hidden_files_can_be_written_both_ways() {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileAttributesW, SetFileAttributesW, FILE_ATTRIBUTE_HIDDEN,
        };
        let dir = temp_dir("hidden");
        let path = dir.join("h.txt");
        fs::write(&path, b"old").unwrap();
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        assert_ne!(
            unsafe { SetFileAttributesW(wide.as_ptr(), FILE_ATTRIBUTE_HIDDEN) },
            0
        );
        write_with(&path, b"direct", false).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"direct");
        write_with(&path, b"atomic", true).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"atomic");
        // 入れ替えでも属性は元のまま
        let attrs = unsafe { GetFileAttributesW(wide.as_ptr()) };
        assert_ne!(
            attrs & FILE_ATTRIBUTE_HIDDEN,
            0,
            "hidden attribute was lost"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_into_missing_directory_fails_without_side_effects() {
        let dir = temp_dir("missing");
        let path = dir.join("no-such-dir").join("a.txt");
        assert!(write(&path, b"x").is_err());
        assert!(!dir.join("no-such-dir").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
