//! GitHub Releases を使った更新の確認・取得・適用
//!
//! 配布元は <https://github.com/Typball/Typball/releases>。Actions のアーティファクトは匿名で
//! 取得できないので使わない。
//!
//! 流れ: [`check`] (別スレッド) → [`Release::package`] で自分の環境向けの添付を選ぶ → [`download`] →
//! [`verify_sha256`] (同じ Release の `SHA256SUMS.txt` と照合) → [`apply`] → 呼び出し側が
//! ウィンドウを閉じる。ここには wx に依存しない処理だけを置き、ダイアログは `update_dialog.rs`。
//!
//! 自分自身の入れ替え (Windows):
//! - zip 版: 実行中の exe は上書きできないがリネームはできるので、`self-replace` で自分を
//!   一時名に変えて新しい exe を置き、新しい exe を起動してから終了する
//! - インストーラー版 (隣に Inno Setup の `unins000.exe` がある): 新しい setup.exe を落とし、
//!   自分が終了するのを待ってからサイレント実行して Typball を起動し直すバッチを cmd で走らせる
//!
//! チャネル (設定 `[update] channel`):
//! - stable: `releases/latest` (タグ `v1.2.3` を打った Release。pre-release は含まれない)。番号 (semver) で比べる
//! - nightly: 固定タグ `nightly` の pre-release (ci.yml が既定ブランチへの push ごとに作り直す) を先に見る。
//!   push ごとのビルドに番号は無いので、**ビルド元のコミット日時** で比べる。Release 本文の `build-time:` 行
//!   (UNIX 秒) と `commit:` 行を読み (無ければ API の `created_at` = そのコミットの日時)、自分のビルドに
//!   埋め込まれたコミット日時 (build.rs が git から取る) より新しければ更新。nightly が無い / 古いときは
//!   安定版の確認も行う。Cargo.toml の番号は nightly の判定に関わらない
//! - 行き来: 安定版 → 開発版はコミット日時の比較でそのまま移れる。開発版 → 安定版は、nightly のビルドが
//!   Cargo.toml と同じ番号を持つので番号では「新しくない」になる。安定版チャネルで今のビルドが nightly なら、
//!   番号が同じ (古い) 安定版も勧めて戻れるようにする ([`decide`])
//!
//! 開発中の確認用に、環境変数 `TYPBALL_UPDATE_API` で問い合わせ先 (`.../releases` までの URL) を差し替えられる。

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use semver::Version;
use serde::Deserialize;

use crate::settings::Channel;

/// Releases ページ (自動更新できない環境ではここを開く)
pub const RELEASES_URL: &str = "https://github.com/Typball/Typball/releases";
const API_RELEASES: &str = "https://api.github.com/repos/Typball/Typball/releases";
/// nightly ビルドを置く固定タグ (ci.yml と合わせる)
pub const NIGHTLY_TAG: &str = "nightly";
/// ビルド元のコミット日時 (UNIX 秒、不明なら 0) と短い SHA (不明なら空)。build.rs が埋め込む
const COMMIT_TIME: &str = env!("TYPBALL_COMMIT_TIME");
const COMMIT: &str = env!("TYPBALL_COMMIT");
/// CI の nightly ビルドなら "nightly" (build.rs)
const BUILD_KIND: &str = env!("TYPBALL_BUILD_KIND");
/// release.yml / ci.yml が添付するハッシュ一覧
const SUMS_NAME: &str = "SHA256SUMS.txt";
/// `download` が中止されたときのエラー文字列
pub const CANCELLED: &str = "cancelled";
/// 更新後の再起動で新しいプロセスに渡す引数 (続けて古いプロセスの PID)
pub const WAIT_PID_ARG: &str = "--wait-pid";
/// 古いプロセスの終了を待つ上限
#[cfg(windows)]
const WAIT_EXIT_TIMEOUT_MS: u32 = 30_000;

/// Release の添付ファイル
#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub name: String,
    #[serde(rename = "browser_download_url")]
    pub url: String,
    /// 大きさ (Content-Length が無いときの進捗表示用)
    #[serde(default)]
    pub size: u64,
}

/// Release の種類と、新旧を比べるための識別子
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// タグの番号
    Stable(Version),
    /// ビルド元のコミット日時 (UNIX 秒) と SHA (短い形でも可)
    Nightly { commit_time: u64, commit: String },
}

