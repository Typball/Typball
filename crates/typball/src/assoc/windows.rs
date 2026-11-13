//! Windows の関連付け (レジストリ)
//!
//! 書く場所は `HKCU\Software\Classes` (ユーザー単位。管理者権限が要らない)。キーの構成はインストーラー
//! (installer/typball.iss の [Registry]) と同じにしてあるので、インストーラーが作った登録をここで
//! 付け替えたり解除したりでき、逆にここで作った登録はアンインストーラーが消せる:
//!
//! ```text
//! Software\Classes\.typ                     (既定) = Typball.typ          ← 拡張子 → ProgID
//! Software\Classes\.typ\OpenWithProgids     Typball.typ = ""
//! Software\Classes\Typball.typ              (既定) = Typst Document       ← ProgID (種類の名前)
//! Software\Classes\Typball.typ\DefaultIcon  (既定) = <exe>,-101           ← アイコン
//! Software\Classes\Typball.typ\shell\open\command  = "<exe>" "%1"         ← ダブルクリック
//! Software\Classes\Typball.typ\shell\edit|view\command                    ← 右クリックの「編集」「プレビュー」
//! Software\Classes\Applications\Typball.exe ...                           ← 「プログラムから開く」の一覧
//! Software\Typball\Capabilities ...  +  Software\RegisteredApplications   ← 設定 → 既定のアプリ の一覧
//! ```
//!
//! 最後の 2 つはインストーラーには無い (Typball を Windows の「既定のアプリ」の画面に出すため)。
//!
//! 注意: Windows 10 以降は、利用者が「プログラムから開く → 常に使う」などで選んだアプリ (UserChoice) が
//! 最優先で、これはアプリからは書き換えられない (ハッシュで保護されている)。そういう拡張子は登録しても
//! 既定のアプリが変わらないので、[`open_system_settings`] で Windows の設定画面へ案内する。
//! 今の既定のアプリは UserChoice も含めて `AssocQueryString` に聞く。

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GdiFlush, SelectObject,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteKeyValueW, RegDeleteKeyW, RegDeleteTreeW, RegGetValueW,
    RegOpenKeyExW, RegQueryInfoKeyW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
    KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
};
use windows_sys::Win32::UI::Shell::{
    AssocQueryStringW, SHChangeNotify, ShellExecuteW, ASSOCF_INIT_IGNOREUNKNOWN,
    ASSOCSTR_EXECUTABLE, ASSOCSTR_FRIENDLYAPPNAME, SHCNE_ASSOCCHANGED, SHCNF_IDLIST,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, DrawIconEx, PrivateExtractIconsW, DI_NORMAL, HICON, SW_SHOWNORMAL,
};

use super::{same_file, Capabilities, DefaultApp, Icon, Kind, Labels, Registered, Status};

/// アプリの名前 (「プログラムから開く」の表示名、RegisteredApplications の値の名前)
const APP_NAME: &str = "Typball";
/// 関連付ける前に `.ext` の既定値だった ProgID を、自分の ProgID のキーに覚えておく値の名前 (解除で戻す)
const PREVIOUS: &str = "Typball.PreviousProgId";
/// アイコンに選べるファイル
pub const ICON_WILDCARD: &str = "*.ico;*.exe;*.dll|*.ico;*.exe;*.dll";

pub fn target() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

pub fn capabilities() -> Capabilities {
    Capabilities {
        unavailable: None,
        icons: true,
        system_settings: true,
        note: "settings.assoc.note_windows",
    }
}

pub fn status(kind: Kind) -> Status {
    Reg::live().status(kind, &target().unwrap_or_default())
}

pub fn register(kind: Kind, icon: &Icon, labels: &Labels) -> Result<(), String> {
    let exe = target().ok_or("current_exe")?;
    Reg::live().register(kind, icon, labels, &exe)
}

pub fn set_icon(kind: Kind, icon: &Icon) -> Result<(), String> {
    Reg::live().set_icon(kind, icon)
}

pub fn unregister(kind: Kind) -> Result<(), String> {
    Reg::live().unregister(kind)
}

/// Windows の設定 → アプリ → 既定のアプリ を開く。Typball を登録済み (RegisteredApplications に
/// 載っている) なら Typball のページを直接開く
pub fn open_system_settings() {
    let reg = Reg::live();
    let uri = if get_string(HKEY_CURRENT_USER, &reg.registered_apps, APP_NAME).is_some() {
        format!("ms-settings:defaultapps?registeredAppUser={APP_NAME}")
    } else {
        "ms-settings:defaultapps".to_string()
    };
    let (verb, uri) = (wide("open"), wide(&uri));
    // SAFETY: どちらも NUL 終端の UTF-16 で、呼び出しの間生きている
    unsafe {
        ShellExecuteW(
            null_mut(),
            verb.as_ptr(),
            uri.as_ptr(),
            null(),
            null(),
            SW_SHOWNORMAL,
        );
    }
}

