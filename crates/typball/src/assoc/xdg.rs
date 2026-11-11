//! Linux など (freedesktop.org の決まりに従うデスクトップ) の関連付け
//!
//! ファイルの種類は MIME タイプで表し、既定のアプリは「MIME タイプ → .desktop ファイル」の対応で決まる:
//!
//! - `~/.local/share/applications/typball.desktop`: アプリの登録。`Exec=` に今動いている実行ファイル、
//!   `MimeType=` に開ける種類を書く (ここに載ると「別のアプリで開く」の一覧に出る)
//! - `~/.config/mimeapps.list` の `[Default Applications]`: 種類ごとの既定のアプリ
//! - `~/.local/share/mime/packages/typball-typst.xml`: `.typ` の MIME タイプ (`text/x-typst`) の定義。
//!   システムが `*.typ` の種類を知らないときだけ置く
//! - `~/.local/share/icons/hicolor/256x256/apps/typball.png`: アプリのアイコン
//!
//! ファイルのアイコンは MIME タイプとアイコンテーマで決まり、アプリごとには変えられないので、
//! アイコンの選択は無い。
//!
//! 実機では未検証 (Windows で開発しているため)。ファイルの読み書きは [`Dirs`] で場所を差し替えて
//! テストしている。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{same_file, Capabilities, DefaultApp, Icon, Kind, Labels, Registered, Status};

const DESKTOP_ID: &str = "typball.desktop";
const APP_NAME: &str = "Typball";
const ICON_NAME: &str = "typball";
/// `.typ` の MIME タイプを自分で定義するときのファイル名
const TYPST_PACKAGE: &str = "typball-typst.xml";
const APP_ICON_SIZE: u32 = 256;
static APP_ICON_RGBA: &[u8] = include_bytes!("../../assets/icons/Typball-256.rgba");
/// アイコンは選べない
pub const ICON_WILDCARD: &str = "";

pub fn target() -> Option<PathBuf> {
    // AppImage の中身は起動のたびに違う場所へマウントされるので、AppImage そのものを登録する
    std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
}

pub fn capabilities() -> Capabilities {
    Capabilities {
        unavailable: None,
        icons: false,
        system_settings: false,
        note: "settings.assoc.note_linux",
    }
}

pub fn status(kind: Kind) -> Status {
    Dirs::from_env().status(kind, &target().unwrap_or_default())
}

pub fn register(kind: Kind, _icon: &Icon, _labels: &Labels) -> Result<(), String> {
    let exe = target().ok_or("current_exe")?;
    let dirs = Dirs::from_env();
    dirs.register(kind, &exe).map_err(|e| e.to_string())?;
    dirs.refresh_caches();
    Ok(())
}

pub fn set_icon(_kind: Kind, _icon: &Icon) -> Result<(), String> {
    Ok(())
}

pub fn unregister(kind: Kind) -> Result<(), String> {
    let dirs = Dirs::from_env();
    dirs.unregister(kind).map_err(|e| e.to_string())?;
    dirs.refresh_caches();
    Ok(())
}

pub fn open_system_settings() {}

pub fn icon_rgba(_kind: Kind, _icon: &Icon, _size: u32) -> Option<Vec<u8>> {
    None
}

pub fn custom_icon_value(path: &Path) -> String {
    path.display().to_string()
}

/// XDG のディレクトリ。テストでは一時ディレクトリに差し替える
struct Dirs {
    /// `$XDG_DATA_HOME` (`~/.local/share`)
    data: PathBuf,
    /// `$XDG_CONFIG_HOME` (`~/.config`)
    config: PathBuf,
    /// `$XDG_DATA_DIRS` (`/usr/local/share:/usr/share`)。読むだけ
    system_data: Vec<PathBuf>,
}

/// typball.desktop の中身のうち、ここで使うもの
#[derive(Debug, PartialEq, Eq)]
struct Desktop {
    exec: PathBuf,
    mimes: Vec<String>,
}

impl Dirs {
    fn from_env() -> Dirs {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        // 仕様: 空や相対パスの値は無視して既定を使う
        let dir = |name: &str, default: PathBuf| {
            std::env::var_os(name)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or(default)
        };
        let system_data = std::env::var("XDG_DATA_DIRS")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".to_string())
            .split(':')
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .collect();
        Dirs {
            data: dir("XDG_DATA_HOME", home.join(".local/share")),
            config: dir("XDG_CONFIG_HOME", home.join(".config")),
            system_data,
        }
    }

