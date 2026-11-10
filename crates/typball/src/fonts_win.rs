//! 同梱フォント (Noto Sans JP) を Windows の GDI にこのプロセス専用のフォントとして登録する
//!
//! Typst 側は typball-core が埋め込みフォントを直接読み、Markdown のプレビューは `/assets/` から
//! @font-face で読むので登録は要らないが、wx の部品 (プレビューウィンドウのテキスト表示や、設定で
//! エディタのフォントに選んだとき) は OS のフォント一覧から名前で引くので、インストールされていない
//! 機械でも使えるように `AddFontMemResourceEx` でメモリから登録する。プロセスが終われば消え、
//! 他のアプリには見えない。

use std::ptr::{addr_of_mut, null};

use windows_sys::Win32::Graphics::Gdi::AddFontMemResourceEx;

/// 起動時に 1 回呼ぶ (ウィンドウを作る前)。登録したフォントの数を返す
pub fn register_embedded() -> u32 {
    let mut total = 0u32;
    for data in typball_core::embedded_font_data() {
        let mut installed = 0u32;
        // SAFETY: data は実行ファイルに埋め込まれた 'static なバイト列で、長さも正しい。
        // 戻り値のハンドルはプロセスの寿命いっぱい登録を保つために捨てる (RemoveFontMemResourceEx は呼ばない)
        unsafe {
            AddFontMemResourceEx(
                data.as_ptr().cast(),
                data.len() as u32,
                null(),
                addr_of_mut!(installed) as *const u32,
            );
        }
        total += installed;
    }
    total
}
