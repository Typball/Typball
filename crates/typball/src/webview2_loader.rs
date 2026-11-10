//! WebView2 の自前ローダー (Windows / MinGW ビルド用)
//!
//! wxWidgets の Edge バックエンド (`src/msw/webview_edge.cpp`) は `wxUSE_WEBVIEW_EDGE_STATIC=ON` で
//! 組むと `WebView2Loader.dll` を読まず、`CreateCoreWebView2EnvironmentWithOptions` と
//! `GetAvailableCoreWebView2BrowserVersionString` を直接呼ぶ。Microsoft の静的ライブラリ
//! (`WebView2LoaderStatic.lib`) は MSVC の CRT に依存していて MinGW ではリンクできないので、
//! ここで同じ 2 関数を Rust で提供する。実行ファイルの隣に DLL を置かずに済む。
//!
//! やっていることはローダー DLL と同じ:
//! 1. レジストリ (`HKLM` → `HKCU` の `SOFTWARE\Microsoft\EdgeUpdate\ClientState\{安定版の GUID}`、
//!    32 bit ビュー) の値 `EBWebView` から Evergreen ランタイムのフォルダを探す。
//!    固定版ランタイムのフォルダが引数か環境変数 `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER` で
//!    渡されたときはそちらを使う
//! 2. その中の `EBWebView\x64\EmbeddedBrowserWebView.dll` を読み込み、内部関数
//!    `CreateWebViewEnvironmentWithOptionsInternal` を呼ぶ
//!
//! 手順は webview/webview (MIT) の組み込みローダー実装
//! (core/include/webview/detail/platform/windows/webview2/loader.hh) に倣った。
//!
//! MSVC ビルドでは wxdragon-sys が `WebView2LoaderStatic.lib` をリンクするので、このモジュールは
//! windows-gnu のときだけコンパイルされる (`main.rs` の `#[cfg]`)。

#![allow(non_snake_case)]

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use windows_sys::Win32::Foundation::{ERROR_SUCCESS, HMODULE, S_OK};
use windows_sys::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, VS_FIXEDFILEINFO,
};
use windows_sys::Win32::System::Com::CoTaskMemAlloc;
use windows_sys::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_WITH_ALTERED_SEARCH_PATH,
};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
    KEY_READ, KEY_WOW64_32KEY, REG_SZ,
};

/// `HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)`: ランタイムが見つからないときにローダー DLL が返す値
const E_RUNTIME_NOT_FOUND: i32 = 0x8007_0002_u32 as i32;
/// `HRESULT_FROM_WIN32(ERROR_INVALID_ARGUMENT)`
const E_INVALIDARG: i32 = 0x8007_0057_u32 as i32;
/// `E_FAIL`
const E_FAIL: i32 = 0x8000_4005_u32 as i32;

/// このローダーが受け入れるランタイムのビルド番号の下限。基本機能だけなら古いランタイムでも
/// 動くので webview/webview と同じ 1150 にしておく
const MIN_BUILD: u32 = 1150;

/// Edge 安定版チャネルの GUID (EdgeUpdate のクライアント ID)
const STABLE_CHANNEL_GUID: &str = "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";

/// ランタイム内部の関数の型。第 1 引数は常に true、第 2 引数はランタイムの種類
/// (0 = インストール済み、1 = 固定版)。残りはローダー DLL の公開関数と同じ
type CreateInternal = unsafe extern "system" fn(
    bool,
    i32,
    *const u16,
    *mut c_void, // ICoreWebView2EnvironmentOptions* (IUnknown* として渡す)
    *mut c_void, // ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler*
) -> i32;

/// 見つけたランタイム
#[derive(Debug, Clone)]
pub struct Runtime {
    /// `EmbeddedBrowserWebView.dll` のパス
    pub dll: PathBuf,
    /// 例: "131.0.2903.70"
    pub version: String,
    /// 固定版 (アプリが同梱する方式) か
    pub embedded: bool,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide_ptr(ptr: *const u16) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let mut len = 0;
    // SAFETY: NUL 終端の UTF-16 文字列が渡される前提 (Win32 の PCWSTR)
    unsafe {
        while *ptr.add(len) != 0 {
            len += 1;
        }
        let slice = std::slice::from_raw_parts(ptr, len);
        Some(String::from_utf16_lossy(slice))
    }
}

/// `<dir>\EBWebView\<arch>\EmbeddedBrowserWebView.dll`
fn client_dll_path(dir: &Path) -> PathBuf {
    let arch = if cfg!(target_arch = "x86_64") {
        "x64"
    } else if cfg!(target_arch = "x86") {
        "x86"
    } else {
        "arm64"
    };
    dir.join("EBWebView")
        .join(arch)
        .join("EmbeddedBrowserWebView.dll")
}

/// "131.0.2903.70" → [131, 0, 2903, 70]
fn parse_version(s: &str) -> [u32; 4] {
    let mut out = [0u32; 4];
    for (i, part) in s.split('.').take(4).enumerate() {
        out[i] = part.trim().parse().unwrap_or(0);
    }
    out
}

