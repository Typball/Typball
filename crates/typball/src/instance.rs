//! シングルインスタンス: 2 つ目の Typball が起動したら、開くファイルを先に動いている Typball に渡して終了する
//!
//! - Windows: 名前付きミューテックス `Local\Typball-instance-<user>` を最初に作った方がサーバーになり、
//!   名前付きパイプ `\\.\pipe\Typball-<user>` で要求を受ける。後から起動した方はパイプにつないで要求を書き、
//!   終了する。パイプがまだ無い (サーバーが起動した直後) ときは少し待って繰り返し、2 秒待っても
//!   つながらなければ普通に起動する。送る前に `AllowSetForegroundWindow` を呼ぶので、受け取った側が
//!   ウィンドウを前面に出せる (これが無いとタスクバーのボタンが点滅するだけになる)。
//! - 他の OS: ランタイムディレクトリの Unix ドメインソケット。つながればクライアント、つながらなければ
//!   (残っていた古いソケットファイルを消して) bind してサーバーになる。
//!
//! 要求は UTF-8 のテキスト: 1 行目が開き方 ("edit" / "view" / "-" = 形式ごとの設定に従う)、2 行目以降が
//! 絶対パス (1 行 1 つ)。受け取るのは別スレッドなので、GUI の操作は `wxdragon::call_after` で
//! GUI スレッドに渡す (main.rs の `open_request`)。
//!
//! 設定 `[instance] mode = "multiple"` かコマンドライン `--new-instance` のときはこのモジュールを使わず、
//! 起動のたびに別プロセスになる。

use std::path::PathBuf;

use crate::settings::OpenMode;

/// 別プロセスから受け取る (送る) 要求
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// `--edit` / `--view`。None なら形式ごとの設定
    pub mode: Option<OpenMode>,
    pub files: Vec<PathBuf>,
}

/// 起動時にどちらの役になったか
pub enum Claim {
    /// 自分が最初の Typball。`Server::listen` で要求を受け始める
    Owner(Server),
    /// 先に動いている Typball に要求を渡した。このプロセスはこのまま終わってよい
    Forwarded,
    /// 先行プロセスがいるはずなのに応答しない (あるいは IPC を用意できない)。普通に起動する
    Alone,
}

pub use imp::{claim, Server};

impl Request {
    fn encode(&self) -> Vec<u8> {
        let mut text = String::new();
        text.push_str(match self.mode {
            Some(OpenMode::Editor) => "edit",
            Some(OpenMode::Viewer) => "view",
            None => "-",
        });
        text.push('\n');
        for path in &self.files {
            text.push_str(&path.to_string_lossy());
            text.push('\n');
        }
        text.into_bytes()
    }

    fn decode(data: &[u8]) -> Option<Request> {
        let text = std::str::from_utf8(data).ok()?;
        let mut lines = text.lines();
        let mode = match lines.next()? {
            "edit" => Some(OpenMode::Editor),
            "view" => Some(OpenMode::Viewer),
            "-" => None,
            _ => return None,
        };
        let files = lines.filter(|l| !l.is_empty()).map(PathBuf::from).collect();
        Some(Request { mode, files })
    }
}

/// 同じ利用者のセッションだけで共有する名前の末尾
fn user_tag() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

