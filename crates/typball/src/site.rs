//! 静的サイトジェネレーター (SSG) のプロジェクト検出と開発サーバーの管理
//!
//! `.md` のあるフォルダから上へ設定ファイルを探して Hugo などのプロジェクトを見つけ、
//! 設定 `[preview]` のコマンド (既定は `hugo server -D -N -M --port {port} -s {root}`) を
//! 子プロセスとして起動する。サーバーはプロジェクトのルートごとに 1 つで、タブをまたいで
//! 共有し、アプリの終了時に止める。
//!
//! Hugo の `-N` (navigateToChanged) により、Typball で content のファイルを保存すると WebView が
//! そのページへ自動で移動する。他の SSG はコマンドと URL を設定で差し替える。

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// 見つかったプロジェクト
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    /// 設定ファイルのあるフォルダ (サーバーの `-s`)
    pub root: PathBuf,
    pub kind: SiteKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiteKind {
    Hugo,
    /// `config.toml` と `content/` はあるが Hugo と断定できないもの (Zola など)
    Other,
}

impl Project {
    /// ルート相対の URL (`/images/x.png`) を探すフォルダ
    pub fn static_dirs(&self) -> Vec<PathBuf> {
        vec![self.root.join("static"), self.root.join("assets")]
    }
}

/// `path` (ファイルかフォルダ) から上へ辿ってプロジェクトを探す
pub fn detect(path: &Path) -> Option<Project> {
    let mut dir = if path.is_dir() {
        Some(path)
    } else {
        path.parent()
    };
    for _ in 0..16 {
        let d = dir?;
        if let Some(kind) = kind_of(d) {
            return Some(Project {
                root: d.to_path_buf(),
                kind,
            });
        }
        dir = d.parent();
    }
    None
}

fn kind_of(dir: &Path) -> Option<SiteKind> {
    const HUGO_CONFIGS: [&str; 3] = ["hugo.toml", "hugo.yaml", "hugo.json"];
    if HUGO_CONFIGS.iter().any(|c| dir.join(c).is_file()) {
        return Some(SiteKind::Hugo);
    }
    let defaults = dir.join("config").join("_default");
    if defaults.is_dir()
        && (HUGO_CONFIGS.iter().any(|c| defaults.join(c).is_file())
            || ["config.toml", "config.yaml", "config.json"]
                .iter()
                .any(|c| defaults.join(c).is_file()))
    {
        return Some(SiteKind::Hugo);
    }
    let has_content = dir.join("content").is_dir();
    if has_content
        && ["config.toml", "config.yaml", "config.json"]
            .iter()
            .any(|c| dir.join(c).is_file())
    {
        // 旧来の Hugo (config.toml) か Zola。themes/ か layouts/ があれば Hugo とみなす
        let hugo_like = dir.join("themes").is_dir() || dir.join("layouts").is_dir();
        return Some(if hugo_like && !dir.join("templates").is_dir() {
            SiteKind::Hugo
        } else {
            SiteKind::Other
        });
    }
    None
}

/// 起動した開発サーバー (プロセスは drop で止まる)
pub struct Server {
    child: RefCell<Option<Child>>,
    pub port: u16,
    pub url: String,
    /// 起動時に使ったコマンド (設定が変わったら作り直すための比較用)
    pub command: String,
    stderr: Arc<Mutex<String>>,
}