/// 公開されている最新版
#[derive(Debug, Clone)]
pub struct Release {
    pub kind: Kind,
    /// 添付ファイル名の頭 (`typball-0.2.0` / `typball-nightly`)
    pub asset_prefix: String,
    pub title: String,
    /// リリースノート (Markdown)
    pub notes: String,
    /// Release ページの URL
    pub url: String,
    pub assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    body: Option<String>,
    html_url: String,
    /// Release の元になったコミットの日時 (RFC 3339)
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    target_commitish: Option<String>,
    #[serde(default)]
    assets: Vec<Asset>,
}

/// 更新確認の結果
#[derive(Debug)]
pub enum Check {
    UpToDate,
    Available(Release),
    /// Release がまだ 1 つも無い (404)
    NoRelease,
    Failed(String),
}

/// 実行ファイルの配置
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    /// zip を展開しただけ (exe を直接入れ替える)
    Portable,
    /// インストーラーで入れた (setup.exe を実行し直す)
    Installer,
}

pub fn current_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("CARGO_PKG_VERSION is semver")
}

/// 自分のビルド元のコミット日時 (UNIX 秒)。不明なら 0 (nightly はどれでも新しいとみなす)
pub fn current_commit_time() -> u64 {
    COMMIT_TIME.trim().parse().unwrap_or(0)
}

pub fn current_commit() -> &'static str {
    COMMIT.trim()
}

pub fn is_nightly_build() -> bool {
    BUILD_KIND.trim() == NIGHTLY_TAG
}

/// 版の表記 (バージョン情報・更新ダイアログ)。"0.1.0"、CI の nightly ビルドなら
/// "0.1.0 (nightly 2026-11-10 12:00 UTC, abc1234)"
pub fn version_label() -> String {
    let version = env!("CARGO_PKG_VERSION");
    if is_nightly_build() && current_commit_time() > 0 {
        format!(
            "{version} ({})",
            nightly_label(current_commit_time(), current_commit())
        )
    } else {
        version.to_string()
    }
}

/// nightly の表記 "nightly 2026-11-10 12:00 UTC, abc1234"
pub fn nightly_label(commit_time: u64, commit: &str) -> String {
    let mut label = format!("{NIGHTLY_TAG} {} UTC", format_utc(commit_time));
    if !commit.is_empty() {
        label.push_str(", ");
        label.push_str(commit);
    }
    label
}

/// 2 つの SHA が同じコミットを指すか (片方が短い形でもよい)
fn same_commit(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && (a.starts_with(b) || b.starts_with(a))
}

/// 今動いているビルド (新旧の判定の基準)。判定をテストできるよう値として渡す
#[derive(Debug, Clone)]
pub struct Build {
    /// Cargo.toml の番号 (nightly も同じ番号を持つ)
    pub version: Version,
    /// ビルド元のコミット日時 (UNIX 秒、不明なら 0)
    pub commit_time: u64,
    pub commit: String,
    /// CI の nightly ビルドか
    pub nightly: bool,
}

impl Build {
    pub fn current() -> Build {
        Build {
            version: current_version(),
            commit_time: current_commit_time(),
            commit: current_commit().to_string(),
            nightly: is_nightly_build(),
        }
    }
}

impl Release {
    pub fn is_nightly(&self) -> bool {
        matches!(self.kind, Kind::Nightly { .. })
    }

    /// 表示用の版 ("0.2.0" / "nightly 2026-11-10 12:00 UTC, abc1234")
    pub fn label(&self) -> String {
        match &self.kind {
            Kind::Stable(v) => v.to_string(),
            Kind::Nightly {
                commit_time,
                commit,
            } => nightly_label(*commit_time, commit),
        }
    }

    /// 「この版はスキップ」で覚える識別子
    pub fn id(&self) -> String {
        match &self.kind {
            Kind::Stable(v) => v.to_string(),
            Kind::Nightly {
                commit_time,
                commit,
            } if commit.is_empty() => format!("{NIGHTLY_TAG}-{commit_time}"),
            Kind::Nightly { commit, .. } => format!("{NIGHTLY_TAG}-{commit}"),
        }
    }

    /// 今動いているビルドより新しいか (テスト用。アプリは [`decide`] で判定する)
    #[cfg(test)]
    pub fn is_newer_than_current(&self) -> bool {
        self.is_newer_than(&Build::current())
    }