    fn desktop_path(&self) -> PathBuf {
        self.data.join("applications").join(DESKTOP_ID)
    }

    fn typst_package_path(&self) -> PathBuf {
        self.data.join("mime/packages").join(TYPST_PACKAGE)
    }

    fn mimeapps_path(&self) -> PathBuf {
        self.config.join("mimeapps.list")
    }

    /// データのディレクトリを優先順に (利用者のもの → システムのもの)
    fn data_dirs(&self) -> impl Iterator<Item = &PathBuf> {
        std::iter::once(&self.data).chain(&self.system_data)
    }

    /// この形式の MIME タイプ。`.typ` はシステムが別の名前で知っていればそれに合わせる
    fn mime(&self, kind: Kind) -> String {
        if kind == Kind::Typ {
            let known = self
                .system_data
                .iter()
                .filter_map(|dir| fs::read_to_string(dir.join("mime/globs2")).ok())
                .find_map(|globs| mime_for_glob(&globs, "*.typ"));
            if let Some(mime) = known {
                return mime;
            }
        }
        kind.mime().to_string()
    }

    fn read_desktop(&self) -> Option<Desktop> {
        parse_desktop(&fs::read_to_string(self.desktop_path()).ok()?)
    }

    fn register(&self, kind: Kind, exe: &Path) -> std::io::Result<()> {
        let mime = self.mime(kind);
        // .typ の種類をシステムが知らなければ定義を置く
        if kind == Kind::Typ && mime == Kind::Typ.mime() {
            write_file(&self.typst_package_path(), TYPST_MIME_XML.as_bytes())?;
        }
        let mut mimes = self.read_desktop().map(|d| d.mimes).unwrap_or_default();
        if !mimes.contains(&mime) {
            mimes.push(mime.clone());
        }
        write_file(&self.desktop_path(), desktop_entry(exe, &mimes).as_bytes())?;
        let icon = self
            .data
            .join(format!(
                "icons/hicolor/{APP_ICON_SIZE}x{APP_ICON_SIZE}/apps"
            ))
            .join(format!("{ICON_NAME}.png"));
        if !icon.exists() {
            write_file(
                &icon,
                &png_rgba(APP_ICON_SIZE, APP_ICON_SIZE, APP_ICON_RGBA),
            )?;
        }
        self.edit_mimeapps(|text| set_default(text, &mime, DESKTOP_ID))
    }

    fn unregister(&self, kind: Kind) -> std::io::Result<()> {
        let mime = self.mime(kind);
        if let Some(desktop) = self.read_desktop() {
            let mimes: Vec<String> = desktop.mimes.into_iter().filter(|m| *m != mime).collect();
            if mimes.is_empty() {
                remove_file(&self.desktop_path())?;
            } else {
                write_file(
                    &self.desktop_path(),
                    desktop_entry(&desktop.exec, &mimes).as_bytes(),
                )?;
            }
        }
        if kind == Kind::Typ {
            remove_file(&self.typst_package_path())?;
        }
        if self.mimeapps_path().exists() {
            self.edit_mimeapps(|text| remove_app(text, &mime, DESKTOP_ID))?;
        }
        Ok(())
    }

    fn edit_mimeapps(&self, edit: impl Fn(&str) -> String) -> std::io::Result<()> {
        let path = self.mimeapps_path();
        let old = fs::read_to_string(&path).unwrap_or_default();
        let new = edit(&old);
        if new != old {
            write_file(&path, new.as_bytes())?;
        }
        Ok(())
    }

    /// この種類の既定のアプリ (.desktop の ID)。利用者の設定 → 以前の置き場所 → システムのキャッシュの順
    fn default_id(&self, mime: &str) -> Option<String> {
        let lists = [
            self.mimeapps_path(),
            self.data.join("applications/mimeapps.list"),
        ];
        let user = lists.iter().find_map(|path| {
            let text = fs::read_to_string(path).ok()?;
            ini_value(&text, "Default Applications", mime)?
                .split(';')
                .find(|id| !id.is_empty() && self.find_desktop(id).is_some())
                .map(str::to_string)
        });
        user.or_else(|| {
            self.data_dirs().find_map(|dir| {
                let text = fs::read_to_string(dir.join("applications/mimeinfo.cache")).ok()?;
                ini_value(&text, "MIME Cache", mime)?
                    .split(';')
                    .find(|id| !id.is_empty())
                    .map(str::to_string)
            })
        })
    }

