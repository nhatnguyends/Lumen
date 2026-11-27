//! Pure-Rust Chromium CDP pipe backend for YouTube Music. UI never waits on I/O.
mod cdp_pipe;
mod player_page;
mod player_window;

use cdp_pipe::{
    BootstrapServer, BrowserCandidate, CdpPipe, ChildProcess, PLAYER_BROWSERS, find_browser,
    player_arguments, sign_in_arguments, spawn_with_cdp_pipe,
};
pub use cdp_pipe::{browser_label, installed_browsers};
use player_page::{
    MUSIC_PAGE_STATE, MUSIC_PLAY_PROBE, PlayerPageState, download_artwork,
    has_auth_session_cookies, music_page_command, music_page_select, sanitize_page_state,
    sanitize_playlist_id, validate_video_id,
};
use player_window::{
    MusicWindowHost, hide_music_background_window, youtube_music_favicon_script,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    fs,
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::Storage::FileSystem::{
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};

#[derive(Clone, Default)]
struct EventNotifier {
    hwnd: Arc<AtomicIsize>,
    msg: Arc<AtomicU32>,
    pending: Arc<AtomicBool>,
}

impl EventNotifier {
    fn new(hwnd: isize, msg: u32) -> Self {
        Self {
            hwnd: Arc::new(AtomicIsize::new(hwnd)),
            msg: Arc::new(AtomicU32::new(msg)),
            pending: Arc::new(AtomicBool::new(false)),
        }
    }

    fn set(&self, hwnd: isize, msg: u32) {
        self.hwnd.store(hwnd, Ordering::Release);
        self.msg.store(msg, Ordering::Release);
        if hwnd == 0 || msg == 0 {
            self.pending.store(false, Ordering::Release);
        }
    }

    fn clear_pending(&self) {
        self.pending.store(false, Ordering::Release);
    }

    fn notify(&self) {
        if !self.pending.swap(true, Ordering::AcqRel) {
            let h = self.hwnd.load(Ordering::Acquire);
            let m = self.msg.load(Ordering::Acquire);
            if h != 0 && m != 0 {
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                        h as windows_sys::Win32::Foundation::HWND,
                        m,
                        0,
                        0,
                    );
                }
            } else {
                self.pending.store(false, Ordering::Release);
            }
        }
    }
}

fn send_event(tx: &Sender<Value>, notifier: &EventNotifier, value: Value) {
    if tx.send(value).is_ok() {
        notifier.notify();
    }
}

pub struct Backend {
    cmd_tx: Option<Sender<Value>>,
    exited_flag: Arc<AtomicBool>,
    stop_flag: Arc<AtomicBool>,
    notifier: EventNotifier,
    worker: Option<thread::JoinHandle<()>>,
    pub events: Receiver<Value>,
}

impl Backend {
    pub fn start(root: &Path) -> Result<Self, String> {
        Self::start_with_notify(root, std::ptr::null_mut(), 0)
    }

    pub fn start_with_notify(
        root: &Path,
        notify_hwnd: windows_sys::Win32::Foundation::HWND,
        notify_msg: u32,
    ) -> Result<Self, String> {
        let root_buf = root.to_path_buf();
        let (cmd_tx, cmd_rx) = mpsc::channel::<Value>();
        let (events_tx, events) = mpsc::channel::<Value>();
        let exited_flag = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::new(AtomicBool::new(false));
        let notifier = EventNotifier::new(notify_hwnd as isize, notify_msg);

        let worker_exited = Arc::clone(&exited_flag);
        let worker_stop = Arc::clone(&stop_flag);
        let worker_notifier = notifier.clone();
        let worker = thread::Builder::new()
            .name("lumen-music-backend".into())
            .spawn(move || {
                run_worker(&root_buf, cmd_rx, events_tx, worker_stop, worker_notifier);
                worker_exited.store(true, Ordering::Release);
            })
            .map_err(|e| format!("Cannot start Music backend thread: {e}"))?;

        Ok(Self {
            cmd_tx: Some(cmd_tx),
            exited_flag,
            stop_flag,
            notifier,
            worker: Some(worker),
            events,
        })
    }

    pub fn set_notify(&self, hwnd: windows_sys::Win32::Foundation::HWND, msg: u32) {
        self.notifier.set(hwnd as isize, msg);
    }

    pub fn clear_notify_pending(&self) {
        self.notifier.clear_pending();
    }

    pub fn send(&mut self, value: Value) -> Result<(), String> {
        let Some(tx) = &self.cmd_tx else {
            return Err("Music adapter is closed".into());
        };
        tx.send(value)
            .map_err(|_| "Music adapter is closed".to_string())
    }