#[cfg(windows)]
mod imp {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::{null, null_mut};
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED,
        GENERIC_WRITE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FlushFileBuffers, ReadFile, WriteFile, OPEN_EXISTING, PIPE_ACCESS_INBOUND,
    };
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, WaitNamedPipeW,
        PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };
    use windows_sys::Win32::System::Threading::CreateMutexW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY};

    use super::{user_tag, Claim, Request};

    /// 先行プロセスのパイプが現れるのを待つ上限
    const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
    /// 要求の大きさの上限 (バイト)
    const MAX_REQUEST: usize = 1 << 20;

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(Some(0)).collect()
    }

    fn pipe_name() -> String {
        format!("\\\\.\\pipe\\Typball-{}", user_tag())
    }

    /// 要求を受けるサーバー。ミューテックスのハンドルはプロセスが終わるまで閉じない
    /// (閉じると別のプロセスがサーバーになれてしまう)
    pub struct Server {
        pipe: String,
        _mutex: usize,
    }

    pub fn claim(request: &Request) -> Claim {
        let mutex_name = wide(&format!("Local\\Typball-instance-{}", user_tag()));
        // SAFETY: 名前は NUL 終端の UTF-16。所有権は取らず、存在するかどうかだけを見る
        let mutex = unsafe { CreateMutexW(null(), 0, mutex_name.as_ptr()) };
        if mutex.is_null() {
            return Claim::Alone;
        }
        // SAFETY: 直前の API のエラーコードを読むだけ
        let exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        if !exists {
            return Claim::Owner(Server {
                pipe: pipe_name(),
                _mutex: mutex as usize,
            });
        }
        let sent = send(&pipe_name(), &request.encode());
        // SAFETY: 自分で作ったハンドル
        unsafe { CloseHandle(mutex) };
        if sent {
            Claim::Forwarded
        } else {
            Claim::Alone
        }
    }

    /// 先行プロセスのパイプに要求を書く。つながるまで少し待つ
    fn send(pipe: &str, payload: &[u8]) -> bool {
        let name = wide(pipe);
        let deadline = Instant::now() + CONNECT_TIMEOUT;
        loop {
            // SAFETY: 名前は NUL 終端。他のポインタ引数は省略可
            let handle = unsafe {
                CreateFileW(
                    name.as_ptr(),
                    GENERIC_WRITE,
                    0,
                    null(),
                    OPEN_EXISTING,
                    0,
                    null_mut(),
                )
            };
            if handle != INVALID_HANDLE_VALUE {
                let mut written = 0u32;
                // SAFETY: 受け取り側がウィンドウを前面に出せるよう権利を渡し、バッファの範囲内で書く
                let ok = unsafe {
                    AllowSetForegroundWindow(ASFW_ANY);
                    let ok = WriteFile(
                        handle,
                        payload.as_ptr(),
                        payload.len() as u32,
                        &mut written,
                        null_mut(),
                    ) != 0;
                    FlushFileBuffers(handle);
                    CloseHandle(handle);
                    ok
                };
                return ok && written as usize == payload.len();
            }
            if Instant::now() >= deadline {
                return false;
            }
            // SAFETY: エラーコードを読む / 名前は NUL 終端
            if unsafe { GetLastError() } == ERROR_PIPE_BUSY {
                unsafe { WaitNamedPipeW(name.as_ptr(), 200) };
            } else {
                // まだパイプが無い (サーバーの起動直後)
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    impl Server {
        /// 別スレッドで要求を受け続け、1 件ごとに `handler` を (そのスレッドで) 呼ぶ
        pub fn listen(self, handler: impl Fn(Request) + Send + 'static) {
            let name = wide(&self.pipe);
            std::thread::spawn(move || loop {
                // SAFETY: 名前は NUL 終端。バイト型・同期 (待つ) モードの受信専用パイプ
                let handle = unsafe {
                    CreateNamedPipeW(
                        name.as_ptr(),
                        PIPE_ACCESS_INBOUND,
                        PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                        PIPE_UNLIMITED_INSTANCES,
                        4096,
                        65536,
                        0,
                        null(),
                    )
                };
                if handle == INVALID_HANDLE_VALUE {
                    return;
                }
                // SAFETY: 同期モードなので接続まで待つ。先にクライアントがつないでいれば ERROR_PIPE_CONNECTED
                let connected = unsafe {
                    ConnectNamedPipe(handle, null_mut()) != 0
                        || GetLastError() == ERROR_PIPE_CONNECTED
                };
                if connected {
                    let mut data = Vec::new();
                    let mut buf = [0u8; 4096];
                    loop {
                        let mut read = 0u32;
                        // SAFETY: buf の範囲内に読む
                        let ok = unsafe {
                            ReadFile(
                                handle,
                                buf.as_mut_ptr(),
                                buf.len() as u32,
                                &mut read,
                                null_mut(),
                            )
                        } != 0;
                        if !ok || read == 0 {
                            break;
                        }
                        data.extend_from_slice(&buf[..read as usize]);
                        if data.len() > MAX_REQUEST {
                            data.clear();
                            break;
                        }
                    }
                    // SAFETY: 自分で作ったハンドル
                    unsafe { DisconnectNamedPipe(handle) };
                    if let Some(request) = Request::decode(&data) {
                        handler(request);
                    }
                }
                // SAFETY: 自分で作ったハンドル
                unsafe { CloseHandle(handle) };
            });
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::io::{Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::time::Duration;

    use super::{user_tag, Claim, Request};

    const MAX_REQUEST: u64 = 1 << 20;

    fn socket_path() -> PathBuf {
        dirs::runtime_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join(format!("Typball-{}.sock", user_tag()))
    }

    pub struct Server {
        listener: UnixListener,
    }

    pub fn claim(request: &Request) -> Claim {
        let path = socket_path();
        if let Ok(mut stream) = UnixStream::connect(&path) {
            let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
            if stream.write_all(&request.encode()).is_ok() {
                return Claim::Forwarded;
            }
        }
        // つながらない: 前回のプロセスが残したソケットファイルを消して自分がサーバーになる
        let _ = std::fs::remove_file(&path);
        match UnixListener::bind(&path) {
            Ok(listener) => Claim::Owner(Server { listener }),
            Err(_) => Claim::Alone,
        }
    }

    impl Server {
        /// 別スレッドで要求を受け続け、1 件ごとに `handler` を (そのスレッドで) 呼ぶ
        pub fn listen(self, handler: impl Fn(Request) + Send + 'static) {
            std::thread::spawn(move || {
                for stream in self.listener.incoming() {
                    let Ok(mut stream) = stream else {
                        continue;
                    };
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut data = Vec::new();
                    let _ = (&mut stream).take(MAX_REQUEST).read_to_end(&mut data);
                    if let Some(request) = Request::decode(&data) {
                        handler(request);
                    }
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips() {
        let request = Request {
            mode: Some(OpenMode::Viewer),
            files: vec![
                PathBuf::from("C:\\docs\\a b.typ"),
                PathBuf::from("/tmp/x.md"),
            ],
        };
        assert_eq!(Request::decode(&request.encode()), Some(request));
        let plain = Request {
            mode: None,
            files: Vec::new(),
        };
        assert_eq!(Request::decode(&plain.encode()), Some(plain));
        assert_eq!(Request::decode(b"garbage\n"), None);
        assert_eq!(Request::decode(b""), None);
    }
}
