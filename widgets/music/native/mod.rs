//! Native Music surface: the dock's wallpaper glass, compositor and typography.
//! Win32 handles and GDI images stay on the UI thread. Browser I/O is isolated.
mod backend;
mod chrome;
mod coverflow;
mod icons;
mod lenses;
mod model;
mod responsive;
mod session;
mod settings;
mod smtc;
mod themes;
mod view;
use crate::{
    compositor::{Buffer, WindowShadow},
    glass::Glass,
    model::wide,
    render::{self, Canvas, Image},
};
use model::{Config, Playback};
use std::{
    cell::RefCell,
    collections::HashMap,
    mem::{size_of, zeroed},
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    rc::Rc,
    time::Instant,
};
use view::{Action, Button};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Controls::WM_MOUSELEAVE, HiDpi::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

const MUSIC_EVENT: u32 = WM_APP + 84;
const PEER_PLAYBACK_MSG: u32 = WM_APP + 86;
const PEER_COMMAND_MSG: u32 = WM_APP + 87;

#[derive(Default)]
struct SharedPlayback {
    owner: usize,
    live: bool,
    playback: Playback,
    art_files: HashMap<String, String>,
    error: String,
    version: u64,
    pending_cmds: std::collections::VecDeque<serde_json::Value>,
}

thread_local! {
    static SHARED_PLAYBACK: RefCell<SharedPlayback> = RefCell::new(SharedPlayback::default());
}