    /// `build` より新しいか。安定版は番号、nightly はコミット日時で比べる
    /// (同じコミットなら日時に関わらず最新)。nightly のビルドも Cargo.toml の番号を持つので、
    /// 同じ番号の安定版は「新しくない」になる (開発版から戻るときは [`decide`] が別に扱う)
    pub fn is_newer_than(&self, build: &Build) -> bool {
        match &self.kind {
            Kind::Stable(v) => *v > build.version,
            Kind::Nightly {
                commit_time,
                commit,
            } => !same_commit(commit, &build.commit) && *commit_time > build.commit_time,
        }
    }

    /// 開発版のビルドから安定版へ戻す提案か (番号は新しくない。ダイアログの文言を変える)
    pub fn is_switch_to_stable(&self) -> bool {
        let build = Build::current();
        build.nightly && !self.is_nightly() && !self.is_newer_than(&build)
    }
}

fn agent(body_timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(format!(
            "Typball/{} (+{RELEASES_URL})",
            env!("CARGO_PKG_VERSION")
        ))
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(body_timeout))
        .build()
        .into()
}

/// テキストを取得する。404 なら `Ok(None)`
fn fetch_text(url: &str) -> Result<Option<String>, String> {
    match agent(Duration::from_secs(30)).get(url).call() {
        Ok(mut resp) => resp
            .body_mut()
            .read_to_string()
            .map(Some)
            .map_err(|e| e.to_string()),
        Err(ureq::Error::StatusCode(404)) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Release 1 つを取得して読む。無ければ `Ok(None)`
fn fetch_release(url: &str) -> Result<Option<Release>, String> {
    match fetch_text(url)? {
        Some(json) => parse_release(&json).map(Some),
        None => Ok(None),
    }
}

/// チャネルに応じて最新版を問い合わせ、今のビルドと比べる (ブロックするので別スレッドで呼ぶ)
pub fn check(channel: Channel) -> Check {
    let base = std::env::var("TYPBALL_UPDATE_API").unwrap_or_else(|_| API_RELEASES.to_string());
    let base = base.trim_end_matches('/');
    let build = Build::current();
    let nightly = if channel == Channel::Nightly {
        match fetch_release(&format!("{base}/tags/{NIGHTLY_TAG}")) {
            // 新しい nightly があれば安定版は見なくてよい
            Ok(Some(n)) if n.is_newer_than(&build) => return Check::Available(n),
            Ok(found) => found,
            Err(e) => return Check::Failed(e),
        }
    } else {
        None
    };
    match fetch_release(&format!("{base}/latest")) {
        Ok(stable) => decide(channel, &build, nightly, stable),
        Err(e) => Check::Failed(e),
    }
}

/// 取得した nightly / 安定版のうち、どれを勧めるか。
///
/// - 開発版チャネル: 新しい nightly → 番号の新しい安定版 (nightly の Release が無いときなど) の順。
///   安定版のビルドからでもコミット日時で比べるので、タグ以降の nightly へ移れる
/// - 安定版チャネル: 番号の新しい安定版。さらに、今のビルドが nightly なら番号が同じ (古い) 安定版も勧める。
///   nightly のビルドは Cargo.toml と同じ番号を持つので、番号だけで比べると開発版から安定版へ戻れない
///   (0.1.0 の nightly から見て安定版 0.1.0 は「新しくない」)
pub fn decide(
    channel: Channel,
    build: &Build,
    nightly: Option<Release>,
    stable: Option<Release>,
) -> Check {
    if channel == Channel::Nightly {
        if let Some(n) = &nightly {
            if n.is_newer_than(build) {
                return Check::Available(n.clone());
            }
        }
    }
    match stable {
        Some(s) if s.is_newer_than(build) => Check::Available(s),
        Some(s) if channel == Channel::Stable && build.nightly => Check::Available(s),
        Some(_) => Check::UpToDate,
        None if nightly.is_some() => Check::UpToDate,
        None => Check::NoRelease,
    }
}

/// Release の JSON を読む。タグ `v1.2.3` なら番号、固定タグ `nightly` なら本文の `build-time:` (UNIX 秒) と
/// `commit:` の行 (ci.yml が書く)、無ければ `created_at` と `target_commitish` を識別子にする
pub fn parse_release(json: &str) -> Result<Release, String> {
    let api: ApiRelease = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let title = api
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| api.tag_name.clone());
    let notes = api.body.unwrap_or_default().replace("\r\n", "\n");
    let kind = if api.tag_name == NIGHTLY_TAG {
        let commit_time = body_field(&notes, "build-time")
            .and_then(|v| v.parse::<u64>().ok())
            .or_else(|| api.created_at.as_deref().and_then(parse_rfc3339))
            .ok_or_else(|| "nightly release has no build time".to_string())?;
        let commit = body_field(&notes, "commit")
            .map(str::to_string)
            .or(api.target_commitish)
            .unwrap_or_default();
        Kind::Nightly {
            commit_time,
            commit: commit.chars().take(12).collect(),
        }
    } else {
        Kind::Stable(
            Version::parse(api.tag_name.trim_start_matches('v'))
                .map_err(|e| format!("tag {}: {e}", api.tag_name))?,
        )
    };
    Ok(Release {
        asset_prefix: match &kind {
            Kind::Stable(v) => format!("typball-{v}"),
            Kind::Nightly { .. } => format!("typball-{NIGHTLY_TAG}"),
        },
        kind,
        title,
        notes,
        url: api.html_url,
        assets: api.assets,
    })
}

/// 本文の `key: value` 行の値
fn body_field<'a>(notes: &'a str, key: &str) -> Option<&'a str> {
    notes.lines().find_map(|line| {
        let (k, v) = line.split_once(':')?;
        (k.trim().eq_ignore_ascii_case(key) && !v.trim().is_empty()).then(|| v.trim())
    })
}

