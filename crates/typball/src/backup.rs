//! 未保存の文書のスナップショット (クラッシュ・強制終了からの復元)
//!
//! Notepad++ の periodic backup / VSCode の hot exit と同じ仕組み。未保存の変更があるタブの
//! 全文を数秒ごとに設定フォルダの `Backup/<pid>/` に書き、保存するか閉じたら消す。
//! 次の起動時に、もう動いていないプロセスのフォルダが残っていれば (= 落ちた・強制終了した)、
//! その中身を「未保存の変更あり」のタブとして開き直す。
//!
//! 動いているプロセスの見分けは `lock` ファイルのロック (`File::try_lock`) で行う。
//! PID の再利用に影響されず、プロセスが死ねば OS がロックを外す。
//!
//! 書き込みは別スレッドで行う ([`write_async`])。設定フォルダが遅いドライブ (ポータブル運用の
//! USB メモリなど) にあっても GUI を止めない。同時に走るのは 1 つだけで、その間に同じ文書の
//! [`remove`] が来たら書き終えてから消す (保存済みなのに古いスナップショットが残らないように)。

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::fileio;
use crate::settings;

/// スナップショットの脇に置くメタ情報 (`<id>.toml`)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Meta {
    /// 元のファイル (無題なら None)
    pub path: Option<String>,
    /// 拡張子 ("txt" / "md" / "typ")
    pub filetype: String,
    /// 文字コード (`fileio::Encoding::id`)
    pub encoding: String,
    /// 無題の番号
    pub untitled: usize,
}

/// 復元した 1 文書
pub struct Recovered {
    /// 書いたプロセスと、その中での文書の番号 (`Tab::backup_id`)。セッション (`session.rs`) の
    /// タブと結び付けて、元のウィンドウの元の位置に戻すのに使う
    pub pid: u32,
    pub id: u64,
    pub meta: Meta,
    pub text: String,
}

/// 自分のフォルダの `lock` を開いたまま持つ
static LOCK: OnceLock<Mutex<Option<File>>> = OnceLock::new();

/// 別スレッドで書いている最中のスナップショット
#[derive(Default)]
struct InFlight {
    /// 書いている文書の id
    id: Option<u64>,
    /// 書いている間に `remove` が来た (書き終えたら消す)
    removed: bool,
    /// 直前の書き込みが失敗した文書の id (GUI スレッドが次の tick で dirty に戻す)
    failed: Option<u64>,
}

static IN_FLIGHT: Mutex<InFlight> = Mutex::new(InFlight {
    id: None,
    removed: false,
    failed: None,
});

fn in_flight() -> std::sync::MutexGuard<'static, InFlight> {
    IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner())
}

/// 別スレッドで書いている最中か (次の tick まで待つ)
pub fn is_writing() -> bool {
    in_flight().id.is_some()
}

/// 直前の書き込みが失敗した文書の id を取り出す (呼ぶと消える)
pub fn take_failed() -> Option<u64> {
    in_flight().failed.take()
}

/// 文書 `id` のスナップショットを別スレッドで書く。別の書き込みが走っていれば false (何もしない)
pub fn write_async(id: u64, text: String, meta: Meta) -> bool {
    {
        let mut state = in_flight();
        if state.id.is_some() {
            return false;
        }
        state.id = Some(id);
        state.removed = false;
    }
    std::thread::spawn(move || {
        let result = write(id, &text, &meta);
        // remove() も同じロックを取るので、ロックを手放してから呼ぶ
        let removed = {
            let mut state = in_flight();
            let removed = state.removed;
            if !removed && result.is_err() {
                state.failed = Some(id);
            }
            state.id = None;
            state.removed = false;
            removed
        };
        if removed {
            remove(id);
        }
    });
    true
}

const LOCK_NAME: &str = "lock";

/// スナップショットのフォルダの名前。設定フォルダの中の名前は、`Settings.toml` と同じく頭を大文字にする
const DIR_NAME: &str = "Backup";
/// 前の版での名前 (全部小文字だった)
const OLD_DIR_NAME: &str = "backup";

/// スナップショット全体のフォルダ (設定ファイルと同じ場所の `Backup`)
pub fn root_dir() -> PathBuf {
    let parent = settings::path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    // 前の版が作った `backup` があれば、最初に 1 回だけ名前を変える
    static RENAMED: std::sync::Once = std::sync::Once::new();
    RENAMED.call_once(|| rename_old_dir(&parent));
    parent.join(DIR_NAME)
}

/// 前の版のフォルダ (`backup`) を今の名前 (`Backup`) に変える。中に残っているスナップショット (落ちたときのもの) も
/// 一緒に移るので、名前を変えた後の起動でも復元できる。
///
/// 大文字と小文字を区別しないファイルシステム (Windows、macOS の既定) では、`Backup` で開いても `backup` が開くので、
/// 何もしなくても動くが、見える名前が小文字のまま残る。名前の変更に失敗しても (ほかの Typball が中のファイルを
/// 開いている、など) 困らない: 区別しないファイルシステムでは同じフォルダを使い続け、区別するファイルシステムでは
/// 新しいフォルダを作る
fn rename_old_dir(parent: &Path) {
    // フォルダの実際の名前を調べる (`exists` は、区別しないファイルシステムではどちらの名前でも true になる)
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    let names: Vec<std::ffi::OsString> = entries
        .flatten()
        .filter(|entry| entry.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|entry| entry.file_name())
        .collect();
    let has = |name: &str| names.iter().any(|n| n == name);
    if has(OLD_DIR_NAME) && !has(DIR_NAME) {
        let _ = fs::rename(parent.join(OLD_DIR_NAME), parent.join(DIR_NAME));
    }
}

