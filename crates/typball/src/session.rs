//! セッション: ウィンドウごとのタブの状態の記録 (復元は `window.rs`、一覧の画面は `session_dialog.rs`)
//!
//! 設定フォルダの `Session/` に、**起動 1 回につき 1 ファイル** (`<開始日時>-<pid>.toml`) を書く。
//! 中身は「どのウィンドウに、どのファイルが、どの順で開いているか」で、動いている間は数秒ごとに
//! (変わったときだけ) 書き直す。だから保存の操作は要らず、落ちても直前の状態が残っている。
//!
//! - **正常に終了した**: 終了した時点で開いていたウィンドウを書き、`ended` に日時を入れる。
//!   「ファイル > 終了」は全ウィンドウ、ウィンドウを 1 つずつ閉じたなら最後の 1 つが「開いていたウィンドウ」。
//! - **途中で閉じたウィンドウ**: 消さずに `closed` (閉じた日時) を付けて同じファイルに残す
//!   (一覧の画面から、そのウィンドウだけ開き直せる)。
//! - **中断された** (落ちた・強制終了・OS のシャットダウン): `ended` が空のまま残る。次の起動で
//!   ウィンドウとタブを開き直し、未保存だった内容は `backup.rs` のスナップショット
//!   (`Backup/<pid>/<番号>.txt`) から戻す。タブごとに書いてある `snapshot` がその番号。
//! - **名前を付けて保存**: `Session/Saved/<名前>.toml` に写しを置く。こちらは自動では消えない。
//!
//! 動いているプロセスの見分けは、`backup.rs` と同じく `<id>.lock` のロックで行う
//! (PID の再利用に影響されず、プロセスが死ねば OS が外す)。
//!
//! 書き込みは別スレッドで行う (設定フォルダが遅いドライブにあっても GUI を止めない)。古い内容が
//! 新しい内容を上書きしないよう、通し番号を付けて順番を守る。終了時の最後の書き込みだけは同期。

use std::cell::RefCell;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::fileio;
use crate::settings;

/// セッションのフォルダの名前 (設定ファイルと同じ場所)
const DIR_NAME: &str = "Session";
/// 名前を付けて保存したセッションのフォルダ (`Session/Saved`)
const SAVED_DIR: &str = "Saved";
const VERSION: u32 = 1;
/// 実行中に閉じたウィンドウを覚えておく数 (古いものから捨てる)
const MAX_CLOSED: usize = 20;
/// 名前を付けて保存するときのファイル名の長さの上限 (文字数)
const MAX_NAME_CHARS: usize = 80;

/// 1 回の起動 (または名前を付けて保存した) ぶんの状態
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub version: u32,
    /// 名前 (名前を付けて保存したものだけ)
    pub name: String,
    /// 始めた日時 ("2026-11-19 15:30:12"、その PC の時刻)
    pub started: String,
    /// 最後に中身が変わった日時
    pub updated: String,
    /// 正常に終了した日時。動いている間と、中断された (落ちた・シャットダウン) ときは空
    pub ended: String,
    /// 中断されたセッションを、次の起動で引き取った (もう自動では復元しない)
    pub recovered: bool,
    /// 書いたプロセス。スナップショット (`Backup/<pid>/`) と結び付ける
    pub pid: u32,
    #[serde(rename = "window")]
    pub windows: Vec<WindowState>,
}

/// ウィンドウ 1 つ分
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowState {
    /// 位置 (画面の座標) と大きさ (DIP)。`settings::Window` と同じ意味
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub maximized: bool,
    /// 選んでいたタブ (`tabs` の添字)
    pub active: usize,
    /// 実行中に閉じたウィンドウなら、閉じた日時。最後まで開いていたものは空
    pub closed: String,
    #[serde(rename = "tab")]
    pub tabs: Vec<TabState>,
}

/// タブ 1 つ分
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TabState {
    /// ファイル (無題なら None)
    pub path: Option<String>,
    /// 拡張子 ("txt" / "md" / "typ" / "tbdg")
    pub filetype: String,
    /// 文字コード (`fileio::Encoding::id`)
    pub encoding: String,
    /// 文字コードを利用者が指定していた (開き直すときもそれで読む)
    pub forced: bool,
    /// 無題の番号
    pub untitled: usize,
    /// 無題のタブに、保存していない内容があった (ファイルのあるタブでは記録しない)。
    /// 終わったセッションに残すのは、こういう無題のタブだけ: 開き直せないが、一覧で
    /// 「このウィンドウには無題の文書もあった」と分かる。作っただけの空の無題は残さない
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub modified: bool,
    /// 未保存の変更のスナップショットの番号 (`Tab::backup_id`)。動いている間だけ意味がある
    pub snapshot: Option<u64>,
    /// キャレットのある行 (1 始まり。0 は不明)
    pub line: i32,
    /// 画面の一番上の行 (文書の行、0 始まり)
    pub top: i32,
}