/// レジストリの文字列値を読む (32 bit ビュー)
fn read_registry_string(root: HKEY, sub_key: &str, value: &str) -> Option<String> {
    let sub_key_w = wide(sub_key);
    let value_w = wide(value);
    let mut key: HKEY = std::ptr::null_mut();
    // SAFETY: すべて有効な NUL 終端文字列と出力先ポインタ
    unsafe {
        if RegOpenKeyExW(
            root,
            sub_key_w.as_ptr(),
            0,
            KEY_READ | KEY_WOW64_32KEY,
            &mut key,
        ) != ERROR_SUCCESS
        {
            return None;
        }
        let mut kind = 0u32;
        let mut size = 0u32;
        let ok = RegQueryValueExW(
            key,
            value_w.as_ptr(),
            std::ptr::null(),
            &mut kind,
            std::ptr::null_mut(),
            &mut size,
        ) == ERROR_SUCCESS
            && kind == REG_SZ
            && size >= 2;
        let mut result = None;
        if ok {
            let mut buf = vec![0u16; (size as usize).div_ceil(2) + 1];
            if RegQueryValueExW(
                key,
                value_w.as_ptr(),
                std::ptr::null(),
                &mut kind,
                buf.as_mut_ptr() as *mut u8,
                &mut size,
            ) == ERROR_SUCCESS
            {
                let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
                result = Some(String::from_utf16_lossy(&buf[..len]));
            }
        }
        RegCloseKey(key);
        result
    }
}

/// ファイルのバージョン情報 ("131.0.2903.70") を読む (固定版ランタイム用)
fn file_version(path: &Path) -> Option<String> {
    let path_w = wide(&path.to_string_lossy());
    // SAFETY: Win32 のバージョン情報 API を、十分な大きさのバッファで呼ぶ
    unsafe {
        let mut handle = 0u32;
        let size = GetFileVersionInfoSizeW(path_w.as_ptr(), &mut handle);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(path_w.as_ptr(), 0, size, data.as_mut_ptr() as *mut c_void) == 0 {
            return None;
        }
        let root = wide("\\");
        let mut info: *mut c_void = std::ptr::null_mut();
        let mut len = 0u32;
        if VerQueryValueW(
            data.as_ptr() as *const c_void,
            root.as_ptr(),
            &mut info,
            &mut len,
        ) == 0
            || info.is_null()
            || (len as usize) < std::mem::size_of::<VS_FIXEDFILEINFO>()
        {
            return None;
        }
        let fixed = &*(info as *const VS_FIXEDFILEINFO);
        Some(format!(
            "{}.{}.{}.{}",
            fixed.dwFileVersionMS >> 16,
            fixed.dwFileVersionMS & 0xffff,
            fixed.dwFileVersionLS >> 16,
            fixed.dwFileVersionLS & 0xffff
        ))
    }
}

/// インストール済み (Evergreen) ランタイムを探す
fn find_installed(root: HKEY) -> Option<Runtime> {
    let sub_key = format!("SOFTWARE\\Microsoft\\EdgeUpdate\\ClientState\\{STABLE_CHANNEL_GUID}");
    let dir = read_registry_string(root, &sub_key, "EBWebView")?;
    let dir = PathBuf::from(dir.trim_end_matches(['\\', '/']));
    let version = dir.file_name()?.to_string_lossy().into_owned();
    if parse_version(&version)[2] < MIN_BUILD {
        return None;
    }
    let dll = client_dll_path(&dir);
    dll.is_file().then_some(Runtime {
        dll,
        version,
        embedded: false,
    })
}

/// 固定版ランタイム (フォルダ指定) を探す
fn find_embedded(dir: &Path) -> Option<Runtime> {
    let dll = client_dll_path(dir);
    if !dll.is_file() {
        return None;
    }
    let version = file_version(&dll)?;
    if parse_version(&version)[2] < MIN_BUILD {
        return None;
    }
    Some(Runtime {
        dll,
        version,
        embedded: true,
    })
}

/// 使えるランタイムを探す。`browser_dir` (または環境変数) があれば固定版、無ければレジストリ
pub fn find_runtime(browser_dir: Option<&str>) -> Option<Runtime> {
    let dir = browser_dir
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
        .or_else(|| std::env::var("WEBVIEW2_BROWSER_EXECUTABLE_FOLDER").ok())
        .filter(|s| !s.trim().is_empty());
    if let Some(dir) = dir {
        return find_embedded(Path::new(&dir));
    }
    find_installed(HKEY_LOCAL_MACHINE).or_else(|| find_installed(HKEY_CURRENT_USER))
}

/// 読み込んだ `EmbeddedBrowserWebView.dll` (プロセスが終わるまで保持する)。
/// HMODULE は生ポインタで Send でないので整数にして持つ
static LOADED: OnceLock<Mutex<Option<(PathBuf, usize)>>> = OnceLock::new();