struct State {
    hwnd: HWND,
    root: PathBuf,
    config: Config,
    glass: Rc<RefCell<Glass>>,
    backend: Option<backend::Backend>,
    backend_live: bool,
    peer_live: bool,
    peer_sync_version: u64,
    smtc: Option<smtc::SmtcClient>,
    latest_smtc: Option<smtc::SmtcState>,
    smtc_active: bool,
    playback: Playback,
    last_cover: Option<String>,
    session_signature: String,
    media_icons: icons::Icons,
    button_lenses: lenses::Lenses,
    received: Instant,
    last_poll: Instant,
    last_seek: Option<Instant>,
    spin_angle: f32,
    spin_updated: Instant,
    spin_active: bool,
    coverflow_anim: Option<coverflow::Transition>,
    anim_clock: Option<crate::motion::FrameClock>,
    frame_queued: bool,
    images: HashMap<String, Image>,
    card_cache: HashMap<String, Image>,
    rounded_art: RefCell<HashMap<String, Image>>,
    art_files: HashMap<String, String>,
    buttons: Vec<Button>,
    seek_track: Option<[f32; 4]>,
    control_plates: Vec<([f32; 4], f32)>,
    hover: Option<Action>,
    pressed: Option<Action>,
    frost: Option<Image>,
    background: Option<Buffer>,
    frame: Option<Buffer>,
    spin_base: Option<Buffer>,
    spin_base_key: String,
    spin_disc_rect: Option<[f32; 4]>,
    skip_spin_disc: bool,
    overlay: Option<Image>,
    shadow: Option<WindowShadow>,
    dpi: f32,
    content_scale: f32,
    ink: u32,
    muted: u32,
    light: bool,
    settings: Option<crate::settings_ui::Host>,
    local_glass: Option<Glass>,
    settings_dirty: bool,
    blur_dirty: bool,
    moving: bool,
    resizing: bool,
    queue_offset: usize,
    error: String,
    smoke: bool,
    inspect: bool,
    revision: u64,
    fusion_revision: u64,
    window_origin: [i32; 2],
    was_low_power_moving: bool,
}
impl State {
    fn position(&self) -> f64 {
        let elapsed = if self.playback.playing && self.pressed != Some(Action::Seek) {
            self.received.elapsed().as_secs_f64().min(2.)
        } else {
            0.
        };
        (self.playback.position + elapsed)
            .max(0.)
            .min(self.playback.duration.max(0.))
    }
    fn scrub_to(&mut self, x: f32) -> Option<f64> {
        let track = self
            .seek_track
            .or_else(|| self.buttons.iter().find(|b| b.action == Action::Seek).map(|b| b.rect))?;
        let span = (track[2] - track[0]).max(1.);
        let fraction = ((x - track[0]) / span).clamp(0., 1.) as f64;
        let target = fraction * self.playback.duration;
        self.playback.position = target;
        self.received = Instant::now();
        self.last_seek = Some(Instant::now());
        Some(target)
    }
    fn commit_seek(&mut self, x: f32) {
        if let Some(target) = self.scrub_to(x) {
            if self.use_smtc() {
                if let Some(smtc) = &self.smtc {
                    smtc.send(smtc::SmtcCommand::Seek(target));
                }
            } else {
                self.send(serde_json::json!({"action":"seek","value":target}));
            }
        }
    }
    fn use_smtc(&self) -> bool {
        !self.backend_live
            && !self.peer_live
            && self
                .latest_smtc
                .as_ref()
                .is_some_and(|s| s.has_session && s.available)
    }
    fn accept_smtc(&mut self, smtc: &smtc::SmtcState) {
        if let (Some(url), Some(file)) = (&smtc.art_url, &smtc.art_file) {
            if session::valid_file(file) {
                if self.art_files.len() > 64 {
                    self.art_files.retain(|u, _| {
                        self.last_cover.as_ref() == Some(u)
                            || self.playback.art.as_ref() == Some(u)
                            || self.playback.queue.iter().any(|t| t.thumb == *u)
                    });
                }
                self.art_files.insert(url.clone(), file.clone());
            }
        }
        let synthetic_id = if !smtc.title.is_empty() || !smtc.artist.is_empty() {
            format!("smtc:{}\0{}", smtc.title, smtc.artist)
        } else {
            String::new()
        };
        let norm_smtc_title = normalize_track_text(&smtc.title);
        let norm_smtc_artist = normalize_track_text(&smtc.artist);
        let norm_cur_title = normalize_track_text(&self.playback.title);
        let norm_cur_artist = normalize_track_text(&self.playback.artist);
        let matches_active = !self.playback.queue.is_empty()
            && !norm_smtc_title.is_empty()
            && (norm_smtc_title == norm_cur_title
                || (!norm_smtc_artist.is_empty()
                    && ((norm_smtc_title == norm_cur_title && norm_smtc_artist == norm_cur_artist)
                        || self.playback.queue.iter().any(|t| {
                            normalize_track_text(&t.title) == norm_smtc_title
                                && normalize_track_text(&t.artist) == norm_smtc_artist
                        }))));
        let (video_id, queue) = if matches_active {
            let mut q = self.playback.queue.clone();
            let matched_id = q
                .iter()
                .position(|t| {
                    normalize_track_text(&t.title) == norm_smtc_title
                        && (norm_smtc_artist.is_empty()
                            || normalize_track_text(&t.artist) == norm_smtc_artist)
                })
                .map(|idx| {
                    for (i, item) in q.iter_mut().enumerate() {
                        item.selected = i == idx;
                    }
                    q[idx].video_id.clone()
                })
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| {
                    if !self.playback.video_id.is_empty()
                        && !self.playback.video_id.starts_with("smtc:")
                    {
                        self.playback.video_id.clone()
                    } else {
                        synthetic_id.clone()
                    }
                });
            (matched_id, q)
        } else {
            (synthetic_id, Vec::new())
        };
        let next = Playback {
            signed_in: self.playback.signed_in,
            auth_known: false,
            album: smtc.album.clone(),
            video_id,
            available: smtc.available,
            playing: smtc.playing,
            title: smtc.title.clone(),
            artist: smtc.artist.clone(),
            art: smtc.art_url.clone(),
            position: smtc.position,
            duration: smtc.duration,
            can_next: smtc.can_next,
            can_previous: smtc.can_previous,
            can_seek: smtc.can_seek,
            queue,
            error: None,
        };
        self.error.clear();
        self.accept_playback(next);
        self.smtc_active = true;
    }
    fn save_local(&self) {
        if !self.smoke {
            if let Err(e) = self.config.save(&self.root) {
                eprintln!("native-music: {e}");
            }
        }
    }
    fn save_floating(&self) {
        if unsafe {
            crate::multi_monitor::record_duplicate_floating(
                self.hwnd,
                self.dpi,
                self.config.floating,
            )
        } {
            return;
        }
        if !crate::multi_monitor::is_duplicate(self.hwnd) {
            self.save_local();
        }
    }
    fn save(&self) {
        if self.smoke {
            return;
        }
        let res = if crate::multi_monitor::is_duplicate(self.hwnd) {
            let mut shared = self.config.clone();
            if let Ok(disk) = Config::load(&self.root) {
                shared.position = disk.position;
                shared.size = disk.size;
                shared.open = disk.open;
                shared.floating = disk.floating;
            }
            shared.save(&self.root)
        } else {
            self.config.save(&self.root)
        };
        match res {
            Ok(()) => unsafe { crate::multi_monitor::notify_peers("music", self.hwnd) },
            Err(e) => eprintln!("native-music: {e}"),
        }
    }
    unsafe fn reload_from_shared(&mut self) {
        let Ok(mut loaded) = Config::load(&self.root) else {
            return;
        };
        loaded.position = self.config.position;
        loaded.size = self.config.size;
        loaded.open = self.config.open;
        loaded.floating = self.config.floating;
        let queue_changed = loaded.queue != self.config.queue;
        let design_changed = loaded.design != self.config.design;
        let browser_changed = loaded.browser != self.config.browser;
        self.config = loaded;
        if browser_changed && self.backend.is_some() {
            self.send(serde_json::json!({
                "action": "setBrowser",
                "browser": self.config.browser
            }));
        }
        if queue_changed {
            self.queue_offset = 0;
        }
        if design_changed {
            self.coverflow_anim = None;
        }
        self.local_glass = None;
        self.blur_dirty = false;
        self.frame = None;
        self.background = None;
        self.clear_spin_base();
        self.overlay = None;
        self.frost = None;
        self.card_cache.clear();
        self.sync_settings();
        if self.settings.is_some() {
            let color = self.surface_color();
            if let Some(host) = &mut self.settings {
                host.set_surface(color, crate::accent::light(color));
            }
        }
        self.sync_spin_timer();
        InvalidateRect(self.hwnd, null(), 0);
    }
    fn publish_to_peers(&mut self) {
        if self.smoke {
            return;
        }
        SHARED_PLAYBACK.with(|shared| {
            let mut shared = shared.borrow_mut();
            shared.owner = self.hwnd as usize;
            shared.live = self.backend_live;
            shared.playback = self.playback.clone();
            shared.art_files = self.art_files.clone();
            shared.error = self.error.clone();
            shared.version = shared.version.wrapping_add(1);
        });
        unsafe {
            crate::multi_monitor::notify_peers_with_message(
                "music",
                self.hwnd,
                PEER_PLAYBACK_MSG,
            );
        }
    }
    fn sync_from_peer(&mut self) {
        if self.smoke || self.backend_live {
            return;
        }
        let snapshot = SHARED_PLAYBACK.with(|shared| {
            let shared = shared.borrow();
            if shared.owner == self.hwnd as usize || shared.version == self.peer_sync_version {
                return None;
            }
            Some((
                shared.version,
                shared.live,
                shared.playback.clone(),
                shared.art_files.clone(),
                shared.error.clone(),
            ))
        });
        let Some((version, live, playback, art_files, error)) = snapshot else {
            return;
        };
        self.peer_sync_version = version;
        self.peer_live = live;
        if !art_files.is_empty() {
            if self.art_files.len() + art_files.len() > 64 {
                self.art_files.retain(|url, _| {
                    self.last_cover.as_ref() == Some(url)
                        || self.playback.art.as_ref() == Some(url)
                        || self.playback.queue.iter().any(|t| t.thumb == *url)
                        || playback.art.as_ref() == Some(url)
                        || playback.queue.iter().any(|t| t.thumb == *url)
                });
            }
            self.art_files.extend(art_files);
        }
        if live {
            self.smtc_active = false;
            self.error = error;
            self.accept_playback(playback);
            if !self.hwnd.is_null() && unsafe { IsWindowVisible(self.hwnd) != 0 } {
                unsafe {
                    SetTimer(self.hwnd, 91, 250, None);
                }
            }
            if self.coverflow_anim.is_some()
                && self.config.design == "coverflow"
                && !self.hwnd.is_null()
                && unsafe { IsWindowVisible(self.hwnd) != 0 }
            {
                if self.anim_clock.is_none() {
                    self.anim_clock = Some(crate::motion::FrameClock::start());
                }
                unsafe {
                    SetTimer(self.hwnd, 94, crate::motion::frame_interval_ms(), None);
                }
            }
        } else {
            self.playback.available = false;
            self.playback.playing = false;
            self.playback.can_next = false;
            self.playback.can_previous = false;
            self.playback.can_seek = false;
            self.error = error;
        }
        if !self.hwnd.is_null() && unsafe { IsWindowVisible(self.hwnd) != 0 } {
            self.restore_art();
            unsafe {
                InvalidateRect(self.hwnd, null(), 0);
            }
        }
        unsafe {
            self.sync_spin_timer();
        }
    }
    fn send(&mut self, value: serde_json::Value) {
        if !self.smoke && self.backend.is_none() {
            let target = SHARED_PLAYBACK.with(|shared| {
                let mut shared = shared.borrow_mut();
                if shared.owner != 0
                    && shared.owner != self.hwnd as usize
                    && unsafe { IsWindow(shared.owner as HWND) != 0 }
                {
                    shared.pending_cmds.push_back(value.clone());
                    Some(shared.owner as HWND)
                } else {
                    None
                }
            });
            if let Some(owner_hwnd) = target {
                unsafe {
                    PostMessageW(owner_hwnd, PEER_COMMAND_MSG, 0, 0);
                }
                return;
            }
        }
        let action = value["action"].as_str().unwrap_or("unknown").to_owned();
        crate::logger::info("music", format!("backend action={action}"));
        if self.backend.as_mut().is_some_and(|b| b.exited()) {
            crate::logger::warn("music", "previous backend exited; restarting");
            self.backend.take();
        }
        if self.backend.is_none() {
            match backend::Backend::start_with_notify(&self.root, self.hwnd, MUSIC_EVENT) {
                Ok(mut b) => {
                    self.error.clear();
                    if let Err(e) = b.send(serde_json::json!({
                        "action": "setBrowser",
                        "browser": self.config.browser
                    })) {
                        crate::logger::warn("music", format!("failed to send initial browser preference: {e}"));
                    }
                    self.backend = Some(b);
                }
                Err(e) => {
                    crate::logger::error("music", format!("backend start failed: {e}"));
                    self.error = e;
                    return;
                }
            }
        }
        if let Err(e) = self.backend.as_mut().unwrap().send(value) {
            crate::logger::error("music", format!("backend send failed ({action}): {e}"));
            self.error = e;
        }
    }
    unsafe fn activate(&mut self, action: Action, x: f32) {
        match action {
            Action::Close => {
                let was_visible = IsWindowVisible(self.hwnd) != 0;
                if !crate::multi_monitor::is_duplicate(self.hwnd) {
                    self.config.open = false;
                    self.save_local();
                }
                ShowWindow(self.hwnd, SW_HIDE);
                if let Some(s) = &mut self.shadow {
                    s.sync(self.hwnd, self.dpi, 0);
                }
                self.frame = None;
                self.background = None;
                self.clear_spin_base();
                self.overlay = None;
                self.shadow = None;
                self.images.clear();
                self.local_glass = None;
                self.card_cache.clear();
                self.rounded_art.borrow_mut().clear();
                self.media_icons.clear();
                self.button_lenses.clear();
                self.buttons.clear();
                self.hover = None;
                self.pressed = None;
                KillTimer(self.hwnd, 93);
                KillTimer(self.hwnd, 94);
                self.spin_active = false;
                self.coverflow_anim = None;
                self.anim_clock = None;
                self.frame_queued = false;
                if self.backend.is_some() {
                    // Retain only a slow metadata drain while hidden playback lives.
                    SetTimer(self.hwnd, 91, 1500, None);
                } else {
                    KillTimer(self.hwnd, 91);
                }
                if was_visible && !self.smoke {
                    crate::multi_monitor::request_hide_widget("music");
                }
            }
            Action::Settings => self.open_settings(),
            Action::PopOut => {
                self.config.floating = !self.config.floating;
                self.background = None;
                self.clear_spin_base();
                self.overlay = None;
                self.frost = None;
                self.card_cache.clear();
                self.window_origin = [i32::MIN, i32::MIN];
                self.fusion_revision = 0;
                self.save_floating();
                self.sync_settings();
                PostMessageW(self.hwnd, WM_APP + 82, 0, 0);
            }
            Action::Queue => {
                self.config.queue = !self.config.queue;
                self.save();
                self.sync_settings();
            }
            Action::Browse => self.send(serde_json::json!({"action":"browse"})),
            Action::SignIn => self.send(serde_json::json!({"action":"signin"})),
            Action::Library => self.send(serde_json::json!({"action":"library"})),
            Action::Previous => {
                if self.use_smtc() {
                    if let Some(smtc) = &self.smtc {
                        smtc.send(smtc::SmtcCommand::Previous);
                    }
                } else {
                    self.send(serde_json::json!({"action":"previous"}));
                }
                if IsWindowVisible(self.hwnd) != 0 {
                    SetTimer(self.hwnd, 91, 16, None);
                }
            }
            Action::Next => {
                if self.use_smtc() {
                    if let Some(smtc) = &self.smtc {
                        smtc.send(smtc::SmtcCommand::Next);
                    }
                } else {
                    self.send(serde_json::json!({"action":"next"}));
                }
                if IsWindowVisible(self.hwnd) != 0 {
                    SetTimer(self.hwnd, 91, 16, None);
                }
            }
            Action::Toggle => {
                if self.use_smtc() {
                    if let Some(smtc) = &self.smtc {
                        smtc.send(smtc::SmtcCommand::Toggle);
                    }
                    if IsWindowVisible(self.hwnd) != 0 {
                        SetTimer(self.hwnd, 91, 80, None);
                    }
                } else if !self.playback.available && self.playback.title.is_empty() {
                    self.send(serde_json::json!({"action":"browse"}));
                } else {
                    self.send(serde_json::json!({"action":"toggle"}));
                }
            }
            Action::Track(id) => self.send(serde_json::json!({"action":"select","videoId":id})),
            Action::Seek => self.commit_seek(x),
        }
        InvalidateRect(self.hwnd, null(), 0);
    }
    unsafe fn bounds(&mut self) {
        if crate::multi_monitor::record_duplicate_bounds(self.hwnd, self.dpi)
            || crate::multi_monitor::is_primary_displaced("music")
        {
            return;
        }
        let mut r = zeroed();
        GetWindowRect(self.hwnd, &mut r);
        self.config.position = Some([r.left, r.top]);
        self.config.size = [
            ((r.right - r.left) as f32 / self.dpi).round() as i32,
            ((r.bottom - r.top) as f32 / self.dpi).round() as i32,
        ];
        self.save_local();
        crate::multi_monitor::record_primary_bounds(self.hwnd, "music", self.dpi);
    }
    unsafe fn poll(&mut self) {
        self.last_poll = Instant::now();
        if let Some(b) = &self.backend {
            b.clear_notify_pending();
        }
        if let Some(s) = &self.smtc {
            s.clear_notify_pending();
        }
        let mut dirty = false;
        let mut backend_dirty = false;
        // Drain outside the receiver borrow, then create images on the UI thread.
        let events: Vec<_> = self
            .backend
            .as_ref()
            .map(|b| b.events.try_iter().take(256).collect())
            .unwrap_or_default();
        let more_backend = events.len() == 256;
        for event in events {
            match event["type"].as_str().unwrap_or("") {
                "state" => {
                    if let Ok(state) = serde_json::from_value::<Playback>(event["state"].clone()) {
                        let has_backend_session =
                            state.available || !state.title.is_empty() || !state.queue.is_empty();
                        let seeking = self.pressed == Some(Action::Seek)
                            || self
                                .last_seek
                                .is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(2500));
                        self.backend_live = has_backend_session || (self.backend_live && seeking);
                        if self.backend_live {
                            self.peer_live = false;
                            self.smtc_active = false;
                        }
                        self.error = state.error.clone().unwrap_or_default();
                        self.accept_playback(state);
                        if IsWindowVisible(self.hwnd) != 0 {
                            SetTimer(self.hwnd, 91, 250, None);
                        }
                        if self.coverflow_anim.is_some()
                            && self.config.design == "coverflow"
                            && IsWindowVisible(self.hwnd) != 0
                        {
                            if self.anim_clock.is_none() {
                                self.anim_clock = Some(crate::motion::FrameClock::start());
                            }
                            SetTimer(self.hwnd, 94, crate::motion::frame_interval_ms(), None);
                        }
                    }
                    if let Some(error) = event["error"].as_str() {
                        crate::logger::warn("music", format!("backend state error: {error}"));
                        if is_backend_disconnect_error(error) {
                            self.backend_live = false;
                            self.playback.available = false;
                            self.playback.playing = false;
                        }
                        self.error = error.into();
                    }
                    dirty = true;
                    backend_dirty = true;
                }
                "error" => {
                    let err = event["error"].as_str().unwrap_or("Playback error");
                    crate::logger::error("music", format!("backend error: {err}"));
                    if is_backend_disconnect_error(err) {
                        self.backend_live = false;
                        self.playback.playing = false;
                    }
                    self.error = err.into();
                    dirty = true;
                    backend_dirty = true;
                }
                "art" => {
                    if let (Some(url), Some(file)) = (event["url"].as_str(), event["file"].as_str())
                    {
                        if file.len() == 68
                            && file.ends_with(".img")
                            && file.as_bytes()[..64].iter().all(|c| c.is_ascii_hexdigit())
                        {
                            if self.art_files.len() > 64 {
                                self.art_files.retain(|url, _| {
                                    self.last_cover.as_ref() == Some(url)
                                        || self.playback.art.as_ref() == Some(url)
                                        || self.playback.queue.iter().any(|t| t.thumb == *url)
                                });
                            }
                            self.art_files.insert(url.into(), file.into());
                            dirty = true;
                            backend_dirty = true;
                        }
                    }
                }
                _ => {}
            }
        }
        if self.backend.as_mut().is_some_and(|b| b.exited()) {
            if self.backend_live {
                backend_dirty = true;
                dirty = true;
            }
            self.backend_live = false;
        }
        if backend_dirty {
            self.publish_to_peers();
        }
        let smtc_updates: Vec<_> = self
            .smtc
            .as_ref()
            .map(|s| s.updates.try_iter().take(64).collect())
            .unwrap_or_default();
        let more_smtc = smtc_updates.len() == 64;
        if (more_backend || more_smtc) && !self.hwnd.is_null() {
            PostMessageW(self.hwnd, MUSIC_EVENT, 0, 0);
        }
        let smtc_changed = !smtc_updates.is_empty();
        if let Some(latest) = smtc_updates.into_iter().last() {
            self.latest_smtc = Some(latest);
        }
        if !self.backend_live && !self.peer_live && (smtc_changed || !self.smtc_active) {
            if let Some(smtc_state) = self.latest_smtc.clone().filter(|s| s.has_session && s.available)
            {
                if !self.smtc_active {
                    crate::logger::info(
                        "music",
                        format!("SMTC fallback active: {} - {}", smtc_state.title, smtc_state.artist),
                    );
                }
                self.accept_smtc(&smtc_state);
                if self.use_smtc() && unsafe { IsWindowVisible(self.hwnd) != 0 } {
                    unsafe {
                        SetTimer(self.hwnd, 91, 250, None);
                    }
                }
                dirty = true;
            } else if self.smtc_active {
                crate::logger::info("music", "SMTC fallback session ended");
                self.smtc_active = false;
                self.playback.available = false;
                self.playback.playing = false;
                self.playback.can_next = false;
                self.playback.can_previous = false;
                self.playback.can_seek = false;
                dirty = true;
            }
        }
        if dirty || self.playback.playing {
            if dirty {
                self.sync_spin_timer();
            }
            if IsWindowVisible(self.hwnd) != 0 {
                if dirty {
                    self.restore_art();
                }
                InvalidateRect(self.hwnd, null(), 0);
            } else if dirty {
                self.remember_view();
            }
        }
    }
    fn restore_art(&mut self) {
        let mut urls: Vec<String> = self.playback.art.iter().cloned().collect();
        urls.extend(self.last_cover.iter().cloned());
        let selected = coverflow::selected_index(&self.playback);
        for slot in [-2, -1, 0, 1, 2] {
            if let Some(i) = coverflow::neighbor_index(selected, self.playback.queue.len(), slot) {
                urls.push(self.playback.queue[i].thumb.clone());
            }
        }
        urls.extend(
            self.playback
                .queue
                .iter()
                .skip(self.queue_offset)
                .take(20)
                .map(|t| t.thumb.clone()),
        );
        let mut seen = std::collections::HashSet::new();
        urls.retain(|u| !u.is_empty() && seen.insert(u.clone()));
        self.images.retain(|url, _| urls.contains(url));
        let mut stale_urls = Vec::new();
        for url in urls {
            if self.images.contains_key(&url) {
                continue;
            }
            if let Some(file) = self.art_files.get(&url) {
                let path = self
                    .root
                    .join("config")
                    .join("native-music-player")
                    .join("art")
                    .join(file);
                if let Some(image) = load_art(&path) {
                    self.images.insert(url, image);
                    self.card_cache.clear();
                } else {
                    stale_urls.push(url);
                }
            }
        }
        for url in stale_urls {
            self.art_files.remove(&url);
        }
        if let Some(url) = self
            .playback
            .art
            .as_ref()
            .filter(|url| self.images.contains_key(*url))
        {
            self.last_cover = Some(url.clone());
        }
        self.remember_view();
    }
    fn clear_spin_base(&mut self) {
        self.spin_base = None;
        self.spin_base_key.clear();
        self.spin_disc_rect = None;
    }
    unsafe fn spin_base_key(&self, w: i32, h: i32, width: f32, height: f32) -> Option<String> {
        let eligible = !self.moving
            && !self.resizing
            && height >= 260.
            && self.config.design != "coverflow"
            && ((self.config.design == "orbit" && width >= 340. && width > height * 1.25)
                || (self.config.design == "disc" && width >= 340.));
        if !eligible {
            return None;
        }
        use std::fmt::Write;
        let header_motion = crate::widget_header::visual(self.hwnd);
        let art_url = self.playback.art.as_deref().unwrap_or("");
        let art_id = self.cover_image(art_url).map_or(0, Image::identity);
        let pos_key = if self.pressed == Some(Action::Seek) {
            self.position().to_bits()
        } else {
            ((self.position() * 2.0).floor() as i64) as u64
        };
        let mut key = String::with_capacity(256);
        let _ = write!(
            key,
            "{w}x{h}\x1f{}\x1f{}\x1f{}\x1f{:?}\x1f{}",
            self.dpi.to_bits(),
            self.revision,
            self.fusion_revision,
            self.window_origin,
            self.was_low_power_moving,
        );
        let _ = write!(
            key,
            "\x1f{}\x1f{}\x1f{}\x1f{}\x1f{}\x1f{}\x1f{}\x1f{}\x1f{}\x1f{}\x1f{}\x1f{}\x1f{}\x1f{}",
            self.config.design,
            self.config.floating,
            self.config.frosted_glass,
            self.config.opacity.to_bits(),
            self.config.tint.to_bits(),
            self.config.frost_strength.to_bits(),
            self.config.blur.to_bits(),
            self.config.palette,
            self.config.queue,
            self.light,
            self.ink,
            self.muted,
            crate::glass::liquid_glass(),
            crate::glass::frosted_glass(),
        );
        let _ = write!(
            key,
            "\x1f{}\x1f{}\x1f{}\x1f{}\x1f{}\x1f{}\x1f{}\x1f{}\x1f{}\x1f{}\x1f{}\x1f{}",
            self.playback.signed_in,
            self.playback.title.len(),
            self.playback.title,
            self.playback.artist.len(),
            self.playback.artist,
            art_url,
            self.playback.available,
            self.playback.playing,
            self.playback.can_previous,
            self.playback.can_next,
            self.playback.can_seek,
            pos_key,
        );
        let _ = write!(
            key,
            "\x1f{}\x1f{}\x1f{}\x1f{:?}\x1f{:?}\x1f{}\x1f{}\x1f{}\x1f{}\x1f{}\x1f{art_id}",
            (self.playback.duration * 2.0).floor() as i64,
            self.error.len(),
            self.error,
            self.hover,
            self.pressed,
            (header_motion.alpha * 1000.0).round() as i32,
            (header_motion.hover[0] * 1000.0).round() as i32,
            (header_motion.hover[1] * 1000.0).round() as i32,
            (header_motion.rotation[0] * 100.0).round() as i32,
            (header_motion.rotation[1] * 100.0).round() as i32,
        );
        Some(key)
    }
    unsafe fn paint(&mut self) {
        let frame_started = Instant::now();
        let mut ps = zeroed();
        BeginPaint(self.hwnd, &mut ps);
        let mut r = zeroed();
        GetClientRect(self.hwnd, &mut r);
        let (w, h) = (r.right, r.bottom);
        if w <= 0 || h <= 0 {
            EndPaint(self.hwnd, &ps);
            return;
        }
        let revision = self.glass.borrow().revision();
        if revision != self.revision && !self.moving {
            self.revision = revision;
            self.background = None;
            self.clear_spin_base();
            if !self.moving { self.overlay = None; }
            if let Some(glass) = &mut self.local_glass {
                glass.sync_from(&self.glass.borrow());
            }
        }
        crate::fusion::prepare(self.hwnd, 34.);
        let fusion_revision = crate::fusion::revision(self.hwnd);
        let mut fusion_changed = false;
        if fusion_revision != self.fusion_revision && !self.moving {
            self.fusion_revision = fusion_revision;
            fusion_changed = true;
        }
        let changed = self
            .frame
            .as_ref()
            .is_none_or(|b| !b.matches_geometry(w, h, self.dpi, 34.));
        if changed {
            self.frame = Buffer::new(w, h, self.dpi, 34.);
            self.background = None;
            self.clear_spin_base();
            if !self.moving { self.overlay = None; }
        }
        let Some(mut frame) = self.frame.take() else {
            EndPaint(self.hwnd, &ps);
            return;
        };
        let (width, height) = (w as f32 / self.dpi, h as f32 / self.dpi);
        let wr = crate::fusion::window_rect(self.hwnd);
        let low_moving = crate::motion::low_power() && crate::fusion::is_moving(self.hwnd);
        let move_backdrop_dirty = crate::glass::frosted_glass()
            && ([wr.left, wr.top] != self.window_origin || low_moving != self.was_low_power_moving);
        if self.background.is_none() || fusion_changed || move_backdrop_dirty {
            self.clear_spin_base();
            let reuse_frost = fusion_changed
                && !move_backdrop_dirty
                && !crate::glass::liquid_glass()
                && self.background.is_some()
                && self.frost.is_some()
                && crate::fusion::group_frosts(
                    self.hwnd,
                    (self.config.frost_strength * 2.55).round() as u8,
                )
                .is_none();
            if !self.moving && !reuse_frost {
                self.card_cache.clear();
            }
            if let Some(mut bg) = self
                .background
                .take()
                .filter(|b| b.matches_geometry(w, h, self.dpi, 34.))
                .or_else(|| Buffer::new(w, h, self.dpi, 34.))
            {
                self.window_origin = [wr.left, wr.top];
                self.was_low_power_moving = low_moving;
                let brush = CreateSolidBrush(if crate::accent::light(self.surface_color()) {
                    0xf5f2f0
                } else {
                    0x231b16
                });
                FillRect(bg.dc, &r, brush);
                DeleteObject(brush);
                if !reuse_frost {
                    self.local_glass = None;
                    if crate::glass::frosted_glass() {
                        let strength = (self.config.frost_strength * 2.55).round() as u8;
                        self.glass
                            .borrow()
                            .draw_blended(bg.dc, self.hwnd, w, h, strength);
                    }
                    if !self.moving {
                        let mut pixels = bg.snapshot();
                        for pixel in pixels.chunks_exact_mut(4) {
                            pixel.swap(0, 2);
                            pixel[3] = 255;
                        }
                        self.frost = Image::from_rgba(w as u32, h as u32, pixels);
                    }
                } else if let Some(frost) = &self.frost {
                    Canvas::new(bg.dc, self.dpi).image(frost, [0., 0., width, height]);
                }
                let hwnd = self.hwnd;
                let dpi_w = w as u32;
                let dpi_h = h as u32;
                bg.paint_frosted_chrome_for(hwnd, crate::glass::frosted_glass(), |chrome_dc| {
                    self.chrome_into(chrome_dc, [dpi_w, dpi_h], [width, height]);
                });
                render::flush(bg.dc);
                if !self.moving {
                    let bg_lum = crate::contrast::sample(
                        bg.dc,
                        self.dpi,
                        [20., 20., width - 20., height - 20.],
                    );
                    let effective_lum = if self.config.design == "turntable" && bg_lum >= 0.20 {
                        1.0
                    } else {
                        bg_lum
                    };
                    (self.ink, self.muted) = crate::contrast::inks(
                        effective_lum,
                        (self.config.design != "turntable").then_some(self.ink),
                    );
                    self.light = crate::contrast::luminance(self.ink) < 0.5;
                }
                bg.background_alpha(255);
                self.background = Some(bg);
            }
        }
        let spin_key = self.spin_base_key(w, h, width, height);
        let fast_hit = if let (Some(key), Some(base), Some(_)) =
            (spin_key.as_ref(), self.spin_base.as_ref(), self.spin_disc_rect)
        {
            *key == self.spin_base_key && base.matches_geometry(w, h, self.dpi, 0.)
        } else {
            false
        };
        if fast_hit {
            let base = self.spin_base.as_ref().unwrap();
            let disc_r = self.spin_disc_rect.unwrap();
            frame.copy_from(base.dc);
            {
                let _scope = render::AlphaTargetScope::new(frame.dc, self.dpi);
                self.spinning_disc(frame.dc, disc_r, width, height);
            }
            frame.present_with_alpha(self.hwnd, true);
            if self.shadow.is_none() {
                crate::widget_interaction::sync_shadow(
                    self.hwnd,
                    self.moving,
                    self.resizing,
                    &mut self.shadow,
                    self.dpi,
                    (self.config.opacity * 2.55) as u8,
                );
            }
        } else {
            if let Some(bg) = &self.background {
                frame.copy_from(bg.dc);
            }
            frame.background_alpha_for(self.hwnd, (self.config.opacity * 2.55) as u8);
            if spin_key.is_some() {
                self.skip_spin_disc = true;
                self.spin_disc_rect = None;
            }
            let batch = render::AlphaBatch::new(frame.dc, self.dpi);
            self.content(frame.dc, width, height);
            drop(batch);
            self.skip_spin_disc = false;
            if !self.moving {
                if let Some(bg) = &self.background {
                    for (rect, radius) in &self.control_plates {
                        frame.retain_control_backing(bg, *rect, *radius, 1.);
                    }
                    crate::settings_ui::retain_header_backing(&mut frame, bg, self.hwnd, width);
                    if let Some(r) = view::popout_button_rect(width, self.playback.signed_in) {
                        let motion = crate::widget_header::visual(self.hwnd);
                        let strength = if self.hover == Some(Action::PopOut) {
                            1.0
                        } else if self.config.floating {
                            0.72
                        } else {
                            0.0
                        };
                        if motion.alpha * strength > 0.001 {
                            frame.retain_control_backing(
                                bg,
                                [r[0] + 1., r[1] + 1., r[2] - 1., r[3] - 1.],
                                15.,
                                motion.alpha * strength,
                            );
                        }
                    }
                }
            }
            if let (Some(key), Some(disc_r)) = (spin_key, self.spin_disc_rect) {
                if let Some(base) = self
                    .spin_base
                    .take()
                    .filter(|b| b.matches_geometry(w, h, self.dpi, 0.))
                    .or_else(|| Buffer::plain(w, h, self.dpi))
                {
                    base.copy_from(frame.dc);
                    self.spin_base_key = key;
                    self.spin_base = Some(base);
                } else {
                    self.spin_base_key.clear();
                }
                {
                    let _scope = render::AlphaTargetScope::new(frame.dc, self.dpi);
                    self.spinning_disc(frame.dc, disc_r, width, height);
                }
            } else {
                self.clear_spin_base();
            }
            frame.present_with_alpha(self.hwnd, true);
            crate::widget_interaction::sync_shadow(
                self.hwnd,
                self.moving,
                self.resizing,
                &mut self.shadow,
                self.dpi,
                (self.config.opacity * 2.55) as u8,
            );
        }
        self.frame = Some(frame);
        EndPaint(self.hwnd, &ps);
        if IsWindowVisible(self.hwnd) != 0
            && !self.moving
            && ((self.coverflow_anim.is_some_and(|a| !a.finished())
                && self.config.design == "coverflow")
                || self.spin_active)
        {
            crate::motion::pace_vsync(
                self.hwnd,
                frame_started,
                &mut self.frame_queued,
                crate::motion::VSYNC_FRAME,
            );
        }
    }
}