    pub fn exited(&mut self) -> bool {
        self.exited_flag.load(Ordering::Acquire)
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Release);
        if let Some(tx) = self.cmd_tx.take() {
            let _ = tx.send(json!({"action": "quit"}));
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct PersistedPlayer {
    browser: Option<String>,
    app_id: String,
    video_id: String,
    #[serde(default)]
    pub resume_playlist_id: String,
    position: f64,
}

struct ArtCache {
    dir: PathBuf,
    map: Arc<Mutex<HashMap<String, String>>>,
    pending: Arc<Mutex<HashSet<String>>>,
    events: Sender<Value>,
    notifier: EventNotifier,
    stop: Arc<AtomicBool>,
}

impl ArtCache {
    fn new(
        directory: &Path,
        events: Sender<Value>,
        notifier: EventNotifier,
        stop: Arc<AtomicBool>,
    ) -> Self {
        let dir = directory.join("art");
        let _ = fs::create_dir_all(&dir);
        Self {
            dir,
            map: Arc::new(Mutex::new(HashMap::new())),
            pending: Arc::new(Mutex::new(HashSet::new())),
            events,
            notifier,
            stop,
        }
    }

    fn enqueue(&self, url: &str) {
        if url.is_empty() || self.stop.load(Ordering::Acquire) {
            return;
        }
        if let Ok(map) = self.map.lock() {
            if map.contains_key(url) {
                return;
            }
        }
        {
            let Ok(mut pending) = self.pending.lock() else {
                return;
            };
            if pending.contains(url) || pending.len() >= 16 {
                return;
            }
            pending.insert(url.to_owned());
        }

        let url_owned = url.to_owned();
        let dir = self.dir.clone();
        let map = Arc::clone(&self.map);
        let pending = Arc::clone(&self.pending);
        let events = self.events.clone();
        let notifier = self.notifier.clone();
        let stop = Arc::clone(&self.stop);
        thread::spawn(move || {
            let res = download_artwork(&url_owned, &dir);
            if let Ok(mut p) = pending.lock() {
                p.remove(&url_owned);
            }
            match res {
                Ok((file, pruned)) => {
                    if let Ok(mut m) = map.lock() {
                        if !pruned.is_empty() {
                            m.retain(|_, v| !pruned.contains(v));
                        }
                        m.insert(url_owned.clone(), file.clone());
                    }
                    if !stop.load(Ordering::Acquire) {
                        send_event(
                            &events,
                            &notifier,
                            json!({"type": "art", "url": url_owned, "file": file}),
                        );
                    }
                }
                Err(err) => {
                    crate::logger::warn(
                        "music",
                        format!("artwork download failed for {url_owned}: {err}"),
                    );
                }
            }
        });
    }
}

struct MusicPlayer {
    directory: PathBuf,
    events: Sender<Value>,
    notifier: EventNotifier,
    stop_flag: Arc<AtomicBool>,
    art: ArtCache,
    preferred_browser: Option<String>,
    browser_name: Option<String>,
    app_id: String,
    resume_video_id: String,
    resume_playlist_id: String,
    resume_position: f64,
    pipe: Option<CdpPipe>,
    child: Option<ChildProcess>,
    sign_in_child: Option<Child>,
    pending_sign_in_reconnect: u8,
    next_sign_in_retry: Option<Instant>,
    pending_cmd: Option<Value>,
    host: Option<MusicWindowHost>,
    playback_id: String,
    playback_session: String,
    sessions: HashMap<String, String>,
    current: Option<PlayerPageState>,
    null_state_polls: u8,
    transition_hold_polls: u8,
}

pub(crate) fn is_target_or_session_error(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("execution context")
        || lower.contains("cannot find context")
        || lower.contains("inspected target")
        || lower.contains("no target")
        || lower.contains("session with given id")
        || lower.contains("target closed")
        || lower.contains("detached")
}

pub(crate) fn should_hold_transition(
    curr: &PlayerPageState,
    next: &PlayerPageState,
    hold_polls: u8,
) -> bool {
    !curr.title.is_empty()
        && next.video_id != curr.video_id
        && !next.available
        && !next.playing
        && hold_polls < 2
}

impl MusicPlayer {
    fn new(
        directory: PathBuf,
        events: Sender<Value>,
        notifier: EventNotifier,
        stop_flag: Arc<AtomicBool>,
    ) -> Self {
        let mut player = Self {
            art: ArtCache::new(
                &directory,
                events.clone(),
                notifier.clone(),
                Arc::clone(&stop_flag),
            ),
            directory,
            events,
            notifier,
            stop_flag,
            preferred_browser: None,
            browser_name: None,
            app_id: String::new(),
            resume_video_id: String::new(),
            resume_playlist_id: String::new(),
            resume_position: 0.0,
            pipe: None,
            child: None,
            sign_in_child: None,
            pending_sign_in_reconnect: 0,
            next_sign_in_retry: None,
            pending_cmd: None,
            host: None,
            playback_id: String::new(),
            playback_session: String::new(),
            sessions: HashMap::new(),
            current: None,
            null_state_polls: 0,
            transition_hold_polls: 0,
        };
        player.load();
        player
    }

    fn load(&mut self) {
        let path = self.directory.join("music-player.json");
        let Ok(bytes) = fs::read(path) else {
            return;
        };
        let Ok(raw) = serde_json::from_slice::<PersistedPlayer>(&bytes) else {
            return;
        };
        if let Some(b) = raw.browser {
            if PLAYER_BROWSERS.contains(&b.as_str()) {
                self.browser_name = Some(b);
            }
        }
        if raw.app_id.len() < 256 && raw.app_id.contains("LumenMusic") {
            self.app_id = raw.app_id;
        }
        if validate_video_id(&raw.video_id).is_ok() {
            self.resume_video_id = raw.video_id;
        }
        self.resume_playlist_id = sanitize_playlist_id(&raw.resume_playlist_id);
        if raw.position.is_finite() {
            self.resume_position = raw.position.max(0.0);
        }
    }

    fn save(&self) {
        if fs::create_dir_all(&self.directory).is_err() {
            return;
        }
        let file = self.directory.join("music-player.json");
        let tmp = self.directory.join("music-player.json.tmp");
        let data = PersistedPlayer {
            browser: self.browser_name.clone(),
            app_id: self.app_id.clone(),
            video_id: self.resume_video_id.clone(),
            resume_playlist_id: self.resume_playlist_id.clone(),
            position: self.resume_position,
        };
        let Ok(bytes) = serde_json::to_vec_pretty(&data) else {
            return;
        };
        if fs::write(&tmp, bytes).is_err() {
            return;
        }
        let moved = unsafe {
            MoveFileExW(
                crate::model::wide(&tmp).as_ptr(),
                crate::model::wide(&file).as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if moved == 0 {
            let _ = fs::remove_file(&tmp);
        }
    }

    fn profile_dir(&self, browser: &str) -> PathBuf {
        self.directory.join("music-player").join(browser)
    }

    fn resolve_browser(&self) -> Option<BrowserCandidate> {
        find_browser(self.preferred_browser.as_deref())
    }

    fn set_preferred_browser(&mut self, browser: &str) {
        let next = if browser.is_empty() || browser == "auto" {
            None
        } else if PLAYER_BROWSERS.contains(&browser) {
            Some(browser.to_owned())
        } else {
            return;
        };
        if self.preferred_browser == next {
            return;
        }
        self.preferred_browser = next;
        let resolved = self.resolve_browser();
        let resolved_name = resolved.as_ref().map(|c| c.browser);
        if (self.ready() || self.child.is_some()) && resolved_name != self.browser_name.as_deref() {
            self.stop();
            self.shutdown_child(1200);
            self.browser_name = None;
            self.app_id.clear();
            self.save();
        }
    }

    fn resume_watch_url(&self, video_id: &str) -> String {
        let mut url = format!("https://music.youtube.com/watch?v={video_id}");
        let list = self
            .current
            .as_ref()
            .map(|c| c.playlist_id.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(self.resume_playlist_id.as_str());
        if !list.is_empty() {
            url.push_str("&list=");
            url.push_str(list);
        }
        url
    }

    fn shutdown_child(&mut self, wait_ms: u32) {
        if let Some(mut child) = self.child.take() {
            if !child.wait_exit(wait_ms) {
                crate::logger::warn(
                    "music",
                    format!("terminating lingering music browser pid={}", child.pid),
                );
                child.terminate();
                let _ = child.wait_exit(500);
            }
        }
    }

    fn is_profile_locked(profile: &Path) -> bool {
        let singleton_lock = profile.join("SingletonLock");
        if singleton_lock.exists() && fs::remove_file(&singleton_lock).is_err() {
            return true;
        }
        let win_lock = profile.join("lockfile");
        if win_lock.exists() {
            match fs::OpenOptions::new()
                .read(true)
                .write(true)
                .share_mode(0)
                .open(&win_lock)
            {
                Ok(file) => {
                    drop(file);
                    let _ = fs::remove_file(&win_lock);
                    false
                }
                Err(err) => err.kind() != std::io::ErrorKind::NotFound,
            }
        } else {
            false
        }
    }

    fn wait_for_profile_unlock(&self, profile: &Path) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if self.stop_flag.load(Ordering::Acquire) || !Self::is_profile_locked(profile) {
                break;
            }
            if !self.sleep_check(Duration::from_millis(100)) {
                break;
            }
        }
    }

    fn ready(&self) -> bool {
        self.pipe.as_ref().is_some_and(|p| !p.is_closed()) && !self.playback_session.is_empty()
    }

    fn emit_state(&self, state: Option<&PlayerPageState>, error: Option<&str>) {
        if self.stop_flag.load(Ordering::Acquire) {
            return;
        }
        send_event(
            &self.events,
            &self.notifier,
            json!({
                "type": "state",
                "state": state,
                "error": error,
            }),
        );
        if let Some(s) = state {
            let current_idx = (!s.video_id.is_empty())
                .then(|| s.queue.iter().position(|item| item.video_id == s.video_id))
                .flatten()
                .or_else(|| s.queue.iter().position(|item| item.selected))
                .or_else(|| {
                    (!s.title.is_empty()).then(|| {
                        s.queue
                            .iter()
                            .position(|item| item.title.eq_ignore_ascii_case(&s.title))
                    })
                    .flatten()
                })
                .or_else(|| (!s.queue.is_empty()).then_some(0));
            if let Some(art) = &s.art {
                self.art.enqueue(art);
            }
            if let Some(idx) = current_idx {
                for offset in -2_isize..=2 {
                    let i = idx as isize + offset;
                    if i >= 0 && (i as usize) < s.queue.len() {
                        self.art.enqueue(&s.queue[i as usize].thumb);
                    }
                }
                for offset in [-4_isize, -3, 3, 4] {
                    let i = idx as isize + offset;
                    if i >= 0 && (i as usize) < s.queue.len() {
                        self.art.enqueue(&s.queue[i as usize].thumb);
                    }
                }
            }
            for item in &s.queue {
                self.art.enqueue(&item.thumb);
            }
        }
    }

    fn sleep_check(&self, dur: Duration) -> bool {
        let deadline = Instant::now() + dur;
        while Instant::now() < deadline {
            if self.stop_flag.load(Ordering::Acquire) {
                return false;
            }
            let step = (deadline - Instant::now()).min(Duration::from_millis(50));
            thread::sleep(step);
        }
        !self.stop_flag.load(Ordering::Acquire)
    }

    fn check_sign_in_exit(&mut self) {
        if let Some(child) = &mut self.sign_in_child {
            if matches!(child.try_wait(), Ok(Some(_)) | Err(_)) {
                self.sign_in_child = None;
                self.pending_sign_in_reconnect = 10;
                self.next_sign_in_retry = Some(Instant::now());
                crate::logger::info(
                    "music",
                    "sign-in browser closed; scheduling background player reconnect",
                );
                self.emit_state(
                    None,
                    Some("Sign-in browser closed. Reconnecting Music..."),
                );
            }
        }
        if self.pending_sign_in_reconnect == 0
            || self.sign_in_child.is_some()
            || self.stop_flag.load(Ordering::Acquire)
        {
            return;
        }
        if self.next_sign_in_retry.is_some_and(|t| Instant::now() < t) {
            return;
        }
        if let Some(found) = self.resolve_browser() {
            let profile = self.profile_dir(found.browser);
            if Self::is_profile_locked(&profile) && self.pending_sign_in_reconnect > 1 {
                self.pending_sign_in_reconnect -= 1;
                self.next_sign_in_retry = Some(Instant::now() + Duration::from_millis(500));
                return;
            }
        }
        match self.launch(Some("https://music.youtube.com/"), false) {
            Ok(()) => {
                self.pending_sign_in_reconnect = 0;
                self.next_sign_in_retry = None;
                let session = self.playback_session.clone();
                let wait_deadline = Instant::now() + Duration::from_millis(2500);
                while Instant::now() < wait_deadline {
                    if self.stop_flag.load(Ordering::Acquire) {
                        break;
                    }
                    if let Ok(raw) = self.evaluate(&session, MUSIC_PAGE_STATE, false) {
                        if !raw.is_null() {
                            break;
                        }
                    }
                    if !self.sleep_check(Duration::from_millis(200)) {
                        break;
                    }
                }
                self.poll();
            }
            Err(err) => {
                self.pending_sign_in_reconnect = self.pending_sign_in_reconnect.saturating_sub(1);
                if self.pending_sign_in_reconnect > 0 {
                    crate::logger::warn(
                        "music",
                        format!(
                            "reconnect after sign-in attempt failed (retries left={}): {err}",
                            self.pending_sign_in_reconnect
                        ),
                    );
                    self.next_sign_in_retry = Some(Instant::now() + Duration::from_millis(500));
                } else {
                    self.next_sign_in_retry = None;
                    crate::logger::warn("music", format!("reconnect after sign-in failed: {err}"));
                    self.emit_state(
                        None,
                        Some("Sign-in browser closed. Press Play to reconnect Music."),
                    );
                }
            }
        }
    }

    fn check_browser_alive(&mut self) {
        if self.playback_session.is_empty() {
            return;
        }
        let pipe_dead = self.pipe.as_ref().map_or(true, |p| p.is_closed());
        let proc_dead = self.child.as_ref().map_or(true, |c| c.is_exited());
        if pipe_dead || proc_dead {
            if let Some(pipe) = self.pipe.take() {
                pipe.close();
            }
            self.child = None;
            self.playback_session.clear();
            self.playback_id.clear();
            self.sessions.clear();
            if let Some(mut host) = self.host.take() {
                host.destroy();
            }
            self.emit_state(None, Some("Background player closed. Press Play to reconnect."));
        }
    }

    fn ensure(&mut self, initial_url: Option<&str>, start_visible: bool) -> Result<(), String> {
        if let Some(child) = &mut self.sign_in_child {
            if matches!(child.try_wait(), Ok(None)) {
                return Err(
                    "Finish signing in, close the sign-in browser window, then press Play.".into(),
                );
            }
            self.sign_in_child = None;
        }
        if self.ready() {
            return Ok(());
        }
        if self.stop_flag.load(Ordering::Acquire) {
            return Err("Music widget closed.".into());
        }
        self.launch(initial_url, start_visible)
    }

    fn launch(&mut self, initial_url: Option<&str>, start_visible: bool) -> Result<(), String> {
        if let Some(url) = initial_url {
            cdp_pipe::validate_player_url(url)?;
        }
        if let Some(mut host) = self.host.take() {
            host.destroy();
        }
        self.current = None;
        self.null_state_polls = 0;
        self.transition_hold_polls = 0;
        self.playback_session.clear();
        self.playback_id.clear();
        self.sessions.clear();
        if let Some(pipe) = self.pipe.take() {
            pipe.close();
        }
        self.shutdown_child(1200);

        let found = self.resolve_browser().ok_or_else(|| {
            "Install a Chromium browser (Brave, Chrome, Edge, CocCoc, Vivaldi, or Opera) to open the music player.".to_string()
        })?;
        let profile = self.profile_dir(found.browser);
        crate::logger::info(
            "music",
            format!(
                "launching {} ({}) with profile={} visible={}",
                found.browser,
                found.executable.display(),
                profile.display(),
                start_visible
            ),
        );

        self.wait_for_profile_unlock(&profile);
        if self.stop_flag.load(Ordering::Acquire) {
            return Err("Music widget closed.".into());
        }

        let nav_url = initial_url.unwrap_or("https://music.youtube.com/");
        let bootstrap = BootstrapServer::start()?;
        let bootstrap_url = bootstrap.url.clone();
        let bootstrap_title = bootstrap.title.clone();
        let base_args = player_arguments(&profile, nav_url)?;
        let mut args: Vec<String> = base_args[..base_args.len() - 1].to_vec();
        args.extend([
            "--remote-debugging-pipe".into(),
            format!("--app={bootstrap_url}"),
            "--window-position=-32000,-32000".into(),
            "--disable-background-mode".into(),
            "--disable-background-timer-throttling".into(),
            "--disable-renderer-backgrounding".into(),
            "--disable-backgrounding-occluded-windows".into(),
        ]);

        fs::create_dir_all(&profile).map_err(|e| e.to_string())?;
        let (child, pipe) = spawn_with_cdp_pipe(&found.executable, &args)?;
        if self.stop_flag.load(Ordering::Acquire) {
            drop(bootstrap);
            pipe.close();
            return Err("Music widget closed.".into());
        }

        if self.browser_name.as_deref() != Some(found.browser) {
            self.app_id.clear();
        }
        self.browser_name = Some(found.browser.to_owned());
        self.pending_sign_in_reconnect = 0;
        self.next_sign_in_retry = None;
        let pid = child.pid;
        self.child = Some(child);
        self.pipe = Some(pipe);
        self.save();

        let startup_result = (|| -> Result<(), String> {
            let mut target_id = String::new();
            for _ in 0..80 {
                if self.stop_flag.load(Ordering::Acquire) {
                    return Err("Music widget closed.".into());
                }
                if self.child.as_ref().map_or(true, |c| c.is_exited()) {
                    return Err("Music browser exited during startup".into());
                }
                let res = self
                    .pipe
                    .as_ref()
                    .ok_or("Music browser disconnected")?
                    .send("Target.getTargets", json!({}), None)?;
                if let Some(found_id) = res["targetInfos"].as_array().and_then(|arr| {
                    arr.iter().find_map(|item| {
                        if item["type"].as_str() == Some("page")
                            && item["url"].as_str() == Some(bootstrap_url.as_str())
                        {
                            item["targetId"].as_str().map(str::to_owned)
                        } else {
                            None
                        }
                    })
                }) {
                    target_id = found_id;
                    break;
                }
                if !self.sleep_check(Duration::from_millis(100)) {
                    return Err("Music widget closed.".into());
                }
            }
            if target_id.is_empty() {
                return Err("Could not create the background music page".into());
            }

            self.playback_id = target_id.clone();
            let session_id = self.attach(&target_id)?;
            self.playback_session = session_id.clone();

            let title_js = format!(
                "document.title={}; true",
                serde_json::to_string(&bootstrap_title).map_err(|e| e.to_string())?
            );
            let _ = self.evaluate(&session_id, &title_js, false)?;

            let mut found_hwnd = None;
            for _ in 0..80 {
                if self.stop_flag.load(Ordering::Acquire) {
                    return Err("Music widget closed.".into());
                }
                if let Some(hwnd) = hide_music_background_window(pid, &bootstrap_title) {
                    found_hwnd = Some(hwnd);
                    break;
                }
                if !self.sleep_check(Duration::from_millis(100)) {
                    return Err("Music widget closed.".into());
                }
            }
            let hwnd = found_hwnd.ok_or("Could not hide the background player window")?;
            if self.stop_flag.load(Ordering::Acquire) {
                return Err("Music widget closed.".into());
            }

            let host = MusicWindowHost::new(hwnd, start_visible)?;
            self.host = Some(host);
            drop(bootstrap);

            if let Some(favicon_js) = youtube_music_favicon_script() {
                if let Some(pipe) = &self.pipe {
                    if let Err(err) = pipe.send("Page.enable", json!({}), Some(&session_id)) {
                        crate::logger::warn("music", format!("Page.enable failed: {err}"));
                    }
                    if let Err(err) = pipe.send(
                        "Page.addScriptToEvaluateOnNewDocument",
                        json!({"source": favicon_js}),
                        Some(&session_id),
                    ) {
                        crate::logger::warn(
                            "music",
                            format!("failed to install favicon script: {err}"),
                        );
                    }
                }
            }

            self.pipe
                .as_ref()
                .ok_or("Music browser disconnected")?
                .send("Page.navigate", json!({"url": nav_url}), Some(&session_id))?;
            if let Some(host) = &self.host {
                host.refresh_icon();
            }
            Ok(())
        })();

        if let Err(err) = startup_result {
            if let Some(pipe) = self.pipe.take() {
                pipe.close();
            }
            if let Some(mut host) = self.host.take() {
                host.destroy();
            }
            self.playback_session.clear();
            self.playback_id.clear();
            self.sessions.clear();
            return Err(format!(
                "Could not start background music. Close the previous dedicated player window and try again. {err}"
            ));
        }

        self.poll();
        Ok(())
    }

    fn attach(&mut self, target_id: &str) -> Result<String, String> {
        if let Some(existing) = self.sessions.get(target_id) {
            return Ok(existing.clone());
        }
        let res = self
            .pipe
            .as_ref()
            .ok_or("Music browser disconnected")?
            .send(
                "Target.attachToTarget",
                json!({"targetId": target_id, "flatten": true}),
                None,
            )?;
        let session_id = res["sessionId"]
            .as_str()
            .ok_or("Missing CDP sessionId")?
            .to_owned();
        self.sessions.insert(target_id.to_owned(), session_id.clone());
        Ok(session_id)
    }

    fn reattach_music_target(&mut self, force: bool) -> Result<bool, String> {
        let res = self
            .pipe
            .as_ref()
            .ok_or("Music browser disconnected")?
            .send("Target.getTargets", json!({}), None)?;
        let mut best: Option<(i32, String, String)> = None;
        if let Some(infos) = res["targetInfos"].as_array() {
            for info in infos {
                if info["type"].as_str() != Some("page") {
                    continue;
                }
                let Some(url) = info["url"].as_str() else {
                    continue;
                };
                if !url.starts_with("https://music.youtube.com") {
                    continue;
                }
                let Some(target_id) = info["targetId"].as_str() else {
                    continue;
                };
                let mut score = 0_i32;
                if url.contains("/watch") {
                    score += 2;
                }
                if !force && target_id == self.playback_id {
                    score += 1;
                } else if force && target_id != self.playback_id {
                    score += 1;
                }
                let better = match &best {
                    None => true,
                    Some((b_score, _, _)) => score > *b_score,
                };
                if better {
                    best = Some((score, target_id.to_owned(), url.to_owned()));
                }
            }
        }
        let Some((_, target_id, url)) = best else {
            return Ok(false);
        };
        if target_id != self.playback_id || force {
            self.sessions.remove(&self.playback_id);
            self.sessions.remove(&target_id);
            let session_id = self.attach(&target_id)?;
            self.playback_id = target_id.clone();
            self.playback_session = session_id.clone();
            crate::logger::info(
                "music",
                format!("reattached music CDP target={target_id} url={url}"),
            );
            return Ok(true);
        }
        Ok(false)
    }

    fn evaluate(
        &self,
        session_id: &str,
        expression: &str,
        user_gesture: bool,
    ) -> Result<Value, String> {
        let result = self
            .pipe
            .as_ref()
            .ok_or("Music browser disconnected")?
            .send(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": user_gesture,
                    "userGesture": user_gesture,
                }),
                Some(session_id),
            )?;
        if let Some(exc) = result.get("exceptionDetails").filter(|v| !v.is_null()) {
            let raw = exc
                .pointer("/exception/description")
                .and_then(|v| v.as_str())
                .or_else(|| exc.get("text").and_then(|v| v.as_str()))
                .unwrap_or("Playback control failed");
            let first = raw.lines().next().unwrap_or("Playback control failed");
            let cleaned = first.strip_prefix("Error:").unwrap_or(first).trim();
            return Err(cleaned.to_owned());
        }
        Ok(result.pointer("/result/value").cloned().unwrap_or(Value::Null))
    }

    fn open(&mut self, url: &str) -> Result<(), String> {
        cdp_pipe::validate_player_url(url)?;
        let was_ready = self.ready();
        self.ensure(Some(url), true)?;
        if was_ready && url != "https://music.youtube.com/" {
            let nav_res = self
                .pipe
                .as_ref()
                .ok_or("Music browser disconnected")?
                .send(
                    "Page.navigate",
                    json!({"url": url}),
                    Some(&self.playback_session),
                );
            if let Err(err) = nav_res {
                if is_target_or_session_error(&err)
                    && self.reattach_music_target(true).unwrap_or(false)
                {
                    self.pipe
                        .as_ref()
                        .ok_or("Music browser disconnected")?
                        .send(
                            "Page.navigate",
                            json!({"url": url}),
                            Some(&self.playback_session),
                        )?;
                } else {
                    return Err(err);
                }
            }
        }
        let host = self
            .host
            .as_ref()
            .ok_or("Music window host is unavailable")?;
        host.show();
        Ok(())
    }

    fn sign_in(&mut self) -> Result<(), String> {
        if let Some(child) = &mut self.sign_in_child {
            if matches!(child.try_wait(), Ok(None)) {
                return Err(
                    "The sign-in browser is already open. Finish signing in and close it before pressing Play.".into(),
                );
            }
            self.sign_in_child = None;
        }
        let found = self.resolve_browser().ok_or_else(|| {
            "Install a Chromium browser (Brave, Chrome, Edge, CocCoc, Vivaldi, or Opera) to sign in to Music.".to_string()
        })?;
        self.pending_sign_in_reconnect = 0;
        self.next_sign_in_retry = None;
        self.stop();
        self.shutdown_child(2000);

        let profile = self.profile_dir(found.browser);
        fs::create_dir_all(&profile).map_err(|e| e.to_string())?;
        self.wait_for_profile_unlock(&profile);
        self.browser_name = Some(found.browser.to_owned());
        self.save();

        let child = Command::new(&found.executable)
            .args(sign_in_arguments(&profile))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Cannot start sign-in browser: {e}"))?;
        crate::logger::info(
            "music",
            format!(
                "sign-in browser opened: browser={}, profile={}",
                found.browser,
                profile.display()
            ),
        );
        self.sign_in_child = Some(child);
        self.emit_state(
            None,
            Some("Sign in in the browser, close that window, then press Play. Your Music profile is kept."),
        );
        Ok(())
    }

    fn sign_out(&mut self) -> Result<(), String> {
        self.ensure(None, false)?;
        let session = self.playback_session.clone();
        let pipe = self.pipe.as_ref().ok_or("Music browser disconnected")?;
        pipe.send("Network.clearBrowserCookies", json!({}), Some(&session))?;
        let _ = pipe.send("Network.clearBrowserCache", json!({}), Some(&session));
        for origin in [
            "https://music.youtube.com",
            "https://www.youtube.com",
            "https://accounts.google.com",
            "https://myaccount.google.com",
        ] {
            let _ = pipe.send(
                "Storage.clearDataForOrigin",
                json!({"origin": origin, "storageTypes": "all"}),
                Some(&session),
            );
        }
        self.resume_video_id.clear();
        self.resume_playlist_id.clear();
        self.resume_position = 0.0;
        self.save();
        pipe.send(
            "Page.navigate",
            json!({"url": "https://music.youtube.com/"}),
            Some(&session),
        )?;
        let signed_out = PlayerPageState {
            signed_in: false,
            auth_known: true,
            ..PlayerPageState::default()
        };
        self.current = Some(signed_out.clone());
        self.emit_state(Some(&signed_out), None);
        Ok(())
    }

    fn instance_info(&self) -> (String, usize) {
        if !self.ready() {
            return (String::new(), 0);
        }
        let page_count = self
            .pipe
            .as_ref()
            .and_then(|p| p.send("Target.getTargets", json!({}), None).ok())
            .and_then(|res| {
                res["targetInfos"].as_array().map(|arr| {
                    arr.iter()
                        .filter(|t| t["type"].as_str() == Some("page"))
                        .count()
                })
            })
            .unwrap_or(0);
        (self.playback_id.clone(), page_count)
    }

    fn play(
        &mut self,
        video_id: &str,
        position: f64,
        cmd_rx: Option<&Receiver<Value>>,
    ) -> Result<(), String> {
        validate_video_id(video_id)?;
        let pos = if position.is_finite() {
            position.max(0.0)
        } else {
            0.0
        };
        let nav_url = self.resume_watch_url(video_id);
        let was_ready = self.ready();
        self.ensure(Some(&nav_url), false)?;
        self.resume_video_id = video_id.to_owned();
        self.resume_position = pos;
        self.save();

        let session = self.playback_session.clone();
        if was_ready {
            let needs_switch = self
                .current
                .as_ref()
                .map_or(true, |c| c.video_id != video_id);
            if needs_switch {
                let mut selected = false;
                if self
                    .current
                    .as_ref()
                    .is_some_and(|c| c.queue.iter().any(|item| item.video_id == video_id))
                {
                    let js = music_page_select(video_id)?;
                    if let Ok(v) = self.evaluate(&session, &js, true) {
                        selected = v.as_bool() == Some(true);
                    }
                }
                if !selected {
                    self.pipe
                        .as_ref()
                        .ok_or("Music browser disconnected")?
                        .send("Page.navigate", json!({"url": nav_url}), Some(&session))?;
                }
            }
        }

        // Poll up to ~6s for music.youtube.com controls (<video> or .play-pause-button).
        // Trigger play once readyState >= 2 OR ~1.5s after controls appear so autoplay
        // gating on readyState < 2 never deadlocks cold-start playback.
        let deadline = Instant::now() + Duration::from_secs(6);
        let mut controls_seen_at: Option<Instant> = None;
        let mut ready_to_play = false;
        while Instant::now() < deadline {
            if self.stop_flag.load(Ordering::Acquire) {
                return Err("Music widget closed.".into());
            }
            if let Some(rx) = cmd_rx {
                if let Ok(next_cmd) = rx.try_recv() {
                    let act = next_cmd["action"].as_str().unwrap_or("");
                    if act == "quit" {
                        self.stop_flag.store(true, Ordering::Release);
                        return Err("Music widget closed.".into());
                    }
                    if matches!(
                        act,
                        "pause" | "toggle" | "select" | "browse" | "library" | "signin" | "signout" | "next" | "previous"
                    ) {
                        self.pending_cmd = Some(next_cmd);
                        return Ok(());
                    }
                }
            }
            if let Ok(probe) = self.evaluate(&session, MUSIC_PLAY_PROBE, false) {
                let err = probe["error"].as_str().unwrap_or("").trim();
                if !err.is_empty() {
                    return Err(err.to_owned());
                }
                let on_origin = probe["onOrigin"].as_bool() == Some(true);
                let has_controls = probe["hasControls"].as_bool() == Some(true);
                let ready_state = probe["readyState"].as_i64().unwrap_or(0);
                if on_origin && has_controls {
                    let seen = *controls_seen_at.get_or_insert_with(Instant::now);
                    if ready_state >= 2 || seen.elapsed() >= Duration::from_millis(1500) {
                        ready_to_play = true;
                        break;
                    }
                }
            }
            if !self.sleep_check(Duration::from_millis(250)) {
                return Err("Music widget closed.".into());
            }
        }
        if !ready_to_play {
            return Err(
                "This track did not load. Browse music to check sign-in or playback availability."
                    .into(),
            );
        }

        let play_js = music_page_command("play", None)?;
        for attempt in 0..=4 {
            match self.evaluate(&session, &play_js, true) {
                Ok(_) => break,
                Err(err) => {
                    let lower = err.to_ascii_lowercase();
                    let retryable = lower.contains("aborterror")
                        || lower.contains("new load")
                        || lower.contains("interrupted");
                    if attempt >= 4 || !retryable {
                        return Err(err);
                    }
                    if !self.sleep_check(Duration::from_millis(300)) {
                        return Err("Music widget closed.".into());
                    }
                }
            }
        }

        if pos > 0.0 {
            let seek_deadline = Instant::now() + Duration::from_millis(1500);
            while Instant::now() < seek_deadline {
                if self.stop_flag.load(Ordering::Acquire) {
                    return Err("Music widget closed.".into());
                }
                if let Ok(probe) = self.evaluate(&session, MUSIC_PLAY_PROBE, false) {
                    if probe["readyState"].as_i64().unwrap_or(0) >= 1 {
                        break;
                    }
                }
                if !self.sleep_check(Duration::from_millis(150)) {
                    return Err("Music widget closed.".into());
                }
            }
            let seek_js = music_page_command("seek", Some(pos))?;
            let _ = self.evaluate(&session, &seek_js, true);
        }

        self.resume_video_id = video_id.to_owned();
        self.resume_position = pos;
        self.save();
        self.publish()
    }

    fn command(
        &mut self,
        action: &str,
        value: Option<f64>,
        cmd_rx: Option<&Receiver<Value>>,
    ) -> Result<(), String> {
        let was_ready = self.ready();
        if !was_ready && !self.resume_video_id.is_empty() && action != "pause" {
            let vid = self.resume_video_id.clone();
            let pos = self.resume_position;
            return self.play(&vid, pos, cmd_rx);
        }
        if action == "toggle"
            && !self.current.as_ref().is_some_and(|c| c.available)
            && self.resume_video_id.is_empty()
        {
            if !was_ready {
                return self.open("https://music.youtube.com/");
            }
            let has_media = self
                .evaluate(
                    &self.playback_session,
                    "[...document.querySelectorAll('video')].some(v => v.src || v.currentSrc) || !!document.getElementById('movie_player')?.getVideoData?.()?.video_id",
                    false,
                )
                .ok()
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !has_media {
                return self.open("https://music.youtube.com/");
            }
        }
        self.ensure(None, false)?;
        if !self.current.as_ref().is_some_and(|c| c.available)
            && !self.resume_video_id.is_empty()
            && action != "pause"
        {
            let vid = self.resume_video_id.clone();
            let pos = self.resume_position;
            return self.play(&vid, pos, cmd_rx);
        }
        let js = music_page_command(action, value)?;
        let session = self.playback_session.clone();
        if let Err(err) = self.evaluate(&session, &js, true) {
            if is_target_or_session_error(&err) && self.reattach_music_target(true).unwrap_or(false)
            {
                let retry_session = self.playback_session.clone();
                self.evaluate(&retry_session, &js, true)?;
            } else {
                return Err(err);
            }
        }
        self.publish()
    }

    fn publish(&mut self) -> Result<(), String> {
        let mut session = self.playback_session.clone();
        let mut raw = match self.evaluate(&session, MUSIC_PAGE_STATE, false) {
            Ok(val) => val,
            Err(err) if is_target_or_session_error(&err) => {
                crate::logger::warn("music", format!("reattaching after evaluate error: {err}"));
                if !self.reattach_music_target(true)? {
                    self.playback_session.clear();
                    self.playback_id.clear();
                    self.sessions.clear();
                    if let Some(mut host) = self.host.take() {
                        host.destroy();
                    }
                    self.emit_state(
                        None,
                        Some("Background player closed. Press Play to reconnect."),
                    );
                    return Ok(());
                }
                session = self.playback_session.clone();
                self.evaluate(&session, MUSIC_PAGE_STATE, false)?
            }
            Err(err) => return Err(err),
        };
        let prev_signed_in = self.current.as_ref().map(|c| c.signed_in);
        let mut state = match sanitize_page_state(&raw, prev_signed_in) {
            Some(s) => s,
            None => {
                self.null_state_polls = self.null_state_polls.saturating_add(1);
                if self.null_state_polls >= 2 && self.reattach_music_target(true).unwrap_or(false) {
                    self.null_state_polls = 0;
                    session = self.playback_session.clone();
                    raw = self.evaluate(&session, MUSIC_PAGE_STATE, false)?;
                    match sanitize_page_state(&raw, prev_signed_in) {
                        Some(s) => s,
                        None => return Ok(()),
                    }
                } else {
                    return Ok(());
                }
            }
        };
        self.null_state_polls = 0;
        if self.stop_flag.load(Ordering::Acquire) {
            return Ok(());
        }
        if !state.signed_in {
            if let Some(pipe) = &self.pipe {
                if let Ok(cookies) = pipe.send(
                    "Network.getCookies",
                    json!({"urls": ["https://music.youtube.com", "https://youtube.com"]}),
                    Some(&session),
                ) {
                    if has_auth_session_cookies(&cookies) {
                        state.signed_in = true;
                        state.auth_known = true;
                    }
                }
            }
        }
        if let Some(curr) = &mut self.current {
            if !curr.title.is_empty() && state.title.is_empty() {
                curr.signed_in = state.signed_in;
                curr.auth_known = state.auth_known;
                let snapshot = curr.clone();
                self.emit_state(Some(&snapshot), None);
                return Ok(());
            } else if should_hold_transition(curr, &state, self.transition_hold_polls) {
                self.transition_hold_polls += 1;
                curr.signed_in = state.signed_in;
                curr.auth_known = state.auth_known;
                let snapshot = curr.clone();
                self.emit_state(Some(&snapshot), None);
                return Ok(());
            } else {
                self.transition_hold_polls = 0;
            }
        } else {
            self.transition_hold_polls = 0;
        }
        if validate_video_id(&state.video_id).is_ok() && (state.available || state.playing) {
            let mut changed = self.resume_video_id != state.video_id;
            self.resume_video_id = state.video_id.clone();
            self.resume_position = state.position;
            if self.resume_playlist_id != state.playlist_id {
                self.resume_playlist_id = state.playlist_id.clone();
                changed = true;
            }
            if changed {
                self.save();
            }
        }
        let track_changed = self.current.as_ref().is_none_or(|c| {
            c.video_id != state.video_id || c.playing != state.playing || c.available != state.available
        });
        if track_changed {
            crate::logger::info(
                "music",
                format!(
                    "track sync: id={} playing={} available={} title={}",
                    state.video_id, state.playing, state.available, state.title
                ),
            );
        }
        self.current = Some(state.clone());
        self.emit_state(Some(&state), None);
        if let Some(host) = &self.host {
            host.ensure_icon();
        }
        Ok(())
    }

    fn poll(&mut self) {
        if !self.ready() {
            return;
        }
        if let Some(host) = &self.host {
            host.ensure_icon();
        }
        if let Err(msg) = self.publish() {
            crate::logger::warn("music", format!("music poll error: {msg}"));
            if !self.stop_flag.load(Ordering::Acquire)
                && self.ready()
                && !is_target_or_session_error(&msg)
            {
                let curr = self.current.clone();
                self.emit_state(curr.as_ref(), Some(&msg));
            }
        }
    }

    fn stop(&mut self) {
        self.save();
        let pipe = self.pipe.take();
        self.playback_session.clear();
        self.playback_id.clear();
        self.sessions.clear();
        self.current = None;
        self.null_state_polls = 0;
        self.transition_hold_polls = 0;
        if let Some(mut host) = self.host.take() {
            host.destroy();
        }
        if let Some(pipe) = pipe {
            if !pipe.is_closed() {
                let _ = pipe.send("Browser.close", json!({}), None);
                pipe.close();
            }
        }
        self.emit_state(None, None);
    }
}

fn dispatch_command(
    player: &mut MusicPlayer,
    msg: &Value,
    cmd_rx: Option<&Receiver<Value>>,
) -> Result<(), String> {
    let action = msg["action"].as_str().unwrap_or("");
    crate::logger::info("music", format!("native music action={action}"));
    match action {
        "browse" => player.open("https://music.youtube.com/"),
        "library" => player.open("https://music.youtube.com/library"),
        "signin" => player.sign_in(),
        "signout" => player.sign_out(),
        "setBrowser" => {
            player.set_preferred_browser(msg["browser"].as_str().unwrap_or("auto"));
            Ok(())
        }
        "layout" => Ok(()),
        "select" => player.play(msg["videoId"].as_str().unwrap_or(""), 0.0, cmd_rx),
        "info" => {
            let (instance_id, page_count) = player.instance_info();
            send_event(
                &player.events,
                &player.notifier,
                json!({
                    "type": "info",
                    "instanceId": instance_id,
                    "pageCount": page_count,
                }),
            );
            Ok(())
        }
        "toggle" | "play" | "pause" | "next" | "previous" | "seek" => {
            player.command(action, msg["value"].as_f64(), cmd_rx)
        }
        _ => Err("Unknown native music action".into()),
    }
}

fn run_worker(
    root: &Path,
    cmd_rx: Receiver<Value>,
    events_tx: Sender<Value>,
    stop_flag: Arc<AtomicBool>,
    notifier: EventNotifier,
) {
    let directory = root.join("config").join("native-music-player");
    let mut player = MusicPlayer::new(
        directory,
        events_tx.clone(),
        notifier.clone(),
        Arc::clone(&stop_flag),
    );
    send_event(&events_tx, &notifier, json!({"type": "ready"}));

    let mut next_poll = Instant::now() + Duration::from_secs(1);
    loop {
        if stop_flag.load(Ordering::Acquire) {
            break;
        }
        player.check_sign_in_exit();
        player.check_browser_alive();

        let next_msg = if let Some(msg) = player.pending_cmd.take() {
            Ok(msg)
        } else {
            let timeout = if player.sign_in_child.is_some() || player.pending_sign_in_reconnect > 0 {
                Duration::from_millis(200)
            } else if player.ready() {
                next_poll.saturating_duration_since(Instant::now())
            } else {
                Duration::from_millis(250)
            };
            cmd_rx.recv_timeout(timeout)
        };

        match next_msg {
            Ok(msg) => {
                if msg["action"].as_str() == Some("quit") {
                    break;
                }
                if let Err(err) = dispatch_command(&mut player, &msg, Some(&cmd_rx)) {
                    crate::logger::error("music", format!("native music error: {err}"));
                    if !stop_flag.load(Ordering::Acquire) {
                        send_event(
                            &events_tx,
                            &notifier,
                            json!({"type": "error", "error": err}),
                        );
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        if stop_flag.load(Ordering::Acquire) {
            break;
        }
        if player.ready() && Instant::now() >= next_poll {
            player.poll();
            next_poll = Instant::now() + Duration::from_secs(1);
        }
    }

    stop_flag.store(true, Ordering::Release);
    player.stop();
    player.shutdown_child(1200);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "lumen-music-backend-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn backend_starts_and_drops_cleanly_without_node() {
        let temp = TempDir::new();
        assert!(!temp.0.join("runtime/node.exe").exists());
        assert!(!temp.0.join("runtime/music-bridge.cjs").exists());

        let mut backend = Backend::start(&temp.0).expect("Backend::start must succeed without Node.js");
        let ready = backend
            .events
            .recv_timeout(Duration::from_secs(2))
            .expect("should emit ready event");
        assert_eq!(ready["type"], "ready");
        assert!(!backend.exited());

        backend.send(json!({"action": "layout"})).unwrap();
        backend.send(json!({"action": "info"})).unwrap();
        let info = backend
            .events
            .recv_timeout(Duration::from_secs(2))
            .expect("should emit info event");
        assert_eq!(info["type"], "info");
        assert_eq!(info["instanceId"], "");
        assert_eq!(info["pageCount"], 0);

        drop(backend);
    }

    #[test]
    fn persisted_player_resume_playlist_id_round_trip_and_legacy_compat() {
        let legacy: PersistedPlayer = serde_json::from_str(
            r#"{"browser":"chrome","appId":"LumenMusic1","videoId":"dQw4w9WgXcQ","position":42.5}"#,
        )
        .expect("legacy JSON without resumePlaylistId should deserialize");
        assert_eq!(legacy.browser.as_deref(), Some("chrome"));
        assert_eq!(legacy.video_id, "dQw4w9WgXcQ");
        assert_eq!(legacy.resume_playlist_id, "");
        assert_eq!(legacy.position, 42.5);

        let full = PersistedPlayer {
            browser: Some("chrome".into()),
            app_id: "LumenMusic1".into(),
            video_id: "dQw4w9WgXcQ".into(),
            resume_playlist_id: "RDAMVMdQw4w9WgXcQ".into(),
            position: 12.0,
        };
        let encoded = serde_json::to_string(&full).unwrap();
        let decoded: PersistedPlayer = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.resume_playlist_id, "RDAMVMdQw4w9WgXcQ");
        assert_eq!(decoded.video_id, "dQw4w9WgXcQ");
        assert_eq!(decoded.position, 12.0);
    }

    #[test]
    fn target_or_session_error_classifier_matches_cdp_errors() {
        assert!(is_target_or_session_error("Execution context was destroyed"));
        assert!(is_target_or_session_error(
            "Cannot find context with specified id"
        ));
        assert!(is_target_or_session_error("Session with given id not found"));
        assert!(is_target_or_session_error("No target with given id found"));
        assert!(!is_target_or_session_error("This track did not load"));
    }

    #[test]
    fn should_hold_transition_releases_after_two_polls_or_playback_ready() {
        let curr = PlayerPageState {
            title: "Old Track".into(),
            video_id: "aaaaaaaaaaa".into(),
            available: true,
            playing: true,
            ..PlayerPageState::default()
        };
        let loading_next = PlayerPageState {
            title: "New Track".into(),
            video_id: "bbbbbbbbbbb".into(),
            available: false,
            playing: false,
            ..PlayerPageState::default()
        };
        assert!(should_hold_transition(&curr, &loading_next, 0));
        assert!(should_hold_transition(&curr, &loading_next, 1));
        assert!(!should_hold_transition(&curr, &loading_next, 2));

        let playing_next = PlayerPageState {
            playing: true,
            ..loading_next.clone()
        };
        assert!(!should_hold_transition(&curr, &playing_next, 0));

        let available_next = PlayerPageState {
            available: true,
            ..loading_next
        };
        assert!(!should_hold_transition(&curr, &available_next, 0));
    }

    #[test]
    fn backend_start_with_notify_posts_window_message_for_ready_and_info() {
        use std::ptr::{null, null_mut};
        use windows_sys::Win32::{
            Foundation::HWND,
            System::LibraryLoader::GetModuleHandleW,
            UI::WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, MSG, PM_REMOVE, PeekMessageW, WM_APP,
            },
        };

        struct HiddenWindow(HWND);
        impl Drop for HiddenWindow {
            fn drop(&mut self) {
                if !self.0.is_null() {
                    unsafe {
                        DestroyWindow(self.0);
                    }
                }
            }
        }

        let _ui = crate::native_ui_test_lock();
        let temp = TempDir::new();
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                null(),
                0,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            )
        };
        assert!(!hwnd.is_null(), "hidden test HWND must be created");
        let _guard = HiddenWindow(hwnd);
        let notify_msg = WM_APP + 84;

        let wait_for_notify = |hwnd: HWND, msg_id: u32| -> bool {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut msg: MSG = unsafe { std::mem::zeroed() };
            while Instant::now() < deadline {
                if unsafe { PeekMessageW(&mut msg, hwnd, msg_id, msg_id, PM_REMOVE) } != 0 {
                    return true;
                }
                thread::sleep(Duration::from_millis(5));
            }
            false
        };

        let mut backend = Backend::start_with_notify(&temp.0, hwnd, notify_msg)
            .expect("Backend::start_with_notify must succeed");
        let ready = backend
            .events
            .recv_timeout(Duration::from_secs(2))
            .expect("should emit ready event");
        assert_eq!(ready["type"], "ready");
        assert!(
            wait_for_notify(hwnd, notify_msg),
            "ready event must post WM_APP + 84 to notify HWND"
        );

        backend.clear_notify_pending();
        backend.send(json!({"action": "info"})).unwrap();
        let info = backend
            .events
            .recv_timeout(Duration::from_secs(2))
            .expect("should emit info event");
        assert_eq!(info["type"], "info");
        assert!(
            wait_for_notify(hwnd, notify_msg),
            "info event must post WM_APP + 84 to notify HWND"
        );

        backend.set_notify(null_mut(), 0);
        drop(backend);
    }

    #[test]
    fn set_browser_command_is_accepted_without_error() {
        let temp = TempDir::new();
        let mut backend = Backend::start(&temp.0).expect("Backend::start must succeed");
        let ready = backend
            .events
            .recv_timeout(Duration::from_secs(2))
            .expect("should emit ready event");
        assert_eq!(ready["type"], "ready");

        backend
            .send(json!({"action": "setBrowser", "browser": "brave"}))
            .unwrap();
        backend
            .send(json!({"action": "setBrowser", "browser": "auto"}))
            .unwrap();
        backend.send(json!({"action": "info"})).unwrap();
        let event = backend
            .events
            .recv_timeout(Duration::from_secs(2))
            .expect("should emit info event without error");
        assert_eq!(event["type"], "info");
        drop(backend);
    }
}
