//! アプリのアイコン (ウィンドウのタイトルバー・タスクバー・Alt+Tab、バージョン情報の大きな絵)
//!
//! - Windows: exe に埋め込んだアイコン (assets/icons/typball.rc の ID 1) を、小 (タイトルバー) と
//!   大 (タスクバー・Alt+Tab) の大きさで読み、WM_SETICON で渡す。ICO の中から OS が大きさの合う画像を選ぶ
//! - macOS: Dock のアイコンは .app の Typball.icns が使われるので何もしない
//! - Linux など: 256x256 の RGBA (assets/icons/Typball-256.rgba) を縮めて wxFrame に渡す
//!
//! バージョン情報の絵はどの OS でも 256x256 の RGBA を画面の拡大率に合わせて縮めて使う。

use wxdragon::prelude::*;

/// assets/icons/Typball-256.rgba の一辺 (design/icons/export_icons.py が書き出す)
const RGBA_SIZE: usize = 256;
static RGBA: &[u8] = include_bytes!("../assets/icons/Typball-256.rgba");

/// フレームにアプリのアイコンを付ける
pub fn apply(frame: &Frame) {
    imp::apply(frame);
}

/// バージョン情報に出す大きなアイコン。`size` は実ピクセル (呼び出し側で from_dip_int する)
pub fn large_bitmap(size: u32) -> Option<Bitmap> {
    let size = size.clamp(16, RGBA_SIZE as u32);
    let data = resample(RGBA, RGBA_SIZE, size as usize);
    Bitmap::from_rgba(&data, size, size)
}

/// 正方形の RGBA を面積平均で縮める (縁が黒ずまないよう、色はアルファで重み付けして平均する)
fn resample(src: &[u8], src_size: usize, dst_size: usize) -> Vec<u8> {
    if dst_size == src_size {
        return src.to_vec();
    }
    let scale = src_size as f64 / dst_size as f64;
    let mut out = vec![0u8; dst_size * dst_size * 4];
    for dy in 0..dst_size {
        let (y0, y1) = (dy as f64 * scale, (dy + 1) as f64 * scale);
        for dx in 0..dst_size {
            let (x0, x1) = (dx as f64 * scale, (dx + 1) as f64 * scale);
            let mut acc = [0.0f64; 4];
            let mut weight = 0.0;
            for sy in y0.floor() as usize..(y1.ceil() as usize).min(src_size) {
                let wy = y1.min(sy as f64 + 1.0) - y0.max(sy as f64);
                for sx in x0.floor() as usize..(x1.ceil() as usize).min(src_size) {
                    let w = wy * (x1.min(sx as f64 + 1.0) - x0.max(sx as f64));
                    if w <= 0.0 {
                        continue;
                    }
                    let p = &src[(sy * src_size + sx) * 4..][..4];
                    let a = p[3] as f64 / 255.0;
                    for c in 0..3 {
                        acc[c] += p[c] as f64 * a * w;
                    }
                    acc[3] += a * w;
                    weight += w;
                }
            }
            let o = &mut out[(dy * dst_size + dx) * 4..][..4];
            if acc[3] > 0.0 {
                for c in 0..3 {
                    o[c] = (acc[c] / acc[3]).round().clamp(0.0, 255.0) as u8;
                }
                o[3] = (acc[3] / weight * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, LoadImageW, SendMessageW, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_SHARED,
        SM_CXICON, SM_CXSMICON, WM_SETICON,
    };
    use wxdragon::prelude::*;

    /// typball.rc のアプリのアイコンの ID
    const APP_ICON_ID: u16 = 1;

    pub fn apply(frame: &Frame) {
        let hwnd = frame.get_handle();
        if hwnd.is_null() {
            return;
        }
        // SAFETY: hwnd は生きている wxFrame のハンドルで、UI スレッドから呼んでいる。
        // LR_SHARED で読んだアイコンは OS が持つので解放しない (ウィンドウごとに読んでも増えない)
        unsafe {
            let module = GetModuleHandleW(std::ptr::null());
            for (kind, metric) in [(ICON_SMALL, SM_CXSMICON), (ICON_BIG, SM_CXICON)] {
                let size = GetSystemMetrics(metric);
                // MAKEINTRESOURCE: ID をそのままポインタとして渡す
                let icon = LoadImageW(
                    module,
                    APP_ICON_ID as usize as *const u16,
                    IMAGE_ICON,
                    size,
                    size,
                    LR_SHARED,
                );
                if !icon.is_null() {
                    SendMessageW(hwnd as _, WM_SETICON, kind as usize, icon as isize);
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use wxdragon::prelude::*;

    pub fn apply(_frame: &Frame) {}
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use wxdragon::prelude::*;

    pub fn apply(frame: &Frame) {
        if let Some(bitmap) = super::large_bitmap(128) {
            frame.set_icon(&bitmap);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_keeps_size_and_alpha_range() {
        let out = resample(RGBA, RGBA_SIZE, 96);
        assert_eq!(out.len(), 96 * 96 * 4);
        // 四隅は透明、中央 (球) は不透明のまま
        assert_eq!(out[3], 0);
        let center = (48 * 96 + 48) * 4;
        assert_eq!(out[center + 3], 255);
    }

    #[test]
    fn resample_averages_a_flat_color() {
        let src: Vec<u8> = [10u8, 20, 30, 255].repeat(4 * 4);
        let out = resample(&src, 4, 3);
        for px in out.chunks(4) {
            assert_eq!(px, [10, 20, 30, 255]);
        }
    }
}
