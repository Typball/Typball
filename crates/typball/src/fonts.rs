//! 同梱フォントを wx の部品から名前で使えるよう、このプロセス専用のフォントとして OS に登録する
//!
//! - Noto Sans JP (typball-core が持つ): プレビューの既定。Typst 側は typball-core が埋め込みフォントを直接読み、
//!   Markdown のプレビューは `/assets/` から @font-face で読むので登録は要らないが、wx の部品 (プレビューウィンドウの
//!   テキスト表示や、設定でエディタのフォントに選んだとき) は OS のフォント一覧から名前で引く
//! - UDEV Gothic (JetBrains Mono + BIZ UDゴシック、SIL OFL 1.1): エディタの既定。半角と全角が 1:2 の等幅で、
//!   ルーラーの桁と文字がそろう。Regular と Bold を手を加えずに同梱する ("UDEV Gothic" は予約フォント名なので、
//!   サブセット化などで中身を変えるなら別の名前にしなければならない)。斜体は OS の合成に任せる
//!
//! インストールされていない機械でも使えるように登録し、プロセスが終われば消え、他のアプリには見えない。
//! wx の初期化後 (GTK はフォントの設定を Pango に渡すので初期化前だと失敗する)、ウィンドウを作る前に 1 回呼ぶ。

/// エディタの既定フォントのファミリー名 (同梱の UDEV Gothic)
pub const EDITOR_FONT_FAMILY: &str = "UDEV Gothic";

/// 同梱の UDEV Gothic。Linux では一度ファイルに書き出すので、その名前も持つ
const EDITOR_FONTS: &[(&str, &[u8])] = &[
    (
        "UDEVGothic-Regular.ttf",
        include_bytes!("../fonts/UDEVGothic-Regular.ttf"),
    ),
    (
        "UDEVGothic-Bold.ttf",
        include_bytes!("../fonts/UDEVGothic-Bold.ttf"),
    ),
];

/// 登録する全フォント (ファイル名, データ)。Noto Sans JP の並びは typball-core の embedded_font_data と同じ
fn all_fonts() -> impl Iterator<Item = (&'static str, &'static [u8])> {
    const NOTO_NAMES: [&str; 2] = ["NotoSansJP-Regular.otf", "NotoSansJP-Bold.otf"];
    NOTO_NAMES
        .into_iter()
        .zip(typball_core::embedded_font_data().iter().copied())
        .chain(EDITOR_FONTS.iter().copied())
}

/// 同梱フォントを登録し、登録できたファイルの数を返す
pub fn register_embedded() -> usize {
    all_fonts()
        .filter(|(name, data)| imp::register(name, data))
        .count()
}

#[cfg(windows)]
mod imp {
    use std::ptr::{addr_of_mut, null};

    use windows_sys::Win32::Graphics::Gdi::AddFontMemResourceEx;

    /// GDI にメモリから登録する (Scintilla は GDI で描くのでこれで名前が引ける)
    pub fn register(_name: &str, data: &'static [u8]) -> bool {
        let mut installed = 0u32;
        // SAFETY: data は実行ファイルに埋め込まれた 'static なバイト列で、長さも正しい。
        // 戻り値のハンドルはプロセスの寿命いっぱい登録を保つために捨てる (RemoveFontMemResourceEx は呼ばない)
        let handle = unsafe {
            AddFontMemResourceEx(
                data.as_ptr().cast(),
                data.len() as u32,
                null(),
                addr_of_mut!(installed) as *const u32,
            )
        };
        !handle.is_null() && installed > 0
    }
}

#[cfg(target_os = "macos")]
mod imp {
    //! wxOSX の wxFont::AddPrivateFont はアプリのバンドルの Resources/Fonts にあるかを確かめるだけで登録しないので、
    //! Core Text にメモリから直接登録する (プロセスの範囲)
    use std::ffi::c_void;
    use std::ptr::null_mut;

    type CFTypeRef = *const c_void;
    type ReleaseCallback = unsafe extern "C" fn(*mut c_void, *const c_void, usize);

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGDataProviderCreateWithData(
            info: *mut c_void,
            data: *const c_void,
            size: usize,
            release: Option<ReleaseCallback>,
        ) -> CFTypeRef;
        fn CGDataProviderRelease(provider: CFTypeRef);
        fn CGFontCreateWithDataProvider(provider: CFTypeRef) -> CFTypeRef;
        fn CGFontRelease(font: CFTypeRef);
    }

    #[link(name = "CoreText", kind = "framework")]
    extern "C" {
        fn CTFontManagerRegisterGraphicsFont(font: CFTypeRef, error: *mut CFTypeRef) -> bool;
    }

    pub fn register(_name: &str, data: &'static [u8]) -> bool {
        // SAFETY: data は 'static なのでデータプロバイダーは解放のコールバック無しで直接参照してよい。
        // 作ったオブジェクトは Create の規則どおり自分で解放する (登録した側が CGFont を保持する)。
        // error は NULL を渡してよく、そうすれば CFError を解放する必要もない
        unsafe {
            let provider =
                CGDataProviderCreateWithData(null_mut(), data.as_ptr().cast(), data.len(), None);
            if provider.is_null() {
                return false;
            }
            let font = CGFontCreateWithDataProvider(provider);
            CGDataProviderRelease(provider);
            if font.is_null() {
                return false;
            }
            let ok = CTFontManagerRegisterGraphicsFont(font, null_mut());
            CGFontRelease(font);
            ok
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    //! fontconfig はファイルからしか読めないので、キャッシュディレクトリに書き出してから wxFont::AddPrivateFont
    //! (FcConfigAppFontAddFile) で登録する。中身が同じなら書き直さない
    use std::fs;
    use std::path::PathBuf;

    fn font_dir() -> PathBuf {
        dirs::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("Typball")
            .join("fonts")
    }

    pub fn register(name: &str, data: &'static [u8]) -> bool {
        let dir = font_dir();
        let path = dir.join(name);
        let fresh = fs::metadata(&path).is_ok_and(|m| m.len() == data.len() as u64)
            && fs::read(&path).is_ok_and(|old| old == data);
        if !fresh {
            // 別の Typball が同時に書いていても壊れたファイルを読まないよう、一時ファイルに書いてから置き換える
            let tmp = dir.join(format!("{name}.{}.tmp", std::process::id()));
            let written = fs::create_dir_all(&dir)
                .and_then(|_| fs::write(&tmp, data))
                .and_then(|_| fs::rename(&tmp, &path));
            if written.is_err() {
                let _ = fs::remove_file(&tmp);
                return false;
            }
        }
        path.to_str()
            .is_some_and(wxdragon::font::Font::add_private_font)
    }
}