impl TabState {
    /// 一覧に出すタブ: ファイルのあるタブと、保存していない内容のあった無題のタブ
    /// (後者は開き直せない。作っただけの空の無題は出さない)
    pub fn is_listed(&self) -> bool {
        self.path.is_some() || self.modified
    }
}

impl WindowState {
    /// 終わったセッションの形にする: スナップショットの番号を落とし (プロセスが終わると意味が無い)、
    /// ファイルのあるタブを残す。`keep_untitled` なら、保存していない内容のあった無題のタブも
    /// 「あった」という記録として残す (開き直せない)
    fn settle(&mut self, keep_untitled: bool) {
        let mut index = 0;
        let mut active = 0;
        let was_active = self.active;
        let mut kept = 0;
        self.tabs.retain(|t| {
            let keep = t.path.is_some() || (keep_untitled && t.modified);
            if keep {
                // 選んでいたタブは、抜けたタブの分だけ前へずれる (抜けたなら先頭)
                if index == was_active {
                    active = kept;
                }
                kept += 1;
            }
            index += 1;
            keep
        });
        for tab in &mut self.tabs {
            tab.snapshot = None;
            if tab.path.is_some() {
                tab.untitled = 0;
                tab.modified = false;
            }
        }
        self.active = active;
    }

    pub fn is_closed(&self) -> bool {
        !self.closed.is_empty()
    }

    /// 開き直せるタブ (ファイルのあるタブ) があるか
    pub fn has_files(&self) -> bool {
        self.tabs.iter().any(|t| t.path.is_some())
    }

    /// ファイルのあるタブの並び (無題を除く)
    fn files(&self) -> Vec<&str> {
        self.tabs.iter().filter_map(|t| t.path.as_deref()).collect()
    }
}

impl Session {
    /// 終わったセッションの形にする ([`WindowState::settle`])。タブの無くなったウィンドウは除く
    fn settle(&mut self, keep_untitled: bool) {
        for window in &mut self.windows {
            window.settle(keep_untitled);
        }
        self.windows.retain(|w| !w.tabs.is_empty());
    }

    /// 開き直せるもの (ファイルのあるタブ) が 1 つも無い
    pub fn is_empty(&self) -> bool {
        !self.windows.iter().any(WindowState::has_files)
    }

    /// 終了時まで開いていたウィンドウ (途中で閉じたものを除く)
    pub fn open_windows(&self) -> Vec<WindowState> {
        self.windows
            .iter()
            .filter(|w| !w.is_closed())
            .cloned()
            .collect()
    }

    /// 開いていたウィンドウの数とタブの数 (一覧の表示用。保存していなかった無題のタブも数える)
    pub fn counts(&self) -> (usize, usize) {
        let tabs: Vec<usize> = self
            .windows
            .iter()
            .filter(|w| !w.is_closed())
            .map(|w| w.tabs.iter().filter(|t| t.is_listed()).count())
            .filter(|n| *n > 0)
            .collect();
        (tabs.len(), tabs.iter().sum())
    }

    fn to_toml(&self) -> io::Result<String> {
        let body = toml::to_string_pretty(self).map_err(|e| io::Error::other(e.to_string()))?;
        Ok(format!(
            "# Typball session (https://github.com/Typball/Typball)\n\n{body}"
        ))
    }

    fn read(path: &Path) -> Option<Session> {
        let text = fs::read_to_string(path).ok()?;
        toml::from_str(&text).ok()
    }

    /// どのウィンドウに、どのファイルが、どの順で開いていたか (途中で閉じたウィンドウかどうかも)。
    /// 日時や位置を除いた中身で、同じセッションの繰り返しを見分けるのに使う
    fn contents(&self) -> Vec<(bool, Vec<Option<String>>)> {
        self.windows
            .iter()
            .map(|w| {
                (
                    w.is_closed(),
                    w.tabs.iter().map(|t| t.path.clone()).collect(),
                )
            })
            .collect()
    }

    /// `updated` 以外が同じか (中身が変わっていなければ書き直さない)
    fn same_content(&self, other: &Session) -> bool {
        let mut a = self.clone();
        a.updated.clone_from(&other.updated);
        a == *other
    }
}

// ---------------------------------------------------------------------------
// 場所
// ---------------------------------------------------------------------------

/// セッションのフォルダ (設定ファイルと同じ場所の `Session`)
pub fn dir() -> PathBuf {
    settings::path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(DIR_NAME)
}

fn session_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.toml"))
}

fn lock_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.lock"))
}

/// 今の日時 ("2026-11-19 15:30:12"、この PC の時刻)。GUI スレッドから呼ぶ
pub fn now() -> String {
    let t = wxdragon::prelude::DateTime::now();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        t.year(),
        t.month(),
        t.day(),
        t.hour(),
        t.minute(),
        t.second()
    )
}