/// RFC 3339 の UTC 時刻 (`2026-11-10T12:00:00Z`、GitHub API の形) を UNIX 秒にする
pub fn parse_rfc3339(s: &str) -> Option<u64> {
    let (date, time) = s.trim().split_once('T')?;
    let time = time.trim_end_matches('Z');
    let time = time.split('.').next()?;
    let mut d = date.split('-').map(|p| p.parse::<i64>());
    let (y, m, day) = (d.next()?.ok()?, d.next()?.ok()?, d.next()?.ok()?);
    let mut t = time.split(':').map(|p| p.parse::<i64>());
    let (h, mi, sec) = (t.next()?.ok()?, t.next()?.ok()?, t.next()?.ok()?);
    let secs = days_from_civil(y, m, day) * 86400 + h * 3600 + mi * 60 + sec;
    u64::try_from(secs).ok()
}

/// UNIX 秒を "2026-11-10 12:00" (UTC) にする
pub fn format_utc(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        rem / 3600,
        rem % 3600 / 60
    )
}

// 暦の計算 (Howard Hinnant の days_from_civil / civil_from_days)
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// 隣に Inno Setup のアンインストーラーがあればインストーラー版
pub fn install_kind() -> InstallKind {
    let exe = std::env::current_exe().ok();
    match exe.as_deref().and_then(Path::parent) {
        Some(dir) if cfg!(windows) && dir.join("unins000.exe").is_file() => InstallKind::Installer,
        _ => InstallKind::Portable,
    }
}

/// この環境向けの添付の名前 (release.yml / ci.yml の命名に合わせる)。自動更新できない環境は None
pub fn package_name(prefix: &str, kind: InstallKind) -> Option<String> {
    if !cfg!(all(windows, target_arch = "x86_64")) {
        return None;
    }
    Some(match kind {
        InstallKind::Portable => format!("{prefix}-windows-x86_64.zip"),
        InstallKind::Installer => format!("{prefix}-windows-x86_64-setup.exe"),
    })
}

impl Release {
    /// この環境向けの添付
    pub fn package(&self, kind: InstallKind) -> Option<&Asset> {
        let name = package_name(&self.asset_prefix, kind)?;
        self.assets.iter().find(|a| a.name == name)
    }

    pub fn sums(&self) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name == SUMS_NAME)
    }
}

/// ダウンロードの置き場 (一時フォルダ)
pub fn download_dir() -> PathBuf {
    std::env::temp_dir().join("Typball-update")
}

