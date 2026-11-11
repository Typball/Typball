//! 未保存の文書のスナップショット (クラッシュ・強制終了からの復元)
//!
//! Notepad++ の periodic backup / VSCode の hot exit と同じ仕組み。未保存の変更があるタブの
//! 全文を数秒ごとに設定フォルダの `backup/<pid>/` に書き、保存するか閉じたら消す。
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

/// スナップショット全体のフォルダ (設定ファイルと同じ場所の `backup`)
pub fn root_dir() -> PathBuf {
    settings::path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("backup")
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
        out.push(Recovered { meta, text });
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
}