    /// .desktop の ID からファイルを探す
    fn find_desktop(&self, id: &str) -> Option<PathBuf> {
        self.data_dirs()
            .map(|dir| dir.join("applications").join(id))
            .find(|path| path.is_file())
    }

    fn status(&self, kind: Kind, exe: &Path) -> Status {
        let mime = self.mime(kind);
        let desktop = self.read_desktop();
        let registered = match &desktop {
            Some(d) if d.mimes.contains(&mime) => {
                if same_file(&d.exec, exe) {
                    Registered::This
                } else {
                    Registered::Other(d.exec.clone())
                }
            }
            _ => Registered::No,
        };
        let default_app = self.default_id(&mime).map(|id| {
            let ours = id == DESKTOP_ID;
            let file = self.find_desktop(&id);
            let name = file
                .as_deref()
                .and_then(|path| fs::read_to_string(path).ok())
                .and_then(|text| ini_value(&text, "Desktop Entry", "Name").map(str::to_string))
                .unwrap_or_else(|| id.trim_end_matches(".desktop").to_string());
            DefaultApp {
                name,
                path: if ours {
                    desktop.as_ref().map(|d| d.exec.clone())
                } else {
                    file
                },
                is_self: ours && registered == Registered::This,
            }
        });
        Status {
            removable: registered != Registered::No,
            default_app,
            registered,
            icon: Icon::Document,
        }
    }

    /// デスクトップ環境のキャッシュを作り直す。道具が無い環境では何もしない (無くても mimeapps.list は効く)
    fn refresh_caches(&self) {
        let _ = Command::new("update-mime-database")
            .arg(self.data.join("mime"))
            .status();
        let _ = Command::new("update-desktop-database")
            .arg(self.data.join("applications"))
            .status();
    }
}

const TYPST_MIME_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<mime-info xmlns="http://www.freedesktop.org/standards/shared-mime-info">
  <mime-type type="text/x-typst">
    <comment>Typst document</comment>
    <sub-class-of type="text/plain"/>
    <glob pattern="*.typ"/>
  </mime-type>
</mime-info>
"#;

/// typball.desktop の中身
fn desktop_entry(exe: &Path, mimes: &[String]) -> String {
    let mime_list: String = mimes.iter().map(|m| format!("{m};")).collect();
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={APP_NAME}\n\
         GenericName=Text Editor\n\
         Comment=Tabbed text editor with live preview for Typst and Markdown\n\
         Exec={} %F\n\
         Icon={ICON_NAME}\n\
         Terminal=false\n\
         Categories=Utility;TextEditor;\n\
         MimeType={mime_list}\n",
        quote_exec(&exe.display().to_string())
    )
}

/// Exec の引数 1 つを引用符で囲む (Desktop Entry の仕様: `"` `` ` `` `$` `\` は `\` で逃がす。
/// さらに値全体が文字列型なので `\` は 2 重になる)
fn quote_exec(arg: &str) -> String {
    let mut out = String::from("\"");
    for c in arg.chars() {
        match c {
            '\\' => out.push_str(r"\\\\"),
            '"' | '`' | '$' => {
                out.push_str(r"\\");
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// [`quote_exec`] で書いた Exec の先頭の引数 (実行ファイル) を取り出す。引用符無しの値も読む
fn exec_program(exec: &str) -> Option<PathBuf> {
    let exec = exec.trim();
    let program = match exec.strip_prefix('"') {
        Some(rest) => {
            let mut out = String::new();
            let mut chars = rest.chars().peekable();
            loop {
                match chars.next()? {
                    '"' => break,
                    '\\' => {
                        // `\\\\` → `\`、`\\"` → `"` (文字列型の逃がしと引用符の中の逃がしの 2 段)
                        if chars.peek() == Some(&'\\') {
                            chars.next();
                        }
                        match chars.next()? {
                            '\\' if chars.peek() == Some(&'\\') => {
                                chars.next();
                                out.push('\\');
                            }
                            c => out.push(c),
                        }
                    }
                    c => out.push(c),
                }
            }
            out
        }
        None => exec.split_whitespace().next()?.to_string(),
    };
    (!program.is_empty()).then(|| PathBuf::from(program))
}

fn parse_desktop(text: &str) -> Option<Desktop> {
    let exec = exec_program(ini_value(text, "Desktop Entry", "Exec")?)?;
    let mimes = ini_value(text, "Desktop Entry", "MimeType")
        .unwrap_or("")
        .split(';')
        .filter(|m| !m.is_empty())
        .map(str::to_string)
        .collect();
    Some(Desktop { exec, mimes })
}

/// `[section]` の中の `key=値` を読む (.desktop / mimeapps.list / mimeinfo.cache 共通の書式)
fn ini_value<'a>(text: &'a str, section: &str, key: &str) -> Option<&'a str> {
    let mut inside = false;
    for line in text.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            inside = name == section;
        } else if inside {
            if let Some((k, v)) = line.split_once('=') {
                if k.trim() == key {
                    return Some(v.trim());
                }
            }
        }
    }
    None
}