/// 日時をファイル名の形にする ("2026-11-19 15:30:12" → "20261119-153012")
fn compact(time: &str) -> String {
    time.chars()
        .filter_map(|c| match c {
            '0'..='9' => Some(c),
            ' ' => Some('-'),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 書き込み (順番を守る)
// ---------------------------------------------------------------------------

/// 書き込みの通し番号 (新しいものほど大きい)
static NEXT_SEQ: AtomicU64 = AtomicU64::new(1);
/// 最後に反映した書き込みの番号。書き込みはこのロックを持って 1 つずつ行う
static WRITTEN: Mutex<u64> = Mutex::new(0);

fn next_seq() -> u64 {
    NEXT_SEQ.fetch_add(1, Ordering::Relaxed)
}

/// `seq` がこれまでに反映したものより新しければ書く (None なら消す)
fn apply(path: &Path, text: Option<&str>, seq: u64) -> io::Result<()> {
    let mut last = WRITTEN.lock().unwrap_or_else(|e| e.into_inner());
    if seq <= *last {
        return Ok(());
    }
    match text {
        Some(text) => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fileio::write(path, text.as_bytes())?;
        }
        None => match fs::remove_file(path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        },
    }
    *last = seq;
    Ok(())
}

// ---------------------------------------------------------------------------
// 動いているセッション (このプロセスのもの)
// ---------------------------------------------------------------------------

struct Live {
    /// ファイル名 (拡張子無し): `<開始日時>-<pid>`
    id: String,
    started: String,
    /// `<id>.lock` を開いたまま持つ (動いている印)
    lock: Option<File>,
    /// 実行中に閉じたウィンドウ (古い順)
    closed: Vec<WindowState>,
    /// 「終了」の途中 (閉じたウィンドウは「最後まで開いていた」扱いにする)
    quitting: bool,
    /// OS のシャットダウンで終わろうとしている (次の起動で開き直せるよう、正常終了の印を付けない)
    shutdown: bool,
    /// 「終了」の途中で閉じたウィンドウ
    exit_windows: Vec<WindowState>,
    /// 最後に書いた (書くよう頼んだ) 内容
    last: Option<Session>,
    /// ファイルを書いたことがある
    written: bool,
    /// 最後のウィンドウが閉じて、終わりの書き込みを済ませた
    finished: bool,
}

thread_local! {
    static LIVE: RefCell<Option<Live>> = const { RefCell::new(None) };
}

/// このプロセスのセッションを始める (ファイルは最初の記録のときに作る)
pub fn start(now: &str) {
    let id = format!("{}-{}", compact(now), std::process::id());
    LIVE.with(|live| {
        *live.borrow_mut() = Some(Live {
            id,
            started: now.to_string(),
            lock: None,
            closed: Vec::new(),
            quitting: false,
            shutdown: false,
            exit_windows: Vec::new(),
            last: None,
            written: false,
            finished: false,
        });
    });
}

/// このプロセスのセッションの名前 (ファイル名、拡張子無し)
fn own_id() -> Option<String> {
    LIVE.with(|live| live.borrow().as_ref().map(|l| l.id.clone()))
}

impl Live {
    /// 今の状態: 「終了」の途中で閉じたウィンドウ → 開いているウィンドウ → 途中で閉じたウィンドウ
    fn compose(&self, open: Vec<WindowState>, now: &str) -> Session {
        let mut windows = self.exit_windows.clone();
        windows.extend(open);
        windows.extend(self.closed.iter().cloned());
        Session {
            version: VERSION,
            name: String::new(),
            started: self.started.clone(),
            updated: now.to_string(),
            ended: String::new(),
            recovered: false,
            pid: std::process::id(),
            windows,
        }
    }

    /// `<id>.lock` を作ってロックする (最初に書く直前)
    fn ensure_lock(&mut self, dir: &Path) {
        if self.lock.is_some() {
            return;
        }
        if fs::create_dir_all(dir).is_err() {
            return;
        }
        if let Ok(file) = File::create(lock_path(dir, &self.id)) {
            if file.try_lock().is_ok() {
                self.lock = Some(file);
            }
        }
    }
}

/// 今のセッションの中身 (書かない)。一覧の画面と「名前を付けて保存」で使う
pub fn snapshot(open: Vec<WindowState>, now: &str) -> Option<Session> {
    LIVE.with(|live| live.borrow().as_ref().map(|l| l.compose(open, now)))
}

/// 開いているウィンドウの状態を記録する (数秒ごと・ウィンドウを閉じたとき)。変わっていなければ何もしない
pub fn record(open: Vec<WindowState>, now: &str) {
    write_current(open, now, false);
}

/// 記録して、書き終わるまで待つ (OS のシャットダウンで、この後すぐプロセスが終わるとき)
pub fn record_now(open: Vec<WindowState>, now: &str) {
    write_current(open, now, true);
}

fn write_current(open: Vec<WindowState>, now: &str, wait: bool) {
    // 最後のウィンドウが閉じた後に、また編集のウィンドウができた (プレビューウィンドウの「編集」など):
    // 新しいセッションとして始め直す
    if LIVE.with(|live| live.borrow().as_ref().is_none_or(|l| l.finished)) {
        start(now);
    }
    let dir = dir();
    let job = LIVE.with(|live| {
        let mut live = live.borrow_mut();
        let live = live.as_mut()?;
        let session = live.compose(open, now);
        if live.last.as_ref().is_some_and(|l| session.same_content(l)) {
            return None;
        }
        live.ensure_lock(&dir);
        live.last = Some(session.clone());
        live.written = true;
        Some((session_path(&dir, &live.id), session))
    });
    let Some((path, session)) = job else {
        return;
    };
    let Ok(text) = session.to_toml() else {
        return;
    };
    let seq = next_seq();
    if wait {
        let _ = apply(&path, Some(&text), seq);
    } else {
        std::thread::spawn(move || {
            let _ = apply(&path, Some(&text), seq);
        });
    }
}

/// ウィンドウが閉じた (他のウィンドウはまだある)。閉じたウィンドウとして覚える。
/// 「終了」の途中なら、最後まで開いていたウィンドウとして覚える
pub fn window_closed(mut state: WindowState, now: &str) {
    state.settle(true);
    if state.tabs.is_empty() {
        return;
    }
    LIVE.with(|live| {
        let mut live = live.borrow_mut();
        let Some(live) = live.as_mut() else {
            return;
        };
        if live.quitting {
            live.exit_windows.push(state);
        } else {
            state.closed = now.to_string();
            live.closed.push(state);
            let excess = live.closed.len().saturating_sub(MAX_CLOSED);
            live.closed.drain(..excess);
        }
    });
}

/// 閉じたウィンドウを開き直した: 「閉じたウィンドウ」として覚えておくのをやめる
/// (閉じた日時とファイルの並びが同じものを外す。タブの一部だけを開き直したものは残す。
/// 無題のタブは開き直せないので比べない)
pub fn forget_closed(restored: &[WindowState]) {
    LIVE.with(|live| {
        if let Some(live) = live.borrow_mut().as_mut() {
            live.closed.retain(|c| {
                !restored
                    .iter()
                    .any(|r| r.is_closed() && r.closed == c.closed && r.files() == c.files())
            });
        }
    });
}

/// 「終了」を始める (ファイル > 終了、更新の再起動、OS のシャットダウン): この後に閉じるウィンドウは
/// 「最後まで開いていた」ものとして記録する。`shutdown` は OS のシャットダウンによるもの
pub fn begin_quit(shutdown: bool) {
    LIVE.with(|live| {
        if let Some(live) = live.borrow_mut().as_mut() {
            live.quitting = true;
            live.shutdown |= shutdown;
        }
    });
}

/// 「終了」が途中で取り消された (未保存の確認でキャンセルした): それまでに閉じたウィンドウは、
/// 途中で閉じたウィンドウに回す
pub fn cancel_quit(now: &str) {
    LIVE.with(|live| {
        let mut live = live.borrow_mut();
        let Some(live) = live.as_mut() else {
            return;
        };
        if !live.quitting {
            return;
        }
        live.quitting = false;
        live.shutdown = false;
        for mut window in std::mem::take(&mut live.exit_windows) {
            window.closed = now.to_string();
            live.closed.push(window);
        }
    });
}

/// 最後のウィンドウが閉じた: 終わりの状態を書く (同期)。開き直せるものが無いセッションと、
/// セッションを残さない設定 (`keep = 0`) ならファイルを消す
pub fn finish(last_window: WindowState, now: &str) {
    let keep = settings::get().session.keep;
    let dir = dir();
    let job = LIVE.with(|live| {
        let mut live = live.borrow_mut();
        let live = live.as_mut()?;
        if live.finished {
            return None;
        }
        let mut last_window = last_window;
        last_window.settle(true);
        if !last_window.tabs.is_empty() {
            live.exit_windows.push(last_window);
        }
        let mut session = live.compose(Vec::new(), now);
        session.settle(true);
        // OS のシャットダウンで終わるときは印を付けない: 次の起動で「中断された」ものとして開き直す
        if !live.shutdown {
            session.ended = now.to_string();
        }
        live.finished = true;
        let lock = live.lock.take();
        let remove = session.is_empty() || (keep == 0 && !live.shutdown);
        Some((live.id.clone(), session, remove, live.written, lock))
    });
    let Some((id, session, remove, written, lock)) = job else {
        return;
    };
    let path = session_path(&dir, &id);
    let seq = next_seq();
    if remove {
        if written {
            let _ = apply(&path, None, seq);
        }
    } else if let Ok(text) = session.to_toml() {
        let _ = apply(&path, Some(&text), seq);
    }
    drop(lock);
    let _ = fs::remove_file(lock_path(&dir, &id));
}

// ---------------------------------------------------------------------------
// 過去のセッション
// ---------------------------------------------------------------------------

/// セッションの状態
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// 正常に終了した
    Ended,
    /// 中断された (落ちた・強制終了・OS のシャットダウン)
    Interrupted,
    /// 別の Typball のプロセスが今使っている
    Running,
    /// 名前を付けて保存したもの
    Saved,
}

/// 一覧の 1 件
#[derive(Debug, Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub status: Status,
    pub session: Session,
}