pub struct Host {
    state: Box<RefCell<State>>,
}
impl Host {
    pub unsafe fn create(
        root: &Path,
        glass: Rc<RefCell<Glass>>,
        inspect: bool,
        smoke: bool,
    ) -> Result<Self, String> {
        let config = Config::load(root)?;
        let instance = GetModuleHandleW(null());
        let class = wide("LumenNativeMusic");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            ..zeroed()
        };
        RegisterClassW(&wc);
        let mut work: RECT = zeroed();
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut work as *mut RECT as _, 0);
        let dpi = GetDpiForSystem() as f32 / 96.;
        let width = (config.size[0] as f32 * dpi) as i32;
        let height = (config.size[1] as f32 * dpi) as i32;
        let p = config.position.unwrap_or([work.left + 80, work.top + 100]);
        let mut rect = RECT {
            left: p[0],
            top: p[1],
            right: p[0] + width,
            bottom: p[1] + height,
        };
        let monitor = MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = zeroed();
        info.cbSize = size_of::<MONITORINFO>() as u32;
        let (mut left, mut top, mut cw, mut ch) = (rect.left, rect.top, width, height);
        if GetMonitorInfoW(monitor, &mut info) != 0 {
            let min_px = model::MIN_SIZE.map(|v| (v as f32 * dpi).round() as i32);
            let clamped = crate::geometry::clamp_rect_to_work(
                [rect.left, rect.top, rect.right, rect.bottom],
                [
                    info.rcWork.left,
                    info.rcWork.top,
                    info.rcWork.right,
                    info.rcWork.bottom,
                ],
                min_px,
            );
            left = clamped[0];
            top = clamped[1];
            cw = clamped[2] - clamped[0];
            ch = clamped[3] - clamped[1];
        }
        let previous = if smoke {
            None
        } else {
            session::Snapshot::load(root)
        };
        let (playback, art_files, last_cover) = previous
            .map(|s| (s.playback, s.art_files.into_iter().collect(), s.last_cover))
            .unwrap_or_default();
        let state = Box::new(RefCell::new(State {
            hwnd: null_mut(),
            root: root.into(),
            config,
            glass,
            backend: None,
            backend_live: false,
            peer_live: false,
            peer_sync_version: 0,
            smtc: if smoke {
                None
            } else {
                Some(smtc::SmtcClient::start(root))
            },
            latest_smtc: None,
            smtc_active: false,
            playback,
            last_cover,
            session_signature: String::new(),
            media_icons: icons::Icons::default(),
            button_lenses: lenses::Lenses::default(),
            received: Instant::now(),
            last_poll: Instant::now(),
            last_seek: None,
            spin_angle: 0.,
            spin_updated: Instant::now(),
            spin_active: false,
            coverflow_anim: None,
            anim_clock: None,
            frame_queued: false,
            images: HashMap::new(),
            card_cache: HashMap::new(),
            rounded_art: RefCell::new(HashMap::new()),
            art_files,
            buttons: Vec::new(),
            seek_track: None,
            control_plates: Vec::new(),
            hover: None,
            pressed: None,
            frost: None,
            background: None,
            frame: None,
            spin_base: None,
            spin_base_key: String::new(),
            spin_disc_rect: None,
            skip_spin_disc: false,
            overlay: None,
            shadow: None,
            dpi,
            content_scale: 1.,
            ink: 0xfdfaf8,
            muted: 0xd7cac1,
            light: false,
            settings: None,
            local_glass: None,
            settings_dirty: false,
            blur_dirty: false,
            moving: false,
            resizing: false,
            queue_offset: 0,
            error: String::new(),
            smoke,
            inspect,
            revision: 0,
            fusion_revision: 0,
            window_origin: [i32::MIN, i32::MIN],
            was_low_power_moving: false,
        }));
        let hwnd = CreateWindowExW(
            crate::gpu::window_ex_style() | if inspect { 0 } else { WS_EX_TOOLWINDOW },
            class.as_ptr(),
            wide("Lumen · YouTube Music").as_ptr(),
            WS_POPUP | WS_THICKFRAME,
            left,
            top,
            cw,
            ch,
            null_mut(),
            null_mut(),
            instance,
            &*state as *const RefCell<State> as _,
        );
        if hwnd.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let (live_dpi, _) = crate::geometry::clamp_window_to_work(hwnd, model::MIN_SIZE, dpi);
        {
            let mut s = state.borrow_mut();
            s.hwnd = hwnd;
            s.dpi = live_dpi;
            if crate::multi_monitor::is_duplicate(hwnd) {
                s.config.floating = false;
            }
        }
        if let Some(smtc) = &state.borrow().smtc {
            smtc.set_notify(hwnd, MUSIC_EVENT);
        }
        if !smoke {
            let has_live_peer = SHARED_PLAYBACK.with(|shared| {
                let shared = shared.borrow();
                shared.live && shared.owner != 0
            });
            if has_live_peer {
                state.borrow_mut().sync_from_peer();
            }
            PostMessageW(hwnd, MUSIC_EVENT, 0, 0);
        }
        state.borrow_mut().restore_art();
        eprintln!(
            "native-music-ready pid={} design={}",
            std::process::id(),
            state.borrow().config.design
        );
        Ok(Self { state })
    }
    pub unsafe fn show(&mut self) {
        let (hwnd, floating, inspect) = {
            let mut s = self.state.borrow_mut();
            if crate::multi_monitor::is_duplicate(s.hwnd) {
                s.config.floating = crate::multi_monitor::duplicate_floating(s.hwnd);
            } else {
                s.config.open = true;
                s.save_local();
            }
            (s.hwnd, s.config.floating, s.inspect)
        };
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetTimer(hwnd, 91, 250, None);
        self.state.borrow_mut().sync_spin_timer();
        crate::window::apply_floating_mode(hwnd, floating, inspect);
        let mut s = self.state.borrow_mut();
        s.poll();
        s.restore_art();
        s.paint();
    }
    pub fn hwnd(&self) -> HWND {
        self.state.borrow().hwnd
    }
    pub fn is_visible(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd()) != 0 }
    }
    pub unsafe fn hide(&mut self) {
        self.state.borrow_mut().activate(Action::Close, 0.);
    }
    pub unsafe fn show_settings(&mut self) {
        self.state.borrow_mut().open_settings();
    }
    pub unsafe fn refresh(&mut self) {
        let mut s = self.state.borrow_mut();
        let shared = s.glass.clone();
        if let Some(glass) = &mut s.local_glass {
            glass.sync_from(&shared.borrow());
        }
        s.background = None;
        s.clear_spin_base();
        s.overlay = None;
        s.frost = None;
        s.window_origin = [i32::MIN, i32::MIN];
        if !s.moving {
            s.card_cache.clear();
        }
        if IsWindowVisible(s.hwnd) != 0 {
            InvalidateRect(s.hwnd, null(), 0);
        }
    }
    pub unsafe fn apply_common(&mut self, opacity: Option<f32>, floating: Option<bool>) {
        let mut s = self.state.borrow_mut();
        if let Some(opacity) = opacity.filter(|v| v.is_finite()) {
            s.config.opacity = opacity.clamp(0., 100.);
            s.background = None;
            s.clear_spin_base();
        }
        if let Some(floating) = floating {
            s.config.floating = floating;
        }
        if opacity.is_some() {
            s.save();
        }
        if floating.is_some() {
            s.save_floating();
        }
        s.sync_settings();
        let (hwnd, inspect) = (s.hwnd, s.inspect);
        drop(s);
        if let Some(floating) = floating {
            crate::window::apply_floating_mode(hwnd, floating, inspect);
        }
        if IsWindowVisible(hwnd) != 0 {
            InvalidateRect(hwnd, null(), 0);
        }
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        unsafe {
            let mut s = self.state.borrow_mut();
            s.flush_settings();
            s.remember_view();
            s.settings.take();
            if let Some(backend) = &s.backend {
                backend.set_notify(null_mut(), 0);
            }
            if let Some(smtc) = &s.smtc {
                smtc.set_notify(null_mut(), 0);
            }
            s.backend.take();
            s.shadow.take();
            if !s.smoke {
                let was_owner = SHARED_PLAYBACK.with(|shared| {
                    let mut shared = shared.borrow_mut();
                    if shared.owner == s.hwnd as usize {
                        shared.owner = 0;
                        shared.live = false;
                        shared.pending_cmds.clear();
                        shared.version = shared.version.wrapping_add(1);
                        true
                    } else {
                        false
                    }
                });
                if was_owner {
                    crate::multi_monitor::notify_peers_with_message(
                        "music",
                        s.hwnd,
                        PEER_PLAYBACK_MSG,
                    );
                }
            }
            let hwnd = s.hwnd;
            // No callback may retain a State pointer while the owner is dropped.
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(s);
            DestroyWindow(hwnd);
        }
    }
}
pub fn should_open(root: &Path) -> bool {
    Config::load(root).map(|c| c.open).unwrap_or(false)
}

