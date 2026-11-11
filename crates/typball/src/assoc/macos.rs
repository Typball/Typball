//! macOS の関連付け (Launch Services)
//!
//! macOS では「開ける書類の種類」はアプリ (.app) の Info.plist が宣言する (`CFBundleDocumentTypes`。
//! CI の .app を組み立てる手順が書く)。アプリが決められるのは「その種類の既定のアプリを自分にする」ことだけで、
//! Finder の「情報を見る → このアプリケーションで開く → すべてを変更」と同じ設定を書き換える。
//!
//! - 種類は UTI (Uniform Type Identifier) で指す。拡張子から `UTTypeCreatePreferredIdentifierForTag` で引く
//!   (.txt → public.plain-text、.md → net.daringfireball.markdown、.typ → Info.plist が宣言した種類。
//!   宣言がまだ Launch Services に届いていなければ `dyn.` で始まる仮の種類になるが、それでも設定できる)
//! - .app として動いていないとき (cargo run で作った実行ファイルを直接動かしたとき) はバンドル ID が無く、
//!   関連付けられない
//! - 「解除」に当たる操作は無い (必ず何かのアプリが既定になる) ので、テキストエディットに戻す
//! - 書類のアイコンは .app の中の icns で決まり、利用者ごとには変えられない
//!
//! 使っている LS* / UTType* の関数は macOS 12 で非推奨になったが、置き換えの NSWorkspace の API は
//! Objective-C からしか呼べないので、C の関数のままにしている。
//!
//! 実機では未検証 (Windows で開発しているため)。

use std::ffi::{c_char, c_void, CStr};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};

use super::{Capabilities, DefaultApp, Icon, Kind, Labels, Registered, Status};

type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFArrayRef = *const c_void;
type CFURLRef = *const c_void;
type CFIndex = isize;

const UTF8: u32 = 0x0800_0100; // kCFStringEncodingUTF8
const ROLES_ALL: u32 = 0xffff_ffff; // kLSRolesAll
const POSIX_PATH: CFIndex = 0; // kCFURLPOSIXPathStyle
/// 解除したときに戻す先 (macOS に必ず入っている)
const FALLBACK_HANDLER: &str = "com.apple.TextEdit";
/// アイコンは選べない
pub const ICON_WILDCARD: &str = "";

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: CFTypeRef);
    fn CFStringCreateWithBytes(
        allocator: CFTypeRef,
        bytes: *const u8,
        length: CFIndex,
        encoding: u32,
        is_external: u8,
    ) -> CFStringRef;
    fn CFStringGetLength(string: CFStringRef) -> CFIndex;
    fn CFStringGetMaximumSizeForEncoding(length: CFIndex, encoding: u32) -> CFIndex;
    fn CFStringGetCString(
        string: CFStringRef,
        buffer: *mut c_char,
        size: CFIndex,
        encoding: u32,
    ) -> u8;
    fn CFArrayGetCount(array: CFArrayRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(array: CFArrayRef, index: CFIndex) -> *const c_void;
    fn CFBundleGetMainBundle() -> CFTypeRef;
    fn CFBundleGetIdentifier(bundle: CFTypeRef) -> CFStringRef;
    fn CFURLCopyFileSystemPath(url: CFURLRef, style: CFIndex) -> CFStringRef;
}

#[link(name = "CoreServices", kind = "framework")]
extern "C" {
    static kUTTagClassFilenameExtension: CFStringRef;
    fn UTTypeCreatePreferredIdentifierForTag(
        tag_class: CFStringRef,
        tag: CFStringRef,
        conforming_to: CFStringRef,
    ) -> CFStringRef;
    fn LSCopyDefaultRoleHandlerForContentType(content_type: CFStringRef, role: u32) -> CFStringRef;
    fn LSSetDefaultRoleHandlerForContentType(
        content_type: CFStringRef,
        role: u32,
        handler_bundle_id: CFStringRef,
    ) -> i32;
    fn LSCopyApplicationURLsForBundleIdentifier(
        bundle_id: CFStringRef,
        error: *mut CFTypeRef,
    ) -> CFArrayRef;
}

/// 自分で解放する CFString (Create / Copy の規則で受け取ったもの)
struct CfString(CFStringRef);

impl CfString {
    fn new(text: &str) -> Option<CfString> {
        // SAFETY: text の範囲のバイト列を渡すだけ。返り値は Create の規則で自分のもの
        let string = unsafe {
            CFStringCreateWithBytes(null(), text.as_ptr(), text.len() as CFIndex, UTF8, 0)
        };
        CfString::owned(string)
    }

    /// Create / Copy 系の関数の返り値を受け取る (NULL なら None)
    fn owned(string: CFStringRef) -> Option<CfString> {
        (!string.is_null()).then_some(CfString(string))
    }

    fn text(&self) -> Option<String> {
        cf_to_string(self.0)
    }
}

impl Drop for CfString {
    fn drop(&mut self) {
        // SAFETY: 自分が持っている参照を 1 つ手放す
        unsafe { CFRelease(self.0) };
    }
}

