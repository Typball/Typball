//! ビルドスクリプト
//!
//! 1. windows-gnu (MinGW) ターゲット: wxdragon-sys は `-lstdc++` を動的リンク指定で出力するため、
//!    そのままだと実行時に libstdc++-6.dll に依存してしまう (PATH 上の別バージョンと衝突して
//!    起動に失敗することがある)。g++ から libstdc++.a の場所を聞き出し、それだけを置いた
//!    ディレクトリを検索パスに加えて静的リンクさせる。libgcc も静的にする。
//! 2. Windows: Common Controls v6 / Per-Monitor DPI / UTF-8 コードページのマニフェストを埋め込む。
//!    wxWidgets はこのマニフェストが無いと警告を出す。
//! 3. ビルド元のコミット日時と SHA (`TYPBALL_COMMIT_TIME` / `TYPBALL_COMMIT`)、CI の nightly なら
//!    `TYPBALL_BUILD_KIND=nightly` を埋め込む。更新機能は nightly の新旧をこのコミット日時で比べる。
//! 4. Windows: アプリと関連付けのアイコン (assets/icons/typball.rc) を埋め込む。
//! 5. 図エディタのアイコン (assets/diagram/icons/<セット>/<種類>.svg) を全部埋め込む表を作る。
//!    アイコンを足すには、SVG をそのフォルダに置くだけでよい (表示名は icons/catalog.json)。

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    set_build_info();
    embed_diagram_icons();

    if target_os == "windows" && target_env == "gnu" {
        link_static_libstdcxx();
    }

    #[cfg(windows)]
    if target_os == "windows" {
        embed_windows_manifest();
        embed_windows_icons();
    }
}

