//! Chromium discovery, Windows CRT FD pipe inheritance, bootstrap HTTP server, and \0-delimited CDP client.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    ffi::OsStr,
    io::{Read, Write},
    mem::{size_of, zeroed},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0},
    Security::SECURITY_ATTRIBUTES,
    System::Threading::{
        CreateProcessW, PROCESS_INFORMATION, STARTUPINFOW, TerminateProcess, WaitForSingleObject,
    },
};

const HANDLE_FLAG_INHERIT: u32 = 0x0000_0001;
const FOPEN: u8 = 0x01;
const FPIPE: u8 = 0x08;
const CDP_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_CDP_BUFFER: usize = 16 * 1024 * 1024;

#[link(name = "kernel32")]
extern "system" {
    fn CreatePipe(
        hReadPipe: *mut HANDLE,
        hWritePipe: *mut HANDLE,
        lpPipeAttributes: *const SECURITY_ATTRIBUTES,
        nSize: u32,
    ) -> i32;
    fn SetHandleInformation(hObject: HANDLE, dwMask: u32, dwFlags: u32) -> i32;
    fn ReadFile(
        hFile: HANDLE,
        lpBuffer: *mut u8,
        nNumberOfBytesToRead: u32,
        lpNumberOfBytesRead: *mut u32,
        lpOverlapped: *mut std::ffi::c_void,
    ) -> i32;
    fn WriteFile(
        hFile: HANDLE,
        lpBuffer: *const u8,
        nNumberOfBytesToWrite: u32,
        lpNumberOfBytesWritten: *mut u32,
        lpOverlapped: *mut std::ffi::c_void,
    ) -> i32;
}

pub const PLAYER_BROWSERS: [&str; 8] = [
    "chrome",
    "edge",
    "brave",
    "coccoc",
    "vivaldi",
    "opera",
    "opera_gx",
    "chromium",
];

#[derive(Clone, Debug)]
pub struct BrowserCandidate {
    pub browser: &'static str,
    pub executable: PathBuf,
}

fn browser_subpaths(browser: &str) -> &'static [&'static [&'static str]] {
    match browser {
        "chrome" => &[&["Google", "Chrome", "Application", "chrome.exe"]],
        "edge" => &[&["Microsoft", "Edge", "Application", "msedge.exe"]],
        "brave" => &[&["BraveSoftware", "Brave-Browser", "Application", "brave.exe"]],
        "coccoc" => &[&["CocCoc", "Browser", "Application", "browser.exe"]],
        "vivaldi" => &[&["Vivaldi", "Application", "vivaldi.exe"]],
        "opera" => &[&["Programs", "Opera", "opera.exe"], &["Opera", "opera.exe"]],
        "opera_gx" => &[
            &["Programs", "Opera GX", "opera.exe"],
            &["Opera GX", "opera.exe"],
        ],
        "chromium" => &[&["Chromium", "Application", "chrome.exe"]],
        _ => &[],
    }
}

pub fn player_browser_candidates_from_roots(roots: &[PathBuf]) -> Vec<BrowserCandidate> {
    let mut out = Vec::new();
    for &browser in &PLAYER_BROWSERS {
        for segments in browser_subpaths(browser) {
            for root in roots {
                let mut path = root.clone();
                for seg in *segments {
                    path.push(seg);
                }
                out.push(BrowserCandidate {
                    browser,
                    executable: path,
                });
            }
        }
    }
    out
}

pub fn player_browser_candidates() -> Vec<BrowserCandidate> {
    let roots: Vec<PathBuf> = [
        "ProgramFiles(x86)",
        "ProgramW6432",
        "ProgramFiles",
        "LOCALAPPDATA",
    ]
    .into_iter()
    .filter_map(std::env::var_os)
    .filter(|v| !v.is_empty())
    .map(PathBuf::from)
    .collect();
    player_browser_candidates_from_roots(&roots)
}

pub fn find_browser(preferred: Option<&str>) -> Option<BrowserCandidate> {
    let mut candidates = player_browser_candidates();
    if let Some(pref) = preferred {
        candidates.sort_by_key(|c| u8::from(c.browser != pref));
    }
    candidates.into_iter().find(|c| c.executable.is_file())
}

pub fn validate_player_url(url: &str) -> Result<(), String> {
    if url.bytes().any(|b| b <= 32 || b == 127) {
        return Err("Invalid YouTube Music player URL".into());
    }
    let Some(rest) = url.strip_prefix("https://music.youtube.com") else {
        return Err("Invalid YouTube Music player URL".into());
    };
    if !rest.is_empty() && !rest.starts_with(['/', '?', '#']) {
        return Err("Invalid YouTube Music player URL".into());
    }
    Ok(())
}