fn load_internal(runtime: &Runtime) -> Option<CreateInternal> {
    let slot = LOADED.get_or_init(|| Mutex::new(None));
    let mut guard = slot.lock().ok()?;
    let module: HMODULE = match guard.as_ref() {
        Some((path, module)) if *path == runtime.dll => *module as HMODULE,
        _ => {
            let path_w = wide(&runtime.dll.to_string_lossy());
            // SAFETY: 有効なパス。ALTERED_SEARCH_PATH で DLL の隣にある依存 DLL も見つかる
            let module = unsafe {
                LoadLibraryExW(
                    path_w.as_ptr(),
                    std::ptr::null_mut(),
                    LOAD_WITH_ALTERED_SEARCH_PATH,
                )
            };
            if module.is_null() {
                return None;
            }
            *guard = Some((runtime.dll.clone(), module as usize));
            module
        }
    };
    const SYMBOL: &[u8] = b"CreateWebViewEnvironmentWithOptionsInternal\0";
    // SAFETY: 読み込み済みのモジュールと NUL 終端のシンボル名
    let proc = unsafe { GetProcAddress(module, SYMBOL.as_ptr()) }?;
    // SAFETY: シンボルの実際の型は CreateInternal (WebView2Loader と同じ呼び出し規約)
    Some(unsafe {
        std::mem::transmute::<unsafe extern "system" fn() -> isize, CreateInternal>(proc)
    })
}

/// WebView2Loader.dll の `CreateCoreWebView2EnvironmentWithOptions` と同じ
#[no_mangle]
pub extern "system" fn CreateCoreWebView2EnvironmentWithOptions(
    browser_executable_folder: *const u16,
    user_data_folder: *const u16,
    environment_options: *mut c_void,
    environment_created_handler: *mut c_void,
) -> i32 {
    if environment_created_handler.is_null() {
        return E_INVALIDARG;
    }
    let browser_dir = from_wide_ptr(browser_executable_folder);
    let Some(runtime) = find_runtime(browser_dir.as_deref()) else {
        return E_RUNTIME_NOT_FOUND;
    };
    let Some(create) = load_internal(&runtime) else {
        return E_FAIL;
    };
    // ユーザーデータフォルダ: 引数が空なら環境変数、それも無ければ null (ランタイムの既定)
    let env_user_data = std::env::var("WEBVIEW2_USER_DATA_FOLDER")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(|s| wide(&s));
    let user_data_ptr = match from_wide_ptr(user_data_folder) {
        Some(s) if !s.trim().is_empty() => user_data_folder,
        _ => env_user_data
            .as_ref()
            .map(|w| w.as_ptr())
            .unwrap_or(std::ptr::null()),
    };
    // SAFETY: ランタイムの内部関数を、ローダー DLL と同じ引数で呼ぶ
    unsafe {
        create(
            true,
            if runtime.embedded { 1 } else { 0 },
            user_data_ptr,
            environment_options,
            environment_created_handler,
        )
    }
}

/// WebView2Loader.dll の `GetAvailableCoreWebView2BrowserVersionString` と同じ。
/// 返す文字列は CoTaskMemAlloc で確保する (呼び出し側が CoTaskMemFree する)
#[no_mangle]
pub extern "system" fn GetAvailableCoreWebView2BrowserVersionString(
    browser_executable_folder: *const u16,
    version_info: *mut *mut u16,
) -> i32 {
    if version_info.is_null() {
        return E_INVALIDARG;
    }
    let browser_dir = from_wide_ptr(browser_executable_folder);
    let Some(runtime) = find_runtime(browser_dir.as_deref()) else {
        return E_RUNTIME_NOT_FOUND;
    };
    let text = wide(&runtime.version);
    // SAFETY: COM のアロケータで確保した領域に NUL 終端込みでコピーする
    unsafe {
        let mem = CoTaskMemAlloc(text.len() * 2) as *mut u16;
        if mem.is_null() {
            return E_FAIL;
        }
        std::ptr::copy_nonoverlapping(text.as_ptr(), mem, text.len());
        *version_info = mem;
    }
    S_OK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parsing_and_dll_path() {
        assert_eq!(parse_version("131.0.2903.70"), [131, 0, 2903, 70]);
        assert_eq!(parse_version("1.2"), [1, 2, 0, 0]);
        let p = client_dll_path(Path::new("C:\\rt\\131.0.2903.70"));
        assert!(p.ends_with("EmbeddedBrowserWebView.dll"));
        assert!(p.to_string_lossy().contains("EBWebView"));
    }

    #[test]
    fn version_string_is_cotaskmem_allocated_or_not_found() {
        // ランタイムの有無はマシン次第なので、どちらの結果でも契約を満たすことだけ見る
        let mut out: *mut u16 = std::ptr::null_mut();
        let hr = GetAvailableCoreWebView2BrowserVersionString(std::ptr::null(), &mut out);
        if hr == S_OK {
            let s = from_wide_ptr(out).unwrap();
            assert!(s.split('.').count() >= 3, "{s}");
            // SAFETY: 上で CoTaskMemAlloc した領域
            unsafe { windows_sys::Win32::System::Com::CoTaskMemFree(out as *const c_void) };
        } else {
            assert_eq!(hr, E_RUNTIME_NOT_FOUND);
            assert!(out.is_null());
        }
    }
}