/// CFString を Rust の文字列にする (解放はしない)
fn cf_to_string(string: CFStringRef) -> Option<String> {
    if string.is_null() {
        return None;
    }
    // SAFETY: string は生きている CFString。バッファは UTF-8 にしたときの最大長 + 終端ぶん確保している
    unsafe {
        let size = CFStringGetMaximumSizeForEncoding(CFStringGetLength(string), UTF8) + 1;
        let mut buffer = vec![0u8; size.max(1) as usize];
        if CFStringGetCString(string, buffer.as_mut_ptr().cast(), size, UTF8) == 0 {
            return None;
        }
        Some(
            CStr::from_ptr(buffer.as_ptr().cast())
                .to_string_lossy()
                .into_owned(),
        )
    }
}

/// 動いている .app のバンドル ID (.app として動いていなければ None)
fn bundle_id() -> Option<String> {
    bundle_path()?;
    // SAFETY: メインバンドルとその ID は Get の規則 (解放しない)。NULL は cf_to_string が弾く
    unsafe {
        let bundle = CFBundleGetMainBundle();
        if bundle.is_null() {
            return None;
        }
        cf_to_string(CFBundleGetIdentifier(bundle))
    }
}

/// 動いている実行ファイルを含む .app (`Typball.app/Contents/MacOS/Typball` の先頭)
fn bundle_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .map(Path::to_path_buf)
}

/// 拡張子に対応する種類 (UTI)
fn content_type(kind: Kind) -> Option<CfString> {
    let ext = CfString::new(kind.ext())?;
    // SAFETY: kUTTagClassFilenameExtension はフレームワークの定数。返り値は Create の規則で自分のもの
    CfString::owned(unsafe {
        UTTypeCreatePreferredIdentifierForTag(kUTTagClassFilenameExtension, ext.0, null())
    })
}

/// 種類の既定のアプリのバンドル ID
fn default_handler(kind: Kind) -> Option<String> {
    let uti = content_type(kind)?;
    // SAFETY: uti は生きている CFString。返り値は Copy の規則で自分のもの
    CfString::owned(unsafe { LSCopyDefaultRoleHandlerForContentType(uti.0, ROLES_ALL) })?.text()
}

/// バンドル ID からアプリ (.app) の場所を探す
fn app_path(bundle_id: &str) -> Option<PathBuf> {
    let id = CfString::new(bundle_id)?;
    // SAFETY: 返り値の配列は Copy の規則で自分のもの (最後に解放する)。要素の URL は配列が持っている
    unsafe {
        let urls = LSCopyApplicationURLsForBundleIdentifier(id.0, null_mut());
        if urls.is_null() {
            return None;
        }
        let path = if CFArrayGetCount(urls) > 0 {
            let url = CFArrayGetValueAtIndex(urls, 0);
            CfString::owned(CFURLCopyFileSystemPath(url, POSIX_PATH)).and_then(|s| s.text())
        } else {
            None
        };
        CFRelease(urls);
        path.map(PathBuf::from)
    }
}

fn set_handler(kind: Kind, bundle_id: &str) -> Result<(), String> {
    let uti = content_type(kind).ok_or("no content type")?;
    let id = CfString::new(bundle_id).ok_or("invalid bundle id")?;
    // SAFETY: どちらも生きている CFString
    match unsafe { LSSetDefaultRoleHandlerForContentType(uti.0, ROLES_ALL, id.0) } {
        0 => Ok(()),
        status => Err(format!("Launch Services error {status}")),
    }
}

pub fn target() -> Option<PathBuf> {
    bundle_path()
}

pub fn capabilities() -> Capabilities {
    Capabilities {
        unavailable: bundle_id()
            .is_none()
            .then_some("settings.assoc.unavailable_macos"),
        icons: false,
        system_settings: false,
        note: "settings.assoc.note_macos",
    }
}

pub fn status(kind: Kind) -> Status {
    let ours = bundle_id();
    let handler = default_handler(kind);
    // Launch Services はバンドル ID を小文字にして返すことがある
    let is_self = match (&ours, &handler) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        _ => false,
    };
    let default_app = handler.map(|id| {
        let path = app_path(&id);
        let name = path
            .as_deref()
            .and_then(Path::file_stem)
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or(id);
        DefaultApp {
            name,
            path,
            is_self,
        }
    });
    Status {
        default_app,
        registered: if is_self {
            Registered::This
        } else {
            Registered::No
        },
        removable: is_self,
        icon: Icon::Document,
    }
}

pub fn register(kind: Kind, _icon: &Icon, _labels: &Labels) -> Result<(), String> {
    let id = bundle_id().ok_or("not running from an application bundle")?;
    set_handler(kind, &id)
}

pub fn set_icon(_kind: Kind, _icon: &Icon) -> Result<(), String> {
    Ok(())
}

pub fn unregister(kind: Kind) -> Result<(), String> {
    set_handler(kind, FALLBACK_HANDLER)
}

pub fn open_system_settings() {}

pub fn icon_rgba(_kind: Kind, _icon: &Icon, _size: u32) -> Option<Vec<u8>> {
    None
}

pub fn custom_icon_value(path: &Path) -> String {
    path.display().to_string()
}