pub fn icon_rgba(kind: Kind, icon: &Icon, size: u32) -> Option<Vec<u8>> {
    let exe = target()?;
    extract_icon(&icon_value(icon, kind, &exe), size)
}

/// 利用者が選んだファイルを DefaultIcon の文字列にする (.ico はそのまま、exe / dll は先頭のアイコン)
pub fn custom_icon_value(path: &Path) -> String {
    let is_ico = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("ico"));
    if is_ico {
        path.display().to_string()
    } else {
        format!("{},0", path.display())
    }
}

/// exe に埋め込んだ書類のアイコンのリソース ID (assets/icons/typball.rc)
fn document_icon_id(kind: Kind) -> i32 {
    match kind {
        Kind::Typ => 101,
        Kind::Md => 102,
        Kind::Txt => 103,
        Kind::Tbdg => 104,
    }
}

/// DefaultIcon に書く文字列。負の数はリソース ID、0 以上は何番目のアイコンか。
/// アプリのアイコンは「先頭 (0)」で指す (ID 1 を `-1` と書くと、-1 に別の意味を持たせている API がある)
fn icon_value(icon: &Icon, kind: Kind, exe: &Path) -> String {
    match icon {
        Icon::Document => format!("{},-{}", exe.display(), document_icon_id(kind)),
        Icon::App => format!("{},0", exe.display()),
        Icon::Custom(value) => value.clone(),
    }
}

/// DefaultIcon の文字列を (ファイル, 番号) に分ける。番号が無ければ 0
fn split_icon(value: &str) -> (&str, i32) {
    fn clean(s: &str) -> &str {
        s.trim().trim_matches('"')
    }
    match value.rsplit_once(',') {
        Some((path, index)) => match index.trim().parse() {
            Ok(index) => (clean(path), index),
            Err(_) => (clean(value), 0),
        },
        None => (clean(value), 0),
    }
}

/// 登録されている DefaultIcon から、どのアイコンを選んでいるかを読み取る。
/// `exe` は登録されている Typball の場所 (そこを指していれば書類 / アプリのアイコン)
fn parse_icon(value: &str, kind: Kind, exe: Option<&Path>) -> Icon {
    let (path, index) = split_icon(value);
    if exe.is_some_and(|exe| same_file(Path::new(path), exe)) {
        if index == -document_icon_id(kind) {
            return Icon::Document;
        }
        if index == 0 {
            return Icon::App;
        }
    }
    Icon::Custom(value.to_string())
}

/// `"C:\...\Typball.exe" --edit "%1"` から実行ファイルのパスを取り出す
fn command_exe(command: &str) -> Option<PathBuf> {
    let command = command.trim();
    let exe = match command.strip_prefix('"') {
        Some(rest) => rest.split('"').next()?,
        None => command.split_whitespace().next()?,
    };
    (!exe.is_empty()).then(|| PathBuf::from(exe))
}

/// レジストリのどこに書くか。本番は [`Reg::live`]、テストは HKCU の下の別の場所
struct Reg {
    /// `Software\Classes` に当たる場所 (HKCU の下)
    classes: String,
    /// `Software\Typball` に当たる場所 (Capabilities を置く)
    app: String,
    /// `Software\RegisteredApplications` に当たる場所
    registered_apps: String,
    /// 本番か。全ユーザー向け (HKLM) の登録と今の既定のアプリも読み、変更をエクスプローラーに知らせる
    live: bool,
}

impl Reg {
    fn live() -> Reg {
        Reg {
            classes: r"Software\Classes".to_string(),
            app: format!(r"Software\{APP_NAME}"),
            registered_apps: r"Software\RegisteredApplications".to_string(),
            live: true,
        }
    }

    fn progid(kind: Kind) -> String {
        format!("{APP_NAME}.{}", kind.ext())
    }

    /// ProgID のキー (HKCU の下)
    fn class(&self, kind: Kind) -> String {
        format!(r"{}\{}", self.classes, Self::progid(kind))
    }

    fn ext_key(&self, kind: Kind) -> String {
        format!(r"{}\.{}", self.classes, kind.ext())
    }

    fn capabilities_key(&self) -> String {
        format!(r"{}\Capabilities", self.app)
    }