fn normalize_track_text(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn is_backend_disconnect_error(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("disconnected")
        || lower.contains("background player closed")
        || lower.contains("sign-in browser closed")
        || lower.contains("music widget closed")
        || lower.contains("music adapter is closed")
        || lower.contains("browser exited")
}

fn load_art(path: &Path) -> Option<Image> {
    let reader = match image::ImageReader::open(path) {
        Ok(r) => r,
        Err(err) => {
            crate::logger::warn(
                "music",
                format!("artwork file unavailable ({}): {err}", path.display()),
            );
            return None;
        }
    };
    let mut reader = match reader.with_guessed_format() {
        Ok(r) => r,
        Err(err) => {
            crate::logger::warn(
                "music",
                format!("artwork format probe failed ({}): {err}", path.display()),
            );
            return None;
        }
    };
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let decoded = match reader.decode() {
        Ok(img) => img,
        Err(err) => {
            crate::logger::warn(
                "music",
                format!("artwork decode failed ({}): {err}", path.display()),
            );
            return None;
        }
    };
    let rgba = decoded.thumbnail(400, 400).into_rgba8();
    let (w, h) = rgba.dimensions();
    let mut pixels = rgba.into_raw();
    for p in pixels.chunks_exact_mut(4) {
        for c in 0..3 {
            p[c] = ((p[c] as u32 * p[3] as u32 + 127) / 255) as u8;
        }
    }
    Image::from_rgba(w, h, pixels)
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // UpdateLayeredWindow can synchronously recalculate the client area while
    // paint owns State. This must stay borderless even on the reentrant path:
    // default THICKFRAME metrics would make each uploaded client bitmap shrink
    // the outer HWND again on the next paint.
    if msg == WM_NCCALCSIZE {
        return 0;
    }
    if msg == WM_ERASEBKGND {
        return 1;
    }
    if msg == WM_NCCREATE {
        let cs = &*(lp as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<State>;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    if msg == WM_GETMINMAXINFO {
        // Sizing constraints also remain available during synchronous native
        // callbacks from paint/show. Prefer the updated DPI in State when free.
        let dpi = (*ptr)
            .try_borrow()
            .map(|s| s.dpi)
            .unwrap_or_else(|_| (GetDpiForWindow(hwnd) as f32 / 96.).max(1.));
        let info = &mut *(lp as *mut MINMAXINFO);
        info.ptMinTrackSize = POINT {
            x: (model::MIN_SIZE[0] as f32 * dpi).ceil() as i32,
            y: (model::MIN_SIZE[1] as f32 * dpi).ceil() as i32,
        };
        info.ptMaxTrackSize = POINT {
            x: (1100. * dpi).round() as i32,
            y: (1000. * dpi).round() as i32,
        };
        return 0;
    }
    let Ok(mut s) = (*ptr).try_borrow_mut() else {
        return DefWindowProcW(hwnd, msg, wp, lp);
    };
    let point = |lp: LPARAM, dpi: f32| [(lp as i16) as f32 / dpi, ((lp >> 16) as i16) as f32 / dpi];
    match msg {
        WM_PAINT => {
            if IsWindowVisible(hwnd) != 0 {
                s.paint();
            } else {
                let mut ps = zeroed();
                BeginPaint(hwnd, &mut ps);
                EndPaint(hwnd, &ps);
            }
            0
        }
        WM_TIMER if wp == settings::FLUSH_TIMER => {
            s.flush_settings();
            0
        }
        WM_TIMER if wp == 93 => {
            let elapsed = s.spin_updated.elapsed().as_secs_f32().min(0.1);
            s.spin_updated = Instant::now();
            if s.playback.playing
                && !s.moving
                && IsWindowVisible(hwnd) != 0
                && matches!(s.config.design.as_str(), "orbit" | "disc" | "turntable")
            {
                s.spin_angle = responsive::advance_spin(s.spin_angle, elapsed);
                if !s.frame_queued {
                    InvalidateRect(hwnd, null(), 0);
                }
            }
            0
        }
        WM_TIMER if wp == 94 => {
            if s.coverflow_anim.is_some_and(|a| !a.finished())
                && s.config.design == "coverflow"
                && IsWindowVisible(hwnd) != 0
            {
                if !s.frame_queued {
                    InvalidateRect(hwnd, null(), 0);
                }
            } else {
                s.coverflow_anim = None;
                KillTimer(hwnd, 94);
                if !s.spin_active {
                    s.anim_clock = None;
                }
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        m if m == crate::motion::VSYNC_FRAME => {
            s.frame_queued = false;
            if s.last_poll.elapsed() >= std::time::Duration::from_millis(200) {
                s.poll();
            }
            let mut dirty = false;
            if s.playback.playing
                && !s.moving
                && IsWindowVisible(hwnd) != 0
                && matches!(s.config.design.as_str(), "orbit" | "disc" | "turntable")
            {
                let elapsed = s.spin_updated.elapsed().as_secs_f32().min(0.1);
                s.spin_updated = Instant::now();
                s.spin_angle = responsive::advance_spin(s.spin_angle, elapsed);
                dirty = true;
            }
            if s.coverflow_anim.is_some_and(|a| !a.finished())
                && s.config.design == "coverflow"
                && IsWindowVisible(hwnd) != 0
            {
                dirty = true;
            }
            if dirty {
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        m if m == settings::NOTIFY => {
            s.settings_changed();
            0
        }
        m if m == crate::multi_monitor::WIDGET_SYNC_MESSAGE => {
            s.reload_from_shared();
            0
        }
        m if m == PEER_PLAYBACK_MSG => {
            s.sync_from_peer();
            0
        }
        m if m == PEER_COMMAND_MSG => {
            let cmds: Vec<_> = SHARED_PLAYBACK.with(|shared| {
                let mut shared = shared.borrow_mut();
                if shared.owner == hwnd as usize {
                    shared.pending_cmds.drain(..).collect()
                } else {
                    Vec::new()
                }
            });
            for cmd in cmds {
                s.send(cmd);
            }
            0
        }
        m if m == MUSIC_EVENT => {
            s.poll();
            s.sync_spin_timer();
            0
        }
        WM_TIMER => {
            s.poll();
            s.sync_spin_timer();
            0
        }
        m if m == WM_APP + 83 => {
            if let Some(glass) = &mut s.local_glass {
                if glass.finish_async() {
                    s.background = None;
                    s.clear_spin_base();
                    s.overlay = None;
                    s.frost = None;
                    InvalidateRect(hwnd, null(), 0);
                }
            }
            0
        }
        m if m == WM_APP + 81 => {
            if s.backend.is_some() {
                s.send(serde_json::json!({"action":"layout"}));
            }
            0
        }
        m if m == WM_APP + 82 => {
            s.background = None;
            s.clear_spin_base();
            s.overlay = None;
            s.frost = None;
            s.card_cache.clear();
            s.window_origin = [i32::MIN, i32::MIN];
            s.fusion_revision = 0;
            let (floating, inspect) = (s.config.floating, s.inspect);
            drop(s);
            crate::window::apply_floating_mode(hwnd, floating, inspect);
            if let Ok(mut s) = (*ptr).try_borrow_mut() {
                let dpi = s.dpi;
                let opacity = (s.config.opacity * 2.55) as u8;
                crate::widget_interaction::sync_shadow(
                    hwnd,
                    s.moving,
                    s.resizing,
                    &mut s.shadow,
                    dpi,
                    opacity,
                );
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_NCHITTEST => {
            let mut p = POINT {
                x: (lp as i16) as i32,
                y: ((lp >> 16) as i16) as i32,
            };
            ScreenToClient(hwnd, &mut p);
            let mut r = zeroed();
            GetClientRect(hwnd, &mut r);
            if crate::compositor::distance(
                p.x as f32 / s.dpi,
                p.y as f32 / s.dpi,
                r.right as f32 / s.dpi,
                r.bottom as f32 / s.dpi,
                34.,
                4.,
            ) > 0.
            {
                return HTTRANSPARENT as isize;
            }
            let edge = crate::geometry::widget_edge(
                p.x as f32 / s.dpi,
                p.y as f32 / s.dpi,
                r.right as f32 / s.dpi,
                r.bottom as f32 / s.dpi,
            );
            if edge != HTCLIENT {
                edge as isize
            } else if s
                .buttons
                .iter()
                .any(|b| view::contains(b.rect, [p.x as f32 / s.dpi, p.y as f32 / s.dpi]))
            {
                HTCLIENT as isize
            } else {
                HTCAPTION as isize
            }
        }
        WM_MOUSEMOVE => {
            let p = point(lp, s.dpi);
            if s.pressed == Some(Action::Seek) {
                s.hover = Some(Action::Seek);
                s.scrub_to(p[0]);
                InvalidateRect(hwnd, null(), 0);
            } else {
                let hover = s
                    .buttons
                    .iter()
                    .rev()
                    .find(|b| view::contains(b.rect, p))
                    .map(|b| b.action.clone());
                if s.hover != hover {
                    s.hover = hover;
                    InvalidateRect(hwnd, null(), 0);
                }
            }
            let mut track = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            TrackMouseEvent(&mut track);
            0
        }
        WM_MOUSELEAVE => {
            if s.pressed != Some(Action::Seek) {
                s.hover = None;
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_LBUTTONDOWN => {
            let p = point(lp, s.dpi);
            s.pressed = s
                .buttons
                .iter()
                .rev()
                .find(|b| view::contains(b.rect, p))
                .map(|b| b.action.clone());
            if s.pressed == Some(Action::PopOut)
                && crate::widget_header::visual(hwnd).alpha < 0.05
            {
                s.pressed = None;
                return 0;
            }
            if s.pressed == Some(Action::Seek) {
                s.scrub_to(p[0]);
            }
            SetCapture(hwnd);
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_LBUTTONUP => {
            let p = point(lp, s.dpi);
            let action = s
                .buttons
                .iter()
                .rev()
                .find(|b| view::contains(b.rect, p))
                .map(|b| b.action.clone());
            let pressed = s.pressed.take();
            ReleaseCapture();
            if pressed == Some(Action::Seek) {
                s.commit_seek(p[0]);
            } else if pressed.is_some() && pressed == action {
                s.activate(action.unwrap(), p[0]);
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_CAPTURECHANGED | WM_CANCELMODE => {
            s.pressed = None;
            if s.moving || s.resizing {
                let st = &mut *s;
                crate::widget_interaction::exit_size_move(hwnd, &mut st.moving, &mut st.resizing);
            }
            if msg == WM_CANCELMODE {
                ReleaseCapture();
            }
            InvalidateRect(hwnd, null(), 0);
            // DefWindowProc must see cancellation of its native move/size loop.
            drop(s);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wp >> 16) as i16) as i32;
            if s.config.queue {
                s.queue_offset = if delta > 0 {
                    s.queue_offset.saturating_sub(1)
                } else {
                    s.queue_offset + 1
                };
                s.restore_art();
                InvalidateRect(hwnd, null(), 0);
            } else if s.config.design == "coverflow" && s.playback.available {
                if delta > 0 && s.playback.can_previous {
                    s.activate(Action::Previous, 0.);
                } else if delta < 0 && s.playback.can_next {
                    s.activate(Action::Next, 0.);
                }
            }
            0
        }
        WM_KEYDOWN if wp == 0x1b => {
            s.pressed = None;
            ReleaseCapture();
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_KEYDOWN if wp == 0x20 => {
            s.activate(Action::Toggle, 0.);
            0
        }
        WM_ENTERSIZEMOVE => {
            crate::widget_interaction::enter_size_move(&mut s.moving);
            0
        }
        WM_SIZING => {
            {
                let s = &mut *s;
                crate::widget_interaction::start_resize(hwnd, &mut s.resizing, s.shadow.as_ref());
            }
            drop(s);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_EXITSIZEMOVE => {
            let s = &mut *s;
            crate::widget_interaction::exit_size_move(hwnd, &mut s.moving, &mut s.resizing);
            s.window_origin = [i32::MIN, i32::MIN];
            s.frost = None;
            s.clear_spin_base();
            s.card_cache.clear();
            s.bounds();
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_SIZE => {
            let width = (lp as u32 & 0xffff) as i32;
            let height = ((lp as u32 >> 16) & 0xffff) as i32;
            if width > 0
                && height > 0
                && s.frame
                    .as_ref()
                    .is_none_or(|b| !b.matches_geometry(width, height, s.dpi, 34.))
            {
                let had_frame = s.frame.is_some();
                s.frame = None;
                s.background = None;
                s.clear_spin_base();
                if !s.moving { s.overlay = None; }
                if s.moving {
                    let s = &mut *s;
                    crate::widget_interaction::start_resize(
                        hwnd,
                        &mut s.resizing,
                        s.shadow.as_ref(),
                    );
                }
                if crate::widget_interaction::should_paint_resize(hwnd, s.moving, had_frame) {
                    InvalidateRect(hwnd, null(), 0);
                }
            }
            0
        }
        windows_sys::Win32::UI::WindowsAndMessaging::WM_MOVE => {
            if crate::widget_interaction::should_paint_move(hwnd, s.resizing) {
                s.window_origin = [i32::MIN, i32::MIN];
                s.frost = None;
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_WINDOWPOSCHANGING | WM_WINDOWPOSCHANGED => {
            let dpi = s.dpi;
            crate::widget_interaction::on_window_pos_changed(
                &*(lp as *const WINDOWPOS),
                s.resizing,
                &mut s.shadow,
                dpi,
            );
            // Default processing synchronously sends WM_SIZE/WM_MOVE.
            drop(s);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_DPICHANGED => {
            let new_dpi = ((wp as u16) as f32 / 96.).max(1.);
            s.dpi = new_dpi;
            s.frame = None;
            s.background = None;
            s.clear_spin_base();
            s.overlay = None;
            s.local_glass = None;
            let r = crate::geometry::clamp_dpi_rect(*(lp as *const RECT), model::MIN_SIZE, new_dpi);
            // SetWindowPos synchronously reenters our size/min/max handlers.
            drop(s);
            SetWindowPos(
                hwnd,
                null_mut(),
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_SETTINGCHANGE | WM_DISPLAYCHANGE => {
            let shared = s.glass.clone();
            if let Some(glass) = &mut s.local_glass {
                glass.sync_from(&shared.borrow());
            }
            if !s.moving && msg == WM_DISPLAYCHANGE {
                s.frame = None;
                s.background = None;
                s.clear_spin_base();
                s.overlay = None;
            }
            let prev_dpi = s.dpi;
            let moving = s.moving;
            let live_dpi = if !moving {
                crate::geometry::window_dpi(hwnd)
            } else {
                prev_dpi
            };
            let pre_dpi_changed = !moving && (prev_dpi - live_dpi).abs() > 0.005;
            if pre_dpi_changed {
                s.dpi = live_dpi;
            }
            drop(s);
            if !moving {
                let (dpi, clamped) =
                    crate::geometry::clamp_window_to_work(hwnd, model::MIN_SIZE, prev_dpi);
                if let Ok(mut s) = (*ptr).try_borrow_mut() {
                    if pre_dpi_changed || (s.dpi - dpi).abs() > 0.005 || clamped {
                        s.dpi = dpi;
                        s.frame = None;
                        s.background = None;
                        s.clear_spin_base();
                        s.overlay = None;
                        s.frost = None;
                        s.local_glass = None;
                        s.card_cache.clear();
                    }
                    if let Some(host) = &mut s.settings {
                        host.refit();
                    }
                }
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_CLOSE => {
            s.activate(Action::Close, 0.);
            0
        }
        WM_NCDESTROY => {
            if let Some(backend) = &s.backend {
                backend.set_notify(null_mut(), 0);
            }
            if let Some(smtc) = &s.smtc {
                smtc.set_notify(null_mut(), 0);
            }
            crate::gpu::remove_surface(hwnd);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(s);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        _ => {
            // WM_NCLBUTTONDOWN/WM_SYSCOMMAND can run a nested native move/size
            // loop until release. Never keep a State borrow across that loop.
            drop(s);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coverflow_keeps_previous_art_across_restart_loading_failure_and_hide() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            let _runtime = render::Runtime::start().unwrap();
            let root = test_root("coverflow-restore");
            let folder = root.join("config/native-music-player/art");
            std::fs::create_dir_all(&folder).unwrap();
            let previous = "https://i.ytimg.com/previous.jpg";
            let incoming = "https://i.ytimg.com/incoming.jpg";
            let old_file = format!("{}.img", "b".repeat(64));
            let new_file = format!("{}.img", "c".repeat(64));
            image::RgbaImage::from_pixel(32, 32, image::Rgba([180, 90, 60, 255]))
                .save_with_format(folder.join(&old_file), image::ImageFormat::Png)
                .unwrap();
            let host = Host::create(
                &root,
                Rc::new(RefCell::new(Glass::new(13., 1.))),
                true,
                false,
            )
            .unwrap();
            {
                let mut s = host.state.borrow_mut();
                s.playback = Playback {
                    title: "Previous track".into(),
                    artist: "Previous artist".into(),
                    art: Some(previous.into()),
                    available: true,
                    ..Playback::default()
                };
                s.art_files.insert(previous.into(), old_file);
                s.restore_art();
                assert!(s.cover_image(previous).is_some());
            }
            drop(host);
            let host = Host::create(
                &root,
                Rc::new(RefCell::new(Glass::new(13., 1.))),
                true,
                false,
            )
            .unwrap();
            {
                let mut s = host.state.borrow_mut();
                assert_eq!(s.playback.title, "Previous track");
                assert!(!s.playback.available && !s.playback.playing);
                let old = s.cover_image(previous).unwrap().identity();
                s.accept_playback(Playback::default());
                assert_eq!(s.playback.title, "Previous track");
                assert_eq!(s.cover_image("").unwrap().identity(), old);
                s.accept_playback(Playback {
                    title: "Incoming track".into(),
                    art: Some(incoming.into()),
                    available: true,
                    ..Playback::default()
                });
                s.restore_art();
                assert_eq!(
                    s.cover_image(incoming).unwrap().identity(),
                    old,
                    "keep previous cover while a replacement is missing or fails"
                );
                image::RgbaImage::from_pixel(32, 32, image::Rgba([60, 110, 190, 255]))
                    .save_with_format(folder.join(&new_file), image::ImageFormat::Png)
                    .unwrap();
                s.art_files.insert(incoming.into(), new_file);
                s.restore_art();
                assert_eq!(s.last_cover.as_deref(), Some(incoming));
                assert_ne!(s.cover_image(incoming).unwrap().identity(), old);
                s.activate(Action::Close, 0.);
                assert!(s.images.is_empty());
                s.restore_art();
                assert!(s.cover_image(incoming).is_some());
            }
            drop(host);
            std::fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    #[ignore = "Exports native Music themes from a hidden, test-owned window"]
    fn capture_music_themes() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            let folder = PathBuf::from(
                std::env::var_os("LUMEN_MUSIC_CAPTURE_DIR").expect("Set LUMEN_MUSIC_CAPTURE_DIR"),
            );
            std::fs::create_dir_all(&folder).unwrap();
            let _runtime = render::Runtime::start().unwrap();
            let glass = Rc::new(RefCell::new(Glass::new(13., 1.)));
            glass.borrow_mut().refresh();
            let host = Host::create(&test_root("theme-capture"), glass, true, true).unwrap();
            let hwnd = host.hwnd();
            let dpi = host.state.borrow().dpi;
            SetWindowPos(
                hwnd,
                null_mut(),
                100,
                100,
                (560. * dpi).round() as i32,
                (480. * dpi).round() as i32,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            {
                let mut s = host.state.borrow_mut();
                // Clearly marked preview data: no browser, account or live media
                // session is contacted or modified for these theme captures.
                s.playback = Playback {
                    video_id: "preview0002".into(),
                    title: "Music preview".into(),
                    artist: "Native theme capture".into(),
                    art: Some("capture-2".into()),
                    available: true,
                    can_seek: true,
                    can_next: true,
                    can_previous: true,
                    position: 84.,
                    duration: 225.,
                    queue: (0..5)
                        .map(|i| model::Track {
                            video_id: format!("preview{i:04}"),
                            title: format!("Preview track {}", i + 1),
                            artist: "Theme preview".into(),
                            thumb: format!("capture-{i}"),
                            selected: i == 2,
                        })
                        .collect(),
                    ..Playback::default()
                };
                for i in 0..5 {
                    let pixels: Vec<u8> = (0..256 * 256)
                        .flat_map(|n| {
                            [
                                40 + (n % 256 / 2) as u8,
                                45 + (n / 256 / 2) as u8,
                                70 + i as u8 * 30,
                                255,
                            ]
                        })
                        .collect();
                    s.images.insert(
                        format!("capture-{i}"),
                        Image::from_rgba(256, 256, pixels).unwrap(),
                    );
                }
                if std::env::var_os("LUMEN_MUSIC_CAPTURE_LAST_SESSION").is_some() {
                    let real_root = crate::model::root();
                    if let Some(previous) = session::Snapshot::load(&real_root) {
                        s.root = real_root;
                        s.playback = previous.playback;
                        s.art_files = previous.art_files.into_iter().collect();
                        s.last_cover = previous.last_cover;
                        s.restore_art();
                        if s.playback.title.is_empty() {
                            s.playback.title = "Last session".into();
                            s.playback.artist = "Cached artwork".into();
                        }
                    }
                }
                for design in model::DESIGNS {
                    let size = match design {
                        "compact" => [360., 180.],
                        "orbit" => [400., 480.],
                        "disc" => [620., 260.],
                        _ => [560., 480.],
                    };
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        100,
                        100,
                        (size[0] * dpi).round() as i32,
                        (size[1] * dpi).round() as i32,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                    s.config.design = design.into();
                    s.config.queue = false;
                    s.background = None;
                    s.overlay = None;
                    s.card_cache.clear();
                    s.paint();
                    let frame = s.frame.as_ref().unwrap();
                    let mut rgba = frame.snapshot();
                    for p in rgba.chunks_exact_mut(4) {
                        p.swap(0, 2);
                        let a = p[3] as u32;
                        if a > 0 {
                            for value in &mut p[..3] {
                                *value = ((*value as u32 * 255 + a / 2) / a).min(255) as u8;
                            }
                        }
                    }
                    image::RgbaImage::from_raw(frame.width as u32, frame.height as u32, rgba)
                        .unwrap()
                        .save(folder.join(format!("music-{design}.png")))
                        .unwrap();
                }
            }
            assert_eq!(IsWindowVisible(hwnd), 0);
        }
    }
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    struct ReentryProbe {
        state: *const RefCell<State>,
        blocked: std::cell::Cell<bool>,
        sizes: std::cell::Cell<u32>,
    }
    unsafe extern "system" fn probe_proc(
        hwnd: HWND,
        msg: u32,
        wp: WPARAM,
        lp: LPARAM,
        _: usize,
        data: usize,
    ) -> LRESULT {
        let probe = &*(data as *const ReentryProbe);
        if matches!(msg, WM_WINDOWPOSCHANGING | WM_SIZE | WM_GETMINMAXINFO) {
            if (*probe.state).try_borrow_mut().is_err() {
                probe.blocked.set(true);
            }
            if msg == WM_SIZE {
                probe.sizes.set(probe.sizes.get() + 1);
            }
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
    fn test_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "lumen-music-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    #[test]
    fn music_chrome_matches_dock_coating_for_light_dark_and_tint_endpoints() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            let _runtime = render::Runtime::start().unwrap();
            let host = Host::create(
                &test_root("dock-material"),
                Rc::new(RefCell::new(Glass::new(13., 1.))),
                true,
                true,
            )
            .unwrap();
            let mut s = host.state.borrow_mut();
            s.dpi = 1.;
            for palette in [
                "ocean",
                "porcelain",
                "gradient:afterglow",
                "gradient:dawn",
                "gradient:dusk",
                "gradient:dust",
                "custom:#12ABEF",
            ] {
                for tint in [0., 24., 100.] {
                    s.config.palette = palette.into();
                    s.config.tint = tint;
                    s.overlay = None;
                    let actual = Buffer::plain(320, 240, 1.).unwrap();
                    let expected = Buffer::plain(320, 240, 1.).unwrap();
                    let rect = RECT {
                        left: 0,
                        top: 0,
                        right: 320,
                        bottom: 240,
                    };
                    let brush = CreateSolidBrush(0xb49b84);
                    FillRect(actual.dc, &rect, brush);
                    FillRect(expected.dc, &rect, brush);
                    DeleteObject(brush);
                    s.chrome_into(actual.dc, [320, 240], [320., 240.]);
                    // Reference the Dock's coating sequence over identical backing.
                    // This guards the former duplicate reflection / 24-vs-.24 bug.
                    let color = s.surface_color();
                    let light = crate::accent::light(color);
                    let pane = [4., 4., 316., 236.];
                    let c = Canvas::new(expected.dc, 1.);
                    if !crate::surface::managed(&s.config.palette) {
                        c.round(pane, 34., color, (tint * 2.55).round() as u8, false, false);
                    }
                    crate::surface::paint(&c, pane, &s.config.palette, tint / 100.);
                    c.coating(pane, 34.);
                    c.draw_pane_overlay([320, 240], 1., tint / 100., light, [0., 0., 320., 240.]);
                    c.widget_corner_reflection(pane, 34., light);
                    drop(c);
                    assert_eq!(
                        actual.snapshot(),
                        expected.snapshot(),
                        "Dock material differs for {palette}, tint {tint}"
                    );
                }
            }
        }
    }
    #[test]
    fn detached_settings_reuse_hwnd_preserve_music_and_isolate_material_changes() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            let _runtime = render::Runtime::start().unwrap();
            let root = test_root("detached-settings");
            let shared = Rc::new(RefCell::new(Glass::new(13., 1.)));
            let host = Host::create(&root, shared.clone(), true, true).unwrap();
            let hit_dpi = host.state.borrow().dpi;
            crate::geometry::assert_widget_corner_hits(host.hwnd(), hit_dpi);
            let hwnd = host.hwnd();
            SetWindowPos(
                hwnd,
                null_mut(),
                40,
                50,
                240,
                180,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let mut before: RECT = zeroed();
            GetWindowRect(hwnd, &mut before);
            let settings_hwnd;
            {
                let mut s = host.state.borrow_mut();
                s.playback.title = "Paused fixture track".into();
                let before_config = serde_json::to_value(&s.config).unwrap();
                assert!(s.ensure_settings());
                settings_hwnd = s.settings.as_ref().unwrap().hwnd();
                assert!(s.ensure_settings());
                assert_eq!(settings_hwnd, s.settings.as_ref().unwrap().hwnd());
                assert_eq!(before_config, serde_json::to_value(&s.config).unwrap());
                let rows = s.settings_rows();
                for required in [102, 103, 105] {
                    assert!(
                        rows.iter().any(|r| matches!(r,
                            crate::settings_ui::Row::Slider { id, .. } if *id == required
                        )),
                        "appearance control {required} must remain a slider"
                    );
                }
                assert_eq!(rows.iter().filter_map(|r|match r {
                    crate::settings_ui::Row::Button{id,..}=>Some(*id),_=>None
                }).collect::<Vec<_>>(),vec![204],"only the explicit sign-in action is a button; appearance controls remain sliders");
                let revision = shared.borrow().revision();
                for (id, value) in [(102, 17.), (103, 60.), (104, 27.), (105, 0.)] {
                    assert!(s.apply_setting(crate::settings_ui::Change {
                        id,
                        value: crate::settings_ui::Value::Number(value)
                    }));
                }
                assert!(!s.apply_setting(crate::settings_ui::Change {
                    id: 104,
                    value: crate::settings_ui::Value::Number(f32::NAN)
                }));
                assert!(!s.apply_setting(crate::settings_ui::Change {
                    id: 201,
                    value: crate::settings_ui::Value::Choice("unknown".into())
                }));
                for palette in ["gradient:afterglow", "custom:#12ABEF"] {
                    assert!(s.apply_setting(crate::settings_ui::Change {
                        id: 106,
                        value: crate::settings_ui::Value::Choice(palette.into())
                    }));
                    assert_eq!(
                        s.config.palette,
                        crate::surface::normalize(palette).unwrap()
                    );
                    assert!(s.background.is_none());
                    assert!(s.overlay.is_none());
                }
                for palette in ["custom", "custom:#GG0000"] {
                    assert!(!s.apply_setting(crate::settings_ui::Change {
                        id: 106,
                        value: crate::settings_ui::Value::Choice(palette.into())
                    }));
                }
                s.flush_settings();
                s.paint();
                assert_eq!(
                    shared.borrow().revision(),
                    revision,
                    "Music cannot mutate the shared blur source"
                );
                assert!(s.local_glass.is_none());
                assert_eq!(s.playback.title, "Paused fixture track");
                assert!(s.buttons.iter().any(|b| b.action == Action::Toggle));
                assert_eq!(s.config.blur, 27.);
                assert_eq!(s.config.tint, 17.);
            }
            let mut name = [0u16; 80];
            let n = GetClassNameW(settings_hwnd, name.as_mut_ptr(), 80);
            assert_eq!(
                String::from_utf16_lossy(&name[..n as usize]),
                "LumenNativeWidgetSettings"
            );
            SendMessageW(settings_hwnd, WM_KEYDOWN, 0x1b, 0);
            SendMessageW(settings_hwnd, WM_CLOSE, 0, 0);
            assert_eq!(IsWindowVisible(settings_hwnd), 0);
            let mut after: RECT = zeroed();
            GetWindowRect(hwnd, &mut after);
            assert_eq!(
                [before.left, before.top, before.right, before.bottom],
                [after.left, after.top, after.right, after.bottom]
            );
            assert_eq!(IsWindowVisible(hwnd), 0, "test never shows a native window");
            assert!(host.state.borrow().backend.is_none());
            drop(host);
        }
    }
    #[test]
    fn native_resize_and_dpi_callbacks_can_reenter_and_persist_final_bounds() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            let root = test_root("resize");
            let host = Host::create(
                &root,
                Rc::new(RefCell::new(Glass::new(13., 1.))),
                true,
                false,
            )
            .unwrap();
            let hwnd = host.hwnd();
            let probe = Box::new(ReentryProbe {
                state: &*host.state,
                blocked: false.into(),
                sizes: 0.into(),
            });
            assert_ne!(
                SetWindowSubclass(
                    hwnd,
                    Some(probe_proc),
                    0x4d555349,
                    &*probe as *const ReentryProbe as usize
                ),
                0
            );
            SendMessageW(hwnd, WM_ENTERSIZEMOVE, 0, 0);
            assert!(host.state.borrow().moving);
            host.state.borrow_mut().frame = Buffer::plain(100, 100, 1.);
            assert!(host.state.borrow().frame.is_some());
            SetWindowPos(
                hwnd,
                null_mut(),
                30,
                40,
                620,
                500,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            assert!(
                probe.sizes.get() > 0,
                "DefWindowProc must deliver synchronous WM_SIZE"
            );
            assert!(
                !probe.blocked.get(),
                "default processing must release State before nested callbacks"
            );
            assert!(host.state.borrow().frame.is_none(),
                "live resize must discard the old-sized frame before mouse release");
            let suggested = RECT {
                left: 60,
                top: 70,
                right: 780,
                bottom: 610,
            };
            SendMessageW(
                hwnd,
                WM_DPICHANGED,
                144 | (144 << 16),
                &suggested as *const RECT as isize,
            );
            assert!(!probe.blocked.get(), "DPI SetWindowPos must not hold State");
            let mut limits: MINMAXINFO = zeroed();
            SendMessageW(
                hwnd,
                WM_GETMINMAXINFO,
                0,
                &mut limits as *mut MINMAXINFO as isize,
            );
            assert_eq!(
                [limits.ptMinTrackSize.x, limits.ptMinTrackSize.y],
                model::MIN_SIZE.map(|v| (v as f32 * 1.5).ceil() as i32)
            );
            SendMessageW(hwnd, WM_EXITSIZEMOVE, 0, 0);
            assert!(!host.state.borrow().moving);
            let mut rect = zeroed();
            GetWindowRect(hwnd, &mut rect);
            let saved = Config::load(&root).unwrap();
            assert_eq!(saved.position, Some([rect.left, rect.top]));
            assert_eq!(
                saved.size,
                [
                    ((rect.right - rect.left) as f32 / 1.5).round() as i32,
                    ((rect.bottom - rect.top) as f32 / 1.5).round() as i32
                ]
            );
            assert_ne!(RemoveWindowSubclass(hwnd, Some(probe_proc), 0x4d555349), 0);
            assert_eq!(
                IsWindowVisible(hwnd),
                0,
                "regression uses only a hidden test-owned HWND"
            );
            drop(host);
            std::fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn repeated_layered_paints_keep_the_native_outer_and_client_size_identical() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            let _runtime = render::Runtime::start().unwrap();
            let root = test_root("paint-size");
            let host = Host::create(
                &root,
                Rc::new(RefCell::new(Glass::new(13., 1.))),
                true,
                true,
            )
            .unwrap();
            let hwnd = host.hwnd();
            SetWindowPos(
                hwnd,
                null_mut(),
                100,
                100,
                640,
                480,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let mut before: RECT = zeroed();
            GetWindowRect(hwnd, &mut before);
            host.state.borrow_mut().paint();
            let background_dc = host.state.borrow().background.as_ref().unwrap().dc;
            for _ in 0..8 {
                SendMessageW(hwnd, WM_SIZE, SIZE_RESTORED as usize, 640 | (480 << 16));
                host.state.borrow_mut().paint();
                assert_eq!(
                    host.state.borrow().background.as_ref().unwrap().dc,
                    background_dc,
                    "unchanged size and playback repaint must reuse wallpaper backing"
                );
                let mut after: RECT = zeroed();
                let mut client: RECT = zeroed();
                GetWindowRect(hwnd, &mut after);
                GetClientRect(hwnd, &mut client);
                assert_eq!(
                    [after.left, after.top, after.right, after.bottom],
                    [before.left, before.top, before.right, before.bottom],
                    "uploading a client-sized bitmap must never shrink the HWND"
                );
                assert_eq!(
                    [client.right, client.bottom],
                    [after.right - after.left, after.bottom - after.top]
                );
            }
            eprintln!("music-paint-regression: eight paints completed");
            assert_eq!(IsWindowVisible(hwnd), 0);
            drop(host);
            eprintln!("music-paint-regression: hidden host released");
        }
    }
    #[test]
    fn disc_rotation_changes_art_without_rebuilding_its_cached_pixels() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            let _runtime = render::Runtime::start().unwrap();
            let host = Host::create(
                &test_root("disc-spin"),
                Rc::new(RefCell::new(Glass::new(13., 1.))),
                true,
                true,
            )
            .unwrap();
            let mut s = host.state.borrow_mut();
            s.dpi = 1.;
            s.config.design = "disc".into();
            s.playback.art = Some("spin-fixture".into());
            s.images.insert(
                "spin-fixture".into(),
                Image::from_rgba(
                    32,
                    32,
                    (0..1024)
                        .flat_map(|i| {
                            [
                                if i % 32 < 16 { 220 } else { 30 },
                                if i / 32 < 16 { 180 } else { 50 },
                                70,
                                255,
                            ]
                        })
                        .collect(),
                )
                .unwrap(),
            );
            let a = Buffer::plain(480, 360, 1.).unwrap();
            let b = Buffer::plain(480, 360, 1.).unwrap();
            s.content(a.dc, 480., 360.);
            let cached = s.rounded_art.borrow().len();
            s.spin_angle = 45.;
            s.content(b.dc, 480., 360.);
            assert!(cached > 0);
            assert_eq!(
                s.rounded_art.borrow().len(),
                cached,
                "rotation must reuse the disc raster"
            );
            assert_ne!(
                a.snapshot(),
                b.snapshot(),
                "rotation must visibly rotate the artwork"
            );
        }
    }
    #[test]
    fn all_music_layouts_keep_controls_inside_surface_without_overlap() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            let _runtime = render::Runtime::start().unwrap();
            for design in model::DESIGNS {
                for size in [
                    [168, 104],
                    [180, 110],
                    [168, 220],
                    [168, 1000],
                    [900, 104],
                    [240, 180],
                    [400, 480],
                    [620, 260],
                    [480, 620],
                    [420, 480],
                    [240, 420],
                    [900, 180],
                    [280, 300],
                    [320, 340],
                    [560, 480],
                    [1000, 920],
                ] {
                    for (queue, dpi, signed_in) in [
                        (false, 1.0_f32, false),
                        (true, 1.0, false),
                        (false, 1.5, false),
                        (true, 1.5, false),
                        (false, 1.0, true),
                        (false, 1.5, true),
                    ] {
                        let mut s = State {
                            hwnd: null_mut(),
                            root: PathBuf::new(),
                            config: Config {
                                design: design.into(),
                                size,
                                queue,
                                ..Config::default()
                            },
                            glass: Rc::new(RefCell::new(Glass::new(13., 1.))),
                            backend: None,
                            backend_live: false,
                            peer_live: false,
                            peer_sync_version: 0,
                            smtc: None,
                            latest_smtc: None,
                            smtc_active: false,
                            playback: Playback {
                                signed_in,
                                video_id: "preview0004".into(),
                                available: true,
                                can_seek: true,
                                can_next: true,
                                can_previous: true,
                                title: "Preview track 4".into(),
                                artist: "Music layout fixture".into(),
                                art: Some("fixture-4".into()),
                                duration: 225.,
                                position: 84.,
                                queue: (0..9)
                                    .map(|i| model::Track {
                                        video_id: format!("preview{i:04}"),
                                        title: format!("Preview track {i}"),
                                        artist: "Music layout fixture".into(),
                                        thumb: format!("fixture-{i}"),
                                        selected: i == 4,
                                    })
                                    .collect(),
                                ..Playback::default()
                            },
                            last_cover: None,
                            session_signature: String::new(),
                            media_icons: icons::Icons::default(),
                            button_lenses: lenses::Lenses::default(),
                            received: Instant::now(),
                            last_poll: Instant::now(),
                            last_seek: None,
                            spin_angle: 0.,
                            spin_updated: Instant::now(),
                            spin_active: false,
                            coverflow_anim: None,
                            anim_clock: None,
                            frame_queued: false,
                            images: HashMap::new(),
                            card_cache: HashMap::new(),
                            rounded_art: RefCell::new(HashMap::new()),
                            art_files: HashMap::new(),
                            buttons: Vec::new(),
                            seek_track: None,
                            control_plates: Vec::new(),
                            hover: None,
                            pressed: None,
                            frost: None,
            background: None,
                            frame: None,
                            spin_base: None,
                            spin_base_key: String::new(),
                            spin_disc_rect: None,
                            skip_spin_disc: false,
                            overlay: None,
                            shadow: None,
                            dpi,
                            content_scale: 1.,
                            ink: 0xffffff,
                            muted: 0xcccccc,
                            light: false,
                            settings: None,
                            local_glass: None,
                            settings_dirty: false,
                            blur_dirty: false,
                            moving: false,
                            resizing: false,
                            queue_offset: 0,
                            error: String::new(),
                            smoke: true,
                            inspect: true,
                            revision: 0,
                            fusion_revision: 0,
                            window_origin: [i32::MIN, i32::MIN],
                            was_low_power_moving: false,
                        };
                        for i in 0..9 {
                            let pixels: Vec<u8> = (0..128 * 128)
                                .flat_map(|n| {
                                    [
                                        35 + (n % 128) as u8,
                                        50 + (n / 128) as u8,
                                        60 + i as u8 * 16,
                                        255,
                                    ]
                                })
                                .collect();
                            s.images.insert(
                                format!("fixture-{i}"),
                                Image::from_rgba(128, 128, pixels).unwrap(),
                            );
                        }
                        let physical = size.map(|v| (v as f32 * dpi).round() as i32);
                        let b = Buffer::new(physical[0], physical[1], dpi, 30.).unwrap();
                        let mut backing = Buffer::new(physical[0], physical[1], dpi, 30.).unwrap();
                        let brush = CreateSolidBrush(0x443355);
                        let rect = RECT {
                            left: 0,
                            top: 0,
                            right: physical[0],
                            bottom: physical[1],
                        };
                        FillRect(backing.dc, &rect, brush);
                        DeleteObject(brush);
                        backing.background_alpha(255);
                        s.background = Some(backing);
                        let batch = render::AlphaBatch::new(b.dc, dpi);
                        s.content(b.dc, size[0] as f32, size[1] as f32);
                        drop(batch);
                        if design != "coverflow" {
                            for action in [Action::Previous, Action::Toggle, Action::Next] {
                                let button = s
                                    .buttons
                                    .iter()
                                    .find(|b| b.action == action)
                                    .expect("media control retained");
                                assert!(
                                    button.rect[2] - button.rect[0] >= 28.
                                        && button.rect[3] - button.rect[1] >= 28.,
                                    "{design}: controls must not shrink with art"
                                );
                            }
                            if signed_in {
                                assert_eq!(
                                    s.buttons
                                        .iter()
                                        .filter(|b| b.action == Action::Browse)
                                        .count(),
                                    1
                                );
                            }
                        }
                        for button in &s.buttons {
                            assert!(
                                !matches!(button.action, Action::Queue | Action::Track(_)),
                                "{design}: playlist UI must be absent at every size"
                            );
                            assert!(
                                button.rect[0] >= 0.
                                    && button.rect[1] >= 0.
                                    && button.rect[2] <= size[0] as f32
                                    && button.rect[3] <= size[1] as f32,
                                "{design} {size:?} {:?}",
                                button.action
                            );
                            if !matches!(button.action, Action::Track(_)) {
                                assert!(
                                    button.rect[0] >= 12.
                                        && button.rect[1] >= 12.
                                        && button.rect[2] <= size[0] as f32 - 12.
                                        && button.rect[3] <= size[1] as f32 - 12.,
                                    "{design} {size:?}: control needs breathing room: {:?}",
                                    button.action
                                );
                            }
                        }
                        let play = s
                            .buttons
                            .iter()
                            .rev()
                            .find(|b| b.action == Action::Toggle)
                            .unwrap();
                        let browse = s.buttons.iter().find(|b| b.action == Action::Browse);
                        if let Some(browse) = browse {
                            assert!(
                                play.rect[3] <= browse.rect[1]
                                    || play.rect[2] <= browse.rect[0]
                                    || browse.rect[2] <= play.rect[0]
                                    || browse.rect[3] <= play.rect[1],
                                "{design} {size:?}: play/browse overlap"
                            );
                        } else {
                            assert!(
                                view::layout_plan(design, size[0] as f32, size[1] as f32, queue)
                                    .is_none(),
                                "Browse may only be hidden in tiny mode"
                            );
                            assert!(
                                s.buttons.iter().all(|b| matches!(
                                    b.action,
                                    Action::Toggle
                                        | Action::Previous
                                        | Action::Next
                                        | Action::Seek
                                        | Action::Library
                                        | Action::Browse
                                        | Action::PopOut
                                        | Action::Settings
                                        | Action::Close
                                        | Action::SignIn
                                )),
                                "small layouts retain media controls but never playlist actions"
                            );
                        }
                        if let Some(popout) = s.buttons.iter().find(|b| b.action == Action::PopOut)
                        {
                            for other in s.buttons.iter().filter(|b| {
                                matches!(
                                    b.action,
                                    Action::Browse
                                        | Action::Settings
                                        | Action::Close
                                        | Action::SignIn
                                )
                            }) {
                                assert!(
                                    popout.rect[2] <= other.rect[0]
                                        || other.rect[2] <= popout.rect[0]
                                        || popout.rect[3] <= other.rect[1]
                                        || other.rect[3] <= popout.rect[1],
                                    "{design} {size:?}: popout/{:?} overlap",
                                    other.action
                                );
                            }
                        }
                        let pixels = b.snapshot();
                        let body =
                            crate::settings_ui::widget_content_top(size[0] as f32, size[1] as f32);
                        let full_reference = !queue
                            && match design {
                                "orbit" => size[0] >= 320 && size[1] as f32 - body >= 350.,
                                _ => false,
                            };
                        if full_reference {
                            // This point is below the media cards, away from
                            // controls. Content must not repaint an inset outer
                            // surface over the shared cached chrome.
                            let sx = (32. * dpi) as usize;
                            let sy = ((size[1] as f32 - 20.) * dpi) as usize;
                            assert!(
                                pixels[(sy * physical[0] as usize + sx) * 4 + 3] <= 8,
                                "{design}: inset theme background painted a second outer panel"
                            );
                        }
                        let x = ((play.rect[0] + play.rect[2]) / 2. * dpi) as usize;
                        let y = ((play.rect[1] + play.rect[3]) / 2. * dpi) as usize;
                        assert!(
                            pixels[(y * physical[0] as usize + x) * 4 + 3] > 240,
                            "playback deck must retain backing over transparent chrome"
                        );
                        assert_eq!(pixels[3], 0, "outer chrome remains transparent");
                        if let Some(folder) = std::env::var_os("LUMEN_MUSIC_PREVIEW_DIR") {
                            let folder = PathBuf::from(folder);
                            std::fs::create_dir_all(&folder).unwrap();
                            let mut rgba = pixels;
                            for p in rgba.chunks_exact_mut(4) {
                                p.swap(0, 2);
                                let alpha = p[3] as u32;
                                if alpha > 0 {
                                    for v in &mut p[..3] {
                                        *v = ((*v as u32 * 255 + alpha / 2) / alpha).min(255) as u8;
                                    }
                                }
                            }
                            image::RgbaImage::from_raw(
                                physical[0] as u32,
                                physical[1] as u32,
                                rgba,
                            )
                            .unwrap()
                            .save(folder.join(format!(
                                "{design}-{}x{}-queue-{queue}-dpi-{dpi}.png",
                                size[0], size[1]
                            )))
                            .unwrap();
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn fusion_change_with_liquid_glass_invalidates_reuse_frost() {
        let _ui = crate::native_ui_test_lock();
        struct Reset(bool);
        impl Drop for Reset {
            fn drop(&mut self) {
                crate::glass::set_liquid_glass(self.0);
            }
        }
        let _reset = Reset(crate::glass::liquid_glass());
        crate::glass::set_liquid_glass(true);

        unsafe {
            let _runtime = render::Runtime::start().unwrap();
            let root = test_root("music-liquid-fusion-invalidation");
            std::fs::create_dir_all(&root).unwrap();
            let wave_path = root.join("wave.png");
            let mut wave_img = image::RgbaImage::new(256, 256);
            for y in 0..256u32 {
                for x in 0..256u32 {
                    wave_img.put_pixel(
                        x,
                        y,
                        image::Rgba([
                            ((x * 3 + y) % 255) as u8,
                            ((y * 2 + 40) % 255) as u8,
                            210,
                            255,
                        ]),
                    );
                }
            }
            wave_img.save(&wave_path).unwrap();
            let glass = Rc::new(RefCell::new(Glass::fixture_tiled_file(&wave_path, 14., 1.)));
            crate::fusion::init(glass.clone());
            let host = Host::create(&root, glass, false, true).unwrap();
            let hwnd = host.state.borrow().hwnd;
            SetWindowPos(
                hwnd,
                null_mut(),
                -19600,
                -20000,
                320,
                260,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            crate::fusion::register_widget(hwnd);
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            crate::fusion::prepare(hwnd, 34.);
            InvalidateRect(hwnd, null(), 0);
            SendMessageW(hwnd, WM_PAINT, 0, 0);
            let frost_before = host
                .state
                .borrow()
                .frost
                .as_ref()
                .expect("initial Music frost image")
                .identity();
            let rev_before = host.state.borrow().fusion_revision;

            // Attach an adjacent widget on Music's left edge without moving Music.
            let neighbor = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW,
                wide("STATIC").as_ptr(),
                wide("Music fusion neighbor").as_ptr(),
                WS_POPUP,
                -19600 - 200 + 8,
                -20000,
                200,
                260,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            );
            assert!(!neighbor.is_null());
            crate::fusion::register_widget(neighbor);
            ShowWindow(neighbor, SW_SHOWNOACTIVATE);
            crate::fusion::prepare(neighbor, 30.);
            let mut nbuf = Buffer::new(200, 260, 1., 30.).unwrap();
            let nb = CreateSolidBrush(0x3070d0);
            FillRect(nbuf.dc, &RECT { left: 0, top: 0, right: 200, bottom: 260 }, nb);
            DeleteObject(nb);
            nbuf.background_alpha(255);
            nbuf.bevel_for(neighbor);
            assert!(nbuf.present_with_alpha(neighbor, true));
            crate::fusion::refresh();
            assert!(crate::fusion::fused(hwnd), "Music must fuse with adjacent neighbor");

            InvalidateRect(hwnd, null(), 0);
            SendMessageW(hwnd, WM_PAINT, 0, 0);
            let s = host.state.borrow();
            let frost_after = s.frost.as_ref().expect("rebuilt Music frost image").identity();
            assert_ne!(
                s.fusion_revision, rev_before,
                "Music must advance cached fusion_revision on attach"
            );
            assert_ne!(
                frost_after, frost_before,
                "reuse_frost must stay disabled under Liquid Glass when fusion_changed fires"
            );
            drop(s);

            DestroyWindow(neighbor);
            drop(host);
            crate::fusion::shutdown();
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn accept_smtc_preserves_queue_when_matching_active_track() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            let root = test_root("smtc-preserve-queue");
            let host = Host::create(
                &root,
                Rc::new(RefCell::new(Glass::new(13., 1.))),
                true,
                true,
            )
            .unwrap();
            {
                let mut s = host.state.borrow_mut();
                let queue: Vec<model::Track> = (0..5)
                    .map(|i| model::Track {
                        video_id: format!("vid0000000{i}"),
                        title: format!("Track {i}"),
                        artist: "Artist".into(),
                        thumb: format!("https://i.ytimg.com/vi/vid0000000{i}/0.jpg"),
                        selected: i == 2,
                    })
                    .collect();
                s.playback = Playback {
                    video_id: "vid00000002".into(),
                    title: "Track 2".into(),
                    artist: "Artist".into(),
                    available: true,
                    playing: true,
                    queue,
                    ..Playback::default()
                };

                // Matching SMTC update (with whitespace/case variance) preserves the 5-card queue.
                let matching_smtc = smtc::SmtcState {
                    has_session: true,
                    available: true,
                    playing: true,
                    title: "  track   2 ".into(),
                    artist: "Artist".into(),
                    ..smtc::SmtcState::default()
                };
                s.accept_smtc(&matching_smtc);
                assert_eq!(s.playback.queue.len(), 5);
                assert_eq!(s.playback.video_id, "vid00000002");

                // Unrelated external track clears the YouTube Music queue.
                let unrelated_smtc = smtc::SmtcState {
                    has_session: true,
                    available: true,
                    playing: true,
                    title: "Completely Different Song".into(),
                    artist: "Other Artist".into(),
                    ..smtc::SmtcState::default()
                };
                s.accept_smtc(&unrelated_smtc);
                assert!(s.playback.queue.is_empty());
            }
            drop(host);
            let _ = std::fs::remove_dir_all(root);
        }
    }

    #[test]
    fn spin_base_fast_path_matches_full_content_at_zero_opacity() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            let _runtime = render::Runtime::start().unwrap();
            let root = test_root("music-spin-base-fast-path");
            std::fs::create_dir_all(&root).unwrap();
            let host = Host::create(
                &root,
                Rc::new(RefCell::new(Glass::new(13., 1.))),
                true,
                true,
            )
            .unwrap();
            {
                let mut s = host.state.borrow_mut();
                let dpi = s.dpi;
                let hwnd = s.hwnd;
                let (w_dip, h_dip) = (520.0f32, 300.0f32);
                let (w_px, h_px) = ((w_dip * dpi).round() as i32, (h_dip * dpi).round() as i32);
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    -19600,
                    -20000,
                    w_px,
                    h_px,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                let pixels: Vec<u8> = (0..128 * 128)
                    .flat_map(|n| [40 + (n % 128) as u8, 70 + (n / 128) as u8, 190, 255])
                    .collect();
                s.images.insert(
                    "spin-test-art".into(),
                    Image::from_rgba(128, 128, pixels).unwrap(),
                );
                s.playback = Playback {
                    title: "Spin Base Parity".into(),
                    artist: "Test Artist".into(),
                    art: Some("spin-test-art".into()),
                    available: true,
                    playing: true,
                    can_previous: true,
                    can_next: true,
                    can_seek: true,
                    position: 30.0,
                    duration: 180.0,
                    ..Playback::default()
                };
                s.config.opacity = 0.0;
                s.config.queue = false;
                s.spin_angle = 27.5;
                for design in ["orbit", "disc"] {
                    s.config.design = design.into();
                    s.received = Instant::now();
                    s.background = None;
                    s.clear_spin_base();
                    s.rounded_art.borrow_mut().clear();

                    // Frame 1 captures spin_base; Frame 2 takes the fast path.
                    s.paint();
                    assert!(s.spin_base.is_some(), "{design}: spin_base populated");
                    assert!(!s.spin_base_key.is_empty(), "{design}: spin_base_key set");
                    assert!(s.spin_disc_rect.is_some(), "{design}: spin_disc_rect set");
                    s.paint();
                    let fast = s.frame.as_ref().unwrap().snapshot();

                    let mut ref_buf = Buffer::new(w_px, h_px, dpi, 34.).unwrap();
                    ref_buf.copy_from(s.background.as_ref().unwrap().dc);
                    ref_buf.background_alpha_for(hwnd, 0);
                    let batch = render::AlphaBatch::new(ref_buf.dc, dpi);
                    s.skip_spin_disc = false;
                    s.content(ref_buf.dc, w_px as f32 / dpi, h_px as f32 / dpi);
                    drop(batch);
                    if let Some(bg) = &s.background {
                        for (rect, radius) in &s.control_plates {
                            ref_buf.retain_control_backing(bg, *rect, *radius, 1.);
                        }
                        crate::settings_ui::retain_header_backing(
                            &mut ref_buf,
                            bg,
                            hwnd,
                            w_px as f32 / dpi,
                        );
                    }
                    ref_buf.present_with_alpha(hwnd, true);
                    let reference = ref_buf.snapshot();

                    let max_diff = fast
                        .iter()
                        .zip(&reference)
                        .map(|(a, b)| a.abs_diff(*b))
                        .max()
                        .unwrap_or(0);
                    assert!(
                        max_diff <= 2,
                        "{design}: spin_base fast path max channel diff {max_diff} > 2"
                    );
                }
            }
            drop(host);
            let _ = std::fs::remove_dir_all(root);
        }
    }

    #[test]
    fn peer_sync_shares_full_queue_and_art_files_and_suppresses_smtc() {
        let _ui = crate::native_ui_test_lock();
        SHARED_PLAYBACK.with(|shared| *shared.borrow_mut() = SharedPlayback::default());
        unsafe {
            let root = test_root("music-peer-sync");
            let host_a = Host::create(
                &root,
                Rc::new(RefCell::new(Glass::new(13., 1.))),
                true,
                true,
            )
            .unwrap();
            let host_b = Host::create(
                &root,
                Rc::new(RefCell::new(Glass::new(13., 1.))),
                true,
                true,
            )
            .unwrap();

            {
                let mut a = host_a.state.borrow_mut();
                a.smoke = false;
                a.backend_live = true;
                let queue: Vec<model::Track> = (0..5)
                    .map(|i| model::Track {
                        video_id: format!("peer000000{i}"),
                        title: format!("Peer Track {i}"),
                        artist: "Peer Artist".into(),
                        thumb: format!("https://i.ytimg.com/vi/peer000000{i}/0.jpg"),
                        selected: i == 1,
                    })
                    .collect();
                a.playback = Playback {
                    video_id: "peer0000001".into(),
                    title: "Peer Track 1".into(),
                    artist: "Peer Artist".into(),
                    art: Some("https://i.ytimg.com/vi/peer0000001/0.jpg".into()),
                    available: true,
                    playing: true,
                    queue,
                    ..Playback::default()
                };
                for i in 0..5 {
                    let url = format!("https://i.ytimg.com/vi/peer000000{i}/0.jpg");
                    let file = format!("{:064x}.img", i + 1);
                    a.art_files.insert(url, file);
                }
                a.publish_to_peers();
                a.smoke = true;
            }

            {
                let mut b = host_b.state.borrow_mut();
                b.smoke = false;
                b.latest_smtc = Some(smtc::SmtcState {
                    has_session: true,
                    available: true,
                    playing: true,
                    title: "SMTC Single Track".into(),
                    artist: "Browser".into(),
                    ..smtc::SmtcState::default()
                });
                b.sync_from_peer();
                assert!(b.peer_live);
                assert!(!b.use_smtc());
                assert_eq!(b.playback.queue.len(), 5);
                assert_eq!(b.art_files.len(), 5);
                assert_eq!(b.playback.title, "Peer Track 1");
                b.smoke = true;
            }

            drop(host_b);
            drop(host_a);
            SHARED_PLAYBACK.with(|shared| *shared.borrow_mut() = SharedPlayback::default());
            let _ = std::fs::remove_dir_all(root);
        }
    }

    #[test]
    fn music_popout_floats_topmost_detaches_fusion_and_preserves_theme_and_layout() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            let _runtime = render::Runtime::start().unwrap();
            let root = test_root("music-popout-floating");
            std::fs::create_dir_all(&root).unwrap();
            let glass = Rc::new(RefCell::new(Glass::new(13., 1.)));
            crate::fusion::init(glass.clone());
            let host = Host::create(&root, glass, false, true).unwrap();
            let hwnd = host.hwnd();
            {
                let mut s = host.state.borrow_mut();
                s.config.design = "orbit".into();
                s.config.palette = "gradient:dusk".into();
                s.config.tint = 42.;
                s.config.opacity = 85.;
                s.dpi = 1.;
            }
            SetWindowPos(
                hwnd,
                null_mut(),
                -19600,
                -20000,
                320,
                260,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            crate::fusion::register_widget(hwnd);
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            crate::fusion::prepare(hwnd, 34.);

            let neighbor = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW,
                wide("STATIC").as_ptr(),
                wide("Music popout neighbor").as_ptr(),
                WS_POPUP,
                -19600 - 200 + 8,
                -20000,
                200,
                260,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            );
            assert!(!neighbor.is_null());
            crate::fusion::register_widget(neighbor);
            ShowWindow(neighbor, SW_SHOWNOACTIVATE);
            crate::fusion::prepare(neighbor, 30.);
            crate::fusion::refresh();
            assert!(crate::fusion::fused(hwnd));
            assert!(crate::fusion::fused(neighbor));

            let mut before_rect: RECT = zeroed();
            GetWindowRect(hwnd, &mut before_rect);

            host.state.borrow_mut().activate(Action::PopOut, 0.);
            SendMessageW(hwnd, WM_APP + 82, 0, 0);

            {
                let s = host.state.borrow();
                assert!(s.config.floating);
                assert_eq!(s.config.design, "orbit");
                assert_eq!(s.config.palette, "gradient:dusk");
                assert_eq!(s.config.tint, 42.);
                assert_eq!(s.config.opacity, 85.);
            }
            let mut after_rect: RECT = zeroed();
            GetWindowRect(hwnd, &mut after_rect);
            assert_eq!(
                [after_rect.left, after_rect.top, after_rect.right, after_rect.bottom],
                [before_rect.left, before_rect.top, before_rect.right, before_rect.bottom]
            );
            assert_ne!(
                GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST,
                0,
                "popped out music widget must float topmost"
            );
            assert!(!crate::desktop_layer::contains(hwnd));
            assert!(crate::fusion::is_excluded(hwnd));
            assert!(!crate::fusion::fused(hwnd));
            assert!(!crate::fusion::fused(neighbor));

            host.state.borrow_mut().activate(Action::PopOut, 0.);
            SendMessageW(hwnd, WM_APP + 82, 0, 0);

            assert!(!host.state.borrow().config.floating);
            assert_eq!(
                GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST,
                0,
                "docked music widget must clear WS_EX_TOPMOST"
            );
            assert!(crate::desktop_layer::contains(hwnd));
            assert!(!crate::fusion::is_excluded(hwnd));
            assert!(crate::fusion::fused(hwnd));
            assert!(crate::fusion::fused(neighbor));

            DestroyWindow(neighbor);
            drop(host);
            crate::fusion::shutdown();
            let _ = std::fs::remove_dir_all(root);
        }
    }

    #[test]
    fn music_popout_only_affects_clicked_instance_across_duplicates() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            let _runtime = render::Runtime::start().unwrap();
            let root = test_root("music-popout-multi-monitor");
            std::fs::create_dir_all(&root).unwrap();
            crate::multi_monitor::configure_root(&root, true);
            let glass = Rc::new(RefCell::new(Glass::new(13., 1.)));
            crate::fusion::init(glass.clone());

            let mut primary = Host::create(&root, glass.clone(), false, true).unwrap();
            let primary_hwnd = primary.hwnd();
            primary.state.borrow_mut().dpi = 1.;
            crate::multi_monitor::register_widget(primary_hwnd, "music");
            SetWindowPos(
                primary_hwnd,
                null_mut(),
                -19600,
                -20000,
                320,
                260,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            crate::fusion::register_widget(primary_hwnd);

            let mut dup = {
                let _guard = crate::multi_monitor::DuplicateCreationGuard::enter();
                Host::create(&root, glass.clone(), false, true).unwrap()
            };
            let dup_hwnd = dup.hwnd();
            dup.state.borrow_mut().dpi = 1.;
            crate::multi_monitor::register_duplicate(dup_hwnd, "music", "disp-2");
            SetWindowPos(
                dup_hwnd,
                null_mut(),
                -16000,
                -20000,
                320,
                260,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            crate::fusion::register_widget(dup_hwnd);

            primary.show();
            dup.show();

            // 1. Popping out primary keeps duplicate docked.
            primary.state.borrow_mut().activate(Action::PopOut, 0.);
            SendMessageW(primary_hwnd, WM_APP + 82, 0, 0);
            assert!(primary.state.borrow().config.floating);
            assert_ne!(
                GetWindowLongPtrW(primary_hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST,
                0
            );
            assert!(crate::fusion::is_excluded(primary_hwnd));

            assert!(!dup.state.borrow().config.floating);
            assert_eq!(
                GetWindowLongPtrW(dup_hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST,
                0
            );
            assert!(!crate::fusion::is_excluded(dup_hwnd));

            // 2. Syncing shared settings (palette) from primary preserves duplicate's docked state.
            {
                let mut s = primary.state.borrow_mut();
                s.config.palette = "gradient:dusk".into();
                s.config.save(&root).unwrap();
            }
            dup.state.borrow_mut().reload_from_shared();
            {
                let ds = dup.state.borrow();
                assert_eq!(ds.config.palette, "gradient:dusk");
                assert!(!ds.config.floating);
            }
            assert_eq!(
                GetWindowLongPtrW(dup_hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST,
                0
            );

            // 3. A newly spawned duplicate does not inherit floating=true from primary config on disk.
            let mut third_dup = {
                let _guard = crate::multi_monitor::DuplicateCreationGuard::enter();
                Host::create(&root, glass.clone(), false, true).unwrap()
            };
            let third_hwnd = third_dup.hwnd();
            third_dup.state.borrow_mut().dpi = 1.;
            crate::multi_monitor::register_duplicate(third_hwnd, "music", "disp-3");
            SetWindowPos(
                third_hwnd,
                null_mut(),
                -12000,
                -20000,
                320,
                260,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            third_dup.show();
            assert!(!third_dup.state.borrow().config.floating);
            assert_eq!(
                GetWindowLongPtrW(third_hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST,
                0
            );

            // 4. Popping out duplicate affects only that duplicate; docking primary leaves duplicate floating.
            dup.state.borrow_mut().activate(Action::PopOut, 0.);
            SendMessageW(dup_hwnd, WM_APP + 82, 0, 0);
            assert!(dup.state.borrow().config.floating);
            assert!(crate::multi_monitor::duplicate_floating(dup_hwnd));
            assert_ne!(
                GetWindowLongPtrW(dup_hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST,
                0
            );
            assert!(crate::fusion::is_excluded(dup_hwnd));
            assert!(!third_dup.state.borrow().config.floating);

            primary.state.borrow_mut().activate(Action::PopOut, 0.);
            SendMessageW(primary_hwnd, WM_APP + 82, 0, 0);
            assert!(!primary.state.borrow().config.floating);
            assert!(dup.state.borrow().config.floating);
            assert_ne!(
                GetWindowLongPtrW(dup_hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST,
                0
            );

            dup.state.borrow_mut().activate(Action::PopOut, 0.);
            SendMessageW(dup_hwnd, WM_APP + 82, 0, 0);
            assert!(!dup.state.borrow().config.floating);
            assert!(!crate::multi_monitor::duplicate_floating(dup_hwnd));
            assert_eq!(
                GetWindowLongPtrW(dup_hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST,
                0
            );
            assert!(!crate::fusion::is_excluded(dup_hwnd));

            drop(third_dup);
            drop(dup);
            drop(primary);
            crate::multi_monitor::shutdown();
            crate::fusion::shutdown();
            let _ = std::fs::remove_dir_all(root);
        }
    }
}

pub(crate) mod package;