/// そのセッションを書いたプロセスがまだ動いているか (`<id>.lock` がロックされているか)
fn is_running(dir: &Path, id: &str) -> bool {
    match File::options().write(true).open(lock_path(dir, id)) {
        Ok(file) => file.try_lock().is_err(),
        Err(_) => false,
    }
}

/// フォルダの中の `.toml` (名前の新しい順)
fn toml_files(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<(String, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "toml"))
        .filter_map(|p| {
            let stem = p.file_stem()?.to_string_lossy().into_owned();
            Some((stem, p))
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out
}

/// 過去のセッション: 名前を付けて保存したもの (名前順) の後に、自動で記録したもの (新しい順)。
/// このプロセスのセッションは含まない
pub fn list() -> Vec<Entry> {
    list_in(&dir(), own_id().as_deref())
}

fn list_in(dir: &Path, own: Option<&str>) -> Vec<Entry> {
    let mut saved: Vec<Entry> = toml_files(&dir.join(SAVED_DIR))
        .into_iter()
        .filter_map(|(stem, path)| {
            let mut session = Session::read(&path)?;
            if session.name.is_empty() {
                session.name = stem;
            }
            Some(Entry {
                path,
                status: Status::Saved,
                session,
            })
        })
        .collect();
    saved.sort_by(|a, b| a.session.name.cmp(&b.session.name));
    let history = toml_files(dir)
        .into_iter()
        .filter(|(id, _)| Some(id.as_str()) != own)
        .filter_map(|(id, path)| {
            let session = Session::read(&path)?;
            let status = if !session.ended.is_empty() {
                Status::Ended
            } else if is_running(dir, &id) {
                Status::Running
            } else {
                Status::Interrupted
            };
            Some(Entry {
                path,
                status,
                session,
            })
        });
    saved.extend(history);
    saved
}

/// 前回のセッション: 自動で記録したもののうち、最後に更新されたもの (動いている別のプロセスのものと、
/// 開いていたウィンドウの無いものを除く)
pub fn previous() -> Option<Session> {
    list()
        .into_iter()
        .filter(|e| matches!(e.status, Status::Ended | Status::Interrupted))
        .map(|e| e.session)
        .filter(|s| s.open_windows().iter().any(WindowState::has_files))
        .max_by(|a, b| a.updated.cmp(&b.updated))
}

/// 中断された (落ちた・強制終了・シャットダウン) セッションを引き取る。引き取ったものには印を付けて、
/// 次の起動でもう一度復元しないようにする (復元の途中でまた落ちても、繰り返さない)。
/// 返すのは引き取る前の中身 (無題のタブとスナップショットの番号が入っている)
pub fn take_interrupted() -> Vec<Session> {
    take_interrupted_in(&dir(), own_id().as_deref())
}

fn take_interrupted_in(dir: &Path, own: Option<&str>) -> Vec<Session> {
    let mut out = Vec::new();
    for (id, path) in toml_files(dir) {
        if Some(id.as_str()) == own {
            continue;
        }
        let Some(session) = Session::read(&path) else {
            continue;
        };
        if !session.ended.is_empty() || session.recovered || is_running(dir, &id) {
            continue;
        }
        let mut settled = session.clone();
        settled.recovered = true;
        settled.settle(true);
        if settled.is_empty() {
            let _ = fs::remove_file(&path);
        } else if let Ok(text) = settled.to_toml() {
            let _ = fileio::write(&path, text.as_bytes());
        }
        let _ = fs::remove_file(lock_path(dir, &id));
        out.push(session);
    }
    // 古い順に返す (開き直す順)
    out.reverse();
    out
}

/// 一覧の 1 件を消す
pub fn delete(entry: &Entry) -> io::Result<()> {
    fs::remove_file(&entry.path)?;
    if entry.status != Status::Saved {
        let _ = fs::remove_file(entry.path.with_extension("lock"));
    }
    Ok(())
}

/// 自動で記録したセッションを新しい順に `keep` 個だけ残して消す (別スレッド)。
/// 動いているプロセスのものと、名前を付けて保存したものは消さない
pub fn prune(keep: usize) {
    let dir = dir();
    let own = own_id();
    std::thread::spawn(move || prune_in(&dir, own.as_deref(), keep));
}

fn prune_in(dir: &Path, own: Option<&str>, keep: usize) {
    let mut kept = 0;
    // 1 つ新しいセッションの中身 (同じものが続いたら、古い方は要らない)
    let mut newer: Option<Vec<(bool, Vec<Option<String>>)>> = None;
    for (id, path) in toml_files(dir) {
        if Some(id.as_str()) == own || is_running(dir, &id) {
            continue;
        }
        // 毎回同じセッションを開き直して終わると、同じ中身のものが並ぶ。新しい方だけ残す
        let contents = Session::read(&path).map(|s| s.contents());
        let duplicate = contents.is_some() && contents == newer;
        if contents.is_some() {
            newer = contents;
        }
        if !duplicate && kept < keep {
            kept += 1;
            continue;
        }
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(lock_path(dir, &id));
    }
    // 持ち主のいなくなったロックファイル (落ちたプロセスがセッションを書く前に作ったもの)
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for path in entries.flatten().map(|e| e.path()) {
        if path.extension().is_none_or(|x| x != "lock") || path.with_extension("toml").exists() {
            continue;
        }
        let Some(id) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
            continue;
        };
        if Some(id.as_str()) != own && !is_running(dir, &id) {
            let _ = fs::remove_file(&path);
        }
    }
}