/// mimeapps.list の `[Default Applications]` で `mime` の既定を `id` にする (他の行はそのまま残す)。
/// 前から載っていたアプリは後ろに残す (`id` が消えたときの次の候補になる)
fn set_default(text: &str, mime: &str, id: &str) -> String {
    const SECTION: &str = "[Default Applications]";
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let start = match lines.iter().position(|l| l.trim() == SECTION) {
        Some(i) => i,
        None => {
            if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push(SECTION.to_string());
            lines.len() - 1
        }
    };
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.trim_start().starts_with('['))
        .map_or(lines.len(), |i| start + 1 + i);
    let existing = (start + 1..end).find(|i| {
        lines[*i]
            .split_once('=')
            .is_some_and(|(k, _)| k.trim() == mime)
    });
    match existing {
        Some(i) => {
            let others: Vec<&str> = lines[i]
                .split_once('=')
                .map(|(_, v)| v)
                .unwrap_or("")
                .split(';')
                .map(str::trim)
                .filter(|v| !v.is_empty() && *v != id)
                .collect();
            let mut value = format!("{id};");
            for other in others {
                value.push_str(other);
                value.push(';');
            }
            lines[i] = format!("{mime}={value}");
        }
        None => {
            // セクションの末尾 (空行の前) に足す
            let mut at = end;
            while at > start + 1 && lines[at - 1].trim().is_empty() {
                at -= 1;
            }
            lines.insert(at, format!("{mime}={id};"));
        }
    }
    lines.join("\n") + "\n"
}

/// mimeapps.list から `mime` に対する `id` を消す (既定・追加した関連付けのどちらからも)。
/// 他のアプリが残らない行は行ごと消す
fn remove_app(text: &str, mime: &str, id: &str) -> String {
    let mut out = Vec::new();
    let mut section = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            section = trimmed.to_string();
        } else if section == "[Default Applications]" || section == "[Added Associations]" {
            if let Some((key, value)) = trimmed.split_once('=') {
                if key.trim() == mime && value.split(';').any(|v| v.trim() == id) {
                    let rest: Vec<&str> = value
                        .split(';')
                        .map(str::trim)
                        .filter(|v| !v.is_empty() && *v != id)
                        .collect();
                    if !rest.is_empty() {
                        out.push(format!("{mime}={};", rest.join(";")));
                    }
                    continue;
                }
            }
        }
        out.push(line.to_string());
    }
    if out.is_empty() {
        String::new()
    } else {
        out.join("\n") + "\n"
    }
}

/// shared-mime-info の globs2 (`重み:MIME タイプ:パターン`) から、パターンに一番重く対応する種類を探す
fn mime_for_glob(globs2: &str, pattern: &str) -> Option<String> {
    globs2
        .lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let mut parts = line.splitn(4, ':');
            let weight: u32 = parts.next()?.parse().ok()?;
            let mime = parts.next()?;
            (parts.next()? == pattern).then_some((weight, mime))
        })
        .max_by_key(|(weight, _)| *weight)
        .map(|(_, mime)| mime.to_string())
}

fn write_file(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, data)
}