    /// 「プログラムから開く」の一覧のキー。exe のファイル名で引かれる
    fn applications_key(&self, exe: &Path) -> String {
        let name = exe
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("{APP_NAME}.exe"));
        format!(r"{}\Applications\{name}", self.classes)
    }

    /// この Typball (`exe`) を `kind` に関連付ける
    fn register(&self, kind: Kind, icon: &Icon, labels: &Labels, exe: &Path) -> Result<(), String> {
        let progid = Self::progid(kind);
        let class = self.class(kind);
        let ext_key = self.ext_key(kind);
        let set =
            |path: &str, name: &str, value: &str| set_string(HKEY_CURRENT_USER, path, name, value);

        // 先に別の ProgID が既定になっていたら覚えておき、解除のときに戻す
        match get_string(HKEY_CURRENT_USER, &ext_key, "") {
            Some(previous) if !previous.is_empty() && !previous.eq_ignore_ascii_case(&progid) => {
                set(&class, PREVIOUS, &previous)?;
            }
            _ => {}
        }
        set(&class, "", kind.type_name())?;
        set(
            &format!(r"{class}\DefaultIcon"),
            "",
            &icon_value(icon, kind, exe),
        )?;
        // 動詞は作り直す (古い版が登録した動詞を残さない)
        let shell = format!(r"{class}\shell");
        delete_tree(HKEY_CURRENT_USER, &shell);
        set(&shell, "", "open")?;
        set(&format!(r"{shell}\open"), "", &labels.open)?;
        write_verbs(&shell, kind.has_preview(), labels, exe)?;

        set(&format!(r"{ext_key}\OpenWithProgids"), &progid, "")?;
        set(&ext_key, "", &progid)?;

        // 「プログラムから開く」の一覧 (関連付けていない形式でも Typball を選べるように全形式を載せる)
        let apps = self.applications_key(exe);
        set(&apps, "FriendlyAppName", APP_NAME)?;
        write_verbs(&format!(r"{apps}\shell"), true, labels, exe)?;
        for k in Kind::ALL {
            set(
                &format!(r"{apps}\SupportedTypes"),
                &format!(".{}", k.ext()),
                "",
            )?;
        }

        // 設定 → 既定のアプリ の一覧
        let caps = self.capabilities_key();
        set(&caps, "ApplicationName", APP_NAME)?;
        set(
            &caps,
            "ApplicationDescription",
            "Tabbed text editor with live preview for Typst and Markdown",
        )?;
        set(
            &format!(r"{caps}\FileAssociations"),
            &format!(".{}", kind.ext()),
            &progid,
        )?;
        set(&self.registered_apps, APP_NAME, &caps)?;

        self.notify();
        Ok(())
    }

    /// 登録済みの関連付けのアイコンを変える。書類 / アプリのアイコンは登録されている exe のものを指す
    fn set_icon(&self, kind: Kind, icon: &Icon) -> Result<(), String> {
        let class = self.class(kind);
        let Some(exe) = get_string(
            HKEY_CURRENT_USER,
            &format!(r"{class}\shell\open\command"),
            "",
        )
        .and_then(|c| command_exe(&c)) else {
            return Ok(());
        };
        set_string(
            HKEY_CURRENT_USER,
            &format!(r"{class}\DefaultIcon"),
            "",
            &icon_value(icon, kind, &exe),
        )?;
        self.notify();
        Ok(())
    }

    /// `kind` の関連付けを解除する (HKCU に書いた分)。関連付ける前の既定の ProgID が分かれば戻す
    fn unregister(&self, kind: Kind) -> Result<(), String> {
        let progid = Self::progid(kind);
        let class = self.class(kind);
        let ext_key = self.ext_key(kind);

        let ours = get_string(HKEY_CURRENT_USER, &ext_key, "")
            .is_some_and(|v| v.eq_ignore_ascii_case(&progid));
        if ours {
            match get_string(HKEY_CURRENT_USER, &class, PREVIOUS) {
                Some(previous) => set_string(HKEY_CURRENT_USER, &ext_key, "", &previous)?,
                None => delete_value(HKEY_CURRENT_USER, &ext_key, ""),
            }
        }
        let open_with = format!(r"{ext_key}\OpenWithProgids");
        delete_value(HKEY_CURRENT_USER, &open_with, &progid);
        delete_if_empty(HKEY_CURRENT_USER, &open_with);
        delete_if_empty(HKEY_CURRENT_USER, &ext_key);

        // 「プログラムから開く」の一覧を消してよいかは、ProgID を消す前に調べる (登録先の exe を読むため)
        let exe = get_string(
            HKEY_CURRENT_USER,
            &format!(r"{class}\shell\open\command"),
            "",
        )
        .and_then(|c| command_exe(&c));
        delete_tree(HKEY_CURRENT_USER, &class);

        let caps = self.capabilities_key();
        delete_value(
            HKEY_CURRENT_USER,
            &format!(r"{caps}\FileAssociations"),
            &format!(".{}", kind.ext()),
        );
        // どの形式も登録が残っていなければ、アプリとしての登録も片付ける
        let none_left = Kind::ALL
            .iter()
            .all(|k| !key_exists(HKEY_CURRENT_USER, &self.class(*k)));
        if none_left {
            delete_tree(HKEY_CURRENT_USER, &caps);
            delete_value(HKEY_CURRENT_USER, &self.registered_apps, APP_NAME);
            delete_if_empty(HKEY_CURRENT_USER, &self.app);
            // インストーラーで入れた Typball の一覧はインストーラーのもの (アンインストールで消える) なので残す
            if let Some(exe) = exe.filter(|exe| !installed_by_setup(exe)) {
                delete_tree(HKEY_CURRENT_USER, &self.applications_key(&exe));
            }
        }
        self.notify();
        Ok(())
    }

    fn status(&self, kind: Kind, exe: &Path) -> Status {
        // HKCU が優先。本番では全ユーザー向け (管理者で実行したインストーラー) の登録も見る
        let mut roots = vec![(HKEY_CURRENT_USER, self.class(kind))];
        if self.live {
            roots.push((
                HKEY_LOCAL_MACHINE,
                format!(r"Software\Classes\{}", Self::progid(kind)),
            ));
        }
        let read = |sub: &str| {
            roots
                .iter()
                .find_map(|(root, class)| get_string(*root, &format!(r"{class}\{sub}"), ""))
        };
        let registered_exe = read(r"shell\open\command").and_then(|c| command_exe(&c));
        let icon = read("DefaultIcon")
            .map(|v| parse_icon(&v, kind, registered_exe.as_deref()))
            .unwrap_or(Icon::Document);
        let registered = match registered_exe {
            None => Registered::No,
            Some(path) if same_file(&path, exe) => Registered::This,
            Some(path) => Registered::Other(path),
        };
        Status {
            default_app: if self.live {
                default_app(kind, exe)
            } else {
                None
            },
            registered,
            removable: key_exists(HKEY_CURRENT_USER, &self.class(kind)),
            icon,
        }
    }

    /// 関連付けが変わったことをエクスプローラーに知らせる (アイコンの表示が更新される)
    fn notify(&self) {
        if self.live {
            // SAFETY: SHCNF_IDLIST で項目を指定しない呼び出し (両方 NULL) は文書どおりの使い方
            unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, null(), null()) };
        }
    }
}