pub fn player_arguments(profile: &Path, url: &str) -> Result<Vec<String>, String> {
    validate_player_url(url)?;
    let profile_str = profile.to_string_lossy();
    Ok(vec![
        format!("--user-data-dir={profile_str}"),
        "--profile-directory=LumenMusic".into(),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--do-not-de-elevate".into(),
        format!("--app={url}"),
    ])
}

pub fn sign_in_arguments(profile: &Path) -> Vec<String> {
    let profile_str = profile.to_string_lossy();
    vec![
        format!("--user-data-dir={profile_str}"),
        "--profile-directory=LumenMusic".into(),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--disable-background-mode".into(),
        "--new-window".into(),
        "--do-not-de-elevate".into(),
        "https://music.youtube.com/".into(),
    ]
}

/// Packs Windows MSVC CRT `STARTUPINFOW.lpReserved2` FD table with 5 slots (FDs 0..4),
/// leaving FDs 0..2 unused (`INVALID_HANDLE_VALUE`, flags=0) and inheriting FDs 3 & 4 as pipes.
pub(crate) fn pack_crt_pipe_fds(fd3_read: isize, fd4_write: isize) -> Vec<u8> {
    const COUNT: usize = 5;
    let handle_size = size_of::<usize>();
    let mut buf = Vec::with_capacity(4 + COUNT + COUNT * handle_size);
    buf.extend_from_slice(&(COUNT as u32).to_le_bytes());
    buf.extend_from_slice(&[0u8, 0u8, 0u8, FOPEN | FPIPE, FOPEN | FPIPE]);
    let invalid = (-1isize) as usize;
    for slot in [
        invalid,
        invalid,
        invalid,
        fd3_read as usize,
        fd4_write as usize,
    ] {
        buf.extend_from_slice(&slot.to_le_bytes());
    }
    buf
}