/// このプロセスのフォルダ
fn own_dir() -> PathBuf {
    root_dir().join(std::process::id().to_string())
}

/// 自分のフォルダを作ってロックする。書く直前に呼ぶ (起動時にフォルダを作っておく必要はない)
fn ensure_own_dir() -> io::Result<PathBuf> {
    let dir = own_dir();
    let lock = LOCK.get_or_init(|| Mutex::new(None));
    let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        fs::create_dir_all(&dir)?;
        let file = File::create(dir.join(LOCK_NAME))?;
        file.try_lock()
            .map_err(|e| io::Error::other(e.to_string()))?;
        *guard = Some(file);
    }
    Ok(dir)
}

fn text_path(dir: &Path, id: u64) -> PathBuf {
    dir.join(format!("{id}.txt"))
}

fn meta_path(dir: &Path, id: u64) -> PathBuf {
    dir.join(format!("{id}.toml"))
}

/// 文書 `id` のスナップショットを書く
pub fn write(id: u64, text: &str, meta: &Meta) -> io::Result<()> {
    let dir = ensure_own_dir()?;
    let meta_text = toml::to_string(meta).map_err(|e| io::Error::other(e.to_string()))?;
    // 本文を先に書く。メタ情報が無くても本文だけは拾える (restore 側で既定値にする)
    fileio::write(&text_path(&dir, id), text.as_bytes())?;
    fileio::write(&meta_path(&dir, id), meta_text.as_bytes())
}

/// 文書 `id` のスナップショットを消す (保存した・閉じた)
pub fn remove(id: u64) {
    {
        let mut state = in_flight();
        if state.id == Some(id) {
            state.removed = true;
        }
    }
    let dir = own_dir();
    let _ = fs::remove_file(text_path(&dir, id));
    let _ = fs::remove_file(meta_path(&dir, id));
}

/// 終了時: 自分のフォルダを消す (未保存のタブは閉じる前に確認済みなので、残っていても要らない)
pub fn discard_own() {
    let dir = own_dir();
    if let Some(lock) = LOCK.get() {
        if let Ok(mut guard) = lock.lock() {
            *guard = None;
        }
    }
    let _ = fs::remove_dir_all(dir);
}

/// もう動いていないプロセスのスナップショットを集めて、そのフォルダを消す
pub fn recover() -> Vec<Recovered> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(root_dir()) else {
        return out;
    };
    let own = own_dir();
    for entry in entries.filter_map(|e| e.ok()) {
        let dir = entry.path();
        if !dir.is_dir() || dir == own {
            continue;
        }
        // lock が取れなければそのプロセスはまだ動いている
        let lock_file = dir.join(LOCK_NAME);
        let held = match File::options().write(true).open(&lock_file) {
            Ok(f) => match f.try_lock() {
                Ok(()) => Some(f),
                Err(_) => continue,
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(_) => continue,
        };
        out.extend(read_dir_snapshots(&dir));
        drop(held);
        let _ = fs::remove_dir_all(&dir);
    }
    out
}

fn read_dir_snapshots(dir: &Path) -> Vec<Recovered> {
    let mut out = Vec::new();
    // フォルダの名前が、書いたプロセスの ID
    let pid = dir
        .file_name()
        .and_then(|n| n.to_string_lossy().parse().ok())
        .unwrap_or(0);
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    let mut ids: Vec<u64> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.strip_suffix(".txt").and_then(|s| s.parse().ok())
        })
        .collect();
    ids.sort_unstable();
    for id in ids {
        let Ok(bytes) = fs::read(text_path(dir, id)) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let meta = fs::read_to_string(meta_path(dir, id))
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default();
        out.push(Recovered {
            pid,
            id,
            meta,
            text,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_in_an_unlocked_dir_are_read_in_id_order() {
        let dir = std::env::temp_dir().join(format!("typball-backup-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(text_path(&dir, 2), "two").unwrap();
        fs::write(
            meta_path(&dir, 2),
            toml::to_string(&Meta {
                path: Some("C:/x/a.md".into()),
                filetype: "md".into(),
                encoding: "utf-8".into(),
                untitled: 0,
            })
            .unwrap(),
        )
        .unwrap();
        fs::write(text_path(&dir, 1), "one").unwrap();
        let got = read_dir_snapshots(&dir);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].text, "one");
        assert_eq!(got[0].meta.path, None);
        assert_eq!(got[1].text, "two");
        assert_eq!(got[1].meta.filetype, "md");
        let _ = fs::remove_dir_all(&dir);
    }

    /// フォルダの中の、フォルダの実際の名前
    fn dir_names(parent: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(parent)
            .unwrap()
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn the_old_folder_name_is_renamed_with_its_contents() {
        let parent =
            std::env::temp_dir().join(format!("typball-backup-rename-{}", std::process::id()));
        let _ = fs::remove_dir_all(&parent);
        // 前の版が残したスナップショット
        let old = parent.join(OLD_DIR_NAME).join("1234");
        fs::create_dir_all(&old).unwrap();
        fs::write(text_path(&old, 7), "unsaved").unwrap();

        rename_old_dir(&parent);
        assert_eq!(dir_names(&parent), [DIR_NAME]);
        let moved = parent.join(DIR_NAME).join("1234");
        assert_eq!(fs::read_to_string(text_path(&moved, 7)).unwrap(), "unsaved");

        // もう一度呼んでも、そのまま
        rename_old_dir(&parent);
        assert_eq!(dir_names(&parent), [DIR_NAME]);
        // 何も無いフォルダでは何も作らない
        let empty = parent.join("empty");
        fs::create_dir_all(&empty).unwrap();
        rename_old_dir(&empty);
        assert!(dir_names(&empty).is_empty());
        let _ = fs::remove_dir_all(&parent);
    }
}