/// 前回の更新で残ったファイルを消す (起動時)。更新直後はインストーラーのバッチがまだ動いている
/// ことがあるので、10 分より新しいものは残す
pub fn cleanup_downloads() {
    let Ok(entries) = fs::read_dir(download_dir()) else {
        return;
    };
    let cutoff = std::time::SystemTime::now() - Duration::from_secs(600);
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .map(|t| t < cutoff)
            .unwrap_or(true);
        if old {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// `url` を `dest` に保存する。`progress(受信済み, 全体)` を呼びながら進め、`cancel` が立てば
/// 途中で止めて [`CANCELLED`] を返す
pub fn download(
    url: &str,
    dest: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<(), String> {
    let agent = agent(Duration::from_secs(600));
    let mut resp = agent.get(url).call().map_err(|e| e.to_string())?;
    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());
    let mut reader = resp.body_mut().as_reader();
    if let Some(dir) = dest.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut file = File::create(dest).map_err(|e| e.to_string())?;
    let mut buf = [0u8; 64 * 1024];
    let mut done = 0u64;
    progress(0, total);
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(file);
            let _ = fs::remove_file(dest);
            return Err(CANCELLED.to_string());
        }
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        progress(done, total);
    }
    file.flush().map_err(|e| e.to_string())?;
    if let Some(total) = total {
        if done != total {
            return Err(format!("short download: {done} of {total} bytes"));
        }
    }
    Ok(())
}

/// `SHA256SUMS.txt` を取得してファイルを照合する
pub fn verify_with_sums(file: &Path, sums_url: &str, name: &str) -> Result<(), String> {
    let sums = fetch_text(sums_url)?.ok_or_else(|| format!("{SUMS_NAME}: not found"))?;
    verify_sha256(file, &sums, name)
}

/// `sha256sum` 形式 (`<hex>  <name>` または `<hex> *<name>`) の一覧から `name` のハッシュを探す
pub fn expected_sha256(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hex, file) = line.trim().split_once(char::is_whitespace)?;
        let file = file.trim_start().trim_start_matches('*');
        // `dist/x.zip` のようにフォルダ付きで書かれていても末尾で合わせる
        let file = file.rsplit(['/', '\\']).next().unwrap_or(file);
        (file == name && hex.len() == 64).then(|| hex.to_ascii_lowercase())
    })
}

pub fn verify_sha256(file: &Path, sums: &str, name: &str) -> Result<(), String> {
    let expected =
        expected_sha256(sums, name).ok_or_else(|| format!("{name}: not listed in {SUMS_NAME}"))?;
    let actual = sha256_hex(file).map_err(|e| e.to_string())?;
    if actual != expected {
        return Err(format!("{name}: checksum mismatch"));
    }
    Ok(())
}

pub fn sha256_hex(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// ダウンロード済みの配布物を適用し、新しい Typball を (`reopen` のファイルを開いて) 起動する。
/// 成功したら呼び出し側は今のウィンドウをすべて閉じる
pub fn apply(package: &Path, kind: InstallKind, reopen: &[PathBuf]) -> Result<(), String> {
    match kind {
        InstallKind::Portable => {
            let new_exe = extract_exe(package)?;
            self_replace::self_replace(&new_exe).map_err(|e| format!("replace: {e}"))?;
            let _ = fs::remove_file(&new_exe);
            let _ = fs::remove_file(package);
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            // 新しいプロセスには自分の終了を待たせる (main.rs)。待たずに起動すると、まだ動いている
            // 自分をシングルインスタンスの先行プロセスとみなし、ファイルを渡して終わってしまう
            Command::new(exe)
                .arg(WAIT_PID_ARG)
                .arg(std::process::id().to_string())
                .args(reopen)
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("restart: {e}"))
        }
        InstallKind::Installer => run_installer(package, reopen),
    }
}

/// プロセス `pid` の終了を待つ (上限 30 秒)。もう無ければすぐ戻る
#[cfg(windows)]
pub fn wait_for_exit(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    };
    // SAFETY: 待つための権限だけでハンドルを開き、待ってから閉じる
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if !handle.is_null() {
            WaitForSingleObject(handle, WAIT_EXIT_TIMEOUT_MS);
            CloseHandle(handle);
        }
    }
}

/// zip 版の自己更新は Windows だけ
#[cfg(not(windows))]
pub fn wait_for_exit(_pid: u32) {}