fn remove_file(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// PNG の書き出し (アプリのアイコン用。圧縮しない)
// ---------------------------------------------------------------------------

/// RGBA を無圧縮の PNG にする。アイコン 1 枚のためだけに PNG のライブラリを増やさない
fn png_rgba(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    // 各行の先頭にフィルタの種類 (0 = なし) を付ける
    let mut raw = Vec::with_capacity(rgba.len() + height as usize);
    for row in rgba.chunks_exact(width as usize * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    // zlib: ヘッダ + 無圧縮ブロック (最大 65535 バイト) の並び + Adler-32
    let mut zlib = vec![0x78, 0x01];
    let mut blocks = raw.chunks(0xffff).peekable();
    while let Some(block) = blocks.next() {
        zlib.push(u8::from(blocks.peek().is_none()));
        let len = block.len() as u16;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    // 8 ビット・RGBA (色の種類 6)・圧縮 / フィルタ / インターレースは既定
    header.extend_from_slice(&[8, 6, 0, 0, 0]);

    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    for (name, data) in [
        (b"IHDR", header.as_slice()),
        (b"IDAT", zlib.as_slice()),
        (b"IEND", &[][..]),
    ] {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = out.len();
        out.extend_from_slice(name);
        out.extend_from_slice(data);
        let crc = crc32(&out[start..]);
        out.extend_from_slice(&crc.to_be_bytes());
    }
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in data {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for byte in data {
        a = (a + *byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    /// 一時ディレクトリの XDG 環境。終わったら消す
    struct TestDirs {
        root: PathBuf,
        dirs: Dirs,
    }

    impl TestDirs {
        fn new() -> TestDirs {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let root = std::env::temp_dir().join(format!(
                "typball-xdg-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let dirs = Dirs {
                data: root.join("home/.local/share"),
                config: root.join("home/.config"),
                system_data: vec![root.join("usr/share")],
            };
            TestDirs { root, dirs }
        }

        fn system_file(&self, relative: &str, text: &str) {
            write_file(&self.root.join("usr/share").join(relative), text.as_bytes()).unwrap();
        }

        fn mimeapps(&self) -> String {
            fs::read_to_string(self.dirs.mimeapps_path()).unwrap_or_default()
        }
    }

    impl Drop for TestDirs {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn register_writes_desktop_entry_and_default() {
        let t = TestDirs::new();
        let exe = Path::new("/opt/My Apps/Typball");
        t.dirs.register(Kind::Md, exe).unwrap();

        let desktop = t.dirs.read_desktop().unwrap();
        assert_eq!(desktop.exec, exe);
        assert_eq!(desktop.mimes, ["text/markdown"]);
        assert_eq!(
            t.mimeapps(),
            "[Default Applications]\ntext/markdown=typball.desktop;\n"
        );
        assert!(t
            .dirs
            .data
            .join("icons/hicolor/256x256/apps/typball.png")
            .is_file());
        // Markdown だけなら .typ の種類の定義は置かない
        assert!(!t.dirs.typst_package_path().exists());

        let status = t.dirs.status(Kind::Md, exe);
        assert_eq!(status.registered, Registered::This);
        assert!(status.removable);
        let app = status.default_app.unwrap();
        assert_eq!(app.name, "Typball");
        assert!(app.is_self);
        // 別の場所の Typball から見ると「別の場所」で、自分ではない
        let other = t.dirs.status(Kind::Md, Path::new("/home/me/dev/Typball"));
        assert_eq!(other.registered, Registered::Other(exe.to_path_buf()));
        assert!(!other.default_app.unwrap().is_self);
        assert_eq!(t.dirs.status(Kind::Txt, exe).registered, Registered::No);
    }

    #[test]
    fn typst_gets_its_own_mime_type_unless_the_system_knows_one() {
        let t = TestDirs::new();
        let exe = Path::new("/usr/bin/Typball");
        t.dirs.register(Kind::Typ, exe).unwrap();
        assert!(t.dirs.typst_package_path().is_file());
        assert_eq!(t.dirs.read_desktop().unwrap().mimes, ["text/x-typst"]);
        t.dirs.unregister(Kind::Typ).unwrap();
        assert!(!t.dirs.typst_package_path().exists());
        assert!(!t.dirs.desktop_path().exists());

        t.system_file(
            "mime/globs2",
            "# comment\n50:text/x-typ-old:*.typ\n60:text/vnd.typst:*.typ\n50:text/plain:*.txt\n",
        );
        assert_eq!(t.dirs.mime(Kind::Typ), "text/vnd.typst");
        t.dirs.register(Kind::Typ, exe).unwrap();
        assert!(!t.dirs.typst_package_path().exists());
        assert_eq!(t.dirs.read_desktop().unwrap().mimes, ["text/vnd.typst"]);
    }

    #[test]
    fn unregister_keeps_other_types_and_other_apps() {
        let t = TestDirs::new();
        let exe = Path::new("/usr/bin/Typball");
        write_file(
            &t.dirs.mimeapps_path(),
            b"[Added Associations]\ntext/plain=typball.desktop;gedit.desktop;\n\n\
              [Default Applications]\ntext/plain=gedit.desktop;\nimage/png=eog.desktop;\n",
        )
        .unwrap();
        t.dirs.register(Kind::Txt, exe).unwrap();
        t.dirs.register(Kind::Md, exe).unwrap();
        assert_eq!(
            t.mimeapps(),
            "[Added Associations]\ntext/plain=typball.desktop;gedit.desktop;\n\n\
             [Default Applications]\ntext/plain=typball.desktop;gedit.desktop;\nimage/png=eog.desktop;\n\
             text/markdown=typball.desktop;\n"
        );

        t.dirs.unregister(Kind::Txt).unwrap();
        assert_eq!(
            t.mimeapps(),
            "[Added Associations]\ntext/plain=gedit.desktop;\n\n\
             [Default Applications]\ntext/plain=gedit.desktop;\nimage/png=eog.desktop;\n\
             text/markdown=typball.desktop;\n"
        );
        assert_eq!(t.dirs.read_desktop().unwrap().mimes, ["text/markdown"]);
        assert_eq!(t.dirs.status(Kind::Txt, exe).registered, Registered::No);

        t.dirs.unregister(Kind::Md).unwrap();
        assert!(!t.dirs.desktop_path().exists());
        assert!(!t.mimeapps().contains("typball"));
    }

    #[test]
    fn default_app_falls_back_to_the_system_cache() {
        let t = TestDirs::new();
        t.system_file(
            "applications/mimeinfo.cache",
            "[MIME Cache]\ntext/plain=org.gnome.TextEditor.desktop;vim.desktop;\n",
        );
        t.system_file(
            "applications/org.gnome.TextEditor.desktop",
            "[Desktop Entry]\nName=Text Editor\nExec=gnome-text-editor %U\n",
        );
        let status = t.dirs.status(Kind::Txt, Path::new("/usr/bin/Typball"));
        let app = status.default_app.unwrap();
        assert_eq!(app.name, "Text Editor");
        assert!(!app.is_self);
        assert_eq!(status.registered, Registered::No);
        assert!(!status.removable);
        // 利用者の設定があればそちらが優先。消えたアプリは飛ばす
        write_file(
            &t.dirs.mimeapps_path(),
            b"[Default Applications]\ntext/plain=gone.desktop;vim.desktop;\n",
        )
        .unwrap();
        t.system_file("applications/vim.desktop", "[Desktop Entry]\nName=Vim\n");
        let status = t.dirs.status(Kind::Txt, Path::new("/usr/bin/Typball"));
        assert_eq!(status.default_app.unwrap().name, "Vim");
    }

    #[test]
    fn exec_quoting_round_trips() {
        for path in [
            "/usr/bin/Typball",
            "/opt/My Apps/Typball",
            r#"/odd/a"b$c`d\e/Typball"#,
            r"C:\Program Files\Typball\Typball.exe",
        ] {
            let exec = format!("{} %F", quote_exec(path));
            assert_eq!(exec_program(&exec), Some(PathBuf::from(path)), "{exec}");
        }
        assert_eq!(quote_exec("/a b/c"), r#""/a b/c""#);
        assert_eq!(
            exec_program("/usr/bin/typball --flag %U"),
            Some(PathBuf::from("/usr/bin/typball"))
        );
        assert_eq!(exec_program(""), None);
    }

    #[test]
    fn checksums_match_known_values() {
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
    }

    #[test]
    fn png_has_the_expected_layout() {
        // 2x1: 赤、半透明の青
        let png = png_rgba(2, 1, &[255, 0, 0, 255, 0, 0, 255, 128]);
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR"));
        assert!(png.ends_with(b"\x00\x00\x00\x00IEND\xae\x42\x60\x82"));
        // IDAT: zlib ヘッダ・最後のブロック・長さ 9 とその補数・フィルタ 0 + 画素
        let idat = png.windows(4).position(|w| w == b"IDAT").unwrap() + 4;
        assert_eq!(
            &png[idat..idat + 16],
            [0x78, 0x01, 1, 9, 0, 0xf6, 0xff, 0, 255, 0, 0, 255, 0, 0, 255, 128]
        );
        // アプリのアイコン (64 KB を超えるので複数ブロックになる)
        let icon = png_rgba(APP_ICON_SIZE, APP_ICON_SIZE, APP_ICON_RGBA);
        assert!(icon.len() > APP_ICON_RGBA.len());
    }
}