/// ビルド元のコミット情報を埋め込む。CI は環境変数 (`TYPBALL_BUILD_KIND` = nightly、`TYPBALL_COMMIT_TIME`、
/// `TYPBALL_COMMIT`) で渡し、無ければ git に聞く (それも無ければ 0 と空)。
/// nightly の新旧はこのコミット日時で比べる (update.rs)。番号 (Cargo.toml) は関わらない
fn set_build_info() {
    for name in [
        "TYPBALL_BUILD_KIND",
        "TYPBALL_COMMIT_TIME",
        "TYPBALL_COMMIT",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    // ブランチを切り替えたら埋め込み直す (同じブランチへのコミットでは HEAD は変わらないので、
    // 手元の表示は次の変更時に追いつく。CI は毎回まっさらなので常に正しい)
    let git_head = manifest_dir.join("../../.git/HEAD");
    if git_head.exists() {
        println!("cargo:rerun-if-changed={}", git_head.display());
    }
    let from_env = |name: &str| {
        env::var(name)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let commit_time = from_env("TYPBALL_COMMIT_TIME")
        .or_else(|| git(&manifest_dir, &["log", "-1", "--format=%ct"]))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let commit = from_env("TYPBALL_COMMIT")
        .or_else(|| git(&manifest_dir, &["rev-parse", "--short=7", "HEAD"]))
        .unwrap_or_default();
    let kind = from_env("TYPBALL_BUILD_KIND").unwrap_or_default();
    println!("cargo:rustc-env=TYPBALL_BUILD_KIND={kind}");
    println!("cargo:rustc-env=TYPBALL_COMMIT_TIME={commit_time}");
    println!("cargo:rustc-env=TYPBALL_COMMIT={commit}");
}

/// 図エディタのアイコンの表 (`$OUT_DIR/diagram_icons.rs`) を作る。中身は
/// `pub static ICONS: &[(&str, &[u8])]` で、名前は "セット/種類"、値は SVG の中身。
/// フォルダ名とファイル名は URL とスタイルにそのまま使うので、英小文字・数字・ハイフンだけにする
fn embed_diagram_icons() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let icons_dir = manifest_dir.join("assets/diagram/icons");
    println!("cargo:rerun-if-changed={}", icons_dir.display());
    let valid = |name: &str| {
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    };
    let mut entries: Vec<(String, PathBuf)> = Vec::new();
    let mut sets: Vec<PathBuf> = std::fs::read_dir(&icons_dir)
        .map(|dir| dir.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    sets.sort();
    for set in sets.iter().filter(|p| p.is_dir()) {
        let set_name = set.file_name().unwrap().to_string_lossy().into_owned();
        assert!(valid(&set_name), "diagram icon set name: {set_name}");
        let mut files: Vec<PathBuf> = std::fs::read_dir(set)
            .map(|dir| dir.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        files.sort();
        for file in files {
            if file.extension().and_then(|e| e.to_str()) != Some("svg") {
                continue;
            }
            let kind = file.file_stem().unwrap().to_string_lossy().into_owned();
            assert!(valid(&kind), "diagram icon name: {set_name}/{kind}");
            entries.push((format!("{set_name}/{kind}"), file));
        }
    }
    let mut code = String::from("pub static ICONS: &[(&str, &[u8])] = &[\n");
    for (name, path) in &entries {
        // パスは Rust の文字列にするので、区切りを / にそろえる (Windows の \ はエスケープになる)
        let path = path.display().to_string().replace('\\', "/");
        code.push_str(&format!("    ({name:?}, include_bytes!({path:?})),\n"));
    }
    code.push_str("];\n");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap_or_default()).join("diagram_icons.rs");
    std::fs::write(&out, code).expect("write diagram_icons.rs");
}

fn git(dir: &PathBuf, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// g++ に libstdc++.a の場所を聞き、OUT_DIR にコピーして検索パスに加える。
fn link_static_libstdcxx() {
    println!("cargo:rustc-link-arg-bins=-static-libgcc");

    let cxx = env::var("CXX_x86_64_pc_windows_gnu")
        .or_else(|_| env::var("CXX"))
        .unwrap_or_else(|_| "g++".to_string());
    let output = match Command::new(&cxx)
        .arg("-print-file-name=libstdc++.a")
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            println!(
                "cargo:warning=could not run {cxx}: {e}; libstdc++ will be linked dynamically"
            );
            return;
        }
    };
    let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    if !path.is_absolute() || !path.exists() {
        println!(
            "cargo:warning=libstdc++.a not found via {cxx}; libstdc++ will be linked dynamically"
        );
        return;
    }
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap()).join("static-libs");
    std::fs::create_dir_all(&out_dir).expect("create static-libs dir");
    std::fs::copy(&path, out_dir.join("libstdc++.a")).expect("copy libstdc++.a");
    println!("cargo:rustc-link-search=native={}", out_dir.display());
}

#[cfg(windows)]
fn embed_windows_manifest() {
    use embed_manifest::manifest::{ActiveCodePage, DpiAwareness, Setting, SupportedOS::*};
    use embed_manifest::{embed_manifest, new_manifest};

    let manifest = new_manifest("Typball.App")
        .active_code_page(ActiveCodePage::Utf8)
        .supported_os(Windows7..=Windows10)
        .dpi_awareness(DpiAwareness::PerMonitorV2)
        .long_path_aware(Setting::Enabled);
    if let Err(e) = embed_manifest(manifest) {
        println!("cargo:warning=Failed to embed manifest: {e}");
    }
}

/// アイコンのリソース (ID 1 = アプリ、101〜103 = 関連付け) を埋め込む。
/// .rc の中のファイル名は .rc と同じフォルダから探させる (windres は -I、rc は作業フォルダで探す)
#[cfg(windows)]
fn embed_windows_icons() {
    let dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default()).join("assets/icons");
    for name in [
        "typball.rc",
        "Typball.ico",
        "doc-typ.ico",
        "doc-md.ico",
        "doc-txt.ico",
    ] {
        println!("cargo:rerun-if-changed={}", dir.join(name).display());
    }
    let include = [dir.as_os_str()];
    if let Err(e) = embed_resource::compile(
        dir.join("typball.rc"),
        embed_resource::ParamsIncludeDirs(include),
    )
    .manifest_optional()
    {
        println!("cargo:warning=Failed to embed icons: {e}");
    }
}