// ---------------------------------------------------------------------------
// 名前を付けて保存
// ---------------------------------------------------------------------------

/// 名前をファイル名にする。使えない文字は `_` に置き換える。空になるなら None
pub fn file_stem_for(name: &str) -> Option<String> {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_control() || r#"\/:*?"<>|"#.contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(MAX_NAME_CHARS)
        .collect();
    // Windows は末尾の空白とピリオドを落とす
    let cleaned = cleaned.trim_end_matches(['.', ' ']).trim_start();
    if cleaned.is_empty() {
        return None;
    }
    // DOS のデバイス名 (NUL.toml などは、ファイルとして扱えない)
    let head = cleaned.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(head.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (head.len() == 4
            && (head.starts_with("COM") || head.starts_with("LPT"))
            && head.ends_with(|c: char| c.is_ascii_digit()));
    Some(if reserved {
        format!("_{cleaned}")
    } else {
        cleaned.to_string()
    })
}

/// 名前を付けて保存したセッションの置き場所 (名前が使えなければ None)
pub fn saved_path(name: &str) -> Option<PathBuf> {
    Some(
        dir()
            .join(SAVED_DIR)
            .join(format!("{}.toml", file_stem_for(name)?)),
    )
}

/// セッションに名前を付けて保存する (`Session/Saved/<名前>.toml`)。開いていたウィンドウの、
/// ファイルのあるタブだけを書く。同じ名前があれば上書きする
pub fn save_named(name: &str, session: &Session, now: &str) -> io::Result<PathBuf> {
    let path = saved_path(name)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty name"))?;
    let mut copy = named_copy(name, session, now);
    // 名前を付けたものは開き直すためのもの: 開き直せない無題のタブは入れない
    copy.settle(false);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fileio::write(&path, copy.to_toml()?.as_bytes())?;
    Ok(path)
}

fn named_copy(name: &str, session: &Session, now: &str) -> Session {
    let mut copy = session.clone();
    copy.name = name.trim().to_string();
    copy.updated = now.to_string();
    copy.ended = now.to_string();
    copy.recovered = false;
    copy.windows.retain(|w| !w.is_closed());
    copy
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(path: Option<&str>, snapshot: u64) -> TabState {
        TabState {
            path: path.map(str::to_string),
            filetype: "txt".to_string(),
            encoding: "utf-8".to_string(),
            forced: false,
            untitled: if path.is_none() { 1 } else { 0 },
            // 無題のタブは、書きかけの内容があったことにする
            modified: path.is_none(),
            snapshot: Some(snapshot),
            line: 3,
            top: 0,
        }
    }

    fn window(tabs: Vec<TabState>, active: usize) -> WindowState {
        WindowState {
            x: Some(10),
            y: Some(20),
            width: Some(800),
            height: Some(600),
            maximized: false,
            active,
            closed: String::new(),
            tabs,
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("typball-session-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, id: &str, session: &Session) {
        fs::write(session_path(dir, id), session.to_toml().unwrap()).unwrap();
    }

    #[test]
    fn settling_keeps_only_files_and_the_active_tab() {
        let mut w = window(
            vec![tab(None, 1), tab(Some("a.txt"), 2), tab(Some("b.txt"), 3)],
            2,
        );
        w.settle(false);
        assert_eq!(w.tabs.len(), 2);
        assert!(w.tabs.iter().all(|t| t.snapshot.is_none()));
        // 選んでいたタブ (b.txt) は、無題が抜けた分だけ前へずれる
        assert_eq!(w.active, 1);

        // 選んでいたのが無題なら先頭へ
        let mut w = window(vec![tab(Some("a.txt"), 1), tab(None, 2)], 1);
        w.settle(false);
        assert_eq!(w.active, 0);

        let mut s = Session {
            windows: vec![
                window(vec![tab(None, 1)], 0),
                window(vec![tab(Some("a.txt"), 2)], 0),
            ],
            ..Session::default()
        };
        assert!(!s.is_empty());
        s.settle(false);
        assert_eq!(s.windows.len(), 1, "a window with no files is dropped");
    }

    #[test]
    fn settling_keeps_the_untitled_tabs_that_had_content() {
        // 作っただけの無題 (空) / 書きかけの無題 / ファイル
        let mut blank = tab(None, 1);
        blank.modified = false;
        let mut w = window(vec![blank.clone(), tab(None, 2), tab(Some("a.txt"), 3)], 2);
        w.tabs[1].untitled = 4;
        w.settle(true);
        assert_eq!(w.tabs.len(), 2, "the blank untitled tab is dropped");
        assert!(w.tabs.iter().all(|t| t.snapshot.is_none()));
        // 書きかけの無題は番号ごと残る (一覧に「無題-4」と出す)。開き直せるものではない
        assert_eq!((w.tabs[0].path.as_deref(), w.tabs[0].untitled), (None, 4));
        assert!(w.tabs[0].is_listed() && w.tabs[0].modified);
        assert_eq!(w.active, 1, "a.txt moved up by one");
        assert_eq!(w.files(), ["a.txt"]);

        let mut s = Session {
            windows: vec![
                window(vec![blank], 0),
                window(vec![tab(None, 2)], 0),
                window(vec![tab(Some("a.txt"), 3), tab(None, 4)], 0),
            ],
            ..Session::default()
        };
        s.settle(true);
        // 空の無題だけのウィンドウは消え、書きかけの無題だけのウィンドウは残る
        assert_eq!(s.windows.len(), 2);
        assert!(!s.windows[0].has_files());
        assert_eq!(s.counts(), (2, 3));
        assert!(!s.is_empty());
        // 無題しか無いセッションは、開き直せるものが無い (ファイルを残さない)
        s.windows.pop();
        assert!(s.is_empty());
    }

    #[test]
    fn a_session_survives_the_file() {
        let mut closed = window(vec![tab(Some(r"C:\dir\閉じた.md"), 9)], 0);
        closed.closed = "2026-11-19 15:40:00".to_string();
        let s = Session {
            version: VERSION,
            started: "2026-11-19 15:30:12".to_string(),
            updated: "2026-11-19 15:45:00".to_string(),
            pid: 1234,
            windows: vec![window(vec![tab(Some("a.txt"), 1), tab(None, 2)], 1), closed],
            ..Session::default()
        };
        let back: Session = toml::from_str(&s.to_toml().unwrap()).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.counts(), (1, 2));
        assert_eq!(back.open_windows().len(), 1);
    }

    #[test]
    fn only_the_time_of_the_update_does_not_count_as_a_change() {
        let a = Session {
            updated: "2026-11-19 15:30:12".to_string(),
            windows: vec![window(vec![tab(Some("a.txt"), 1)], 0)],
            ..Session::default()
        };
        let mut b = a.clone();
        b.updated = "2026-11-19 15:30:18".to_string();
        assert!(b.same_content(&a));
        b.windows[0].tabs[0].line = 99;
        assert!(!b.same_content(&a));
    }

    #[test]
    fn names_become_file_names() {
        assert_eq!(compact("2026-11-19 15:30:12"), "20261119-153012");
        assert_eq!(file_stem_for("  仕事用  ").as_deref(), Some("仕事用"));
        assert_eq!(file_stem_for("a/b:c?").as_deref(), Some("a_b_c_"));
        assert_eq!(file_stem_for("nul").as_deref(), Some("_nul"));
        assert_eq!(
            file_stem_for("com1.backup").as_deref(),
            Some("_com1.backup")
        );
        assert_eq!(file_stem_for("comet").as_deref(), Some("comet"));
        assert_eq!(file_stem_for("name. ").as_deref(), Some("name"));
        assert_eq!(file_stem_for(" . "), None);
        assert_eq!(file_stem_for(""), None);
    }

    #[test]
    fn a_named_copy_has_the_open_windows_only() {
        let mut closed = window(vec![tab(Some("c.txt"), 3)], 0);
        closed.closed = "2026-11-19 15:40:00".to_string();
        let s = Session {
            windows: vec![window(vec![tab(Some("a.txt"), 1), tab(None, 2)], 0), closed],
            ..Session::default()
        };
        let mut copy = named_copy(" 仕事 ", &s, "2026-11-19 16:00:00");
        copy.settle(false);
        assert_eq!(copy.name, "仕事");
        assert_eq!(copy.ended, "2026-11-19 16:00:00");
        assert_eq!(copy.windows.len(), 1);
        assert_eq!(copy.windows[0].tabs.len(), 1);
    }

    #[test]
    fn interrupted_sessions_are_taken_once() {
        let dir = temp_dir("interrupted");
        let base = Session {
            version: VERSION,
            started: "2026-11-18 10:00:00".to_string(),
            updated: "2026-11-18 10:30:00".to_string(),
            pid: 42,
            windows: vec![window(vec![tab(Some("a.txt"), 1), tab(None, 2)], 1)],
            ..Session::default()
        };
        // 正常に終了したもの
        let mut ended = base.clone();
        ended.ended = "2026-11-18 10:30:00".to_string();
        write(&dir, "20261118-100000-42", &ended);
        // 中断されたもの (ロックファイルは残っているが、誰も持っていない)
        write(&dir, "20261118-110000-43", &base);
        fs::write(lock_path(&dir, "20261118-110000-43"), "").unwrap();
        // 別のプロセスが動いているもの
        write(&dir, "20261118-120000-44", &base);
        let held = File::create(lock_path(&dir, "20261118-120000-44")).unwrap();
        held.try_lock().unwrap();
        // 自分のもの
        write(&dir, "20261118-130000-45", &base);

        let own = Some("20261118-130000-45");
        let statuses: Vec<Status> = list_in(&dir, own).iter().map(|e| e.status).collect();
        assert_eq!(
            statuses,
            [Status::Running, Status::Interrupted, Status::Ended],
            "newest first, without the own session"
        );

        let taken = take_interrupted_in(&dir, own);
        assert_eq!(taken.len(), 1);
        // 引き取る前の中身 (無題のタブとスナップショットの番号) が返る
        assert_eq!(taken[0].windows[0].tabs.len(), 2);
        assert_eq!(taken[0].windows[0].tabs[1].snapshot, Some(2));
        // ファイルには印が付き、スナップショットの番号は消える (書きかけの無題は「あった」記録として残る)
        let marked = Session::read(&session_path(&dir, "20261118-110000-43")).unwrap();
        assert!(marked.recovered);
        assert_eq!(marked.windows[0].tabs.len(), 2);
        assert!(marked.windows[0].tabs.iter().all(|t| t.snapshot.is_none()));
        assert_eq!(marked.windows[0].active, 1);
        assert!(!lock_path(&dir, "20261118-110000-43").exists());
        // 2 回目は何も返さない
        assert!(take_interrupted_in(&dir, own).is_empty());

        drop(held);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn pruning_keeps_the_newest_and_the_running() {
        let dir = temp_dir("prune");
        let session = |file: &str| Session {
            ended: "x".to_string(),
            windows: vec![window(vec![tab(Some(file), 1)], 0)],
            ..Session::default()
        };
        let s = session("a.txt");
        for id in ["1-1", "2-1", "3-1", "4-1", "5-1"] {
            write(&dir, id, &session(&format!("{id}.txt")));
        }
        // 一番古いものは別のプロセスが動いている
        let held = File::create(lock_path(&dir, "1-1")).unwrap();
        held.try_lock().unwrap();
        // 持ち主のいないロックファイル
        fs::write(lock_path(&dir, "0-1"), "").unwrap();
        fs::create_dir_all(dir.join(SAVED_DIR)).unwrap();
        fs::write(dir.join(SAVED_DIR).join("名前.toml"), s.to_toml().unwrap()).unwrap();

        prune_in(&dir, Some("5-1"), 2);
        let left: Vec<String> = toml_files(&dir).into_iter().map(|(id, _)| id).collect();
        assert_eq!(left, ["5-1", "4-1", "3-1", "1-1"]);
        assert!(!lock_path(&dir, "0-1").exists());
        assert!(dir.join(SAVED_DIR).join("名前.toml").exists());

        drop(held);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn pruning_drops_the_older_of_identical_sessions() {
        let dir = temp_dir("prune-same");
        let session = |file: &str, time: &str| Session {
            started: time.to_string(),
            ended: time.to_string(),
            windows: vec![window(vec![tab(Some(file), 1)], 0)],
            ..Session::default()
        };
        // 同じ中身が 3 回続き (日時だけ違う)、その前に別の中身、さらに前にまた同じ中身
        write(&dir, "1-1", &session("a.txt", "1"));
        write(&dir, "2-1", &session("b.txt", "2"));
        write(&dir, "3-1", &session("a.txt", "3"));
        write(&dir, "4-1", &session("a.txt", "4"));
        write(&dir, "5-1", &session("a.txt", "5"));

        prune_in(&dir, None, 10);
        let left: Vec<String> = toml_files(&dir).into_iter().map(|(id, _)| id).collect();
        // 続いたものは一番新しいものだけ。間に別のものを挟んだ古いものは残る
        assert_eq!(left, ["5-1", "2-1", "1-1"]);
        let _ = fs::remove_dir_all(&dir);
    }
}