fn quote_win_arg(arg: &OsStr) -> String {
    let s = arg.to_string_lossy();
    if !s.is_empty() && !s.bytes().any(|b| matches!(b, b' ' | b'\t' | b'"')) {
        return s.into_owned();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    let mut backslashes = 0usize;
    for ch in s.chars() {
        if ch == '\\' {
            backslashes += 1;
        } else if ch == '"' {
            for _ in 0..(backslashes * 2 + 1) {
                out.push('\\');
            }
            out.push('"');
            backslashes = 0;
        } else {
            for _ in 0..backslashes {
                out.push('\\');
            }
            backslashes = 0;
            out.push(ch);
        }
    }
    for _ in 0..(backslashes * 2) {
        out.push('\\');
    }
    out.push('"');
    out
}

fn build_command_line(exe: &Path, args: &[String]) -> Vec<u16> {
    let mut line = quote_win_arg(exe.as_os_str());
    for arg in args {
        line.push(' ');
        line.push_str(&quote_win_arg(OsStr::new(arg)));
    }
    crate::model::wide(line)
}

fn random_bytes_32() -> [u8; 32] {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut hasher = Sha256::new();
    hasher.update(now.to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(seq.to_le_bytes());
    hasher.update((&seq as *const u64 as usize).to_le_bytes());
    hasher.finalize().into()
}

pub(crate) fn random_hex(bytes: usize) -> String {
    let digest = random_bytes_32();
    let mut out = String::with_capacity(bytes * 2);
    for &b in digest.iter().take(bytes) {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

pub struct BootstrapServer {
    pub url: String,
    pub title: String,
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl BootstrapServer {
    pub fn start() -> Result<Self, String> {
        let title = format!("Lumen Music Background {}", random_hex(8));
        let bootstrap_path = format!("/lumen-background/{}", random_hex(12));
        let mut listener = None;
        let mut last_err = None;
        for _ in 0..9 {
            let r = random_bytes_32();
            let raw = u16::from_le_bytes([r[0], r[1]]);
            let port = 20000 + (raw % 40000);
            match TcpListener::bind(("127.0.0.1", port)) {
                Ok(l) => {
                    listener = Some((l, port));
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                    last_err = Some(e);
                }
                Err(e) => return Err(format!("Cannot start Music bootstrap server: {e}")),
            }
        }
        let (listener, port) = listener.ok_or_else(|| {
            format!(
                "Cannot start Music bootstrap server: {}",
                last_err
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "address in use".into())
            )
        })?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("Bootstrap server configuration failed: {e}"))?;

        let url = format!("http://127.0.0.1:{port}{bootstrap_path}");
        let shutdown = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&shutdown);
        let expected_path = bootstrap_path;
        let page_title = title.clone();
        let thread = thread::spawn(move || {
            while !flag.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => handle_bootstrap_conn(stream, &expected_path, &page_title),
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            url,
            title,
            shutdown,
            thread: Some(thread),
        })
    }
}

impl Drop for BootstrapServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

fn handle_bootstrap_conn(mut stream: TcpStream, expected_path: &str, title: &str) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let mut req = [0u8; 4096];
    let mut total = 0usize;
    while total < req.len() {
        match stream.read(&mut req[total..]) {
            Ok(0) => break,
            Ok(n) => {
                total += n;
                if req[..total].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            Err(_) => return,
        }
    }
    let header_text = String::from_utf8_lossy(&req[..total]);
    let mut parts = header_text.lines().next().unwrap_or("").split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    if method == "GET" && path == expected_path {
        let body = format!("<!doctype html><title>{title}</title>");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; frame-ancestors 'none'\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(response.as_bytes());
    } else {
        let _ = stream.write_all(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
    }
}

struct OwnedHandle(HANDLE);
unsafe impl Send for OwnedHandle {}
unsafe impl Sync for OwnedHandle {}

impl OwnedHandle {
    fn new(handle: HANDLE) -> Option<Self> {
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            None
        } else {
            Some(Self(handle))
        }
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

pub struct ChildProcess {
    pub pid: u32,
    process: OwnedHandle,
}

impl ChildProcess {
    pub fn is_exited(&self) -> bool {
        unsafe { WaitForSingleObject(self.process.0, 0) == WAIT_OBJECT_0 }
    }

    pub fn wait_exit(&self, timeout_ms: u32) -> bool {
        unsafe { WaitForSingleObject(self.process.0, timeout_ms) == WAIT_OBJECT_0 }
    }

    pub fn terminate(&mut self) {
        unsafe {
            let _ = TerminateProcess(self.process.0, 1);
        }
    }
}

struct PipeState {
    closed: AtomicBool,
    next_id: AtomicU64,
    write_handle: Mutex<Option<OwnedHandle>>,
    pending: Mutex<HashMap<u64, mpsc::Sender<Result<Value, String>>>>,
}

impl PipeState {
    fn close(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        // Drop write handle first so the browser sees EOF on FD 3 and exits cleanly.
        if let Ok(mut guard) = self.write_handle.lock() {
            guard.take();
        }
        if let Ok(mut pending) = self.pending.lock() {
            for (_, tx) in pending.drain() {
                let _ = tx.send(Err("Music browser disconnected".into()));
            }
        }
    }
}

pub struct CdpPipe {
    state: Arc<PipeState>,
    _reader: Option<JoinHandle<()>>,
}

impl CdpPipe {
    fn new(write_handle: OwnedHandle, read_handle: OwnedHandle) -> Self {
        let state = Arc::new(PipeState {
            closed: AtomicBool::new(false),
            next_id: AtomicU64::new(0),
            write_handle: Mutex::new(Some(write_handle)),
            pending: Mutex::new(HashMap::new()),
        });
        let reader_state = Arc::clone(&state);
        let reader = thread::spawn(move || {
            let _read_guard = read_handle;
            let handle = _read_guard.0;
            let mut acc = Vec::with_capacity(8192);
            let mut chunk = [0u8; 65536];
            loop {
                let mut n = 0u32;
                let ok = unsafe {
                    ReadFile(
                        handle,
                        chunk.as_mut_ptr(),
                        chunk.len() as u32,
                        &mut n,
                        null_mut(),
                    )
                };
                if ok == 0 || n == 0 {
                    break;
                }
                acc.extend_from_slice(&chunk[..n as usize]);
                while let Some(pos) = acc.iter().position(|&b| b == 0) {
                    if pos > 0 {
                        if let Ok(msg) = serde_json::from_slice::<Value>(&acc[..pos]) {
                            if let Some(id) = msg.get("id").and_then(|v| v.as_u64()) {
                                let tx = reader_state
                                    .pending
                                    .lock()
                                    .ok()
                                    .and_then(|mut map| map.remove(&id));
                                if let Some(tx) = tx {
                                    let reply = if let Some(err) =
                                        msg.get("error").filter(|v| !v.is_null())
                                    {
                                        let m = err
                                            .get("message")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("CDP error")
                                            .to_string();
                                        Err(m)
                                    } else {
                                        Ok(msg.get("result").cloned().unwrap_or(Value::Null))
                                    };
                                    let _ = tx.send(reply);
                                }
                            }
                        }
                    }
                    acc.drain(..=pos);
                }
                if acc.len() > MAX_CDP_BUFFER {
                    break;
                }
            }
            reader_state.close();
        });
        Self {
            state,
            _reader: Some(reader),
        }
    }

    pub fn is_closed(&self) -> bool {
        self.state.closed.load(Ordering::Acquire)
    }

    pub fn close(&self) {
        self.state.close();
    }

    pub fn send(
        &self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
    ) -> Result<Value, String> {
        if self.is_closed() {
            return Err("Music browser disconnected".into());
        }
        let id = self.state.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = mpsc::channel();
        {
            let mut pending = self
                .state
                .pending
                .lock()
                .map_err(|_| "CDP pipe lock poisoned".to_string())?;
            if self.is_closed() {
                return Err("Music browser disconnected".into());
            }
            pending.insert(id, tx);
        }

        let mut payload = serde_json::Map::new();
        payload.insert("id".into(), Value::from(id));
        payload.insert("method".into(), Value::from(method));
        payload.insert("params".into(), params);
        if let Some(session) = session_id {
            payload.insert("sessionId".into(), Value::from(session));
        }
        let mut bytes = serde_json::to_vec(&Value::Object(payload)).map_err(|e| e.to_string())?;
        bytes.push(0);

        let write_ok = {
            let guard = self
                .state
                .write_handle
                .lock()
                .map_err(|_| "CDP pipe lock poisoned".to_string())?;
            if let Some(handle) = guard.as_ref() {
                write_all_handle(handle.0, &bytes)
            } else {
                false
            }
        };
        if !write_ok {
            if let Ok(mut pending) = self.state.pending.lock() {
                pending.remove(&id);
            }
            self.close();
            return Err("Music browser disconnected".into());
        }

        match rx.recv_timeout(CDP_TIMEOUT) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Ok(mut pending) = self.state.pending.lock() {
                    pending.remove(&id);
                }
                Err(format!("Music player timed out ({method})"))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if let Ok(mut pending) = self.state.pending.lock() {
                    pending.remove(&id);
                }
                Err("Music browser disconnected".into())
            }
        }
    }
}

impl Drop for CdpPipe {
    fn drop(&mut self) {
        self.close();
    }
}

fn write_all_handle(handle: HANDLE, mut bytes: &[u8]) -> bool {
    while !bytes.is_empty() {
        let mut written = 0u32;
        let chunk_len = bytes.len().min(u32::MAX as usize) as u32;
        let ok = unsafe { WriteFile(handle, bytes.as_ptr(), chunk_len, &mut written, null_mut()) };
        if ok == 0 || written == 0 {
            return false;
        }
        bytes = &bytes[written as usize..];
    }
    true
}

pub fn spawn_with_cdp_pipe(exe: &Path, args: &[String]) -> Result<(ChildProcess, CdpPipe), String> {
    let sa = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: 1,
    };

    let mut fd3_read_raw: HANDLE = null_mut();
    let mut fd3_write_raw: HANDLE = null_mut();
    if unsafe { CreatePipe(&mut fd3_read_raw, &mut fd3_write_raw, &sa, 0) } == 0 {
        return Err(format!(
            "Cannot create browser command pipe: {}",
            std::io::Error::last_os_error()
        ));
    }
    let fd3_read = OwnedHandle::new(fd3_read_raw).ok_or("Invalid pipe read handle")?;
    let fd3_write = OwnedHandle::new(fd3_write_raw).ok_or("Invalid pipe write handle")?;
    if unsafe { SetHandleInformation(fd3_write.0, HANDLE_FLAG_INHERIT, 0) } == 0 {
        return Err("Cannot protect browser command pipe handle".into());
    }

    let mut fd4_read_raw: HANDLE = null_mut();
    let mut fd4_write_raw: HANDLE = null_mut();
    if unsafe { CreatePipe(&mut fd4_read_raw, &mut fd4_write_raw, &sa, 0) } == 0 {
        return Err(format!(
            "Cannot create browser response pipe: {}",
            std::io::Error::last_os_error()
        ));
    }
    let fd4_read = OwnedHandle::new(fd4_read_raw).ok_or("Invalid response read handle")?;
    let fd4_write = OwnedHandle::new(fd4_write_raw).ok_or("Invalid response write handle")?;
    if unsafe { SetHandleInformation(fd4_read.0, HANDLE_FLAG_INHERIT, 0) } == 0 {
        return Err("Cannot protect browser response pipe handle".into());
    }

    let mut crt_fds = pack_crt_pipe_fds(fd3_read.0 as isize, fd4_write.0 as isize);
    let mut si: STARTUPINFOW = unsafe { zeroed() };
    si.cb = size_of::<STARTUPINFOW>() as u32;
    si.cbReserved2 = crt_fds.len() as u16;
    si.lpReserved2 = crt_fds.as_mut_ptr();

    let mut pi: PROCESS_INFORMATION = unsafe { zeroed() };
    let exe_w = crate::model::wide(exe);
    let mut cmd_w = build_command_line(exe, args);

    let ok = unsafe {
        CreateProcessW(
            exe_w.as_ptr(),
            cmd_w.as_mut_ptr(),
            null(),
            null(),
            1,
            0,
            null(),
            null(),
            &si,
            &mut pi,
        )
    };
    // Release child pipe ends in the parent immediately so EOF works reliably.
    drop(fd3_read);
    drop(fd4_write);

    if ok == 0 {
        return Err(format!(
            "Cannot launch Music browser: {}",
            std::io::Error::last_os_error()
        ));
    }
    if !pi.hThread.is_null() {
        unsafe {
            CloseHandle(pi.hThread);
        }
    }
    let process = OwnedHandle::new(pi.hProcess).ok_or("Invalid browser process handle")?;
    let child = ChildProcess {
        pid: pi.dwProcessId,
        process,
    };
    let pipe = CdpPipe::new(fd3_write, fd4_read);
    Ok((child, pipe))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crt_pipe_fd_buffer_layout_matches_msvc_ioinit() {
        let fd3 = 0x1234_isize;
        let fd4 = 0x5678_isize;
        let buf = pack_crt_pipe_fds(fd3, fd4);
        let hsz = size_of::<usize>();
        assert_eq!(buf.len(), 4 + 5 + 5 * hsz);

        let count = u32::from_le_bytes(buf[0..4].try_into().unwrap());
        assert_eq!(count, 5);
        assert_eq!(&buf[4..9], &[0, 0, 0, FOPEN | FPIPE, FOPEN | FPIPE]);

        let invalid = (-1isize as usize).to_le_bytes();
        let handles_start = 9;
        for slot in 0..3 {
            let offset = handles_start + slot * hsz;
            assert_eq!(&buf[offset..offset + hsz], &invalid);
        }
        let s3 = handles_start + 3 * hsz;
        let s4 = handles_start + 4 * hsz;
        assert_eq!(
            usize::from_le_bytes(buf[s3..s3 + hsz].try_into().unwrap()),
            fd3 as usize
        );
        assert_eq!(
            usize::from_le_bytes(buf[s4..s4 + hsz].try_into().unwrap()),
            fd4 as usize
        );
    }

    #[test]
    fn player_url_and_arguments_validation() {
        let profile = Path::new(r"C:\Temp\profile");
        let base_args = player_arguments(profile, "https://music.youtube.com/").unwrap();
        assert!(base_args.iter().any(|a| a == "--do-not-de-elevate"));
        assert!(base_args[..base_args.len() - 1]
            .iter()
            .any(|a| a == "--do-not-de-elevate"));
        assert!(sign_in_arguments(profile)
            .iter()
            .any(|a| a == "--do-not-de-elevate"));
        assert!(player_arguments(profile, "https://music.youtube.com/library").is_ok());
        assert!(player_arguments(profile, "http://music.youtube.com/").is_err());
        assert!(player_arguments(profile, "https://music.youtube.com.evil.com/").is_err());
        assert!(player_arguments(profile, "https://music.youtube.com/\n--evil").is_err());
    }

    #[test]
    fn windows_command_line_quoting_handles_spaces_and_quotes() {
        assert_eq!(quote_win_arg(OsStr::new("plain")), "plain");
        assert_eq!(
            quote_win_arg(OsStr::new(r"C:\Program Files\Chrome\chrome.exe")),
            r#""C:\Program Files\Chrome\chrome.exe""#
        );
        assert_eq!(quote_win_arg(OsStr::new(r#"a"b"#)), r#""a\"b""#);
    }
}