/// 動詞 (右クリックメニューの項目) のコマンドを書く。open は引数無しで渡し、設定 `open_mode` に従って
/// エディタかプレビューウィンドウで開く。`edit_view` なら「編集」(--edit) と「プレビュー」(--view) も出す
fn write_verbs(shell: &str, edit_view: bool, labels: &Labels, exe: &Path) -> Result<(), String> {
    let exe = exe.display();
    let set = |path: String, value: String| set_string(HKEY_CURRENT_USER, &path, "", &value);
    set(format!(r"{shell}\open\command"), format!(r#""{exe}" "%1""#))?;
    if edit_view {
        set(format!(r"{shell}\edit"), labels.edit.clone())?;
        set(
            format!(r"{shell}\edit\command"),
            format!(r#""{exe}" --edit "%1""#),
        )?;
        set(format!(r"{shell}\view"), labels.view.clone())?;
        set(
            format!(r"{shell}\view\command"),
            format!(r#""{exe}" --view "%1""#),
        )?;
    }
    Ok(())
}

/// インストーラーで入れた Typball か (隣にアンインストーラーがある。update.rs の install_kind と同じ判定)
fn installed_by_setup(exe: &Path) -> bool {
    exe.parent()
        .is_some_and(|dir| dir.join("unins000.exe").is_file())
}

/// 今この拡張子を開くアプリ (利用者が Windows の設定で選んだものを含む)
fn default_app(kind: Kind, exe: &Path) -> Option<DefaultApp> {
    let ext = wide(&format!(".{}", kind.ext()));
    let query = |what| {
        let mut buffer = [0u16; 1024];
        let mut len = buffer.len() as u32;
        // SAFETY: ext は NUL 終端、buffer は len 文字ぶんある。成功すると NUL 終端の文字列が入る
        let hr = unsafe {
            AssocQueryStringW(
                ASSOCF_INIT_IGNOREUNKNOWN,
                what,
                ext.as_ptr(),
                null(),
                buffer.as_mut_ptr(),
                &mut len,
            )
        };
        if hr != 0 {
            return None;
        }
        let end = buffer.iter().position(|c| *c == 0).unwrap_or(0);
        (end > 0).then(|| String::from_utf16_lossy(&buffer[..end]))
    };
    let path = query(ASSOCSTR_EXECUTABLE).map(PathBuf::from);
    // 関連付けの無い拡張子には「アプリを選ぶ」画面 (OpenWith.exe) が返ることがある
    if path
        .as_deref()
        .and_then(Path::file_name)
        .is_some_and(|n| n.eq_ignore_ascii_case("OpenWith.exe"))
    {
        return None;
    }
    let name = query(ASSOCSTR_FRIENDLYAPPNAME).or_else(|| {
        let stem = path.as_deref()?.file_stem()?;
        Some(stem.to_string_lossy().into_owned())
    })?;
    Some(DefaultApp {
        is_self: path.as_deref().is_some_and(|p| same_file(p, exe)),
        name,
        path,
    })
}

/// DefaultIcon の文字列が指すアイコンを `size` × `size` の RGBA (アルファは乗算していない) にする
fn extract_icon(value: &str, size: u32) -> Option<Vec<u8>> {
    let (path, index) = split_icon(value);
    let path = wide(path);
    let side = size as i32;
    let mut icon: HICON = null_mut();
    // SAFETY: path は NUL 終端。取り出すのは 1 個で、icon はその 1 個ぶんの置き場
    let found = unsafe {
        PrivateExtractIconsW(
            path.as_ptr(),
            index,
            side,
            side,
            &mut icon,
            null_mut(),
            1,
            0,
        )
    };
    if found == 0 || found == u32::MAX || icon.is_null() {
        return None;
    }
    // 透明な 32 ビットの DIB にアイコンを描くと、アルファを乗算済みの BGRA が得られる
    // SAFETY: 作った GDI オブジェクトはこの関数の中で選び直して解放する。bits は hbitmap が生きている間
    // size * size * 4 バイトを指す (CreateDIBSection が 0 で初期化している)
    let pixels = unsafe {
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = side;
        info.bmiHeader.biHeight = -side; // 上から下へ
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let dc = CreateCompatibleDC(null_mut());
        let mut bits: *mut c_void = null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        let mut pixels = None;
        if !bitmap.is_null() && !bits.is_null() {
            let old = SelectObject(dc, bitmap);
            DrawIconEx(dc, 0, 0, icon, side, side, 0, null_mut(), DI_NORMAL);
            GdiFlush();
            let len = (size * size * 4) as usize;
            pixels = Some(std::slice::from_raw_parts(bits as *const u8, len).to_vec());
            SelectObject(dc, old);
        }
        if !bitmap.is_null() {
            DeleteObject(bitmap);
        }
        DeleteDC(dc);
        DestroyIcon(icon);
        pixels
    }?;
    Some(bgra_premultiplied_to_rgba(pixels))
}

/// 乗算済み BGRA → 乗算していない RGBA。アルファを持たない古い形式のアイコン (全画素のアルファが 0) は、
/// 色のある画素を不透明として扱う
fn bgra_premultiplied_to_rgba(mut pixels: Vec<u8>) -> Vec<u8> {
    let has_alpha = pixels.chunks_exact(4).any(|p| p[3] != 0);
    for p in pixels.chunks_exact_mut(4) {
        let (b, g, r) = (p[0], p[1], p[2]);
        let a = if has_alpha {
            p[3]
        } else if b | g | r != 0 {
            255
        } else {
            0
        };
        let straight = |c: u8| match a {
            0 => 0,
            255 => c,
            a => ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8,
        };
        p.copy_from_slice(&[straight(r), straight(g), straight(b), a]);
    }
    pixels
}

// ---------------------------------------------------------------------------
// レジストリの読み書き (文字列だけ)
// ---------------------------------------------------------------------------

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// 文字列の値を読む。`name` が空ならキーの既定値。キーや値が無ければ None
fn get_string(root: HKEY, path: &str, name: &str) -> Option<String> {
    let (path, name) = (wide(path), wide(name));
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
    // SAFETY: 文字列は NUL 終端。1 回目で必要なバイト数を聞き、2 回目はその大きさのバッファを渡す
    unsafe {
        let mut bytes: u32 = 0;
        let rc = RegGetValueW(
            root,
            path.as_ptr(),
            name.as_ptr(),
            flags,
            null_mut(),
            null_mut(),
            &mut bytes,
        );
        if rc != ERROR_SUCCESS {
            return None;
        }
        let mut buffer = vec![0u16; (bytes as usize).div_ceil(2) + 1];
        let mut bytes = (buffer.len() * 2) as u32;
        let rc = RegGetValueW(
            root,
            path.as_ptr(),
            name.as_ptr(),
            flags,
            null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut bytes,
        );
        if rc != ERROR_SUCCESS {
            return None;
        }
        let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
        Some(String::from_utf16_lossy(&buffer[..end]))
    }
}

/// 文字列の値を書く (キーが無ければ作る)。`name` が空ならキーの既定値
fn set_string(root: HKEY, path: &str, name: &str, value: &str) -> Result<(), String> {
    let (wide_path, wide_name, data) = (wide(path), wide(name), wide(value));
    // SAFETY: 文字列は NUL 終端。開いたキーは必ず閉じる。data のバイト数は終端の NUL を含む
    let rc = unsafe {
        let mut key: HKEY = null_mut();
        let rc = RegCreateKeyExW(
            root,
            wide_path.as_ptr(),
            0,
            null(),
            0,
            KEY_SET_VALUE,
            null(),
            &mut key,
            null_mut(),
        );
        if rc == ERROR_SUCCESS {
            let rc = RegSetValueExW(
                key,
                wide_name.as_ptr(),
                0,
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            );
            RegCloseKey(key);
            rc
        } else {
            rc
        }
    };
    if rc == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!(
            "{path}: {}",
            std::io::Error::from_raw_os_error(rc as i32)
        ))
    }
}

/// 値を消す (無ければ何もしない)
fn delete_value(root: HKEY, path: &str, name: &str) {
    let (path, name) = (wide(path), wide(name));
    // SAFETY: 文字列は NUL 終端
    unsafe { RegDeleteKeyValueW(root, path.as_ptr(), name.as_ptr()) };
}

/// キーを中身ごと消す (無ければ何もしない)
fn delete_tree(root: HKEY, path: &str) {
    let path = wide(path);
    // SAFETY: 文字列は NUL 終端
    unsafe { RegDeleteTreeW(root, path.as_ptr()) };
}

fn key_exists(root: HKEY, path: &str) -> bool {
    open_for_query(root, path).is_some_and(|key| {
        // SAFETY: 今開いたキーを閉じる
        unsafe { RegCloseKey(key) };
        true
    })
}

fn open_for_query(root: HKEY, path: &str) -> Option<HKEY> {
    let path = wide(path);
    let mut key: HKEY = null_mut();
    // SAFETY: 文字列は NUL 終端。成功したら呼び出し側が閉じる
    let rc = unsafe { RegOpenKeyExW(root, path.as_ptr(), 0, KEY_QUERY_VALUE, &mut key) };
    (rc == ERROR_SUCCESS).then_some(key)
}

/// 値もサブキーも無いキーを消す (他のアプリの登録が残っているキーは消さない)
fn delete_if_empty(root: HKEY, path: &str) {
    let Some(key) = open_for_query(root, path) else {
        return;
    };
    let (mut subkeys, mut values) = (0u32, 0u32);
    // SAFETY: key は今開いたキー。数だけを受け取り、他の出力は要らないので NULL
    let rc = unsafe {
        let rc = RegQueryInfoKeyW(
            key,
            null_mut(),
            null_mut(),
            null(),
            &mut subkeys,
            null_mut(),
            null_mut(),
            &mut values,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
        );
        RegCloseKey(key);
        rc
    };
    if rc == ERROR_SUCCESS && subkeys == 0 && values == 0 {
        let path = wide(path);
        // SAFETY: 文字列は NUL 終端
        unsafe { RegDeleteKeyW(root, path.as_ptr()) };
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    /// 本物の関連付けに触らないよう、HKCU の下のテスト専用の場所に書く。終わったら丸ごと消す
    struct TestReg {
        root: String,
        reg: Reg,
    }

    impl TestReg {
        fn new() -> TestReg {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let root = format!(
                r"Software\TypballTest-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let reg = Reg {
                classes: format!(r"{root}\Classes"),
                app: format!(r"{root}\Typball"),
                registered_apps: format!(r"{root}\RegisteredApplications"),
                live: false,
            };
            TestReg { root, reg }
        }

        fn get(&self, path: &str, name: &str) -> Option<String> {
            get_string(HKEY_CURRENT_USER, &format!(r"{}\{path}", self.root), name)
        }

        fn exists(&self, path: &str) -> bool {
            key_exists(HKEY_CURRENT_USER, &format!(r"{}\{path}", self.root))
        }
    }

    impl Drop for TestReg {
        fn drop(&mut self) {
            delete_tree(HKEY_CURRENT_USER, &self.root);
        }
    }

    fn labels() -> Labels {
        Labels {
            open: "Open with Typball".to_string(),
            edit: "Edit with Typball".to_string(),
            view: "Preview with Typball".to_string(),
        }
    }

    const EXE: &str = r"Z:\apps\Typball\Typball.exe";

    #[test]
    fn register_writes_the_same_keys_as_the_installer() {
        let t = TestReg::new();
        let exe = Path::new(EXE);
        t.reg
            .register(Kind::Typ, &Icon::Document, &labels(), exe)
            .unwrap();

        assert_eq!(t.get(r"Classes\.typ", "").as_deref(), Some("Typball.typ"));
        assert_eq!(
            t.get(r"Classes\.typ\OpenWithProgids", "Typball.typ")
                .as_deref(),
            Some("")
        );
        assert_eq!(
            t.get(r"Classes\Typball.typ", "").as_deref(),
            Some("Typst Document")
        );
        assert_eq!(
            t.get(r"Classes\Typball.typ\DefaultIcon", "").as_deref(),
            Some(r"Z:\apps\Typball\Typball.exe,-101")
        );
        assert_eq!(
            t.get(r"Classes\Typball.typ\shell", "").as_deref(),
            Some("open")
        );
        assert_eq!(
            t.get(r"Classes\Typball.typ\shell\open\command", "")
                .as_deref(),
            Some(r#""Z:\apps\Typball\Typball.exe" "%1""#)
        );
        assert_eq!(
            t.get(r"Classes\Typball.typ\shell\view\command", "")
                .as_deref(),
            Some(r#""Z:\apps\Typball\Typball.exe" --view "%1""#)
        );
        assert_eq!(
            t.get(r"Classes\Applications\Typball.exe\SupportedTypes", ".md")
                .as_deref(),
            Some("")
        );
        assert_eq!(
            t.get(r"Typball\Capabilities\FileAssociations", ".typ")
                .as_deref(),
            Some("Typball.typ")
        );
        assert_eq!(
            t.get("RegisteredApplications", "Typball"),
            Some(format!(r"{}\Typball\Capabilities", t.root))
        );

        let status = t.reg.status(Kind::Typ, exe);
        assert_eq!(status.registered, Registered::This);
        assert!(status.removable);
        assert_eq!(status.icon, Icon::Document);
        // 別の場所の Typball から見ると「別の場所」
        let other = t.reg.status(Kind::Typ, Path::new(r"Z:\dev\Typball.exe"));
        assert_eq!(other.registered, Registered::Other(PathBuf::from(EXE)));
        // 登録していない形式
        let md = t.reg.status(Kind::Md, exe);
        assert_eq!(md.registered, Registered::No);
        assert!(!md.removable);
    }

    #[test]
    fn plain_text_has_no_edit_and_view_verbs() {
        let t = TestReg::new();
        t.reg
            .register(Kind::Txt, &Icon::Document, &labels(), Path::new(EXE))
            .unwrap();
        assert!(t.exists(r"Classes\Typball.txt\shell\open\command"));
        assert!(!t.exists(r"Classes\Typball.txt\shell\edit"));
        assert!(!t.exists(r"Classes\Typball.txt\shell\view"));
        assert_eq!(
            t.get(r"Classes\Typball.txt\DefaultIcon", "").as_deref(),
            Some(r"Z:\apps\Typball\Typball.exe,-103")
        );
    }

    #[test]
    fn diagrams_register_like_the_installer() {
        let t = TestReg::new();
        t.reg
            .register(Kind::Tbdg, &Icon::Document, &labels(), Path::new(EXE))
            .unwrap();
        assert_eq!(
            t.get(r"Classes\Typball.tbdg", "").as_deref(),
            Some("Typball Diagram")
        );
        assert_eq!(
            t.get(r"Classes\Typball.tbdg\DefaultIcon", "").as_deref(),
            Some(r"Z:\apps\Typball\Typball.exe,-104")
        );
        // 図もプレビューウィンドウで開けるので、「編集」「プレビュー」も出す
        assert!(t.exists(r"Classes\Typball.tbdg\shell\edit\command"));
        assert!(t.exists(r"Classes\Typball.tbdg\shell\view\command"));
        t.reg.unregister(Kind::Tbdg).unwrap();
        assert!(!t.exists(r"Classes\Typball.tbdg"));
    }

    #[test]
    fn unregister_removes_everything_it_wrote() {
        let t = TestReg::new();
        let exe = Path::new(EXE);
        t.reg
            .register(Kind::Typ, &Icon::Document, &labels(), exe)
            .unwrap();
        t.reg
            .register(Kind::Md, &Icon::App, &labels(), exe)
            .unwrap();

        t.reg.unregister(Kind::Typ).unwrap();
        assert!(!t.exists(r"Classes\Typball.typ"));
        assert!(!t.exists(r"Classes\.typ"));
        // もう片方の形式とアプリとしての登録は残る
        assert!(t.exists(r"Classes\Typball.md"));
        assert!(t.exists(r"Classes\Applications\Typball.exe"));
        assert_eq!(
            t.get(r"Typball\Capabilities\FileAssociations", ".typ"),
            None
        );
        assert_eq!(
            t.get(r"Typball\Capabilities\FileAssociations", ".md")
                .as_deref(),
            Some("Typball.md")
        );

        t.reg.unregister(Kind::Md).unwrap();
        assert!(!t.exists(r"Classes\Typball.md"));
        assert!(!t.exists(r"Classes\.md"));
        assert!(!t.exists(r"Classes\Applications\Typball.exe"));
        assert!(!t.exists("Typball"));
        assert_eq!(t.get("RegisteredApplications", "Typball"), None);
        assert_eq!(t.reg.status(Kind::Md, exe).registered, Registered::No);
    }

    #[test]
    fn unregister_restores_the_previous_handler_and_keeps_other_apps() {
        let t = TestReg::new();
        let exe = Path::new(EXE);
        let ext = format!(r"{}\Classes\.md", t.root);
        set_string(HKEY_CURRENT_USER, &ext, "", "Other.md").unwrap();
        set_string(
            HKEY_CURRENT_USER,
            &format!(r"{ext}\OpenWithProgids"),
            "Other.md",
            "",
        )
        .unwrap();

        t.reg
            .register(Kind::Md, &Icon::Document, &labels(), exe)
            .unwrap();
        assert_eq!(t.get(r"Classes\.md", "").as_deref(), Some("Typball.md"));
        // もう一度登録しても、覚えておいた元の ProgID は失わない
        t.reg
            .register(Kind::Md, &Icon::Document, &labels(), exe)
            .unwrap();

        t.reg.unregister(Kind::Md).unwrap();
        assert_eq!(t.get(r"Classes\.md", "").as_deref(), Some("Other.md"));
        assert_eq!(
            t.get(r"Classes\.md\OpenWithProgids", "Other.md").as_deref(),
            Some("")
        );
        assert_eq!(t.get(r"Classes\.md\OpenWithProgids", "Typball.md"), None);
    }

    #[test]
    fn unregister_leaves_an_extension_another_app_took_over() {
        let t = TestReg::new();
        let exe = Path::new(EXE);
        t.reg
            .register(Kind::Md, &Icon::Document, &labels(), exe)
            .unwrap();
        // 後から別のアプリが既定になった
        let ext = format!(r"{}\Classes\.md", t.root);
        set_string(HKEY_CURRENT_USER, &ext, "", "Other.md").unwrap();
        t.reg.unregister(Kind::Md).unwrap();
        assert_eq!(t.get(r"Classes\.md", "").as_deref(), Some("Other.md"));
    }

    #[test]
    fn icon_can_be_changed_after_registering() {
        let t = TestReg::new();
        let exe = Path::new(EXE);
        // 未登録なら何もしない
        t.reg.set_icon(Kind::Md, &Icon::App).unwrap();
        assert!(!t.exists(r"Classes\Typball.md"));

        t.reg
            .register(Kind::Md, &Icon::Document, &labels(), exe)
            .unwrap();
        t.reg.set_icon(Kind::Md, &Icon::App).unwrap();
        assert_eq!(
            t.get(r"Classes\Typball.md\DefaultIcon", "").as_deref(),
            Some(r"Z:\apps\Typball\Typball.exe,0")
        );
        assert_eq!(t.reg.status(Kind::Md, exe).icon, Icon::App);

        let custom = Icon::Custom(r"C:\icons\note.ico".to_string());
        t.reg.set_icon(Kind::Md, &custom).unwrap();
        assert_eq!(t.reg.status(Kind::Md, exe).icon, custom);
    }

    #[test]
    fn icon_strings() {
        assert_eq!(split_icon(r"C:\a\b.exe,-101"), (r"C:\a\b.exe", -101));
        assert_eq!(split_icon(r#""C:\a b\c.dll", 3"#), (r"C:\a b\c.dll", 3));
        assert_eq!(split_icon(r"C:\a,b\c.ico"), (r"C:\a,b\c.ico", 0));
        assert_eq!(
            custom_icon_value(Path::new(r"C:\x\y.ICO")),
            r"C:\x\y.ICO".to_string()
        );
        assert_eq!(
            custom_icon_value(Path::new(r"C:\x\y.dll")),
            r"C:\x\y.dll,0".to_string()
        );
        // インストーラーは引用符無しで書く。どちらでも読める
        let exe = Path::new(EXE);
        assert_eq!(
            parse_icon(r#""Z:\apps\typball\TYPBALL.EXE",-102"#, Kind::Md, Some(exe)),
            Icon::Document
        );
        // 別の形式の書類のアイコンを指していたら「選んだもの」
        assert!(matches!(
            parse_icon(r"Z:\apps\Typball\Typball.exe,-101", Kind::Md, Some(exe)),
            Icon::Custom(_)
        ));
    }

    #[test]
    fn command_line_to_exe() {
        assert_eq!(
            command_exe(r#""C:\Program Files\Typball\Typball.exe" --edit "%1""#),
            Some(PathBuf::from(r"C:\Program Files\Typball\Typball.exe"))
        );
        assert_eq!(
            command_exe(r"C:\Typball\Typball.exe %1"),
            Some(PathBuf::from(r"C:\Typball\Typball.exe"))
        );
        assert_eq!(command_exe("  "), None);
    }

    #[test]
    fn premultiplied_pixels_become_straight_rgba() {
        // 半透明の赤 (乗算済み BGRA)、透明、不透明の青
        let out = bgra_premultiplied_to_rgba(vec![0, 0, 128, 128, 0, 0, 0, 0, 255, 0, 0, 255]);
        assert_eq!(out, [255, 0, 0, 128, 0, 0, 0, 0, 0, 0, 255, 255]);
        // アルファの無いアイコン: 色のある画素は不透明
        let out = bgra_premultiplied_to_rgba(vec![10, 20, 30, 0, 0, 0, 0, 0]);
        assert_eq!(out, [30, 20, 10, 255, 0, 0, 0, 0]);
    }
}
