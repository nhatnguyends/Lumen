//! Native Clock, with a detached shared-kit settings window and compact faces.
#[path = "afterglow.rs"]
mod afterglow;
mod face;
mod model;
#[path = "settings.rs"]
mod settings;
#[path = "visuals.rs"]
mod visuals;
use crate::{
    compositor::{Buffer, WindowShadow},
    glass::Glass,
    model::wide,
    render::{self, Canvas},
};
use model::*;
use std::{
    cell::RefCell,
    mem::{size_of, zeroed},
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    rc::Rc,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{LibraryLoader::GetModuleHandleW, Time::DYNAMIC_TIME_ZONE_INFORMATION},
    UI::{
        HiDpi::*,
        Input::KeyboardAndMouse::{
            ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
        },
        WindowsAndMessaging::*,
    },
};
const TICK: usize = 71;
const ANIMATION: usize = 72;
pub struct Host {
    state: Box<RefCell<State>>,
}
struct State {
    hwnd: HWND,
    root: PathBuf,
    config: Config,
    zone: Option<DYNAMIC_TIME_ZONE_INFORMATION>,
    glass: Rc<RefCell<Glass>>,
    dpi: f32,
    inspect: bool,
    smoke: bool,
    moving: bool,
    frame: Option<Buffer>,
    background: Option<Buffer>,
    key: String,
    shadow: Option<WindowShadow>,
    ink: u32,
    muted: u32,
    pressed: Option<usize>,
    hover: Option<usize>,
    error: String,
    settings: Option<crate::settings_ui::Host>,
    frost: Option<render::Image>,
    frost_key: String,
    theme_texture: Option<(String, render::Image)>,
    location_label: String,
    afterglow_content: Option<(String, render::Image)>,
}
pub fn should_open(root: &Path) -> bool {
    root.join("config/native-clock.json").exists()
        && Config::load(root).map(|c| c.open).unwrap_or(false)
}
impl Host {
    pub unsafe fn create(
        root: &Path,
        glass: Rc<RefCell<Glass>>,
        inspect: bool,
        smoke: bool,
    ) -> Result<Self, String> {
        let config = Config::load(root)?;
        let zone = zone(&config.time_zone)?;
        let location_label = model::location_label(&config.time_zone);
        let class = wide("LumenNativeClockWindow");
        let instance = GetModuleHandleW(null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: instance,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: class.as_ptr(),
            ..zeroed()
        };
        if RegisterClassW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let dpi = GetDpiForSystem() as f32 / 96.;
        let size = config.size.map(|v| (v as f32 * dpi).round() as i32);
        let mut work: RECT = zeroed();
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut work as *mut _ as _, 0);
        let pos = config.position.unwrap_or([work.left + 72, work.top + 72]);
        let bounds = RECT {
            left: pos[0],
            top: pos[1],
            right: pos[0] + size[0],
            bottom: pos[1] + size[1],
        };
        let monitor = MonitorFromRect(&bounds, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = zeroed();
        info.cbSize = size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(monitor, &mut info);
        let x = bounds.left.clamp(
            info.rcWork.left,
            (info.rcWork.right - size[0]).max(info.rcWork.left),
        );
        let y = bounds.top.clamp(
            info.rcWork.top,
            (info.rcWork.bottom - size[1]).max(info.rcWork.top),
        );
        let mut state = Box::new(RefCell::new(State {
            hwnd: null_mut(),
            root: root.into(),
            config,
            zone,
            glass,
            dpi,
            inspect,
            smoke,
            moving: false,
            frame: None,
            background: None,
            key: String::new(),
            shadow: None,
            ink: 0xfdfaf8,
            muted: 0xd7cac1,
            pressed: None,
            hover: None,
            error: String::new(),
            settings: None,
            frost: None,
            frost_key: String::new(),
            theme_texture: None,
            location_label,
            afterglow_content: None,
        }));
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED
                | if inspect {
                    WS_EX_APPWINDOW
                } else {
                    WS_EX_TOOLWINDOW
                },
            class.as_ptr(),
            wide("Lumen · Native Clock").as_ptr(),
            WS_POPUP,
            x,
            y,
            size[0],
            size[1],
            null_mut(),
            null_mut(),
            instance,
            &mut *state as *mut _ as _,
        );
        if hwnd.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        state.borrow_mut().hwnd = hwnd;
        println!(
            "native-clock-created pid={} theme={} shared_wallpaper=true",
            std::process::id(),
            state.borrow().config.theme
        );
        Ok(Self { state })
    }
    pub fn hwnd(&self) -> HWND {
        self.state.borrow().hwnd
    }
    pub unsafe fn is_visible(&self) -> bool {
        IsWindowVisible(self.hwnd()) != 0
    }
    pub unsafe fn hide(&mut self) {
        self.state.borrow_mut().action(1);
    }
    pub unsafe fn show_settings(&mut self) {
        self.state.borrow_mut().open_settings();
    }
    pub unsafe fn apply_common(&mut self, opacity: Option<f32>, floating: Option<bool>) {
        let (hwnd, topmost, inspect) = {
            let mut s = self.state.borrow_mut();
            if let Some(v) = opacity.filter(|v| v.is_finite()) {
                s.config.opacity = v.clamp(0., 100.);
            }
            if let Some(v) = floating {
                s.config.floating = v;
            }
            s.key.clear();
            s.save();
            s.sync_settings();
            (s.hwnd, s.config.floating, s.inspect)
        };
        if floating.is_some() {
            crate::window::apply_window_mode(hwnd, topmost, inspect);
        }
        if IsWindowVisible(hwnd) != 0 {
            InvalidateRect(hwnd, null(), 0);
        }
    }
    pub unsafe fn show(&mut self) {
        let hwnd = self.hwnd();
        {
            let mut s = self.state.borrow_mut();
            s.config.open = true;
            s.pressed = None;
            s.hover = None;
            s.key.clear();
            s.save();
        }
        let (floating, inspect) = {
            let s = self.state.borrow();
            (s.config.floating, s.inspect)
        };
        ShowWindow(hwnd, if inspect { SW_SHOW } else { SW_SHOWNOACTIVATE });
        crate::window::apply_window_mode(hwnd, floating, inspect);
        let mut s = self.state.borrow_mut();
        s.arm();
        s.paint();
    }
    pub unsafe fn refresh(&mut self) {
        let s = self.state.borrow();
        if IsWindowVisible(s.hwnd) != 0 {
            if !s.moving && s.background_key() != s.key {
                InvalidateRect(s.hwnd, null(), 0);
            }
            if !s.config.floating && !s.inspect {
                crate::window::pin_to_desktop(s.hwnd);
            }
        }
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        unsafe {
            self.state.borrow_mut().settings.take();
            let hwnd = self.hwnd();
            if IsWindow(hwnd) != 0 {
                DestroyWindow(hwnd);
            }
        }
    }
}
#[derive(Clone)]
struct Button {
    id: usize,
    r: [f32; 4],
}
impl State {
    unsafe fn dimensions(&self) -> (f32, f32) {
        let mut r = zeroed();
        GetClientRect(self.hwnd, &mut r);
        (r.right as f32 / self.dpi, r.bottom as f32 / self.dpi)
    }
    unsafe fn buttons(&self) -> Vec<Button> {
        let (w, _) = self.dimensions();
        crate::settings_ui::widget_header_hits(w)
            .into_iter()
            .map(|(a, r)| Button {
                id: if a == crate::settings_ui::HeaderAction::Settings {
                    0
                } else {
                    1
                },
                r,
            })
            .collect()
    }
    unsafe fn hit(&self, x: f32, y: f32) -> Option<usize> {
        self.buttons()
            .iter()
            .find(|b| x >= b.r[0] && x <= b.r[2] && y >= b.r[1] && y <= b.r[3])
            .map(|b| b.id)
    }
    unsafe fn save(&mut self) {
        if !self.smoke {
            if let Err(e) = self.config.save(&self.root) {
                eprintln!("native-clock-save: {e}");
                self.error = e;
            }
        }
    }
    unsafe fn save_bounds(&mut self) {
        let mut r = zeroed();
        GetWindowRect(self.hwnd, &mut r);
        self.config.position = Some([r.left, r.top]);
        self.config.size = [
            ((r.right - r.left) as f32 / self.dpi).round() as i32,
            ((r.bottom - r.top) as f32 / self.dpi).round() as i32,
        ];
        self.save();
    }
    unsafe fn arm_animation(&self) {
        KillTimer(self.hwnd, ANIMATION);
        if let Some(ms) = afterglow::interval(
            &self.config,
            IsWindowVisible(self.hwnd) != 0 && IsIconic(self.hwnd) == 0,
            self.moving,
        ) {
            SetTimer(self.hwnd, ANIMATION, ms, None);
        }
    }
    unsafe fn arm(&self) {
        KillTimer(self.hwnd, TICK);
        self.arm_animation();
        if IsWindowVisible(self.hwnd) != 0 {
            let t = model::now(&self.zone);
            let period = if self.config.show_seconds {
                1000
            } else {
                60000
            };
            let elapsed = if self.config.show_seconds {
                t.wMilliseconds as u32
            } else {
                t.wSecond as u32 * 1000 + t.wMilliseconds as u32
            };
            SetTimer(self.hwnd, TICK, (period - elapsed).max(10), None);
        }
    }
    unsafe fn action(&mut self, id: usize) {
        match id {
            0 => {
                self.open_settings();
                return;
            }
            1 => {
                self.config.open = false;
                self.save_bounds();
                KillTimer(self.hwnd, TICK);
                KillTimer(self.hwnd, ANIMATION);
                self.shadow.take();
                ShowWindow(self.hwnd, SW_HIDE);
                self.frame = None;
                self.background = None;
                self.frost = None;
                self.theme_texture = None;
                self.afterglow_content = None;
                self.frost_key.clear();
                return;
            }
            10..=18 => self.config.theme = THEMES[id - 10].0.into(),
            _ => return,
        }
        self.key.clear();
        self.arm_animation();
        self.save();
        self.sync_settings();
        InvalidateRect(self.hwnd, null(), 0);
    }
    unsafe fn background_key(&self) -> String {
        let mut r = zeroed();
        GetWindowRect(self.hwnd, &mut r);
        format!(
            "{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            self.dpi,
            self.config.theme,
            self.config.palette,
            self.config.frosted_glass,
            self.config.tint,
            self.config.opacity,
            self.config.frost_strength,
            self.config.blur,
            self.glass.borrow().revision(),
            crate::fusion::revision(self.hwnd)
        )
    }
    /// Sample the shared SHARP source, then build only this viewport's local frost.
    /// Never mutate the common Glass blur. Reuse texture while live-resizing.
    unsafe fn draw_frost(&mut self, dc: HDC, w: i32, h: i32) {
        let mut pos = zeroed();
        GetWindowRect(self.hwnd, &mut pos);
        let key = format!(
            "{}:{}:{}:{}:{}:{}:{}:{}",
            pos.left,
            pos.top,
            w,
            h,
            self.dpi,
            self.config.blur,
            self.config.frost_strength,
            self.glass.borrow().revision()
        );
        if self.frost.is_none() || (!self.moving && self.frost_key != key) {
            let pad = (self.config.blur * self.dpi * 3.).ceil().min(240.) as i32;
            let (sw, sh) = (w + pad * 2, h + pad * 2);
            if let Some(sample) = Buffer::plain(sw, sh, self.dpi) {
                let brush = CreateSolidBrush(self.surface_color());
                let r = RECT {
                    left: 0,
                    top: 0,
                    right: sw,
                    bottom: sh,
                };
                FillRect(sample.dc, &r, brush);
                DeleteObject(brush);
                SetViewportOrgEx(sample.dc, pad, pad, null_mut());
                self.glass
                    .borrow()
                    .draw_blended(sample.dc, self.hwnd, sw, sh, 0);
                SetViewportOrgEx(sample.dc, 0, 0, null_mut());
                let mut source = sample.snapshot();
                for p in source.chunks_exact_mut(4) {
                    p.swap(0, 2);
                    p[3] = 255;
                }
                if let Some(sharp) = image::RgbaImage::from_raw(sw as u32, sh as u32, source) {
                    let blurred = if self.config.blur > 0. {
                        image::imageops::blur(&sharp, (self.config.blur * self.dpi).max(0.1))
                    } else {
                        sharp.clone()
                    };
                    let mix = self.config.frost_strength / 100.;
                    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
                    for y in 0..h {
                        for x in 0..w {
                            let a = sharp.get_pixel((x + pad) as u32, (y + pad) as u32);
                            let b = blurred.get_pixel((x + pad) as u32, (y + pad) as u32);
                            let l =
                                0.2126 * b[0] as f32 + 0.7152 * b[1] as f32 + 0.0722 * b[2] as f32;
                            for ch in 0..3 {
                                let boost = (l + (b[ch] as f32 - l) * 1.28) * 0.92;
                                pixels.push(
                                    (a[ch] as f32 * (1. - mix) + boost.clamp(0., 255.) * mix)
                                        .round() as u8,
                                );
                            }
                            pixels.push(255);
                        }
                    }
                    self.frost = render::Image::from_rgba(w as u32, h as u32, pixels);
                    self.frost_key = key;
                }
            }
        }
        if let Some(image) = &self.frost {
            Canvas::new(dc, self.dpi)
                .image(image, [0., 0., w as f32 / self.dpi, h as f32 / self.dpi]);
        }
    }
    /// Cache time/date/typography once per displayed second (or minute).
    /// Animation frames draw only the vector emblem over this native-size sprite.
    unsafe fn paint_content(&mut self, dc: HDC, w: f32, h: f32) {
        let now = model::now(&self.zone);
        self.paint_content_at(dc, w, h, &now);
    }
    unsafe fn paint_content_at(&mut self, dc: HDC, w: f32, h: f32, now: &SYSTEMTIME) {
        if self.config.theme != "sunset" {
            self.afterglow_content = None;
            face::draw(
                &Canvas::new(dc, self.dpi),
                &self.config,
                &now,
                w,
                h,
                self.ink,
                self.muted,
            );
            return;
        }
        let key = afterglow::content_key(&self.config, &now, w, h, self.dpi, self.ink, self.muted);
        if self
            .afterglow_content
            .as_ref()
            .is_none_or(|(k, _)| *k != key)
        {
            if let Some(layer) = Buffer::plain(
                (w * self.dpi).round() as i32,
                (h * self.dpi).round() as i32,
                self.dpi,
            ) {
                layer.replace_pixels(&vec![0; (layer.width * layer.height * 4) as usize]);
                let Some(batch) = render::AlphaBatch::new(layer.dc, self.dpi) else {
                    self.afterglow_content = None;
                    let c = Canvas::new(dc, self.dpi);
                    face::draw_content(&c, &self.config, &now, w, h, self.ink, self.muted, false);
                    face::draw_afterglow_motion(&c, &self.config, &now, w, h, self.ink);
                    return;
                };
                face::draw_content(
                    &Canvas::new(layer.dc, self.dpi),
                    &self.config,
                    &now,
                    w,
                    h,
                    self.ink,
                    self.muted,
                    false,
                );
                drop(batch);
                let mut rgba = layer.snapshot();
                // Image::from_rgba wraps PARGB. Retain coverage in RGB so cached
                // glyph edges stay antialiased rather than turning into white fringes.
                for p in rgba.chunks_exact_mut(4) {
                    p.swap(0, 2);
                }
                self.afterglow_content =
                    render::Image::from_rgba(layer.width as u32, layer.height as u32, rgba)
                        .map(|i| (key.clone(), i));
            }
        }
        let c = Canvas::new(dc, self.dpi);
        match &self.afterglow_content {
            Some((k, image)) if *k == key => c.image_snapped(image, [0., 0., w, h]),
            _ => face::draw_content(&c, &self.config, &now, w, h, self.ink, self.muted, false),
        }
        face::draw_afterglow_motion(&c, &self.config, &now, w, h, self.ink);
    }
    unsafe fn paint(&mut self) {
        let mut r = zeroed();
        GetClientRect(self.hwnd, &mut r);
        if r.right <= 0 || r.bottom <= 0 {
            return;
        }
        crate::fusion::prepare(self.hwnd, 34.);
        let Some(mut frame) = self
            .frame
            .take()
            .filter(|b| b.matches_geometry(r.right, r.bottom, self.dpi, 34.))
            .or_else(|| Buffer::new(r.right, r.bottom, self.dpi, 34.))
        else {
            return;
        };
        let (w, h) = self.dimensions();
        let pane = [4., 4., w - 4., h - 4.];
        let key = self.background_key();
        let reusable = self
            .background
            .as_ref()
            .is_some_and(|b| b.width == r.right && b.height == r.bottom && b.dpi == self.dpi);
        if !reusable || (!self.moving && key != self.key) {
            let brush = CreateSolidBrush(self.surface_color());
            FillRect(frame.dc, &r, brush);
            DeleteObject(brush);
            let wallpaper_available = self.glass.borrow().available();
            if self.config.frosted_glass && wallpaper_available {
                self.draw_frost(frame.dc, r.right, r.bottom);
            }
            let light = crate::accent::light(self.surface_color());
            let tint = if self.config.frosted_glass {
                self.config.tint
            } else {
                100.
            };
            {
                let c = Canvas::new(frame.dc, self.dpi);
                if self.config.palette == "theme" {
                    let key = format!("{}:{}:{}", self.config.theme, r.right, r.bottom);
                    if self.theme_texture.as_ref().is_none_or(|(k, _)| *k != key) {
                        self.theme_texture = render::Image::from_rgba(
                            256,
                            256,
                            visuals::pixels(&self.config, 256, 256, w / h),
                        )
                        .map(|i| (key, i));
                    }
                    if let Some((_, image)) = &self.theme_texture {
                        // Legacy artwork remains recognizable even when frost is on.
                        c.image_alpha(
                            image,
                            [0., 0., w, h],
                            if self.config.frosted_glass {
                                0.86 + 0.14 * tint / 100.
                            } else {
                                1.
                            },
                        );
                    }
                } else if !crate::surface::managed(&self.config.palette) {
                    c.round(
                        pane,
                        34.,
                        self.surface_color(),
                        (tint * 2.55).round() as u8,
                        false,
                        false,
                    );
                }
                crate::surface::paint(&c, pane, &self.config.palette, tint / 100.);
                c.coating(pane, 34.);
                if let Some(overlay) = Canvas::pane_overlay(
                    [r.right as u32, r.bottom as u32],
                    self.dpi,
                    tint / 100.,
                    light,
                ) {
                    c.image(&overlay, [0., 0., w, h]);
                }
                c.widget_corner_reflection(pane, 34., light);
            }
            frame.bevel_for(self.hwnd);
            let l = crate::contrast::sample(frame.dc, self.dpi, [20., 20., w - 20., h - 20.]);
            (self.ink, self.muted) = crate::contrast::inks(l, Some(self.ink));
            let Some(bg) = self
                .background
                .take()
                .filter(|b| b.width == r.right && b.height == r.bottom && b.dpi == self.dpi)
                .or_else(|| Buffer::plain(r.right, r.bottom, self.dpi))
            else {
                return;
            };
            bg.copy_from(frame.dc);
            self.background = Some(bg);
            self.key = key;
        } else {
            frame.copy_from(self.background.as_ref().unwrap().dc);
        }
        let batch = render::AlphaBatch::new(frame.dc, self.dpi);
        frame.background_alpha((self.config.opacity * 2.55).round() as u8);
        {
            let c = Canvas::new(frame.dc, self.dpi);
            self.paint_content(frame.dc, w, h);
            let header_action = |id: usize| {
                if id == 0 {
                    crate::settings_ui::HeaderAction::Settings
                } else {
                    crate::settings_ui::HeaderAction::Hide
                }
            };
            crate::settings_ui::draw_widget_header(
                frame.dc,
                self.hwnd,
                self.dpi,
                w,
                if self.config.theme == "sunset" {
                    ""
                } else {
                    "Clock"
                },
                self.ink,
                self.hover.map(header_action),
                self.pressed.map(header_action),
            );
            if self.config.theme == "sunset" {
                face::draw_location(&c, w, h, &self.location_label, self.ink);
            }
            if !self.error.is_empty() {
                c.round(
                    [12., h - 43., w - 12., h - 10.],
                    8.,
                    face::color("terminal"),
                    245,
                    false,
                    false,
                );
                c.text(
                    &self.error,
                    [20., h - 42., w - 20., h - 11.],
                    10.,
                    false,
                    0xe6e6ff,
                    0,
                );
            }
        }
        let preserve = batch.is_some();
        drop(batch);
        if let Some(bg) = &self.background {
            crate::settings_ui::retain_header_backing(&mut frame, bg, self.hwnd, w);
        }
        frame.present_with_alpha(self.hwnd, preserve);
        self.frame = Some(frame);
        if self.shadow.is_none() {
            self.shadow = WindowShadow::new(self.hwnd);
        }
        if let Some(shadow) = &mut self.shadow {
            shadow.sync(
                self.hwnd,
                self.dpi,
                (self.config.opacity * 2.55).round() as u8,
            );
        }
    }
}
fn xy(l: LPARAM) -> (i32, i32) {
    ((l as u16 as i16) as i32, ((l >> 16) as u16 as i16) as i32)
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_NCCALCSIZE || msg == WM_NCPAINT {
        return 0;
    }
    if msg == WM_NCACTIVATE {
        return 1;
    }
    if msg == WM_NCCREATE {
        let create = &*(lp as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<State>;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    if msg == WM_GETMINMAXINFO {
        let dpi = (*ptr)
            .try_borrow()
            .map(|s| s.dpi)
            .unwrap_or_else(|_| GetDpiForWindow(hwnd) as f32 / 96.);
        let info = &mut *(lp as *mut MINMAXINFO);
        info.ptMinTrackSize = POINT {
            x: (MIN_SIZE as f32 * dpi).ceil() as i32,
            y: (MIN_SIZE as f32 * dpi).ceil() as i32,
        };
        info.ptMaxTrackSize = POINT {
            x: (MAX_SIZE as f32 * dpi).round() as i32,
            y: (MAX_SIZE as f32 * dpi).round() as i32,
        };
        return 0;
    }
    let Ok(mut s) = (*ptr).try_borrow_mut() else {
        return DefWindowProcW(hwnd, msg, wp, lp);
    };
    match msg {
        WM_CREATE => {
            s.hwnd = hwnd;
            s.dpi = GetDpiForWindow(hwnd) as f32 / 96.;
            0
        }
        WM_PAINT => {
            let mut ps = zeroed();
            BeginPaint(hwnd, &mut ps);
            if IsWindowVisible(hwnd) != 0 {
                s.paint();
            }
            EndPaint(hwnd, &ps);
            0
        }
        WM_ERASEBKGND => 1,
        settings::SETTINGS_CHANGED => {
            s.settings_changed();
            0
        }
        WM_TIMER if wp == ANIMATION => {
            if afterglow::interval(
                &s.config,
                IsWindowVisible(hwnd) != 0 && IsIconic(hwnd) == 0,
                s.moving,
            )
            .is_some()
            {
                InvalidateRect(hwnd, null(), 0);
            } else {
                KillTimer(hwnd, ANIMATION);
            }
            0
        }
        WM_TIMER if wp == TICK => {
            s.arm();
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_TIMECHANGE => {
            s.location_label = model::location_label(&s.config.time_zone);
            s.arm();
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_DISPLAYCHANGE | WM_SETTINGCHANGE => {
            s.key.clear();
            s.frost_key.clear();
            s.glass.borrow_mut().refresh();
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_NCHITTEST => {
            let (x, y) = xy(lp);
            let mut p = POINT { x, y };
            ScreenToClient(hwnd, &mut p);
            let (w, h) = s.dimensions();
            let x = p.x as f32 / s.dpi;
            let y = p.y as f32 / s.dpi;
            if crate::compositor::distance(x, y, w, h, 34., 4.) > 0. {
                return HTTRANSPARENT as isize;
            }
            let edge = crate::geometry::widget_edge(x, y, w, h);
            let hit = if edge != HTCLIENT {
                edge
            } else if s.hit(x, y).is_some() {
                HTCLIENT
            } else {
                HTCAPTION
            };
            hit as isize
        }
        WM_ENTERSIZEMOVE => {
            s.moving = true;
            s.arm_animation();
            0
        }
        WM_EXITSIZEMOVE => {
            s.moving = false;
            s.arm_animation();
            s.key.clear();
            s.frost_key.clear();
            s.save_bounds();
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_WINDOWPOSCHANGED => {
            if let Some(shadow) = &mut s.shadow {
                shadow.follow_position(
                    &*(lp as *const WINDOWPOS),
                    GetDpiForWindow(hwnd) as f32 / 96.,
                );
            }
            drop(s);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_SHOWWINDOW if wp == 0 => {
            KillTimer(hwnd, ANIMATION);
            KillTimer(hwnd, TICK);
            drop(s);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_SIZE => {
            if wp as u32 == SIZE_MINIMIZED {
                KillTimer(hwnd, ANIMATION);
            } else {
                s.arm_animation();
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_DPICHANGED => {
            s.dpi = (wp as u16) as f32 / 96.;
            s.key.clear();
            s.frost_key.clear();
            let r = *(lp as *const RECT);
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
            0
        }
        WM_MOUSEMOVE => {
            let mut track = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            TrackMouseEvent(&mut track);
            let (x, y) = xy(lp);
            let hit = s.hit(x as f32 / s.dpi, y as f32 / s.dpi);
            if hit != s.hover {
                s.hover = hit;
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        message
            if message == windows_sys::Win32::UI::Controls::WM_MOUSELEAVE
                || message == WM_NCMOUSEMOVE =>
        {
            if s.hover.take().is_some() {
                InvalidateRect(hwnd, null(), 0);
            }
            drop(s);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = xy(lp);
            s.pressed = s.hit(x as f32 / s.dpi, y as f32 / s.dpi);
            SetCapture(hwnd);
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_LBUTTONUP => {
            let (x, y) = xy(lp);
            let pressed = s.pressed.take();
            let hit = s.hit(x as f32 / s.dpi, y as f32 / s.dpi);
            ReleaseCapture();
            if let Some(id) = pressed.filter(|p| Some(*p) == hit) {
                s.action(id);
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_CAPTURECHANGED | WM_CANCELMODE => {
            s.pressed = None;
            if msg == WM_CANCELMODE {
                ReleaseCapture();
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_CONTEXTMENU => {
            s.open_settings();
            0
        }
        WM_KEYDOWN => {
            match wp {
                0x1b => {
                    s.pressed = None;
                    ReleaseCapture();
                }
                0x25 | 0x27 => {
                    let i = THEMES
                        .iter()
                        .position(|t| t.0 == s.config.theme)
                        .unwrap_or(0);
                    s.action(10 + if wp == 0x27 { (i + 1) % 9 } else { (i + 8) % 9 });
                }
                _ => {}
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_CLOSE => {
            s.action(1);
            0
        }
        WM_DESTROY => {
            KillTimer(hwnd, TICK);
            KillTimer(hwnd, ANIMATION);
            s.settings.take();
            s.shadow.take();
            0
        }
        WM_NCDESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(s);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        _ => {
            drop(s);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
    }
}
#[cfg(test)]
#[path = "tests.rs"]
mod tests;

pub(crate) mod package;
