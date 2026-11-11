//! Native Music surface: the dock's wallpaper glass, compositor and typography.
//! Win32 handles and GDI images stay on the UI thread. Browser I/O is isolated.
mod backend;
mod chrome;
mod coverflow;
mod icons;
mod lenses;
mod model;
mod session;
mod settings;
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

struct State {
    hwnd: HWND,
    root: PathBuf,
    config: Config,
    glass: Rc<RefCell<Glass>>,
    backend: Option<backend::Backend>,
    playback: Playback,
    last_cover: Option<String>,
    session_signature: String,
    media_icons: icons::Icons,
    button_lenses: lenses::Lenses,
    received: Instant,
    images: HashMap<String, Image>,
    card_cache: HashMap<String, Image>,
    rounded_art: RefCell<HashMap<String, Image>>,
    art_files: HashMap<String, String>,
    buttons: Vec<Button>,
    control_plates: Vec<([f32; 4], f32)>,
    hover: Option<Action>,
    pressed: Option<Action>,
    background: Option<Buffer>,
    frame: Option<Buffer>,
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
    queue_offset: usize,
    error: String,
    smoke: bool,
    inspect: bool,
    revision: u64,
    fusion_revision: u64,
}
impl State {
    fn position(&self) -> f64 {
        let elapsed = if self.playback.playing {
            self.received.elapsed().as_secs_f64().min(2.)
        } else {
            0.
        };
        (self.playback.position + elapsed)
            .max(0.)
            .min(self.playback.duration.max(0.))
    }
    fn save(&self) {
        if !self.smoke {
            if let Err(e) = self.config.save(&self.root) {
                eprintln!("native-music: {e}");
            }
        }
    }
    fn send(&mut self, value: serde_json::Value) {
        if self.backend.as_mut().is_some_and(|b| b.exited()) {
            self.backend.take();
        }
        if self.backend.is_none() {
            match backend::Backend::start(&self.root) {
                Ok(b) => {
                    self.backend = Some(b);
                    self.error.clear();
                }
                Err(e) => {
                    self.error = e;
                    return;
                }
            }
        }
        if let Err(e) = self.backend.as_mut().unwrap().send(value) {
            self.error = e;
        }
    }
    unsafe fn activate(&mut self, action: Action, x: f32) {
        match action {
            Action::Close => {
                self.config.open = false;
                self.save();
                ShowWindow(self.hwnd, SW_HIDE);
                if let Some(s) = &mut self.shadow {
                    s.sync(self.hwnd, self.dpi, 0);
                }
                self.frame = None;
                self.background = None;
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
                if self.backend.is_some() {
                    // Retain only a slow metadata drain while hidden playback lives.
                    SetTimer(self.hwnd, 91, 1500, None);
                } else {
                    KillTimer(self.hwnd, 91);
                }
            }
            Action::Settings => self.open_settings(),
            Action::Queue => {
                self.config.queue = !self.config.queue;
                self.save();
                self.sync_settings();
            }
            Action::Browse => self.send(serde_json::json!({"action":"browse"})),
            Action::Library => self.send(serde_json::json!({"action":"library"})),
            Action::Previous => self.send(serde_json::json!({"action":"previous"})),
            Action::Next => self.send(serde_json::json!({"action":"next"})),
            Action::Toggle => {
                if !self.playback.available && self.playback.title.is_empty() {
                    self.send(serde_json::json!({"action":"browse"}));
                } else {
                    self.send(serde_json::json!({"action":"toggle"}));
                }
            }
            Action::Track(id) => self.send(serde_json::json!({"action":"select","videoId":id})),
            Action::Seek => {
                if let Some(b) = self.buttons.iter().find(|b| b.action == Action::Seek) {
                    let fraction = ((x - b.rect[0]) / (b.rect[2] - b.rect[0])).clamp(0., 1.) as f64;
                    self.send(serde_json::json!({"action":"seek","value":fraction * self.playback.duration}));
                }
            }
        }
        InvalidateRect(self.hwnd, null(), 0);
    }
    unsafe fn bounds(&mut self) {
        let mut r = zeroed();
        GetWindowRect(self.hwnd, &mut r);
        self.config.position = Some([r.left, r.top]);
        self.config.size = [
            ((r.right - r.left) as f32 / self.dpi).round() as i32,
            ((r.bottom - r.top) as f32 / self.dpi).round() as i32,
        ];
        self.save();
    }
    unsafe fn poll(&mut self) {
        let mut dirty = false;
        // Drain outside the receiver borrow, then create images on the UI thread.
        let events: Vec<_> = self
            .backend
            .as_ref()
            .map(|b| b.events.try_iter().take(64).collect())
            .unwrap_or_default();
        for event in events {
            match event["type"].as_str().unwrap_or("") {
                "state" => {
                    if let Ok(state) = serde_json::from_value::<Playback>(event["state"].clone()) {
                        self.error = state.error.clone().unwrap_or_default();
                        self.accept_playback(state);
                    }
                    if let Some(error) = event["error"].as_str() {
                        self.error = error.into();
                        self.playback.available = false;
                        self.playback.playing = false;
                    }
                    dirty = true;
                }
                "error" => {
                    self.error = event["error"].as_str().unwrap_or("Playback error").into();
                    self.playback.playing = false;
                    dirty = true;
                }
                "art" => {
                    if let (Some(url), Some(file)) = (event["url"].as_str(), event["file"].as_str())
                    {
                        if file.len() == 68
                            && file.ends_with(".img")
                            && file.as_bytes()[..64].iter().all(|c| c.is_ascii_hexdigit())
                        {
                            if self.art_files.len() > 160 {
                                self.art_files.retain(|url, _| {
                                    self.last_cover.as_ref() == Some(url)
                                        || self.playback.art.as_ref() == Some(url)
                                        || self.playback.queue.iter().any(|t| t.thumb == *url)
                                });
                            }
                            self.art_files.insert(url.into(), file.into());
                            dirty = true;
                        }
                    }
                }
                _ => {}
            }
        }
        if dirty || self.playback.playing {
            if IsWindowVisible(self.hwnd) != 0 {
                self.restore_art();
                InvalidateRect(self.hwnd, null(), 0);
            }
            self.remember_view();
        }
    }
    fn restore_art(&mut self) {
        let mut urls: Vec<String> = self.playback.art.iter().cloned().collect();
        urls.extend(self.last_cover.iter().cloned());
        if let Some(i) = self
            .playback
            .queue
            .iter()
            .position(|t| t.selected || t.video_id == self.playback.video_id)
        {
            urls.extend(
                self.playback
                    .queue
                    .iter()
                    .skip(i.saturating_sub(2))
                    .take(5)
                    .map(|t| t.thumb.clone()),
            );
        }
        urls.extend(
            self.playback
                .queue
                .iter()
                .skip(self.queue_offset)
                .take(20)
                .map(|t| t.thumb.clone()),
        );
        self.images.retain(|url, _| urls.contains(url));
        for url in urls {
            if self.images.contains_key(&url) {
                continue;
            }
            if let Some(file) = self.art_files.get(&url) {
                if let Some(image) =
                    load_art(&self.root.join("config/native-music-player/art").join(file))
                {
                    self.images.insert(url, image);
                    self.card_cache.clear();
                }
            }
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
    unsafe fn paint(&mut self) {
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
            self.overlay = None;
            if let Some(glass) = &mut self.local_glass {
                glass.refresh();
            }
        }
        crate::fusion::prepare(self.hwnd, 34.);
        let fusion_revision = crate::fusion::revision(self.hwnd);
        if fusion_revision != self.fusion_revision {
            self.fusion_revision = fusion_revision;
            self.background = None;
        }
        let changed = self
            .frame
            .as_ref()
            .is_none_or(|b| !b.matches_geometry(w, h, self.dpi, 34.));
        if changed {
            self.frame = Buffer::new(w, h, self.dpi, 34.);
            self.background = None;
            self.overlay = None;
        }
        let Some(mut frame) = self.frame.take() else {
            EndPaint(self.hwnd, &ps);
            return;
        };
        let (width, height) = (w as f32 / self.dpi, h as f32 / self.dpi);
        if self.background.is_none() {
            self.card_cache.clear();
            if let Some(mut bg) = Buffer::new(w, h, self.dpi, 34.) {
                let brush = CreateSolidBrush(if crate::accent::light(self.surface_color()) {
                    0xf5f2f0
                } else {
                    0x231b16
                });
                FillRect(bg.dc, &r, brush);
                DeleteObject(brush);
                if self.config.frosted_glass {
                    if self.local_glass.is_none() {
                        let mut glass = Glass::new(self.config.blur, self.dpi);
                        glass.refresh();
                        self.local_glass = Some(glass);
                    }
                    self.local_glass.as_ref().unwrap().draw_blended(
                        bg.dc,
                        self.hwnd,
                        w,
                        h,
                        (self.config.frost_strength * 2.55).round() as u8,
                    );
                } else {
                    self.glass.borrow().draw_blended(bg.dc, self.hwnd, w, h, 0);
                }
                self.chrome(&mut bg, [width, height]);
                render::flush(bg.dc);
                (self.ink, self.muted) = crate::contrast::inks(
                    crate::contrast::sample(bg.dc, self.dpi, [20., 20., width - 20., height - 20.]),
                    Some(self.ink),
                );
                self.light = crate::contrast::luminance(self.ink) < 0.5;
                bg.bevel_for(self.hwnd);
                bg.background_alpha(255);
                self.background = Some(bg);
            }
        }
        if let Some(bg) = &self.background {
            frame.copy_from(bg.dc);
        }
        frame.background_alpha((self.config.opacity * 2.55) as u8);
        let batch = render::AlphaBatch::new(frame.dc, self.dpi);
        self.content(frame.dc, width, height);
        drop(batch);
        if let Some(bg) = &self.background {
            for (rect, radius) in &self.control_plates {
                frame.retain_control_backing(bg, *rect, *radius, 1.);
            }
            crate::settings_ui::retain_header_backing(&mut frame, bg, self.hwnd, width);
        }
        frame.present_with_alpha(self.hwnd, true);
        if self.shadow.is_none() {
            self.shadow = WindowShadow::new(self.hwnd);
        }
        if let Some(shadow) = &mut self.shadow {
            shadow.sync(self.hwnd, self.dpi, (self.config.opacity * 2.55) as u8);
        }
        self.frame = Some(frame);
        EndPaint(self.hwnd, &ps);
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
        if GetMonitorInfoW(monitor, &mut info) != 0 {
            rect.left = rect.left.clamp(
                info.rcWork.left,
                (info.rcWork.right - width).max(info.rcWork.left),
            );
            rect.top = rect.top.clamp(
                info.rcWork.top,
                (info.rcWork.bottom - height).max(info.rcWork.top),
            );
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
            playback,
            last_cover,
            session_signature: String::new(),
            media_icons: icons::Icons::default(),
            button_lenses: lenses::Lenses::default(),
            received: Instant::now(),
            images: HashMap::new(),
            card_cache: HashMap::new(),
            rounded_art: RefCell::new(HashMap::new()),
            art_files,
            buttons: Vec::new(),
            control_plates: Vec::new(),
            hover: None,
            pressed: None,
            background: None,
            frame: None,
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
            queue_offset: 0,
            error: String::new(),
            smoke,
            inspect,
            revision: 0,
            fusion_revision: 0,
        }));
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | if inspect { 0 } else { WS_EX_TOOLWINDOW },
            class.as_ptr(),
            wide("Lumen · YouTube Music").as_ptr(),
            WS_POPUP | WS_THICKFRAME,
            rect.left,
            rect.top,
            width,
            height,
            null_mut(),
            null_mut(),
            instance,
            &*state as *const RefCell<State> as _,
        );
        if hwnd.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        state.borrow_mut().hwnd = hwnd;
        state.borrow_mut().restore_art();
        SetTimer(hwnd, 91, 250, None);
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
            s.config.open = true;
            s.save();
            (s.hwnd, s.config.floating, s.inspect)
        };
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetTimer(hwnd, 91, 250, None);
        crate::window::apply_window_mode(hwnd, floating, inspect);
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
        s.background = None;
        s.overlay = None;
        if IsWindowVisible(s.hwnd) != 0 {
            InvalidateRect(s.hwnd, null(), 0);
        }
    }
    pub unsafe fn apply_common(&mut self, opacity: Option<f32>, floating: Option<bool>) {
        let mut s = self.state.borrow_mut();
        if let Some(opacity) = opacity.filter(|v| v.is_finite()) {
            s.config.opacity = opacity.clamp(0., 100.);
            s.background = None;
        }
        if let Some(floating) = floating {
            s.config.floating = floating;
        }
        s.save();
        s.sync_settings();
        let (hwnd, inspect) = (s.hwnd, s.inspect);
        drop(s);
        if let Some(floating) = floating {
            crate::window::apply_window_mode(hwnd, floating, inspect);
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
            s.backend.take();
            s.shadow.take();
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

fn load_art(path: &Path) -> Option<Image> {
    let mut reader = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let rgba = reader.decode().ok()?.thumbnail(400, 400).into_rgba8();
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
        m if m == settings::NOTIFY => {
            s.settings_changed();
            0
        }
        WM_TIMER => {
            s.poll();
            0
        }
        m if m == WM_APP + 81 => {
            if s.backend.is_some() {
                s.send(serde_json::json!({"action":"layout"}));
            }
            0
        }
        m if m == WM_APP + 82 => {
            let (floating, inspect) = (s.config.floating, s.inspect);
            drop(s);
            crate::window::apply_window_mode(hwnd, floating, inspect);
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
            s.hover = None;
            InvalidateRect(hwnd, null(), 0);
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
            if pressed.is_some() && pressed == action {
                s.activate(action.unwrap(), p[0]);
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_CAPTURECHANGED | WM_CANCELMODE => {
            s.pressed = None;
            if msg == WM_CANCELMODE {
                s.moving = false;
            }
            InvalidateRect(hwnd, null(), 0);
            // DefWindowProc must see cancellation of its native move/size loop.
            drop(s);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_MOUSEWHEEL => {
            if s.config.queue {
                let delta = ((wp >> 16) as i16) as i32;
                s.queue_offset = if delta > 0 {
                    s.queue_offset.saturating_sub(1)
                } else {
                    s.queue_offset + 1
                };
                s.restore_art();
                InvalidateRect(hwnd, null(), 0);
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
            s.moving = true;
            0
        }
        WM_EXITSIZEMOVE => {
            s.moving = false;
            s.background = None;
            s.bounds();
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_SIZE => {
            s.frame = None;
            s.background = None;
            s.overlay = None;
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_WINDOWPOSCHANGED => {
            let dpi = s.dpi;
            if let Some(shadow) = &mut s.shadow {
                shadow.follow_position(&*(lp as *const WINDOWPOS), dpi);
            }
            // Default processing synchronously sends WM_SIZE/WM_MOVE.
            drop(s);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_DPICHANGED => {
            s.dpi = (wp as u16) as f32 / 96.;
            s.frame = None;
            s.background = None;
            s.overlay = None;
            s.local_glass = None;
            let r = *(lp as *const RECT);
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
            if !s.moving {
                s.background = None;
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_CLOSE => {
            s.activate(Action::Close, 0.);
            0
        }
        WM_NCDESTROY => {
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
                        "lumen" => [360., 180.],
                        "orbit" => [400., 480.],
                        "disc" => [620., 260.],
                        "sky" => [480., 620.],
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
                    let mut actual = Buffer::plain(320, 240, 1.).unwrap();
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
                    s.chrome(&mut actual, [320., 240.]);
                    // Reference the Dock's coating sequence over identical backing.
                    // This guards the former duplicate reflection / 24-vs-.24 bug.
                    let color = s.surface_color();
                    let light = crate::accent::light(color);
                    let pane = [4., 4., 316., 236.];
                    let overlay = Canvas::pane_overlay([320, 240], 1., tint / 100., light).unwrap();
                    let c = Canvas::new(expected.dc, 1.);
                    if !crate::surface::managed(&s.config.palette) {
                        c.round(pane, 34., color, (tint * 2.55).round() as u8, false, false);
                    }
                    crate::surface::paint(&c, pane, &s.config.palette, tint / 100.);
                    c.coating(pane, 34.);
                    c.image(&overlay, [0., 0., 320., 240.]);
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
                for required in [102, 103, 104, 105] {
                    assert!(
                        rows.iter().any(|r| matches!(r,
                            crate::settings_ui::Row::Slider { id, .. } if *id == required
                        )),
                        "appearance control {required} must remain a slider"
                    );
                }
                assert!(!rows
                    .iter()
                    .any(|r| matches!(r, crate::settings_ui::Row::Button { .. })));
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
                assert!(s.local_glass.is_some());
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
            for _ in 0..8 {
                host.state.borrow_mut().paint();
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
                    for (queue, dpi) in [(false, 1.0_f32), (true, 1.0), (false, 1.5), (true, 1.5)] {
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
                            playback: Playback {
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
                            images: HashMap::new(),
                            card_cache: HashMap::new(),
                            rounded_art: RefCell::new(HashMap::new()),
                            art_files: HashMap::new(),
                            buttons: Vec::new(),
                            control_plates: Vec::new(),
                            hover: None,
                            pressed: None,
                            background: None,
                            frame: None,
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
                            queue_offset: 0,
                            error: String::new(),
                            smoke: true,
                            inspect: true,
                            revision: 0,
                            fusion_revision: 0,
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
                        for button in &s.buttons {
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
                            assert!(s.buttons.iter().all(|b| matches!(b.action,
                                Action::Toggle | Action::Settings | Action::Close)),
                                "tiny layout must hide secondary actions even when queue is enabled");
                        }
                        let pixels = b.snapshot();
                        let body =
                            crate::settings_ui::widget_content_top(size[0] as f32, size[1] as f32);
                        let full_reference = !queue
                            && match design {
                                "orbit" => size[0] >= 320 && size[1] as f32 - body >= 350.,
                                "sky" => size[0] >= 420 && size[1] as f32 - body >= 420.,
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
}

pub(crate) mod package;