/// zip から実行ファイル (自分と同じ名前) を取り出す
fn extract_exe(zip_path: &Path) -> Result<PathBuf, String> {
    let exe_name = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "Typball.exe".to_string());
    let file = File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let index = (0..archive.len())
        .find(|&i| {
            archive
                .by_index(i)
                .ok()
                .and_then(|f| f.name().ok().map(|n| n.into_owned()))
                .and_then(|name| {
                    name.rsplit(['/', '\\'])
                        .next()
                        .map(|n| n.eq_ignore_ascii_case(&exe_name))
                })
                .unwrap_or(false)
        })
        .ok_or_else(|| format!("{exe_name} not found in {}", zip_path.display()))?;
    let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
    let out = download_dir().join(format!("{exe_name}.new"));
    let mut out_file = File::create(&out).map_err(|e| e.to_string())?;
    std::io::copy(&mut entry, &mut out_file).map_err(|e| e.to_string())?;
    Ok(out)
}

/// setup.exe を「自分が終わってから」サイレント実行し、終わったら Typball を起動し直すバッチを cmd で走らせる。
/// Inno Setup は更新中にショートカットや「アプリと機能」の表示も合わせて直す
#[cfg(windows)]
fn run_installer(setup: &Path, reopen: &[PathBuf]) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let script = installer_script(setup, &exe, std::process::id(), reopen);
    let script_path = download_dir().join("update.cmd");
    fs::write(&script_path, script).map_err(|e| e.to_string())?;
    Command::new("cmd")
        .arg("/d")
        .arg("/c")
        .arg(&script_path)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("cmd: {e}"))
}

#[cfg(not(windows))]
fn run_installer(_setup: &Path, _reopen: &[PathBuf]) -> Result<(), String> {
    Err("installer packages are only used on Windows".to_string())
}