impl Server {
    /// ポートに接続できるか (サーバーが受け付け始めたか)
    pub fn is_ready(&self) -> bool {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], self.port));
        TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok()
    }

    /// プロセスが終わっていればその説明 (終了コードと stderr の先頭)
    pub fn exit_error(&self) -> Option<String> {
        let mut child = self.child.borrow_mut();
        let status = child.as_mut()?.try_wait().ok().flatten()?;
        let stderr = self.stderr.lock().map(|s| s.clone()).unwrap_or_default();
        let head: String = stderr.lines().take(3).collect::<Vec<_>>().join(" / ");
        Some(if head.is_empty() {
            format!("exit {status}")
        } else {
            format!("exit {status}: {head}")
        })
    }

    /// stderr の末尾 `lines` 行 (コンソールペイン用)
    pub fn stderr_tail(&self, lines: usize) -> String {
        let all = self.stderr.lock().map(|s| s.clone()).unwrap_or_default();
        let v: Vec<&str> = all.lines().collect();
        let start = v.len().saturating_sub(lines);
        v[start..].join("\n")
    }

    pub fn stop(&self) {
        if let Some(mut child) = self.child.borrow_mut().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

thread_local! {
    static SERVERS: RefCell<HashMap<PathBuf, Rc<Server>>> = RefCell::new(HashMap::new());
}

/// プロジェクトのサーバーを返す (無ければ起動する)。コマンドが変わっていたら起動し直す
pub fn server_for(root: &Path, command: &str, url_template: &str) -> Result<Rc<Server>, String> {
    let root = root.to_path_buf();
    let existing = SERVERS.with(|s| s.borrow().get(&root).cloned());
    if let Some(server) = existing {
        if server.command == command && server.exit_error().is_none() {
            return Ok(server);
        }
        server.stop();
        SERVERS.with(|s| s.borrow_mut().remove(&root));
    }
    let server = Rc::new(start(&root, command, url_template)?);
    SERVERS.with(|s| s.borrow_mut().insert(root, server.clone()));
    Ok(server)
}

/// すべてのサーバーを止める (アプリの終了時)
pub fn stop_all() {
    SERVERS.with(|s| s.borrow_mut().clear());
}

fn start(root: &Path, command: &str, url_template: &str) -> Result<Server, String> {
    let port = free_port().ok_or_else(|| "no free port".to_string())?;
    let root_str = root.to_string_lossy().into_owned();
    let args: Vec<String> = split_command(command)
        .into_iter()
        .map(|a| {
            a.replace("{port}", &port.to_string())
                .replace("{root}", &root_str)
        })
        .collect();
    let (program, rest) = args
        .split_first()
        .ok_or_else(|| "empty command".to_string())?;
    let mut cmd = Command::new(program);
    cmd.args(rest)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|e| format!("{program}: {e}"))?;
    #[cfg(windows)]
    assign_to_job(&child);
    let stderr = Arc::new(Mutex::new(String::new()));
    if let Some(mut pipe) = child.stderr.take() {
        let sink = stderr.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            while let Ok(n) = pipe.read(&mut buf) {
                if n == 0 {
                    break;
                }
                if let Ok(mut s) = sink.lock() {
                    if s.len() < 8192 {
                        s.push_str(&String::from_utf8_lossy(&buf[..n]));
                    }
                }
            }
        });
    }
    Ok(Server {
        child: RefCell::new(Some(child)),
        port,
        url: url_template.replace("{port}", &port.to_string()),
        command: command.to_string(),
        stderr,
    })
}

/// 子プロセスを「親が終わると一緒に終わる」ジョブオブジェクトに入れる (Windows)。
/// Windows では親が落ちても子は残るので、Typball が異常終了しても開発サーバーが残らないようにする。
/// ジョブのハンドルは閉じずに持ち続け、プロセスの終了時に OS が閉じたときに子が止まる
#[cfg(windows)]
fn assign_to_job(child: &Child) {
    use std::os::windows::io::AsRawHandle;
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    static JOB: OnceLock<usize> = OnceLock::new();
    let job = *JOB.get_or_init(|| {
        // SAFETY: Win32 のジョブオブジェクト API を正しい大きさの構造体で呼ぶ
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return 0;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const std::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            job as usize
        }
    });
    if job != 0 {
        // SAFETY: 有効なジョブとプロセスのハンドル
        unsafe {
            AssignProcessToJobObject(job as HANDLE, child.as_raw_handle() as HANDLE);
        }
    }
}

/// 空いているポートを OS に選ばせる
fn free_port() -> Option<u16> {
    TcpListener::bind(("127.0.0.1", 0))
        .ok()?
        .local_addr()
        .ok()
        .map(|a| a.port())
}

/// コマンド行を引数に分ける (二重引用符でまとめられる)
pub fn split_command(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut has_token = false;
    for c in s.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                has_token = true;
            }
            c if c.is_whitespace() && !quoted => {
                if has_token {
                    out.push(std::mem::take(&mut cur));
                    has_token = false;
                }
            }
            c => {
                cur.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_command_handles_quotes() {
        assert_eq!(
            split_command("hugo server -s \"C:\\My Site\" --port {port}"),
            vec!["hugo", "server", "-s", "C:\\My Site", "--port", "{port}"]
        );
        assert_eq!(split_command("  "), Vec::<String>::new());
    }

    #[test]
    fn detect_hugo_project() {
        let dir = std::env::temp_dir().join(format!("typball-site-{}", std::process::id()));
        let content = dir.join("content").join("posts");
        std::fs::create_dir_all(&content).unwrap();
        std::fs::write(dir.join("hugo.toml"), "baseURL = 'https://example.org/'\n").unwrap();
        let md = content.join("hello.md");
        std::fs::write(&md, "# hi\n").unwrap();
        let project = detect(&md).unwrap();
        assert_eq!(project.kind, SiteKind::Hugo);
        assert_eq!(project.root, dir);
        assert!(project.static_dirs()[0].ends_with("static"));
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(detect(Path::new("C:\\nonexistent\\x.md")).is_none());
    }
}
