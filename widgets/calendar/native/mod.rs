//! Calendar shares the dock's UI thread, wallpaper source and native compositor.
mod artwork;
mod layout;
mod model;
mod reference;
mod typography;
mod view;
use crate::{
    compositor::{Buffer, WindowShadow},
    glass::Glass,
    model::wide,
    render::{self, Canvas},
};
use model::{Config, Date, MONTHS, THEMES};
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
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::WM_MOUSELEAVE,
        HiDpi::*,
        Input::KeyboardAndMouse::{
            ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
        },
        WindowsAndMessaging::*,
    },
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Previous,
    Next,
    Today,
    Day(usize),
    WeekDay(usize),
    Weekly(bool),
    Month(i32),
    Settings,
    Hide,
}
const APPLY_MODE: u32 = WM_APP + 82;
const SETTINGS_CHANGED: u32 = WM_APP + 84;
struct Button {
    rect: [f32; 4],
    action: Action,
}
struct Drag {
    edge: u32,
    start: POINT,
    bounds: RECT,
}
struct State {
    #[cfg(test)]
    size_events: u32,
    hwnd: HWND,
    root: PathBuf,
    config: Config,
    glass: Rc<RefCell<Glass>>,
    dpi: f32,
    today: Date,
    cursor: Date,
    selected: Date,
    keyboard_day: bool,
    settings_host: Option<crate::settings_ui::Host>,
    inspect: bool,
    smoke: bool,
    frame: Option<Buffer>,
    frame_radius: f32,
    artwork: artwork::Artwork,
    typo: typography::Typography,
    backdrop: Option<Buffer>,
    key: String,
    shadow: Option<WindowShadow>,
    pressed: Option<Action>,
    hover: Option<Action>,
    hover_alpha: f32,
    drag: Option<Drag>,
    ink: Option<u32>,
}
pub struct Host {
    state: Box<RefCell<State>>,
}
pub fn should_open(root: &Path) -> bool {
    Config::load(root).map(|c| c.open).unwrap_or(false)
}
impl Host {
    pub unsafe fn create(
        root: &Path,
        _glass: Rc<RefCell<Glass>>,
        inspect: bool,
        smoke: bool,
    ) -> Result<Self, String> {
        let config = Config::load(root)?;
        let today = Date::today();
        let class = wide("LumenNativeCalendarWindow");
        let instance = GetModuleHandleW(null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: class.as_ptr(),
            ..zeroed()
        };
        if RegisterClassW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let dpi = GetDpiForSystem() as f32 / 96.;
        let glass = Rc::new(RefCell::new(Glass::new(config.blur, dpi)));
        glass.borrow_mut().refresh();
        let mut state = Box::new(RefCell::new(State {
            #[cfg(test)]
            size_events: 0,
            hwnd: null_mut(),
            root: root.into(),
            config,
            glass,
            dpi,
            today,
            cursor: Date { day: 1, ..today },
            selected: today,
            keyboard_day: false,
            settings_host: None,
            inspect,
            smoke,
            frame: None,
            frame_radius: layout::CHROME_RADIUS,
            artwork: artwork::Artwork::default(),
            typo: typography::Typography::default(),
            backdrop: None,
            key: String::new(),
            shadow: None,
            pressed: None,
            hover: None,
            hover_alpha: 0.,
            drag: None,
            ink: None,
        }));
        let c = &state.borrow().config.clone();
        let mut work: RECT = zeroed();
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut work as *mut _ as _, 0);
        let w = (c.size[0] as f32 * dpi).round() as i32;
        let h = (c.size[1] as f32 * dpi).round() as i32;
        let p = c.position.unwrap_or([work.right - w - 32, work.top + 60]);
        let mut bounds = RECT {
            left: p[0],
            top: p[1],
            right: p[0] + w,
            bottom: p[1] + h,
        };
        let monitor = MonitorFromRect(&bounds, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = zeroed();
        info.cbSize = size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) != 0 {
            bounds.left = bounds.left.clamp(
                info.rcWork.left,
                (info.rcWork.right - w).max(info.rcWork.left),
            );
            bounds.top = bounds.top.clamp(
                info.rcWork.top,
                (info.rcWork.bottom - h).max(info.rcWork.top),
            );
        }
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED
                | if inspect {
                    WS_EX_APPWINDOW
                } else {
                    WS_EX_TOOLWINDOW
                },
            class.as_ptr(),
            wide("Lumen · Calendar").as_ptr(),
            WS_POPUP,
            bounds.left,
            bounds.top,
            w,
            h,
            null_mut(),
            null_mut(),
            instance,
            &mut *state as *mut _ as _,
        );
        if hwnd.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        // WM_NCCREATE installs our state before default title initialization.
        // Set it explicitly so window discovery/accessibility sees the name.
        SetWindowTextW(hwnd, wide("Lumen · Calendar").as_ptr());
        {
            let mut s = state.borrow_mut();
            s.hwnd = hwnd;
            s.dpi = GetDpiForWindow(hwnd) as f32 / 96.;
        }
        Ok(Self { state })
    }
    pub unsafe fn show(&mut self) {
        let hwnd = self.state.borrow().hwnd;
        {
            let mut s = self.state.borrow_mut();
            s.config.open = true;
            s.today = Date::today();
            if s.shadow.is_none() {
                s.shadow = WindowShadow::new(hwnd);
            }
            s.save();
        }
        SetTimer(hwnd, 1, 60_000, None);
        ShowWindow(
            hwnd,
            if self.state.borrow().inspect {
                SW_SHOW
            } else {
                SW_SHOWNOACTIVATE
            },
        );
        let (floating, inspect) = {
            let s = self.state.borrow();
            (s.config.floating, s.inspect)
        };
        crate::window::apply_window_mode(hwnd, floating, inspect);
        self.state.borrow_mut().paint();
        println!(
            "native-calendar-ready pid={} shared_wallpaper=true",
            std::process::id()
        );
    }
    pub fn hwnd(&self) -> HWND {
        self.state.borrow().hwnd
    }
    pub unsafe fn is_visible(&self) -> bool {
        IsWindowVisible(self.hwnd()) != 0
    }
    pub unsafe fn hide(&mut self) {
        self.state.borrow_mut().activate(Action::Hide);
    }
    pub unsafe fn show_settings(&mut self) {
        self.show();
        let mut s = self.state.borrow_mut();
        s.open_settings();
    }
    /// Apply an explicit General Settings action; absent fields stay unchanged.
    pub unsafe fn apply_common(&mut self, opacity: Option<f32>, floating: Option<bool>) {
        let mut s = self.state.borrow_mut();
        if let Some(opacity) = opacity.filter(|v| v.is_finite()) {
            s.config.opacity = opacity.clamp(0., 100.);
        }
        if let Some(floating) = floating {
            s.config.floating = floating;
        }
        s.save();
        s.key.clear();
        let rows = s.settings_rows();
        if let Some(host) = &mut s.settings_host {
            host.set_rows(rows);
        }
        let hwnd = s.hwnd;
        let inspect = s.inspect;
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
            self.state.borrow_mut().settings_host.take();
            let hwnd = self.state.borrow().hwnd;
            if IsWindow(hwnd) != 0 {
                DestroyWindow(hwnd);
            }
        }
    }
}
fn contains(r: [f32; 4], p: [f32; 2]) -> bool {
    p[0] >= r[0] && p[0] < r[2] && p[1] >= r[1] && p[1] < r[3]
}
fn point(l: LPARAM) -> POINT {
    POINT {
        x: l as i16 as i32,
        y: (l >> 16) as i16 as i32,
    }
}
fn color(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}
impl State {
    unsafe fn size(&self) -> [f32; 2] {
        let mut r: RECT = zeroed();
        GetClientRect(self.hwnd, &mut r);
        [r.right as f32 / self.dpi, r.bottom as f32 / self.dpi]
    }
    fn settings_rows(&self) -> Vec<crate::settings_ui::Row> {
        use crate::settings_ui::{Choice, Row};
        let c = &self.config;
        let mut rows = vec![
            Row::section("Background"),
            Row::toggle(
                1,
                "Frosted glass",
                "Blend this widget's design with the wallpaper",
                c.frosted,
            ),
            Row::slider(2, "Glass tint", c.tint, 0., 100., "%"),
            Row::slider(3, "Frost strength", c.frost_strength, 0., 100., "%"),
            Row::slider(4, "Blur size", c.blur, 0., 80., "px"),
            Row::slider(5, "Background opacity", c.opacity, 0., 100., "%"),
            Row::note("Background opacity lets desktop windows show through; dates stay clear."),
            Row::choices(
                6,
                "Surface palette",
                &c.palette,
                crate::settings_ui::palette_choices(),
            ),
            Row::section("Calendar"),
            Row::choices(
                7,
                "Theme",
                &c.theme,
                THEMES
                    .iter()
                    .map(|t| Choice::new(*t, format!("{}{}", t[..1].to_uppercase(), &t[1..]), None))
                    .collect(),
            ),
            Row::choices(
                8,
                "Week starts",
                if c.week_starts_on == 1 {
                    "monday"
                } else {
                    "sunday"
                },
                vec![
                    Choice::new("monday", "Monday", None),
                    Choice::new("sunday", "Sunday", None),
                ],
            ),
            Row::section("Window"),
        ];
        if c.theme == "lumen" {
            rows.insert(
                rows.len() - 2,
                Row::choices(
                    10,
                    "Lumen surface",
                    &c.lumen_surface,
                    vec![
                        Choice::new("glass", "Glass", None),
                        Choice::new("paper", "Paper", Some(0xf2ece6)),
                        Choice::new("midnight", "Midnight", Some(0x191919)),
                    ],
                ),
            );
        }
        rows
    }
    unsafe fn open_settings(&mut self) {
        self.prepare_settings(true);
    }
    unsafe fn prepare_settings(&mut self, show: bool) {
        let rows = self.settings_rows();
        if self.settings_host.is_none() {
            match crate::settings_ui::Host::create(
                self.hwnd,
                "Calendar settings",
                "Changes here apply only to Calendar.",
                self.glass.clone(),
                rows.clone(),
                SETTINGS_CHANGED,
            ) {
                Ok(host) => self.settings_host = Some(host),
                Err(e) => {
                    eprintln!("calendar-settings: {e}");
                    return;
                }
            }
        }
        let surface =
            crate::surface::color(&self.config.palette, self.glass.borrow().wallpaper_accent());
        if let Some(host) = &mut self.settings_host {
            host.set_rows(rows);
            host.set_surface(surface, crate::contrast::luminance(surface) > 0.42);
            if show {
                host.show();
            }
        }
    }
    unsafe fn settings_changed(&mut self) {
        let changes = self
            .settings_host
            .as_mut()
            .map(|h| h.take_changes())
            .unwrap_or_default();
        self.apply_settings_changes(changes);
    }
    unsafe fn apply_settings_changes(&mut self, changes: Vec<crate::settings_ui::Change>) {
        use crate::settings_ui::Value;
        for change in changes {
            match (change.id, change.value) {
                (1, Value::Bool(v)) => self.config.frosted = v,
                (2, Value::Number(v)) => self.config.tint = v.clamp(0., 100.),
                (3, Value::Number(v)) => self.config.frost_strength = v.clamp(0., 100.),
                (4, Value::Number(v)) => {
                    self.config.blur = v.clamp(0., 80.);
                    self.glass.borrow_mut().set_blur(self.config.blur, self.dpi);
                }
                (5, Value::Number(v)) => self.config.opacity = v.clamp(0., 100.),
                (6, Value::Choice(v)) if crate::surface::valid(&v) => {
                    self.config.palette = crate::surface::normalize(&v).unwrap_or(v)
                }
                (7, Value::Choice(v)) if THEMES.contains(&v.as_str()) => self.config.theme = v,
                (8, Value::Choice(v)) => {
                    self.config.week_starts_on = if v == "sunday" { 0 } else { 1 }
                }
                (9, Value::Bool(v)) => {
                    self.config.floating = v;
                    PostMessageW(self.hwnd, APPLY_MODE, 0, 0);
                }
                (10, Value::Choice(v)) if matches!(v.as_str(), "glass" | "paper" | "midnight") => {
                    self.config.lumen_surface = v
                }
                _ => {}
            }
        }
        self.key.clear();
        self.save();
        let rows = self.settings_rows();
        let surface =
            crate::surface::color(&self.config.palette, self.glass.borrow().wallpaper_accent());
        if let Some(host) = &mut self.settings_host {
            host.set_rows(rows);
            host.set_surface(surface, crate::contrast::luminance(surface) > 0.42);
        }
        InvalidateRect(self.hwnd, null(), 0);
    }
    fn save(&self) {
        if !self.smoke {
            if let Err(e) = self.config.save(&self.root) {
                eprintln!("native-calendar save: {e}");
            }
        }
    }
    unsafe fn bounds(&mut self) {
        let mut r: RECT = zeroed();
        GetWindowRect(self.hwnd, &mut r);
        self.config.position = Some([r.left, r.top]);
        self.config.size = [
            ((r.right - r.left) as f32 / self.dpi).round() as i32,
            ((r.bottom - r.top) as f32 / self.dpi).round() as i32,
        ];
        self.save();
    }
    unsafe fn buttons(&self) -> Vec<Button> {
        let [w, h] = self.size();
        let layout = layout::Layout::new(&self.config.theme, w, h);
        let header = crate::settings_ui::widget_header_hits(w);
        let mut b = vec![
            Button {
                rect: header[0].1,
                action: Action::Settings,
            },
            Button {
                rect: header[1].1,
                action: Action::Hide,
            },
            Button {
                rect: layout.previous,
                action: Action::Previous,
            },
            Button {
                rect: layout.next,
                action: Action::Next,
            },
            Button {
                rect: layout.today,
                action: Action::Today,
            },
        ];
        for i in 0..42 {
            b.push(Button {
                rect: layout.day(i),
                action: Action::Day(i),
            });
        }
        if layout::is_reference(&self.config.theme) && !layout::tiny(w, h) {
            let r = layout::ReferenceLayout::new(&self.config.theme, w, h);
            if self.config.theme == "mist" {
                b.push(Button {
                    rect: r.weekly,
                    action: Action::Weekly(true),
                });
                b.push(Button {
                    rect: r.monthly,
                    action: Action::Weekly(false),
                });
                if self.config.mist_weekly {
                    b.retain(|b| !matches!(b.action, Action::Day(_)));
                    for i in 0..7 {
                        b.push(Button {
                            rect: r.week_day(i),
                            action: Action::WeekDay(i),
                        });
                    }
                }
            }
            if self.config.theme == "agenda" {
                for i in 0..12 {
                    b.push(Button {
                        rect: r.months[i],
                        action: Action::Month(i as i32 + 1),
                    });
                }
            }
        }
        b
    }
    unsafe fn hit(&self, p: POINT) -> Option<Action> {
        self.buttons()
            .iter()
            .find(|b| contains(b.rect, [p.x as f32 / self.dpi, p.y as f32 / self.dpi]))
            .map(|b| b.action)
    }
    unsafe fn activate(&mut self, a: Action) {
        match a {
            Action::Previous | Action::Next => {
                let delta = if a == Action::Previous { -1 } else { 1 };
                let [w, h] = self.size();
                let tiny = layout::tiny(w, h);
                if self.config.theme == "mist" && self.config.mist_weekly && !tiny {
                    self.selected = self.selected.add_days(delta * 7);
                    self.cursor = Date {
                        day: 1,
                        ..self.selected
                    };
                } else {
                    self.cursor =
                        self.cursor
                            .month_offset(if self.config.theme == "agenda" && !tiny {
                                delta * 12
                            } else {
                                delta
                            });
                    if layout::is_reference(&self.config.theme) {
                        self.selected = Date {
                            day: self.selected.day.min(self.cursor.days_in_month()),
                            ..self.cursor
                        };
                    }
                }
            }
            Action::Today => {
                self.today = Date::today();
                self.selected = self.today;
                self.cursor = Date {
                    day: 1,
                    ..self.today
                };
            }
            Action::Day(i) => {
                self.selected = self.cursor.matrix(self.config.week_starts_on == 1)[i];
                if layout::is_reference(&self.config.theme) {
                    self.cursor = Date {
                        day: 1,
                        ..self.selected
                    };
                }
            }
            Action::WeekDay(i) => {
                self.selected = self.selected.week(self.config.week_starts_on == 1)[i];
                self.cursor = Date {
                    day: 1,
                    ..self.selected
                };
            }
            Action::Weekly(weekly) => self.config.mist_weekly = weekly,
            Action::Month(month) => {
                self.cursor.month = month;
                self.selected = Date {
                    day: self.selected.day.min(self.cursor.days_in_month()),
                    ..self.cursor
                };
            }
            Action::Settings => {
                self.open_settings();
            }
            Action::Hide => {
                if let Some(settings) = &mut self.settings_host {
                    settings.hide();
                }
                self.config.open = false;
                KillTimer(self.hwnd, 1);
                KillTimer(self.hwnd, 2);
                self.pressed = None;
                self.hover = None;
                self.drag = None;
                self.save();
                ShowWindow(self.hwnd, SW_HIDE);
                self.frame = None;
                self.artwork.clear();
                self.typo.clear();
                self.backdrop = None;
                self.shadow = None;
                self.key.clear();
                return;
            }
        }
        self.save();
        InvalidateRect(self.hwnd, null(), 0);
    }
    unsafe fn paint(&mut self) {
        let mut ps = zeroed();
        BeginPaint(self.hwnd, &mut ps);
        let mut r: RECT = zeroed();
        GetClientRect(self.hwnd, &mut r);
        let [w, h] = self.size();
        let radius = layout::Layout::new(&self.config.theme, w, h).outer_radius;
        if radius != self.frame_radius {
            self.frame = None;
            self.backdrop = None;
            self.frame_radius = radius;
        }
        crate::fusion::prepare(self.hwnd, radius);
        let Some(mut frame) = self
            .frame
            .take()
            .filter(|f| f.matches_geometry(r.right, r.bottom, self.dpi, radius))
            .or_else(|| Buffer::new(r.right, r.bottom, self.dpi, radius))
        else {
            EndPaint(self.hwnd, &ps);
            return;
        };
        let mut position: RECT = zeroed();
        GetWindowRect(self.hwnd, &mut position);
        let theme = self.config.theme.as_str();
        let accent =
            crate::surface::color(&self.config.palette, self.glass.borrow().wallpaper_accent());
        let light = matches!(theme, "almanac" | "agenda" | "soft");
        let material = match theme {
            "lumen" if self.config.palette == "wallpaper-auto" => {
                match self.config.lumen_surface.as_str() {
                    "paper" => Some(artwork::Material::LumenPaper),
                    "midnight" => Some(artwork::Material::LumenMidnight),
                    _ => None,
                }
            }
            "almanac" => Some(artwork::Material::Almanac),
            "blocks" => Some(artwork::Material::Blocks),
            "pop" => Some(artwork::Material::Pop),
            "prism" => Some(artwork::Material::Prism),
            "grove" => Some(artwork::Material::Grove),
            "mist" => Some(artwork::Material::Mist),
            "agenda" => Some(artwork::Material::Agenda),
            "soft" => Some(artwork::Material::Soft),
            _ => None,
        };
        let base = material
            .map(|m| {
                let c = artwork::sample(m, 0.5, 0.5, w, h);
                color(c[0] as u32, c[1] as u32, c[2] as u32)
            })
            .unwrap_or(color(18, 22, 30));
        let key = format!(
            "{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
            position.left,
            position.top,
            r.right,
            r.bottom,
            self.dpi,
            theme,
            self.config.frosted,
            self.config.tint,
            self.config.opacity,
            self.glass.borrow().revision(),
            crate::fusion::revision(self.hwnd),
            self.config.blur,
            self.config.frost_strength,
            self.config.palette,
            self.config.lumen_surface
        );
        let frozen = self.drag.is_some()
            && self
                .backdrop
                .as_ref()
                .is_some_and(|f| f.width == r.right && f.height == r.bottom && f.dpi == self.dpi);
        if self.key != key && !frozen || self.backdrop.is_none() {
            let brush = CreateSolidBrush(base);
            FillRect(frame.dc, &r, brush);
            DeleteObject(brush);
            if self.config.frosted {
                self.glass.borrow().draw_blended(
                    frame.dc,
                    self.hwnd,
                    r.right,
                    r.bottom,
                    (self.config.frost_strength * 2.55).round() as u8,
                );
            }
            // Fill the complete backing. The compositor owns the single rounded
            // contour; independently rounding this material multiplies edge alpha.
            let pane = layout::background_rect(w, h);
            let chrome = [4., 4., w - 4., h - 4.];
            let blend = if self.config.frosted {
                self.config.tint / 100.
            } else {
                1.
            };
            if let Some(material) = material {
                self.artwork
                    .draw(frame.dc, self.dpi, pane, 0., material, blend);
            }
            let c = Canvas::new(frame.dc, self.dpi);
            if !crate::surface::managed(&self.config.palette)
                && (theme == "lumen" && material.is_none()
                    || self.config.palette != "wallpaper-auto")
            {
                c.round(pane, 0., accent, (blend * 255.).round() as u8, false, false);
            }
            crate::surface::paint(&c, pane, &self.config.palette, blend);
            self.paint_agenda_sidebar(frame.dc, w, h);
            c.coating(chrome, radius);
            if let Some(overlay) = Canvas::pane_overlay(
                [r.right as u32, r.bottom as u32],
                self.dpi,
                self.config.tint / 100.,
                light,
            ) {
                c.image(&overlay, [0., 0., w, h]);
            }
            c.widget_corner_reflection(chrome, radius, light);
            drop(c);
            frame.bevel_for(self.hwnd);
            self.ink = Some(
                crate::contrast::inks(
                    crate::contrast::sample(frame.dc, self.dpi, [20., 48., w - 20., h - 20.]),
                    self.ink,
                )
                .0,
            );
            if let Some(back) = Buffer::plain(r.right, r.bottom, self.dpi) {
                back.copy_from(frame.dc);
                self.backdrop = Some(back);
            }
            self.key = key;
        } else if let Some(back) = &self.backdrop {
            frame.copy_from(back.dc);
        }
        frame.background_alpha((self.config.opacity * 2.55).round() as u8);
        let _batch = render::AlphaBatch::new(frame.dc, self.dpi);
        let ink = self.ink.unwrap_or(color(248, 250, 253));
        self.paint_calendar(frame.dc, w, h, ink, accent);
        drop(_batch);
        if let Some(bg) = &self.backdrop {
            for button in self.buttons() {
                let selected = matches!(button.action, Action::Today)
                    || matches!(button.action,Action::Day(i) if self.cursor.matrix(self.config.week_starts_on==1)[i]==self.selected)
                    || self.hover == Some(button.action)
                    || self.pressed == Some(button.action);
                if selected && !matches!(button.action, Action::Settings | Action::Hide) {
                    let radius = if layout::tiny(w, h) {
                        4.
                    } else if matches!(self.config.theme.as_str(), "pop" | "prism" | "grove") {
                        (button.rect[3] - button.rect[1]) / 2.
                    } else {
                        8.
                    };
                    frame.retain_control_backing(
                        bg,
                        button.rect,
                        radius,
                        if self.hover == Some(button.action)
                            && !matches!(button.action, Action::Today)
                        {
                            self.hover_alpha
                        } else {
                            1.
                        },
                    );
                }
            }
            crate::settings_ui::retain_header_backing(&mut frame, bg, self.hwnd, w);
        }
        frame.present_with_alpha(self.hwnd, true);
        self.frame = Some(frame);
        if let Some(shadow) = &mut self.shadow {
            shadow.sync_with_radius(
                self.hwnd,
                self.dpi,
                (self.config.opacity * 2.55).round() as u8,
                radius,
            );
        }
        EndPaint(self.hwnd, &ps);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = &*(lp as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        return 1;
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<State>;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let Ok(mut s) = (*ptr).try_borrow_mut() else {
        return DefWindowProcW(hwnd, msg, wp, lp);
    };
    match msg {
        SETTINGS_CHANGED => {
            s.settings_changed();
            0
        }
        WM_GETMINMAXINFO => {
            let p = &mut *(lp as *mut MINMAXINFO);
            p.ptMinTrackSize = POINT {
                x: (model::MIN_WIDTH as f32 * s.dpi).ceil() as i32,
                y: (model::MIN_HEIGHT as f32 * s.dpi).ceil() as i32,
            };
            0
        }
        APPLY_MODE => {
            let floating = s.config.floating;
            let inspect = s.inspect;
            drop(s);
            crate::window::apply_window_mode(hwnd, floating, inspect);
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_PAINT => {
            s.paint();
            0
        }
        WM_ERASEBKGND => 1,
        WM_CLOSE => {
            s.activate(Action::Hide);
            0
        }
        WM_NCHITTEST => {
            let mut p = point(lp);
            ScreenToClient(hwnd, &mut p);
            let [w, h] = s.size();
            crate::geometry::widget_edge(p.x as f32 / s.dpi, p.y as f32 / s.dpi, w, h) as isize
        }
        WM_NCLBUTTONDOWN if wp as u32 != HTCLIENT => {
            let mut r = zeroed();
            GetWindowRect(hwnd, &mut r);
            let mut p = zeroed();
            GetCursorPos(&mut p);
            s.drag = Some(Drag {
                edge: wp as u32,
                start: p,
                bounds: r,
            });
            SetCapture(hwnd);
            0
        }
        WM_LBUTTONDOWN => {
            s.keyboard_day = false;
            let p = point(lp);
            s.pressed = s.hit(p);
            if s.pressed.is_none() {
                let mut r = zeroed();
                GetWindowRect(hwnd, &mut r);
                let mut p = zeroed();
                GetCursorPos(&mut p);
                s.drag = Some(Drag {
                    edge: HTCAPTION,
                    start: p,
                    bounds: r,
                });
            }
            SetCapture(hwnd);
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_MOUSEMOVE => {
            let p = point(lp);
            if let Some(d) = &s.drag {
                let mut pos = zeroed();
                GetCursorPos(&mut pos);
                let dx = pos.x - d.start.x;
                let dy = pos.y - d.start.y;
                let mut r = if d.edge == HTCAPTION {
                    RECT {
                        left: d.bounds.left + dx,
                        top: d.bounds.top + dy,
                        right: d.bounds.right + dx,
                        bottom: d.bounds.bottom + dy,
                    }
                } else {
                    crate::geometry::resized(
                        d.bounds,
                        d.edge,
                        dx,
                        dy,
                        [
                            (model::MIN_WIDTH as f32 * s.dpi).ceil() as i32,
                            (model::MIN_HEIGHT as f32 * s.dpi).ceil() as i32,
                        ],
                    )
                };
                if d.edge != HTCAPTION {
                    crate::fusion::snap_resize_rect(
                        hwnd,
                        &mut r,
                        d.edge,
                        [
                            (model::MIN_WIDTH as f32 * s.dpi).ceil() as i32,
                            (model::MIN_HEIGHT as f32 * s.dpi).ceil() as i32,
                        ],
                    );
                }
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
                let Ok(mut s) = (*ptr).try_borrow_mut() else {
                    return 0;
                };
                let dpi = s.dpi;
                let opacity = (s.config.opacity * 2.55).round() as u8;
                let radius = s.frame_radius;
                if let Some(shadow) = &mut s.shadow {
                    shadow.sync_with_radius(hwnd, dpi, opacity, radius);
                }
                InvalidateRect(hwnd, null(), 0);
            } else {
                let hover = s.hit(p);
                if hover != s.hover {
                    s.hover = hover;
                    s.hover_alpha = 0.;
                    SetTimer(hwnd, 2, 16, None);
                }
                let mut track = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                TrackMouseEvent(&mut track);
            }
            0
        }
        WM_LBUTTONUP => {
            let a = s.pressed.take();
            let moved = s.drag.take().is_some();
            ReleaseCapture();
            if moved {
                s.key.clear();
                s.bounds();
            } else if let Some(a) = a {
                if s.hit(point(lp)) == Some(a) {
                    s.activate(a);
                }
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_CAPTURECHANGED | WM_CANCELMODE => {
            if s.drag.take().is_some() {
                s.key.clear();
                s.bounds();
            }
            s.pressed = None;
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_MOUSELEAVE => {
            s.hover = None;
            s.hover_alpha = 0.;
            KillTimer(hwnd, 2);
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_TIMER if wp == 2 => {
            s.hover_alpha = (s.hover_alpha + 0.13).min(1.);
            if s.hover_alpha >= 1. {
                KillTimer(hwnd, 2);
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_TIMER if wp == 1 => {
            let next = Date::today();
            let changed = next != s.today;
            s.today = next;
            let wallpaper = s.glass.borrow_mut().refresh();
            if changed || wallpaper {
                s.key.clear();
                InvalidateRect(hwnd, null(), 0);
            }
            if !s.config.floating && !s.inspect && IsWindowVisible(hwnd) != 0 {
                crate::window::pin_to_desktop(hwnd);
            }
            0
        }
        WM_KEYDOWN => {
            match wp as u32 {
                0x1B => {
                    s.pressed = None;

                    if let Some(d) = s.drag.take() {
                        s.key.clear();
                        drop(s);
                        SetWindowPos(
                            hwnd,
                            null_mut(),
                            d.bounds.left,
                            d.bounds.top,
                            d.bounds.right - d.bounds.left,
                            d.bounds.bottom - d.bounds.top,
                            SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                        ReleaseCapture();
                    }
                }
                key @ (0x21 | 0x22) => {
                    s.cursor = s.cursor.month_offset(if key == 0x21 { -1 } else { 1 });
                    if layout::is_reference(&s.config.theme) {
                        s.selected = Date {
                            day: s.selected.day.min(s.cursor.days_in_month()),
                            ..s.cursor
                        };
                    }
                }
                0x24 => {
                    s.keyboard_day = true;
                    s.activate(Action::Today);
                }
                key @ (0x25..=0x28) => {
                    s.keyboard_day = true;
                    s.selected = s.selected.add_days(match key {
                        0x25 => -1,
                        0x26 => -7,
                        0x27 => 1,
                        _ => 7,
                    });
                    s.cursor = Date {
                        day: 1,
                        ..s.selected
                    };
                }
                _ => {}
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_SIZE => {
            #[cfg(test)]
            {
                s.size_events += 1;
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_KILLFOCUS => {
            s.keyboard_day = false;
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_WINDOWPOSCHANGED => {
            let dpi = s.dpi;
            if let Some(shadow) = &mut s.shadow {
                shadow.follow_position(&*(lp as *const WINDOWPOS), dpi);
            }
            drop(s);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_DPICHANGED => {
            s.dpi = (wp as u16) as f32 / 96.;
            s.key.clear();
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
            if let Ok(mut s) = (*ptr).try_borrow_mut() {
                s.bounds();
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_DISPLAYCHANGE | WM_SETTINGCHANGE => {
            s.glass.borrow_mut().refresh();
            s.key.clear();
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_NCDESTROY => {
            KillTimer(hwnd, 1);
            KillTimer(hwnd, 2);
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
mod tests {
    use super::*;
    #[test]
    fn compact_themes_keep_all_date_targets_inside_bounds_and_blur_is_private() {
        let _lock = crate::native_ui_test_lock();
        let _runtime = render::Runtime::start().unwrap();
        let root =
            std::env::temp_dir().join(format!("lumen-calendar-compact-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        unsafe {
            let shared = Rc::new(RefCell::new(Glass::new(13., 1.)));
            let revision = shared.borrow().revision();
            let host = Host::create(&root, shared.clone(), true, true).unwrap();
            assert!(!Rc::ptr_eq(&host.state.borrow().glass, &shared));
            for dpi in [1., 1.25, 1.5, 2.] {
                for size in [
                    [168, 190],
                    [168, 520],
                    [800, 190],
                    [240, 280],
                    [240, 520],
                    [800, 280],
                    [420, 460],
                ] {
                    host.state.borrow_mut().dpi = dpi;
                    SetWindowPos(
                        host.hwnd(),
                        null_mut(),
                        0,
                        0,
                        (size[0] as f32 * dpi).round() as i32,
                        (size[1] as f32 * dpi).round() as i32,
                        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                    for theme in THEMES {
                        let mut s = host.state.borrow_mut();
                        s.config.theme = theme.into();
                        let [w, h] = s.size();
                        s.paint();
                        let days: Vec<_> = s
                            .buttons()
                            .into_iter()
                            .filter(|b| matches!(b.action, Action::Day(_) | Action::WeekDay(_)))
                            .collect();
                        let tiny = layout::tiny(w, h);
                        assert_eq!(days.len(), if theme == "mist" && !tiny { 7 } else { 42 });
                        let minimum_day = if tiny {
                            [18., 12.]
                        } else if layout::is_reference(theme) {
                            [24., 12.]
                        } else {
                            [24., 20.]
                        };
                        for day in days {
                            assert!(
                                day.rect[0] >= 10.
                                    && day.rect[2] <= w - 10.
                                    && day.rect[1] >= 48.
                                    && day.rect[3] <= h - 10.
                            );
                            assert!(
                                day.rect[2] - day.rect[0] >= minimum_day[0]
                                    && day.rect[3] - day.rect[1] >= minimum_day[1]
                            );
                        }
                        assert!(s
                            .frame
                            .as_ref()
                            .unwrap()
                            .snapshot()
                            .chunks_exact(4)
                            .any(|p| p[3] > 200));
                    }
                }
            }
            let mut s = host.state.borrow_mut();
            let size = s.size();
            let selected = s.selected;
            s.apply_settings_changes(vec![
                crate::settings_ui::Change {
                    id: 4,
                    value: crate::settings_ui::Value::Number(42.),
                },
                crate::settings_ui::Change {
                    id: 5,
                    value: crate::settings_ui::Value::Number(33.),
                },
                crate::settings_ui::Change {
                    id: 6,
                    value: crate::settings_ui::Value::Choice("gradient:afterglow".into()),
                },
            ]);
            assert_eq!(s.config.blur, 42.);
            assert_eq!(s.config.opacity, 33.);
            assert_eq!(s.config.palette, "gradient:afterglow");
            s.paint();
            s.apply_settings_changes(vec![crate::settings_ui::Change {
                id: 6,
                value: crate::settings_ui::Value::Choice("custom:#12abef".into()),
            }]);
            assert_eq!(s.config.palette, "custom:#12ABEF");
            s.paint();
            assert_eq!(s.size(), size);
            assert_eq!(s.selected, selected);
            assert_eq!(shared.borrow().revision(), revision);
            assert_eq!(s.config.theme, *THEMES.last().unwrap());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn hidden_calendar_settings_preserve_geometry_and_hide_releases_surfaces() {
        let _ui = crate::native_ui_test_lock();
        let _runtime = render::Runtime::start().unwrap();
        let root =
            std::env::temp_dir().join(format!("lumen-calendar-hidden-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        unsafe {
            let glass = Rc::new(RefCell::new(Glass::new(13., 1.)));
            let host = Host::create(&root, glass, true, true).unwrap();
            let mut title = [0u16; 80];
            let length = GetWindowTextW(host.hwnd(), title.as_mut_ptr(), title.len() as i32);
            assert_eq!(
                String::from_utf16_lossy(&title[..length as usize]),
                "Lumen · Calendar"
            );
            // SetWindowPos synchronously enters WM_WINDOWPOSCHANGED, whose
            // default handling sends WM_SIZE back into this same state cell.
            let before_events = host.state.borrow().size_events;
            let mut bounds: RECT = zeroed();
            GetWindowRect(host.hwnd(), &mut bounds);
            SetWindowPos(
                host.hwnd(),
                null_mut(),
                0,
                0,
                bounds.right - bounds.left + 20,
                bounds.bottom - bounds.top + 20,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            assert!(
                host.state.borrow().size_events > before_events,
                "nested WM_SIZE must not be suppressed by a RefCell guard"
            );
            let mut s = host.state.borrow_mut();
            let before = s.size();
            s.prepare_settings(false);
            assert_eq!(s.size(), before);
            let settings_hwnd = s.settings_host.as_ref().unwrap().hwnd();
            assert_ne!(settings_hwnd, s.hwnd);
            s.prepare_settings(false);
            assert_eq!(s.settings_host.as_ref().unwrap().hwnd(), settings_hwnd);
            s.paint();
            assert!(s.frame.is_some() && s.backdrop.is_some());
            s.activate(Action::Hide);
            assert!(s.frame.is_none() && s.backdrop.is_none() && s.shadow.is_none());
            assert!(!s.config.open);
        }
        assert!(!root.join("config/native-calendar.json").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn native_calendar_renders_themes_settings_and_transparency_without_writing_old_state() {
        let _ui = crate::native_ui_test_lock();
        let _runtime = render::Runtime::start().unwrap();
        let root =
            std::env::temp_dir().join(format!("lumen-calendar-render-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        unsafe {
            let glass = Rc::new(RefCell::new(Glass::new(13., 1.)));
            glass.borrow_mut().refresh();
            let host = Host::create(&root, glass, true, true).unwrap();
            let hit_dpi = host.state.borrow().dpi;
            crate::geometry::assert_widget_corner_hits(host.hwnd(), hit_dpi);
            let hwnd = host.state.borrow().hwnd;
            assert_eq!(IsWindowVisible(hwnd), 0);
            {
                let mut state = host.state.borrow_mut();
                for i in 0..THEMES.len() {
                    state.config.theme = THEMES[i].into();
                    state.paint();
                    let frame = state.frame.as_ref().unwrap();
                    assert!(frame.snapshot().chunks_exact(4).any(|p| p[3] > 200));
                    assert_eq!(
                        state
                            .buttons()
                            .iter()
                            .filter(|b| matches!(b.action, Action::Day(_) | Action::WeekDay(_)))
                            .count(),
                        if state.config.theme == "mist" && state.config.mist_weekly {
                            7
                        } else {
                            42
                        }
                    );
                }
                let today = state.today;
                state.activate(Action::Next);
                state.activate(Action::Day(15));
                state.activate(Action::Today);
                assert_eq!(state.selected, today);
                assert_eq!(state.cursor.month, today.month);
                state.config.week_starts_on = 0;
                assert_eq!(state.config.week_starts_on, 0);
                assert!(state
                    .settings_rows()
                    .iter()
                    .any(|r| matches!(r, crate::settings_ui::Row::Slider { id: 5, .. })));
                state.config.opacity = 0.;
                state.paint();
                let pixels = state.frame.as_ref().unwrap().snapshot();
                assert!(pixels
                    .chunks_exact(4)
                    .all(|p| p[..3].iter().all(|v| *v <= p[3].saturating_add(1))));
                assert!(pixels.chunks_exact(4).any(|p| p[3] == 255));
                assert!(pixels.chunks_exact(4).any(|p| p[3] == 1));
                state.activate(Action::Hide);
            }
            assert_eq!(IsWindowVisible(hwnd), 0);
            drop(host);
            assert_eq!(IsWindow(hwnd), 0);
        }
        assert!(!root.join("config/native-calendar.json").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "Writes a development preview from the widget-owned render buffer"]
    fn render_calendar_preview() {
        let _ui = crate::native_ui_test_lock();
        let _runtime = render::Runtime::start().unwrap();
        let root = crate::model::root();
        unsafe {
            let glass = Rc::new(RefCell::new(Glass::new(13., 1.)));
            glass.borrow_mut().refresh();
            let host = Host::create(&root, glass, true, true).unwrap();
            let mut sheet = image::RgbaImage::new(1260, 1380);
            for i in 0..THEMES.len() {
                let mut s = host.state.borrow_mut();
                s.dpi = 1.;
                s.config.size = [420, 460];
                SetWindowPos(
                    s.hwnd,
                    null_mut(),
                    80,
                    80,
                    420,
                    460,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                s.config.opacity = 100.;
                s.config.tint = 18.;
                s.config.theme = THEMES[i].into();
                s.paint();
                let frame = s.frame.as_ref().unwrap();
                let mut pixels = frame.snapshot();
                for p in pixels.chunks_exact_mut(4) {
                    p.swap(0, 2);
                    let alpha = p[3] as u32;
                    if alpha > 0 {
                        for c in &mut p[..3] {
                            *c = ((*c as u32 * 255 + alpha / 2) / alpha).min(255) as u8;
                        }
                    }
                }
                let image =
                    image::RgbaImage::from_raw(frame.width as u32, frame.height as u32, pixels)
                        .unwrap();
                image::imageops::overlay(
                    &mut sheet,
                    &image,
                    (i % 3) as i64 * 420,
                    (i / 3) as i64 * 460,
                );
            }
            std::fs::create_dir_all(root.join(".scratch")).unwrap();
            sheet
                .save(root.join(".scratch/calendar-native-themes.png"))
                .unwrap();
        }
    }
}

pub(crate) mod package;