/// インストーラー版の更新バッチ: プロセス `pid` (自分) が終わるのを待ち (最長 60 秒)、setup.exe を
/// サイレント実行し、終わったら `exe` を `reopen` のファイル付きで起動して自分 (バッチ) を消す。
/// ping は sleep の代わり (コンソールが無いと timeout が使えない)
fn installer_script(setup: &Path, exe: &Path, pid: u32, reopen: &[PathBuf]) -> String {
    let args: String = reopen
        .iter()
        .map(|p| format!(" \"{}\"", p.display()))
        .collect();
    format!(
        "@echo off\r\n\
         set n=0\r\n\
         :wait\r\n\
         set /a n+=1\r\n\
         if %n% gtr 60 goto run\r\n\
         tasklist /FI \"PID eq {pid}\" 2>nul | find \"{pid}\" >nul && (ping -n 2 127.0.0.1 >nul & goto wait)\r\n\
         :run\r\n\
         \"{setup}\" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /CLOSEAPPLICATIONS\r\n\
         start \"\" \"{exe}\"{args}\r\n\
         del \"%~f0\"\r\n",
        setup = setup.display(),
        exe = exe.display(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // "## Changes" が r#"..."# や r##"..."## を閉じてしまうので ### で囲む
    const JSON: &str = r###"{
        "tag_name": "v9.8.7",
        "name": "Typball v9.8.7",
        "body": "## Changes\r\n- one\r\n",
        "html_url": "https://github.com/Typball/Typball/releases/tag/v9.8.7",
        "prerelease": false,
        "assets": [
            {"name": "typball-9.8.7-windows-x86_64.zip", "browser_download_url": "https://example.invalid/a.zip", "size": 10},
            {"name": "typball-9.8.7-windows-x86_64-setup.exe", "browser_download_url": "https://example.invalid/s.exe", "size": 20},
            {"name": "SHA256SUMS.txt", "browser_download_url": "https://example.invalid/sums", "size": 3}
        ]
    }"###;

    const NIGHTLY_JSON: &str = r###"{
        "tag_name": "nightly",
        "name": "Typball nightly 2026-11-10 12:00 UTC, abc1234",
        "body": "build-time: 1794312000\r\ncommit: abc1234def5678\r\n\r\n開発版です\r\n",
        "html_url": "https://github.com/Typball/Typball/releases/tag/nightly",
        "created_at": "2026-11-09T00:00:00Z",
        "target_commitish": "0000000",
        "prerelease": true,
        "assets": [
            {"name": "typball-nightly-windows-x86_64.zip", "browser_download_url": "https://example.invalid/n.zip", "size": 10}
        ]
    }"###;

    #[test]
    fn nightly_release_is_identified_by_commit_time() {
        let n = parse_release(NIGHTLY_JSON).unwrap();
        assert!(n.is_nightly());
        assert_eq!(n.asset_prefix, "typball-nightly");
        // 本文の build-time / commit が created_at / target_commitish より優先される
        assert_eq!(
            n.kind,
            Kind::Nightly {
                commit_time: 1794312000,
                commit: "abc1234def56".to_string()
            }
        );
        assert_eq!(n.label(), "nightly 2026-11-10 12:00 UTC, abc1234def56");
        assert_eq!(n.id(), "nightly-abc1234def56");
        // 手元のビルドはコミット不明 (0) なので新しいとみなす
        assert!(n.is_newer_than_current() || current_commit_time() >= 1794312000);
        if cfg!(all(windows, target_arch = "x86_64")) {
            assert!(n.package(InstallKind::Portable).is_some());
            assert!(n.package(InstallKind::Installer).is_none());
        }
        // 本文に無ければ created_at を使う
        let fallback = parse_release(
            r#"{"tag_name":"nightly","body":"","html_url":"","created_at":"2026-11-09T00:00:00Z","target_commitish":"0123456789abcdef"}"#,
        )
        .unwrap();
        assert_eq!(
            fallback.kind,
            Kind::Nightly {
                commit_time: 1794268800 - 86400,
                commit: "0123456789ab".to_string()
            }
        );
        assert!(parse_release(r#"{"tag_name":"nightly","html_url":""}"#).is_err());
        assert!(same_commit("abc1234", "abc1234def5678") && !same_commit("", "abc"));
        assert!(version_label().starts_with(env!("CARGO_PKG_VERSION")));
    }

    fn stable(v: &str) -> Release {
        Release {
            kind: Kind::Stable(Version::parse(v).unwrap()),
            asset_prefix: format!("typball-{v}"),
            title: String::new(),
            notes: String::new(),
            url: String::new(),
            assets: Vec::new(),
        }
    }

    fn nightly(commit_time: u64, commit: &str) -> Release {
        Release {
            kind: Kind::Nightly {
                commit_time,
                commit: commit.to_string(),
            },
            asset_prefix: "typball-nightly".to_string(),
            title: String::new(),
            notes: String::new(),
            url: String::new(),
            assets: Vec::new(),
        }
    }

    fn build(version: &str, commit_time: u64, commit: &str, nightly: bool) -> Build {
        Build {
            version: Version::parse(version).unwrap(),
            commit_time,
            commit: commit.to_string(),
            nightly,
        }
    }

    /// 勧められた版の表記 (無ければ "up-to-date" / "no-release")
    fn offer(check: Check) -> String {
        match check {
            Check::Available(r) => r.label(),
            Check::UpToDate => "up-to-date".to_string(),
            Check::NoRelease => "no-release".to_string(),
            Check::Failed(e) => format!("failed: {e}"),
        }
    }

    /// 安定版と開発版を行き来できるか。時系列: コミット c0 (t=1000) で v0.1.0 を公開 (nightly も c0)、
    /// c2 (t=2000) は 0.1.0 のままの開発、c3 (t=3000) で 0.1.1 に上げて公開 (nightly も c3)
    #[test]
    fn channels_can_be_switched_both_ways() {
        use Channel::{Nightly, Stable};
        let n0 = || nightly(1000, "c0");
        let n2 = || nightly(2000, "c2");
        let n3 = || nightly(3000, "c3");
        let s010 = || stable("0.1.0");
        let s011 = || stable("0.1.1");
        let n_c2 = nightly(2000, "c2").label();
        let n_c3 = nightly(3000, "c3").label();

        // 安定版 0.1.0 (c0) を使っている
        let st = build("0.1.0", 1000, "c0", false);
        assert_eq!(offer(decide(Stable, &st, None, Some(s010()))), "up-to-date");
        assert_eq!(offer(decide(Stable, &st, None, Some(s011()))), "0.1.1");
        // → 開発版チャネルにすると、タグ以降の nightly へ移れる (同じコミットの nightly は勧めない)
        assert_eq!(offer(decide(Nightly, &st, Some(n2()), Some(s010()))), n_c2);
        assert_eq!(
            offer(decide(Nightly, &st, Some(n0()), Some(s010()))),
            "up-to-date"
        );

        // 開発版 (0.1.0 のまま、c2) を使っている
        let nb = build("0.1.0", 2000, "c2", true);
        assert_eq!(
            offer(decide(Nightly, &nb, Some(n2()), Some(s010()))),
            "up-to-date"
        );
        assert_eq!(offer(decide(Nightly, &nb, Some(n3()), Some(s011()))), n_c3);
        // nightly の Release が作り直しの最中で無くても、番号の新しい安定版は勧める
        assert_eq!(offer(decide(Nightly, &nb, None, Some(s011()))), "0.1.1");
        // → 安定版チャネルに戻すと、番号が同じでも安定版 0.1.0 へ戻れる
        assert_eq!(offer(decide(Stable, &nb, None, Some(s010()))), "0.1.0");
        assert!(!s010().is_newer_than(&nb));
        // 安定版がまだ無ければ戻り先は無い
        assert_eq!(offer(decide(Stable, &nb, None, None)), "no-release");

        // 安定版 0.1.0 に戻った後は、同じ 0.1.0 を勧め続けない
        let back = build("0.1.0", 1000, "c0", false);
        assert_eq!(
            offer(decide(Stable, &back, None, Some(s010()))),
            "up-to-date"
        );

        // Release が 1 つも無い / nightly だけある
        assert_eq!(offer(decide(Stable, &st, None, None)), "no-release");
        assert_eq!(offer(decide(Nightly, &nb, Some(n2()), None)), "up-to-date");
        assert_eq!(offer(decide(Nightly, &st, Some(n0()), None)), "up-to-date");
    }

    #[test]
    fn utc_time_round_trips() {
        assert_eq!(parse_rfc3339("2026-11-10T12:00:00Z"), Some(1794312000));
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(format_utc(1794312000), "2026-11-10 12:00");
        assert_eq!(format_utc(951782400), "2000-02-29 00:00");
        assert_eq!(parse_rfc3339("2000-02-29T00:00:00Z"), Some(951782400));
        assert!(parse_rfc3339("yesterday").is_none());
    }

    #[test]
    fn parses_release_and_picks_package() {
        let r = parse_release(JSON).unwrap();
        assert_eq!(r.kind, Kind::Stable(Version::new(9, 8, 7)));
        assert!(!r.is_nightly());
        assert_eq!(r.asset_prefix, "typball-9.8.7");
        assert_eq!(r.title, "Typball v9.8.7");
        assert_eq!(r.notes, "## Changes\n- one\n");
        assert_eq!(r.label(), "9.8.7");
        assert!(r.is_newer_than_current());
        assert_eq!(r.sums().unwrap().size, 3);
        if cfg!(all(windows, target_arch = "x86_64")) {
            assert_eq!(
                r.package(InstallKind::Installer).unwrap().name,
                "typball-9.8.7-windows-x86_64-setup.exe"
            );
            assert!(r
                .package(InstallKind::Portable)
                .unwrap()
                .name
                .ends_with(".zip"));
        } else {
            assert!(r.package(InstallKind::Portable).is_none());
        }
        assert!(parse_release(r#"{"tag_name":"nightly","html_url":""}"#).is_err());
    }

    #[test]
    fn installer_script_waits_then_runs_setup_and_restarts() {
        let script = installer_script(
            Path::new("C:\\tmp\\typball-1.0.0-windows-x86_64-setup.exe"),
            Path::new("C:\\Apps\\Typball\\Typball.exe"),
            4242,
            &[PathBuf::from("C:\\docs\\a b.md")],
        );
        assert!(script.contains("tasklist /FI \"PID eq 4242\""));
        assert!(script.contains(
            "\"C:\\tmp\\typball-1.0.0-windows-x86_64-setup.exe\" /VERYSILENT /SUPPRESSMSGBOXES"
        ));
        assert!(script
            .contains("start \"\" \"C:\\Apps\\Typball\\Typball.exe\" \"C:\\docs\\a b.md\"\r\n"));
        assert!(script.ends_with("del \"%~f0\"\r\n"));
    }

    #[test]
    fn sha256_sums_are_matched_by_name() {
        let dir = std::env::temp_dir().join(format!("typball-update-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.zip");
        fs::write(&file, b"abc").unwrap();
        let hex = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(sha256_hex(&file).unwrap(), hex);
        let sums = format!("{hex}  dist/a.zip\n0000 *other\n");
        assert_eq!(expected_sha256(&sums, "a.zip").as_deref(), Some(hex));
        assert!(verify_sha256(&file, &sums, "a.zip").is_ok());
        assert!(verify_sha256(&file, &sums, "other").is_err());
        assert!(verify_sha256(&file, &sums.replace('b', "c"), "a.zip").is_err());
        fs::remove_dir_all(&dir).unwrap();
    }
}
