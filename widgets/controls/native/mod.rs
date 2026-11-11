//! Native Control Center. Same UI thread/compositor/settings kit as the other packages.
mod backend;
mod display_restore;
mod icons;
mod lenses;
mod model;
pub(crate) mod package;
use crate::{
    compositor::{Buffer, WindowShadow},
    glass::Glass,
    model::wide,
    render::{self, Canvas},
    settings_ui::{self, Choice, HeaderAction, Row, Value},
};
use backend::{Command, Snapshot, Worker};
use model::{Config, Control, Kind, KINDS};
use std::{
    cell::RefCell,
    mem::{size_of, zeroed},
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    rc::Rc,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Controls::WM_MOUSELEAVE, HiDpi::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};
const SETTINGS: u32 = WM_APP + 170;
const EDIT: u32 = WM_APP + 171;
const CHOOSE: u32 = WM_APP + 172;
const RENAME: u32 = WM_APP + 173;
const MANAGER_TOGGLE: u32 = WM_APP + 176;
const MANAGER_SYNC: u32 = WM_APP + 177;
const MANAGER_SETTINGS: u32 = WM_APP + 178;
#[derive(Clone, Copy, Debug, PartialEq)]
enum Action {
    Control(usize),
    Device(usize, usize),
    Mute(usize),
    Slider(usize),
    Edit,
    Add,
    Candidate(usize),
    Search,
    Scroll(bool),
    Remove(usize),
    Rename(usize),
    Move(usize),
    Settings,
    Hide,
}
struct Drag {
    edge: u32,
    start: POINT,
    bounds: RECT,
}
pub struct Host {
    state: Box<RefCell<State>>,
}
struct State {
    hwnd: HWND,
    root: PathBuf,
    config: Config,
    icons: icons::Icons,
    queued: Option<Command>,
    glass: Rc<RefCell<Glass>>,
    dpi: f32,
    inspect: bool,
    smoke: bool,
    worker: Option<Worker>,
    snapshot: Snapshot,
    pending: bool,
    last_poll: Instant,
    error: String,
    settings: Option<settings_ui::Host>,
    editor: Option<settings_ui::Host>,
    chooser: Option<settings_ui::Host>,
    candidates: Vec<Control>,
    choosing: Option<usize>,
    adding: bool,
    frame: Option<Buffer>,
    backdrop: Option<Buffer>,
    fade_base: Option<Buffer>,
    key: String,
    shadow: Option<WindowShadow>,
    ink: Option<u32>,
    hits: Vec<(Action, [f32; 4])>,
    pressed: Option<Action>,
    hover: Option<Action>,
    drag: Option<Drag>,
    slider: Option<(usize, f32)>,
    scroll: f32,
    scroll_target: f32,
    manager_window: Option<Box<Host>>,
    manager_parent: HWND,
    manager_query: String,
    search_focus: bool,
    scroll_drag: Option<(bool, f32)>,
    editing: bool,
    manager: bool,
    moving: Option<(usize, POINT, usize, bool)>,
    button_lenses: lenses::Lenses,
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
        let class = wide("LumenNativeControlCenter");
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
        let glass = Rc::new(RefCell::new(Glass::new(config.blur, dpi)));
        glass.borrow_mut().refresh();
        let mut work: RECT = zeroed();
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut work as *mut _ as _, 0);
        let w = (config.size[0] as f32 * dpi) as i32;
        let h = (config.size[1] as f32 * dpi) as i32;
        let p = config.position.unwrap_or([work.left + 48, work.top + 80]);
        let mut bounds = RECT {
            left: p[0],
            top: p[1],
            right: p[0] + w,
            bottom: p[1] + h,
        };
        let mut info: MONITORINFO = zeroed();
        info.cbSize = size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(
            MonitorFromRect(&bounds, MONITOR_DEFAULTTONEAREST),
            &mut info,
        );
        bounds.left = bounds.left.clamp(
            info.rcWork.left,
            (info.rcWork.right - w).max(info.rcWork.left),
        );
        bounds.top = bounds.top.clamp(
            info.rcWork.top,
            (info.rcWork.bottom - h).max(info.rcWork.top),
        );
        let mut state = Box::new(RefCell::new(State {
            hwnd: null_mut(),
            root: root.into(),
            config,
            icons: icons::Icons::default(),
            queued: None,
            glass,
            dpi,
            inspect,
            smoke,
            worker: if smoke {
                None
            } else {
                Some(Worker::new(root)?)
            },
            snapshot: Snapshot::default(),
            pending: false,
            last_poll: Instant::now() - Duration::from_secs(10),
            error: String::new(),
            settings: None,
            editor: None,
            chooser: None,
            candidates: vec![],
            choosing: None,
            adding: false,
            frame: None,
            backdrop: None,
            fade_base: None,
            key: String::new(),
            shadow: None,
            ink: None,
            hits: vec![],
            pressed: None,
            hover: None,
            drag: None,
            slider: None,
            scroll: 0.,
            scroll_target: 0.,
            manager_window: None,
            manager_parent: null_mut(),
            manager_query: String::new(),
            search_focus: false,
            scroll_drag: None,
            editing: false,
            manager: false,
            moving: None,
            button_lenses: lenses::Lenses::default(),
        }));
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED
                | if inspect {
                    WS_EX_APPWINDOW
                } else {
                    WS_EX_TOOLWINDOW
                },
            class.as_ptr(),
            wide("Lumen · Control Center").as_ptr(),
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
        state.borrow_mut().hwnd = hwnd;
        state.borrow_mut().dpi = GetDpiForWindow(hwnd) as f32 / 96.;
        SetWindowTextW(hwnd, wide("Lumen · Control Center").as_ptr());
        Ok(Self { state })
    }
    pub fn hwnd(&self) -> HWND {
        self.state.borrow().hwnd
    }
    pub unsafe fn show(&mut self) {
        let hwnd = self.hwnd();
        {
            let mut s = self.state.borrow_mut();
            s.config.open = true;
            s.save();
            s.read();
            s.shadow = WindowShadow::new(hwnd);
        }
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let (f, i) = {
            let s = self.state.borrow();
            (s.config.floating, s.inspect)
        };
        crate::window::apply_window_mode(hwnd, f, i);
        SetTimer(hwnd, 1, 200, None);
        self.state.borrow_mut().paint();
        println!(
            "native-controls-ready pid={} runtime=rust",
            std::process::id()
        );
    }
    pub unsafe fn hide(&mut self) {
        self.state.borrow_mut().hide();
    }
    pub fn is_visible(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd()) != 0 }
    }
    pub unsafe fn show_settings(&mut self) {
        self.show();
        self.state.borrow_mut().open_settings();
    }
    pub unsafe fn apply_common(&mut self, opacity: Option<f32>, floating: Option<bool>) {
        let mut s = self.state.borrow_mut();
        if let Some(v) = opacity {
            s.config.opacity = v.clamp(0., 100.);
        }
        if let Some(v) = floating {
            s.config.floating = v;
        }
        s.key.clear();
        s.save();
        let rows = s.settings_rows();
        if let Some(ui) = &mut s.settings {
            ui.set_rows(rows);
        }
        let hwnd = s.hwnd;
        let f = s.config.floating;
        let i = s.inspect;
        drop(s);
        crate::window::apply_window_mode(hwnd, f, i);
        InvalidateRect(hwnd, null(), 0);
    }
    pub unsafe fn refresh(&mut self) {
        let mut s = self.state.borrow_mut();
        s.key.clear();
        if IsWindowVisible(s.hwnd) != 0 {
            InvalidateRect(s.hwnd, null(), 0);
        }
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        unsafe {
            let mut s = self.state.borrow_mut();
            s.settings.take();
            s.editor.take();
            s.chooser.take();
            let hwnd = s.hwnd;
            drop(s);
            if IsWindow(hwnd) != 0 {
                DestroyWindow(hwnd);
            }
        }
    }
}
fn inside(r: [f32; 4], p: [f32; 2]) -> bool {
    p[0] >= r[0] && p[0] < r[2] && p[1] >= r[1] && p[1] < r[3]
}
fn point(lp: LPARAM) -> POINT {
    POINT {
        x: lp as i16 as i32,
        y: (lp >> 16) as i16 as i32,
    }
}
fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | g << 8 | b << 16
}
impl State {
    unsafe fn size(&self) -> [f32; 2] {
        let mut r: RECT = zeroed();
        GetClientRect(self.hwnd, &mut r);
        [r.right as f32 / self.dpi, r.bottom as f32 / self.dpi]
    }
    fn save(&mut self) {
        if !self.smoke {
            if let Err(e) = self.config.save(&self.root) {
                self.error = e;
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
    fn read(&mut self) {
        if !self.pending {
            if let Some(worker) = &self.worker {
                worker.read();
                self.pending = true;
                self.last_poll = Instant::now();
                unsafe {
                    SetTimer(self.hwnd, 1, 100, None);
                }
            }
        }
    }
    fn command(&mut self, c: Control, value: f32) {
        if c.kind.is_volume() {
            if c.kind.is_input() {
                self.snapshot.mic_volume = Some(value);
            } else {
                self.snapshot.volume = Some(value);
            }
        }
        if self.pending {
            if c.kind.is_volume() {
                self.queued = Some(Command { control: c, value });
            }
            return;
        }
        if let Some(worker) = &self.worker {
            worker.command(Command { control: c, value });
            self.pending = true;
            self.last_poll = Instant::now();
            self.error.clear();
            unsafe {
                SetTimer(self.hwnd, 1, 100, None);
            }
        }
    }
    unsafe fn hide(&mut self) {
        self.config.open = false;
        self.save();
        KillTimer(self.hwnd, 1);
        KillTimer(self.hwnd, 2);
        self.pressed = None;
        self.drag = None;
        self.slider = None;
        self.moving = None;
        self.manager = false;
        ShowWindow(self.hwnd, SW_HIDE);
        for ui in [&mut self.settings, &mut self.editor, &mut self.chooser]
            .into_iter()
            .flatten()
        {
            ui.hide();
        }
        self.frame = None;
        self.backdrop = None;
        self.fade_base = None;
        self.shadow = None;
        self.button_lenses.clear();
        self.icons.clear();
        self.key.clear();
    }
    unsafe fn hit(&self, p: POINT) -> Option<Action> {
        let p = [p.x as f32 / self.dpi, p.y as f32 / self.dpi];
        let [w, h] = self.size();
        for (action, r) in settings_ui::widget_header_hits(w) {
            if inside(r, p) {
                return Some(if action == HeaderAction::Settings {
                    Action::Settings
                } else {
                    Action::Hide
                });
            }
        }
        if let Some((a, _)) = self.hits.iter().rev().find(|(a, r)| {
            (matches!(a, Action::Scroll(_)) || (self.manager && *a == Action::Hide))
                && inside(*r, p)
        }) {
            return Some(*a);
        }
        self.hits
            .iter()
            .rev()
            .find(|(_, r)| inside(*r, p) && p[1] >= 54. && p[1] < h - 12.)
            .map(|(a, _)| *a)
            .or_else(|| {
                if inside([18., 12., 68., 42.], p) {
                    Some(Action::Edit)
                } else if w >= 220. && inside([76., 12., 130., 42.], p) {
                    if self.editing {
                        Some(Action::Add)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
    }
    fn settings_rows(&self) -> Vec<Row> {
        let c = &self.config;
        vec![
            Row::section("Design"),
            Row::choices(
                7,
                "Theme",
                &c.design,
                [
                    ("lumen", "Lumen"),
                    ("cloud", "Cloud"),
                    ("soft-violet", "Soft Violet"),
                    ("liquid-pastel", "Liquid Pastel"),
                ]
                .into_iter()
                .map(|(k, v)| Choice::new(k, v, None))
                .collect(),
            ),
            Row::section("Background"),
            Row::toggle(
                1,
                "Frosted glass",
                "Blend with desktop wallpaper",
                c.frosted,
            ),
            Row::slider(2, "Glass tint", c.tint, 0., 100., "%"),
            Row::slider(3, "Frost strength", c.frost_strength, 0., 100., "%"),
            Row::slider(4, "Blur size", c.blur, 0., 80., "px"),
            Row::slider(5, "Background opacity", c.opacity, 0., 100., "%"),
            Row::choices(
                6,
                "Surface palette",
                &c.palette,
                settings_ui::palette_choices(),
            ),
            Row::section("Window"),
            Row::button(9, "Edit controls"),
            Row::note("All controls run natively. Changes apply only on release."),
        ]
    }
    unsafe fn open_settings(&mut self) {
        let rows = self.settings_rows();
        if self.settings.is_none() {
            match settings_ui::Host::create(
                self.hwnd,
                "Control Center settings",
                "Appearance and window preferences.",
                self.glass.clone(),
                rows.clone(),
                SETTINGS,
            ) {
                Ok(ui) => self.settings = Some(ui),
                Err(e) => self.error = e,
            }
        }
        if let Some(ui) = &mut self.settings {
            ui.set_rows(rows);
            ui.show();
        }
    }
    fn editor_rows(&self) -> Vec<Row> {
        let mut rows = vec![Row::section("Your controls")];
        for (i, c) in self.config.controls.iter().enumerate() {
            rows.push(Row::choices(
                2000 + i as u32,
                &c.label,
                "",
                [
                    ("up", "Move up"),
                    ("down", "Move down"),
                    ("rename", "Rename"),
                    ("remove", "Remove"),
                ]
                .into_iter()
                .map(|(k, l)| Choice::new(k, l, None))
                .collect(),
            ));
        }
        rows.push(Row::section("Add"));
        rows.push(Row::button(1, "Add control…"));
        rows.push(Row::note(
            "Choose any display, audio output or microphone. Volume follows the default device.",
        ));
        rows
    }
    unsafe fn open_editor(&mut self) {
        if !self.manager_parent.is_null() {
            self.hide();
            return;
        }
        self.editing = !self.editing;
        self.moving = None;
        self.read();
    }
    unsafe fn open_manager(&mut self) {
        self.candidates = self.catalog();
        if self.manager_window.is_none() {
            match Host::create(&self.root, self.glass.clone(), true, true) {
                Ok(host) => {
                    let mut child = host.state.borrow_mut();
                    child.manager_parent = self.hwnd;
                    child.manager = true;
                    child.config = self.config.clone();
                    child.config.floating = true;
                    child.snapshot = self.snapshot.clone();
                    child.candidates = self.candidates.clone();
                    let hwnd = child.hwnd;
                    drop(child);
                    SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, self.hwnd as isize);
                    let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                    SetWindowLongPtrW(
                        hwnd,
                        GWL_EXSTYLE,
                        (style & !(WS_EX_APPWINDOW as isize)) | WS_EX_TOOLWINDOW as isize,
                    );
                    let mut work: RECT = zeroed();
                    SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut work as *mut _ as _, 0);
                    let dpi = self.dpi;
                    let width = (620. * dpi).min((work.right - work.left) as f32) as i32;
                    let height = (540. * dpi).min((work.bottom - work.top) as f32) as i32;
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        work.left + (work.right - work.left - width) / 2,
                        work.top + (work.bottom - work.top - height) / 2,
                        width,
                        height,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                    self.manager_window = Some(Box::new(host));
                }
                Err(e) => self.error = e,
            }
        }
        if let Some(ui) = &mut self.manager_window {
            {
                let mut child = ui.state.borrow_mut();
                child.manager = true;
                child.config.controls = self.config.controls.clone();
                child.config.floating = true;
                child.snapshot = self.snapshot.clone();
                child.candidates = self.candidates.clone();
                child.scroll = 0.;
                child.scroll_target = 0.;
                child.manager_query.clear();
            }
            ui.show();
            SetForegroundWindow(ui.hwnd());
        }
    }
    fn catalog(&self) -> Vec<Control> {
        let mut out = vec![];
        let mut add = |kind: Kind, target: String, label: String| {
            out.push(Control {
                id: String::new(),
                kind,
                target,
                label,
            })
        };
        for kind in KINDS {
            if matches!(kind, Kind::Mute | Kind::MicMute) {
                continue;
            }
            if kind == Kind::Hdr {
                for d in &self.snapshot.displays {
                    if d.hdr_supported {
                        add(kind, d.id.clone(), format!("{} · HDR", d.name));
                    }
                }
            } else {
                add(
                    kind,
                    String::new(),
                    if kind == Kind::Display {
                        "Displays".into()
                    } else {
                        kind.name().into()
                    },
                );
            }
        }
        out
    }
    unsafe fn open_chooser(&mut self, index: Option<usize>) {
        self.choosing = index;
        self.adding = index.is_none();
        self.candidates = if let Some(index) = index {
            let c = &self.config.controls[index];
            let kind = c.kind;
            (if kind.is_input() {
                &self.snapshot.inputs
            } else {
                &self.snapshot.outputs
            })
            .iter()
            .map(|d| Control {
                id: c.id.clone(),
                kind,
                target: d.id.clone(),
                label: d.name.clone(),
            })
            .collect()
        } else {
            self.catalog()
        };
        let mut rows = vec![Row::section(if self.adding {
            "Available controls"
        } else {
            "Choose device"
        })];
        for (i, c) in self.candidates.iter().enumerate() {
            rows.push(Row::button(1000 + i as u32, &c.label));
        }
        if self.candidates.is_empty() {
            rows.push(Row::note(
                "No connected devices found. Refresh after connecting a device.",
            ));
        }
        self.chooser.take();
        match settings_ui::Host::create(
            self.hwnd,
            if self.adding {
                "Add control"
            } else {
                "Choose audio device"
            },
            "Select an item below.",
            self.glass.clone(),
            rows,
            CHOOSE,
        ) {
            Ok(mut ui) => {
                ui.show();
                self.chooser = Some(ui)
            }
            Err(e) => self.error = e,
        }
    }
    unsafe fn activate(&mut self, a: Action) {
        match a {
            Action::Scroll(_) => {}
            Action::Search => {
                self.search_focus = true;
                SetFocus(self.hwnd);
            }
            Action::Settings => {
                if self.manager_parent.is_null() {
                    self.open_settings();
                } else {
                    PostMessageW(self.manager_parent, MANAGER_SETTINGS, 0, 0);
                }
            }
            Action::Hide => {
                if !self.manager_parent.is_null() {
                    self.hide();
                    return;
                }
                if self.manager {
                    self.manager = false;
                    self.scroll = 0.;
                    self.scroll_target = 0.;
                } else {
                    self.hide();
                }
            }
            Action::Edit => self.open_editor(),
            Action::Add => self.open_manager(),
            Action::Candidate(i) => {
                if !self.manager_parent.is_null() {
                    PostMessageW(self.manager_parent, MANAGER_TOGGLE, i, 0);
                    return;
                }
                if let Some(c) = self.candidates.get(i).cloned() {
                    if let Some(index) = self
                        .config
                        .controls
                        .iter()
                        .position(|x| x.kind == c.kind && x.target == c.target)
                    {
                        self.config.controls.remove(index);
                    } else {
                        self.config.add(c.kind, c.target, c.label);
                    }
                    self.save();
                    if let Some(ui) = &self.manager_window {
                        PostMessageW(ui.hwnd(), MANAGER_SYNC, 0, 0);
                    }
                }
            }
            Action::Remove(i) => {
                if i < self.config.controls.len() {
                    self.config.controls.remove(i);
                    self.save();
                }
            }
            Action::Rename(i) => {
                PostMessageW(self.hwnd, RENAME, i, 0);
            }
            Action::Move(_) => {}
            Action::Control(i) => {
                if self.editing {
                    return;
                }
                if let Some(c) = self.config.controls.get(i).cloned() {
                    if c.kind.is_group() {
                        return;
                    } else {
                        let (on, _, available) = self.info(&c);
                        if available {
                            self.command(c, if on { 0. } else { 1. });
                        } else {
                            self.error = "This device or feature is unavailable".into();
                        }
                    }
                }
            }
            Action::Mute(i) => {
                if !self.editing {
                    if let Some(mut c) = self.config.controls.get(i).cloned() {
                        let mic = c.kind.is_input();
                        c.kind = if mic { Kind::MicMute } else { Kind::Mute };
                        let muted = if mic {
                            self.snapshot.mic_muted
                        } else {
                            self.snapshot.muted
                        };
                        self.command(c, if muted { 0. } else { 1. });
                    }
                }
            }
            Action::Device(i, j) => {
                if !self.editing {
                    if let Some(mut c) = self.config.controls.get(i).cloned() {
                        if let Some((id, _, on, enabled)) = self.devices(c.kind).get(j) {
                            if *enabled {
                                c.target = id.clone();
                                let value = if c.kind == Kind::Display && *on {
                                    0.
                                } else {
                                    1.
                                };
                                self.command(c, value);
                            }
                        }
                    }
                }
            }
            Action::Slider(_) => {}
        }
        InvalidateRect(self.hwnd, null(), 0);
    }
    fn tiles(&self, w: f32, h: f32, scroll: f32) -> (Vec<model::Tile>, f32, bool) {
        model::layout_devices(
            &self.config.controls,
            w,
            h,
            scroll,
            [
                self.snapshot.outputs.len(),
                self.snapshot.inputs.len(),
                self.snapshot.displays.len(),
            ],
        )
    }
    fn scrollbar(&self) -> Option<([f32; 4], [f32; 4], f32, bool)> {
        let [w, h] = unsafe { self.size() };
        let (_, max, horizontal) = if self.manager {
            let items: Vec<_> = self
                .candidates
                .iter()
                .filter(|c| {
                    c.label
                        .to_lowercase()
                        .contains(&self.manager_query.to_lowercase())
                })
                .cloned()
                .collect();
            let (_, _, max) = model::manager_layout(&items, w, h, 0.);
            (vec![], max, false)
        } else {
            self.tiles(w, h, 0.)
        };
        if max <= 0. {
            return None;
        }
        let top = if self.manager { 142. } else { 54. };
        let track = if horizontal {
            [18., h - 9., w - 18., h - 5.]
        } else {
            [
                w - 11.,
                top,
                w - 7.,
                h - if self.manager { 60. } else { 18. },
            ]
        };
        let length = if horizontal {
            track[2] - track[0]
        } else {
            track[3] - track[1]
        };
        let thumb_len = (length * length / (length + max)).clamp(24., length.max(24.));
        let offset = (length - thumb_len) * (self.scroll / max).clamp(0., 1.);
        let thumb = if horizontal {
            [
                track[0] + offset,
                track[1],
                track[0] + offset + thumb_len,
                track[3],
            ]
        } else {
            [
                track[0],
                track[1] + offset,
                track[2],
                track[1] + offset + thumb_len,
            ]
        };
        Some((track, thumb, max, horizontal))
    }
    fn devices(&self, kind: Kind) -> Vec<(String, String, bool, bool)> {
        if kind == Kind::Display {
            self.snapshot
                .displays
                .iter()
                .map(|d| {
                    (
                        d.id.clone(),
                        if d.primary {
                            format!("{} · Main Display", d.name)
                        } else {
                            d.name.clone()
                        },
                        d.enabled,
                        !d.primary,
                    )
                })
                .collect()
        } else {
            let list = if kind == Kind::Microphone {
                &self.snapshot.inputs
            } else {
                &self.snapshot.outputs
            };
            list.iter()
                .map(|d| (d.id.clone(), d.name.clone(), d.selected, true))
                .collect()
        }
    }
    fn info(&self, c: &Control) -> (bool, String, bool) {
        let s = &self.snapshot;
        match c.kind {
            Kind::Volume | Kind::MicVolume => {
                let value = if c.kind.is_input() {
                    s.mic_volume
                } else {
                    s.volume
                };
                (
                    false,
                    value
                        .map(|v| format!("{}%", v.round()))
                        .unwrap_or_else(|| "Unavailable".into()),
                    value.is_some(),
                )
            }
            Kind::Audio | Kind::Microphone => {
                let list = if c.kind.is_input() {
                    &s.inputs
                } else {
                    &s.outputs
                };
                let d = list.iter().find(|d| {
                    if c.target.is_empty() {
                        d.selected
                    } else {
                        d.id == c.target
                    }
                });
                (
                    d.is_some_and(|d| d.selected),
                    d.map(|d| d.name.clone())
                        .unwrap_or_else(|| "Choose device".into()),
                    !list.is_empty(),
                )
            }
            Kind::Mute | Kind::MicMute => {
                let muted = if c.kind.is_input() {
                    s.mic_muted
                } else {
                    s.muted
                };
                (
                    muted,
                    if muted { "Muted" } else { "On" }.into(),
                    if c.kind.is_input() {
                        s.mic_volume.is_some()
                    } else {
                        s.volume.is_some()
                    },
                )
            }
            Kind::Display | Kind::Hdr => {
                let d = s.displays.iter().find(|d| d.id == c.target);
                let on = d.is_some_and(|d| {
                    if c.kind == Kind::Hdr {
                        d.hdr_enabled
                    } else {
                        d.enabled
                    }
                });
                (
                    on,
                    d.map(|d| d.name.clone())
                        .unwrap_or_else(|| "Disconnected".into()),
                    d.is_some_and(|d| {
                        if c.kind == Kind::Hdr {
                            d.hdr_supported && d.enabled
                        } else {
                            !d.primary || !d.enabled
                        }
                    }),
                )
            }
        }
    }
    unsafe fn paint(&mut self) {
        let mut ps = zeroed();
        BeginPaint(self.hwnd, &mut ps);
        let mut r: RECT = zeroed();
        GetClientRect(self.hwnd, &mut r);
        let [w, h] = self.size();
        crate::fusion::prepare(self.hwnd, 30.);
        let Some(mut frame) = self
            .frame
            .take()
            .filter(|f| f.matches_geometry(r.right, r.bottom, self.dpi, 30.))
            .or_else(|| Buffer::new(r.right, r.bottom, self.dpi, 30.))
        else {
            EndPaint(self.hwnd, &ps);
            return;
        };
        let mut pos: RECT = zeroed();
        GetWindowRect(self.hwnd, &mut pos);
        let key = format!(
            "{}:{}:{}:{}:{}:{}:{}:{}",
            pos.left,
            pos.top,
            w,
            h,
            self.dpi,
            self.glass.borrow().revision(),
            crate::fusion::revision(self.hwnd),
            serde_json::to_string(&self.config).unwrap_or_default()
        );
        let frozen = self.drag.is_some()
            && self
                .backdrop
                .as_ref()
                .is_some_and(|f| f.width == r.right && f.height == r.bottom);
        if self.backdrop.is_none() || self.key != key && !frozen {
            let design = self.config.design.as_str();
            let light = design != "lumen";
            let base = match design {
                "cloud" => rgb(237, 244, 244),
                "soft-violet" => rgb(231, 230, 241),
                "liquid-pastel" => rgb(230, 233, 232),
                _ => rgb(26, 30, 38),
            };
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
            let c = Canvas::new(frame.dc, self.dpi);
            let pane = [0., 0., w, h];
            let tint = if self.config.frosted {
                self.config.tint / 100.
            } else {
                1.
            };
            if crate::surface::managed(&self.config.palette) {
                crate::surface::paint(&c, pane, &self.config.palette, tint);
            } else {
                c.round(
                    pane,
                    30.,
                    crate::surface::color(
                        &self.config.palette,
                        self.glass.borrow().wallpaper_accent(),
                    ),
                    (tint * if light { 64. } else { 255. }) as u8,
                    false,
                    false,
                );
            }
            if design == "soft-violet" {
                c.round(pane, 30., rgb(229, 233, 232), 205, false, false);
            }
            c.coating(pane, 30.);
            if let Some(noise) =
                Canvas::pane_overlay([r.right as u32, r.bottom as u32], self.dpi, tint, light)
            {
                c.image(&noise, pane);
            }
            c.widget_corner_reflection(pane, 30., light);
            drop(c);
            frame.bevel_for(self.hwnd);
            self.ink = Some(
                crate::contrast::inks(
                    crate::contrast::sample(frame.dc, self.dpi, [18., 54., w - 18., h - 18.]),
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
        if self
            .fade_base
            .as_ref()
            .is_none_or(|b| b.width != r.right || b.height != r.bottom)
        {
            self.fade_base = Buffer::plain(r.right, r.bottom, self.dpi);
        }
        if let Some(base) = &self.fade_base {
            base.copy_from(frame.dc);
        }
        let ink = self.ink.unwrap_or(0xffffff);
        let light = crate::contrast::luminance(ink) < 0.5;
        let accent = if self.config.design == "soft-violet" {
            rgb(118, 86, 226)
        } else if self.config.design == "cloud" {
            rgb(66, 186, 198)
        } else {
            crate::surface::color(&self.config.palette, self.glass.borrow().wallpaper_accent())
        };
        let (_, max, _) = self.tiles(w, h, 0.);
        let max = if self.manager { 100000. } else { max };
        self.scroll_target = self.scroll_target.clamp(0., max);
        self.scroll = self.scroll.clamp(0., max);
        let (tiles, _, _) = self.tiles(w, h, self.scroll);
        self.hits.clear();
        let saved = SaveDC(frame.dc);
        IntersectClipRect(
            frame.dc,
            (14. * self.dpi) as i32,
            (if self.manager { 142. } else { 54. } * self.dpi) as i32,
            ((w - 14.) * self.dpi) as i32,
            ((h - if self.manager { 60. } else { 12. }) * self.dpi) as i32,
        );
        let batch = render::AlphaBatch::new(frame.dc, self.dpi);
        for tile in if self.manager { vec![] } else { tiles } {
            let r = tile.rect;
            if r[3] < 54. || r[1] > h - 12. || r[2] < 14. || r[0] > w - 14. {
                continue;
            }
            let c = &self.config.controls[tile.index];
            let (on, status, available) = self.info(c);
            let action = if c.kind.is_volume() {
                Action::Slider(tile.index)
            } else {
                Action::Control(tile.index)
            };
            self.hits.push((action, r));
            let cv = Canvas::new(frame.dc, self.dpi);
            let hovered = self.hover == Some(action);
            let material = lenses::material(&self.config.design);
            let radius = lenses::radius(material, r[3] - r[1]);
            let light = light || material == lenses::CLOUD || material == lenses::SOFT;
            let ink = if material == lenses::CLOUD {
                rgb(38, 52, 59)
            } else if material == lenses::SOFT {
                rgb(57, 48, 82)
            } else {
                ink
            };
            let card_accent = if self.config.design == "liquid-pastel" {
                match tile.index % 3 {
                    0 => rgb(169, 150, 212),
                    1 => rgb(134, 184, 169),
                    _ => rgb(218, 165, 150),
                }
            } else {
                accent
            };
            cv.soft_shadow(
                [r[0] + 1., r[1] + 1., r[2] - 1., r[3] - 1.],
                radius,
                if material == lenses::SOFT {
                    27.
                } else if material == lenses::CLOUD {
                    12.
                } else if hovered {
                    24.
                } else {
                    16.
                },
                if material == lenses::SOFT { 7. } else { 5. },
                if material == lenses::SOFT { 4. } else { 2. },
            );
            let style = material
                | lenses::ENABLED
                | if light { lenses::LIGHT } else { 0 }
                | if on && !c.kind.is_group() {
                    lenses::ACTIVE | lenses::PRIMARY
                } else {
                    0
                }
                | if hovered { lenses::HOVER } else { 0 }
                | if self.pressed == Some(action) {
                    lenses::PRESSED
                } else {
                    0
                };
            self.button_lenses
                .draw(&cv, self.dpi, r, card_accent, style);
            if self.moving.is_some_and(|(from, _, to, active)| {
                active && (from == tile.index || to == tile.index)
            }) {
                cv.round(r, radius, card_accent, 230, true, false);
            }
            if c.kind.is_group() {
                self.icons
                    .draw(self.dpi, &cv, c.kind, r[0] + 12., r[1] + 7., 18., ink, true);
                cv.text(
                    &c.label,
                    [
                        r[0] + 38.,
                        r[1] + 1.,
                        r[2] - 10.,
                        r[1] + if h <= 160. { 20. } else { 27. },
                    ],
                    if h <= 160. { 9. } else { 11. },
                    true,
                    ink,
                    0,
                );
                let devices = self.devices(c.kind);
                if devices.is_empty() {
                    cv.text(
                        "No devices connected",
                        [r[0] + 12., r[1] + 28., r[2] - 12., r[3] - 4.],
                        10.,
                        false,
                        ink,
                        0,
                    );
                }
                for (j, (_, name, on, enabled)) in devices.iter().enumerate() {
                    let rr = if h <= 240. {
                        [
                            r[0] + 12. + j as f32 * 176.,
                            r[1] + if h <= 160. { 20. } else { 27. },
                            r[0] + 180. + j as f32 * 176.,
                            r[3] - 3.,
                        ]
                    } else {
                        [
                            r[0] + 12.,
                            r[1] + 30. + j as f32 * 34.,
                            r[2] - 12.,
                            r[1] + 62. + j as f32 * 34.,
                        ]
                    };
                    let toggle = [
                        rr[2] - 32.,
                        (rr[1] + rr[3]) / 2. - 8.,
                        rr[2] - 2.,
                        (rr[1] + rr[3]) / 2. + 8.,
                    ];
                    if c.kind == Kind::Display && !enabled {
                        let device = name.trim_end_matches(" · Main Display");
                        if rr[3] - rr[1] >= 29. {
                            cv.text(
                                device,
                                [rr[0], rr[1], toggle[0] - 7., rr[1] + 18.],
                                11.,
                                true,
                                ink,
                                0,
                            );
                            cv.text(
                                "Main Display",
                                [rr[0], rr[1] + 17., toggle[0] - 7., rr[3]],
                                8.,
                                false,
                                ink,
                                0,
                            );
                        } else {
                            cv.text(
                                &format!("Main Display · {device}"),
                                [rr[0], rr[1], toggle[0] - 7., rr[3]],
                                10.,
                                true,
                                ink,
                                0,
                            );
                        }
                    } else {
                        cv.text(
                            name,
                            [rr[0], rr[1], toggle[0] - 7., rr[3]],
                            11.,
                            *on,
                            ink,
                            0,
                        );
                    }
                    cv.round(
                        toggle,
                        8.,
                        if *on && *enabled { accent } else { ink },
                        if *on && *enabled { 220 } else { 45 },
                        false,
                        false,
                    );
                    let x = if *on { toggle[2] - 14. } else { toggle[0] + 2. };
                    cv.round(
                        [x, toggle[1] + 2., x + 12., toggle[3] - 2.],
                        6.,
                        0xffffff,
                        if *enabled { 255 } else { 120 },
                        false,
                        false,
                    );
                    if *enabled && !self.editing {
                        self.hits.push((Action::Device(tile.index, j), rr));
                    }
                }
            } else {
                let small = r[3] - r[1] < 80.;
                let tiny = r[3] - r[1] < 54.;
                let collapsed = false;
                let iy = r[1]
                    + if tiny {
                        6.
                    } else if small {
                        10.
                    } else {
                        17.
                    };
                if (material == lenses::CLOUD
                    || material == lenses::LIQUID
                    || material == lenses::SOFT)
                    && !tiny
                    && !collapsed
                {
                    let disc = [r[0] + 9., r[1] + 7., r[0] + 39., r[1] + 37.];
                    if material == lenses::CLOUD || material == lenses::SOFT {
                        cv.soft_shadow(disc, 15., 12., 3., 1.);
                        cv.round(disc, 15., 0xffffff, 150, false, false);
                    } else {
                        self.button_lenses.draw(
                            &cv,
                            self.dpi,
                            disc,
                            card_accent,
                            lenses::ENABLED
                                | lenses::LIQUID
                                | if light { lenses::LIGHT } else { 0 },
                        );
                    }
                }
                self.icons.draw(
                    self.dpi,
                    &cv,
                    if c.kind.is_volume()
                        && if c.kind.is_input() {
                            self.snapshot.mic_muted
                        } else {
                            self.snapshot.muted
                        }
                    {
                        if c.kind.is_input() {
                            Kind::MicMute
                        } else {
                            Kind::Mute
                        }
                    } else {
                        c.kind
                    },
                    if collapsed {
                        if c.kind.is_volume() {
                            r[0] + 9.
                        } else {
                            (r[0] + r[2]) / 2. - 10.
                        }
                    } else {
                        r[0] + if (material == lenses::CLOUD || material == lenses::LIQUID) && !tiny
                        {
                            16.
                        } else {
                            14.
                        }
                    },
                    if collapsed {
                        (r[1] + r[3]) / 2. - 10.
                    } else if (material == lenses::CLOUD || material == lenses::LIQUID) && !tiny {
                        r[1] + 14.
                    } else {
                        iy
                    },
                    if small { 20. } else { 22. },
                    ink,
                    available,
                );
                let label_x = r[0]
                    + if (material == lenses::CLOUD
                        || material == lenses::LIQUID
                        || material == lenses::SOFT)
                        && !tiny
                    {
                        49.
                    } else if small {
                        37.
                    } else {
                        14.
                    };
                let label_y = r[1]
                    + if tiny {
                        0.
                    } else if small {
                        if material == lenses::CLOUD {
                            7.
                        } else {
                            4.
                        }
                    } else {
                        43.
                    };
                cv.text_alpha(
                    if collapsed { "" } else { &c.label },
                    [
                        label_x,
                        label_y,
                        r[2] - if c.kind.is_volume() { 45. } else { 12. },
                        label_y + 22.,
                    ],
                    if small { 10.5 } else { 12. },
                    true,
                    ink,
                    0,
                    if available { 255 } else { 115 },
                );
                if c.kind.is_volume() {
                    if !self.editing {
                        self.hits.push((
                            Action::Mute(tile.index),
                            [
                                r[0] + 5.,
                                r[1] + 4.,
                                r[0] + 38.,
                                r[1] + if tiny { 24. } else { 34. },
                            ],
                        ));
                    }
                    let v = self
                        .slider
                        .filter(|(i, _)| *i == tile.index)
                        .map(|(_, v)| v)
                        .or(if c.kind.is_input() {
                            self.snapshot.mic_volume
                        } else {
                            self.snapshot.volume
                        })
                        .unwrap_or(0.);
                    let track_h = if material == lenses::CLOUD && !tiny {
                        12.
                    } else if material == lenses::SOFT {
                        10.
                    } else if material == lenses::LIQUID {
                        9.
                    } else {
                        6.
                    };
                    let y = if collapsed {
                        (r[1] + r[3] - track_h) / 2.
                    } else {
                        r[3] - if tiny { 10. } else { track_h + 12. }
                    };
                    let track = [
                        r[0] + if collapsed { 38. } else { 14. },
                        y,
                        r[2] - 14.,
                        y + track_h,
                    ];
                    cv.round(
                        track,
                        track_h / 2.,
                        ink,
                        if material == lenses::SOFT { 42 } else { 30 },
                        false,
                        false,
                    );
                    cv.round(
                        [
                            track[0],
                            track[1],
                            track[0] + (track[2] - track[0]) * v / 100.,
                            track[3],
                        ],
                        track_h / 2.,
                        card_accent,
                        230,
                        false,
                        false,
                    );
                    let x = track[0] + (track[2] - track[0]) * v / 100.;
                    let cy = y + track_h / 2.;
                    let thumb = [x - 7., cy - 7., x + 7., cy + 7.];
                    cv.soft_shadow(thumb, 7., 16., 3., 2.);
                    self.button_lenses.draw(
                        &cv,
                        self.dpi,
                        thumb,
                        accent,
                        lenses::material(&self.config.design)
                            | lenses::ENABLED
                            | if light { lenses::LIGHT } else { 0 },
                    );
                    if small && !collapsed {
                        cv.text(
                            &format!("{}%", v.round()),
                            [r[2] - 44., r[1] + 2., r[2] - 10., r[1] + 23.],
                            9.,
                            false,
                            ink,
                            2,
                        );
                    }
                    if !small {
                        cv.text(
                            &format!("{}%", v.round()),
                            [r[0] + 45., r[1] + 13., r[2] - 14., r[1] + 37.],
                            12.,
                            false,
                            ink,
                            2,
                        );
                    }
                } else {
                    let y = if small { r[1] + 28. } else { r[1] + 67. };
                    cv.text_alpha(
                        if tiny || collapsed { "" } else { &status },
                        [label_x, y, r[2] - 12., y + 19.],
                        if small { 8.5 } else { 10. },
                        false,
                        ink,
                        0,
                        if available { 153 } else { 90 },
                    );
                    if on && !collapsed {
                        cv.round(
                            [r[2] - 22., r[1] + 16., r[2] - 16., r[1] + 22.],
                            3.,
                            accent,
                            255,
                            false,
                            false,
                        );
                    }
                }
            }
            if self.editing {
                self.hits.push((Action::Move(tile.index), r));
                let editing_this = self.hover.is_some_and(|a|matches!(a,Action::Move(i)|Action::Remove(i)|Action::Rename(i) if i==tile.index)) || self.moving.is_some_and(|(i,_,_,_)|i==tile.index);
                if editing_this {
                    for (a, x, label) in [
                        (Action::Remove(tile.index), r[0] + 3., "×"),
                        (Action::Rename(tile.index), r[2] - 23., "✎"),
                    ] {
                        let button = [x, r[1] + 2., x + 20., r[1] + 22.];
                        cv.round(
                            button,
                            10.,
                            if light { 0xffffff } else { 0x292330 },
                            230,
                            false,
                            false,
                        );
                        cv.text(label, button, 14., true, ink, 1);
                        self.hits.push((a, button));
                    }
                    cv.text(
                        "⠿",
                        [r[2] - 44., r[1] + 2., r[2] - 24., r[1] + 22.],
                        13.,
                        false,
                        ink,
                        1,
                    );
                }
            }
        }
        if self.manager {
            let filtered: Vec<_> = self
                .candidates
                .iter()
                .enumerate()
                .filter(|(_, c)| {
                    c.label
                        .to_lowercase()
                        .contains(&self.manager_query.to_lowercase())
                })
                .collect();
            let items: Vec<_> = filtered.iter().map(|(_, c)| (*c).clone()).collect();
            let (_, _, max) = model::manager_layout(&items, w, h, 0.);
            self.scroll_target = self.scroll_target.clamp(0., max);
            self.scroll = self.scroll.clamp(0., max);
            let (tiles, headers, _) = model::manager_layout(&items, w, h, self.scroll);
            let cv = Canvas::new(frame.dc, self.dpi);
            for (title, rect) in headers {
                cv.text(title, rect, 11., true, ink, 0);
            }
            for tile in tiles {
                let original = filtered[tile.index].0;
                let candidate = &self.candidates[original];
                let r = tile.rect;
                if r[3] < 54. || r[1] > h - 12. {
                    continue;
                }
                let added = self
                    .config
                    .controls
                    .iter()
                    .any(|c| c.kind == candidate.kind && c.target == candidate.target);
                let soft = self.config.design == "soft-violet";
                cv.round(r, 22., ink, if added { 45 } else { 22 }, true, false);
                let preview = [r[0] + 9., r[1] + 9., r[2] - 9., r[1] + 88.];
                cv.soft_shadow(preview, 22., 20., 7., 3.);
                self.button_lenses.draw(
                    &cv,
                    self.dpi,
                    preview,
                    accent,
                    lenses::material(&self.config.design)
                        | lenses::ENABLED
                        | if light { lenses::LIGHT } else { 0 },
                );
                let disc = [r[0] + 20., r[1] + 28., r[0] + 54., r[1] + 62.];
                cv.soft_shadow(disc, 17., 14., 3., 2.);
                cv.round(
                    disc,
                    17.,
                    0xffffff,
                    if soft { 205 } else { 75 },
                    false,
                    false,
                );
                self.icons.draw(
                    self.dpi,
                    &cv,
                    candidate.kind,
                    r[0] + 27.,
                    r[1] + 35.,
                    20.,
                    ink,
                    true,
                );
                let toggle = [r[2] - 50., r[1] + 23., r[2] - 20., r[1] + 40.];
                cv.round(
                    toggle,
                    8.5,
                    if added { accent } else { ink },
                    if added { 210 } else { 45 },
                    false,
                    false,
                );
                let x = if added {
                    toggle[2] - 15.
                } else {
                    toggle[0] + 2.
                };
                cv.round(
                    [x, toggle[1] + 2., x + 13., toggle[3] - 2.],
                    6.5,
                    0xffffff,
                    245,
                    false,
                    false,
                );
                if candidate.kind.is_volume() {
                    let track = [r[0] + 66., r[1] + 50., r[2] - 21., r[1] + 62.];
                    cv.round(track, 6., ink, 35, false, false);
                    let thumb = track[0] + (track[2] - track[0]) * 0.6;
                    cv.round(
                        [track[0], track[1], thumb, track[3]],
                        6.,
                        accent,
                        200,
                        false,
                        false,
                    );
                    let circle = [thumb - 9., track[1] - 3., thumb + 9., track[3] + 3.];
                    cv.soft_shadow(circle, 9., 12., 3., 2.);
                    cv.round(circle, 9., 0xffffff, 255, false, false);
                } else if candidate.kind.is_group() {
                    cv.round(
                        [r[0] + 66., r[1] + 44., r[2] - 58., r[1] + 48.],
                        2.,
                        ink,
                        95,
                        false,
                        false,
                    );
                    cv.round(
                        [r[0] + 66., r[1] + 55., r[2] - 38., r[1] + 59.],
                        2.,
                        ink,
                        65,
                        false,
                        false,
                    );
                }
                cv.text(
                    &candidate.label,
                    [r[0] + 10., r[1] + 97., r[2] - 10., r[1] + 118.],
                    12.,
                    true,
                    ink,
                    0,
                );
                let detail = if candidate.kind.is_volume() {
                    "Volume · tap icon to mute"
                } else if candidate.kind.is_group() {
                    "All connected devices"
                } else {
                    "Display HDR toggle"
                };
                cv.text(
                    detail,
                    [r[0] + 10., r[1] + 119., r[2] - 10., r[3] - 4.],
                    9.,
                    false,
                    ink,
                    0,
                );
                if added || self.config.controls.len() < 40 {
                    self.hits.push((
                        Action::Candidate(original),
                        [
                            toggle[0] - 8.,
                            toggle[1] - 8.,
                            toggle[2] + 8.,
                            toggle[3] + 8.,
                        ],
                    ));
                }
            }
        }
        drop(batch);
        RestoreDC(frame.dc, saved);
        let batch = render::AlphaBatch::new(frame.dc, self.dpi);
        let c = Canvas::new(frame.dc, self.dpi);
        if self.manager {
            c.text(
                "CONTROL LIBRARY",
                [24., 14., w - 70., 32.],
                9.,
                false,
                ink,
                0,
            );
            c.text(
                "Control manager",
                [24., 35., w - 70., 70.],
                22.,
                true,
                ink,
                0,
            );
            let search = [24., 82., w - 24., 122.];
            c.round(search, 16., ink, 18, true, false);
            c.text(
                if self.manager_query.is_empty() {
                    "Search controls…"
                } else {
                    &self.manager_query
                },
                [38., 84., w - 38., 120.],
                12.,
                false,
                ink,
                0,
            );
            self.hits.push((Action::Search, search));
            let close = [w - 58., 18., w - 24., 52.];
            c.soft_shadow(close, 17., 10., 3., 1.);
            c.round(close, 17., 0xffffff, 110, false, false);
            c.text("×", close, 18., false, ink, 1);
            self.hits.push((Action::Hide, close));
            c.text(
                &format!("{} / 40 controls", self.config.controls.len()),
                [24., h - 46., w - 110., h - 18.],
                10.,
                false,
                ink,
                0,
            );
            let done = [w - 94., h - 48., w - 24., h - 18.];
            c.soft_shadow(done, 15., 12., 3., 2.);
            c.round(done, 15., 0xffffff, 130, false, false);
            c.text("Done", done, 11., false, ink, 1);
            self.hits.push((Action::Hide, done));
        }
        for (label, r) in [
            (
                if self.manager {
                    ""
                } else if self.editing {
                    "Done"
                } else {
                    "Edit"
                },
                [18., 12., 68., 42.],
            ),
            (
                if self.editing && !self.manager && w >= 220. {
                    "+ Add"
                } else {
                    ""
                },
                [76., 12., 130., 42.],
            ),
        ] {
            if !label.is_empty() {
                c.soft_shadow(r, 15., 14., 3., 1.);
                self.button_lenses.draw(
                    &c,
                    self.dpi,
                    r,
                    accent,
                    lenses::ENABLED
                        | if light { lenses::LIGHT } else { 0 }
                        | if self.editing { lenses::ACTIVE } else { 0 },
                );
                if label == "Edit" {
                    self.icons.pencil(
                        &c,
                        self.dpi,
                        [r[0] + 6., r[1] + 8., r[0] + 20., r[1] + 22.],
                        ink,
                    );
                    c.text(
                        label,
                        [r[0] + 22., r[1], r[2] - 3., r[3]],
                        10.,
                        true,
                        ink,
                        1,
                    );
                } else {
                    c.text(label, r, 10., true, ink, 1);
                }
            }
        }
        if let Some((track, thumb, _, horizontal)) = self.scrollbar() {
            c.round(track, 2., ink, 28, false, false);
            c.round(thumb, 2., ink, 120, false, false);
            self.hits.push((
                Action::Scroll(horizontal),
                [track[0] - 5., track[1] - 4., track[2] + 5., track[3] + 4.],
            ));
        }
        let title = "";
        if !self.manager && settings_ui::widget_title_visible(w, h) {
            c.text(
                if self.manager {
                    "Control manager"
                } else {
                    "Control Center"
                },
                [144., 12., w - 100., 42.],
                12.,
                true,
                ink,
                0,
            );
        }
        settings_ui::draw_widget_header(
            frame.dc,
            self.hwnd,
            self.dpi,
            w,
            title,
            ink,
            None,
            self.pressed.and_then(|a| match a {
                Action::Settings => Some(HeaderAction::Settings),
                Action::Hide => Some(HeaderAction::Hide),
                _ => None,
            }),
        );
        if self.config.controls.is_empty() && !self.manager {
            c.text(
                "Use Edit to add controls",
                [20., 54., w - 20., h - 18.],
                12.,
                false,
                ink,
                1,
            );
        }
        if !self.error.is_empty() {
            c.round(
                [18., h - 42., w - 18., h - 12.],
                12.,
                if light { 0xffffff } else { 0x211b24 },
                235,
                false,
                false,
            );
            c.text(
                &self.error,
                [26., h - 40., w - 26., h - 14.],
                9.,
                false,
                ink,
                0,
            );
        }
        drop(c);
        drop(batch);
        if let Some(bg) = &self.backdrop {
            let clip = SaveDC(frame.dc);
            IntersectClipRect(
                frame.dc,
                (14. * self.dpi) as i32,
                (if self.manager { 142. } else { 54. } * self.dpi) as i32,
                ((w - 14.) * self.dpi) as i32,
                ((h - if self.manager { 60. } else { 12. }) * self.dpi) as i32,
            );
            for (action, r) in &self.hits {
                if matches!(action, Action::Settings | Action::Hide) {
                    continue;
                }
                let radius = if r[1] < 54. {
                    15.
                } else {
                    lenses::radius(lenses::material(&self.config.design), r[3] - r[1])
                };
                frame.retain_control_backing(bg, *r, radius, 1.);
            }
            RestoreDC(frame.dc, clip);
            frame.retain_control_backing(bg, [18., 12., 68., 42.], 15., 1.);
            if self.editing && !self.manager && w >= 220. {
                frame.retain_control_backing(bg, [76., 12., 130., 42.], 15., 1.);
            }
            settings_ui::retain_header_backing(&mut frame, bg, self.hwnd, w);
        }
        if let Some(bg) = &self.fade_base {
            let max = self.scrollbar().map_or(0., |(_, _, max, _)| max);
            if !self.manager && h<=240. {
                frame.fade_horizontal_edges(bg,[14.,54.,w-14.,h-12.],self.scroll>0.5,self.scroll<max-0.5);
            }else{frame.fade_list_edges(
                bg,
                if self.manager { 142. } else { 54. },
                h - if self.manager { 60. } else { 12. },
                self.scroll > 0.5,
                self.scroll < max - 0.5,
            );}
        }
        if !self.editing && !self.manager {
            let reveal=crate::widget_header::visual(self.hwnd).alpha;
            if let Some(bg)=&self.fade_base {frame.fade_control_region(bg,[10.,5.,75.,51.],reveal);}
        }
        frame.present_with_alpha(self.hwnd, true);
        self.frame = Some(frame);
        if let Some(shadow) = &mut self.shadow {
            shadow.sync_with_radius(
                self.hwnd,
                self.dpi,
                (self.config.opacity * 2.55).round() as u8,
                30.,
            );
        }
        EndPaint(self.hwnd, &ps);
    }
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let c = &*(lp as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, c.lpCreateParams as isize);
        return 1;
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut RefCell<State>;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let Ok(mut s) = (*ptr).try_borrow_mut() else {
        return DefWindowProcW(hwnd, msg, wp, lp);
    };
    match msg {
        WM_PAINT => {
            s.paint();
            0
        }
        WM_ERASEBKGND => 1,
        WM_CLOSE => {
            s.hide();
            0
        }
        MANAGER_SETTINGS => {
            s.open_settings();
            0
        }
        MANAGER_TOGGLE => {
            s.activate(Action::Candidate(wp));
            InvalidateRect(hwnd, null(), 0);
            0
        }
        MANAGER_SYNC => {
            let parent = s.manager_parent;
            if !parent.is_null() && IsWindow(parent) != 0 {
                let parent_ptr = GetWindowLongPtrW(parent, GWLP_USERDATA) as *mut RefCell<State>;
                if !parent_ptr.is_null() {
                    if let Ok(owner) = (*parent_ptr).try_borrow() {
                        s.config.controls = owner.config.controls.clone();
                        s.snapshot = owner.snapshot.clone();
                        s.candidates = owner.candidates.clone();
                    }
                }
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        SETTINGS => {
            let changes = s
                .settings
                .as_mut()
                .map(|u| u.take_changes())
                .unwrap_or_default();
            for change in changes {
                match (change.id, change.value) {
                    (1, Value::Bool(v)) => s.config.frosted = v,
                    (2, Value::Number(v)) => s.config.tint = v.clamp(0., 100.),
                    (3, Value::Number(v)) => s.config.frost_strength = v.clamp(0., 100.),
                    (4, Value::Number(v)) => {
                        s.config.blur = v.clamp(0., 80.);
                        s.glass.borrow_mut().set_blur(s.config.blur, s.dpi);
                    }
                    (5, Value::Number(v)) => s.config.opacity = v.clamp(0., 100.),
                    (6, Value::Choice(v)) if crate::surface::valid(&v) => {
                        s.config.palette = crate::surface::normalize(&v).unwrap_or(v)
                    }
                    (7, Value::Choice(v))
                        if matches!(
                            v.as_str(),
                            "lumen" | "cloud" | "soft-violet" | "liquid-pastel"
                        ) =>
                    {
                        s.config.design = v
                    }
                    (8, Value::Bool(v)) => {
                        s.config.floating = v;
                        PostMessageW(hwnd, WM_APP + 175, 0, 0);
                    }
                    (9, Value::Action) => s.open_editor(),
                    _ => {}
                }
            }
            s.save();
            s.key.clear();
            InvalidateRect(hwnd, null(), 0);
            0
        }
        EDIT => {
            let changes = s
                .editor
                .as_mut()
                .map(|u| u.take_changes())
                .unwrap_or_default();
            for c in changes {
                match (c.id, c.value) {
                    (1, Value::Action) => s.open_chooser(None),
                    (id, Value::Choice(action)) if id >= 2000 => {
                        let i = (id - 2000) as usize;
                        if i < s.config.controls.len() {
                            match action.as_str() {
                                "up" if i > 0 => s.config.controls.swap(i, i - 1),
                                "down" if i + 1 < s.config.controls.len() => {
                                    s.config.controls.swap(i, i + 1)
                                }
                                "remove" => {
                                    s.config.controls.remove(i);
                                }
                                "rename" => {
                                    PostMessageW(hwnd, RENAME, i, 0);
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
            s.save();
            let rows = s.editor_rows();
            if let Some(ui) = &mut s.editor {
                ui.set_rows(rows);
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        CHOOSE => {
            let changes = s
                .chooser
                .as_mut()
                .map(|u| u.take_changes())
                .unwrap_or_default();
            for c in changes {
                if let (id, Value::Action) = (c.id, c.value) {
                    if id >= 1000 {
                        if let Some(c) = s.candidates.get((id - 1000) as usize).cloned() {
                            if s.adding {
                                s.config.add(c.kind, c.target, c.label);
                                s.save();
                                let rows = s.editor_rows();
                                if let Some(ui) = &mut s.editor {
                                    ui.set_rows(rows);
                                }
                            } else {
                                s.command(c, 1.);
                            }
                            if let Some(ui) = &mut s.chooser {
                                ui.hide();
                            }
                        }
                    }
                }
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        RENAME => {
            let original = s.config.controls.get(wp).cloned();
            let owner = s.editor.as_ref().map(|u| u.hwnd()).unwrap_or(hwnd);
            drop(s);
            if let Some(c) = original {
                if let Some(label) = crate::text_prompt::choose(owner, "Rename control", &c.label) {
                    if let Ok(mut s) = (*ptr).try_borrow_mut() {
                        if let Some(item) =
                            s.config.controls.iter_mut().find(|item| item.id == c.id)
                        {
                            item.label = label;
                        }
                        s.save();
                        let rows = s.editor_rows();
                        if let Some(ui) = &mut s.editor {
                            ui.set_rows(rows);
                        }
                        InvalidateRect(hwnd, null(), 0);
                    }
                }
            }
            0
        }
        m if m == WM_APP + 174 => {
            let dpi = s.dpi;
            drop(s);
            SetWindowPos(
                hwnd,
                null_mut(),
                0,
                0,
                (wp as f32 * dpi) as i32,
                (lp as f32 * dpi) as i32,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            if let Ok(mut s) = (*ptr).try_borrow_mut() {
                s.scroll = 0.;
                s.scroll_target = 0.;
                s.bounds();
            }
            0
        }
        m if m == WM_APP + 175 => {
            let (f, i) = (s.config.floating, s.inspect);
            drop(s);
            crate::window::apply_window_mode(hwnd, f, i);
            0
        }
        WM_NCHITTEST => {
            let mut p = point(lp);
            ScreenToClient(hwnd, &mut p);
            let [w, h] = s.size();
            crate::geometry::widget_edge(p.x as f32 / s.dpi, p.y as f32 / s.dpi, w, h) as isize
        }
        WM_GETMINMAXINFO => {
            let p = &mut *(lp as *mut MINMAXINFO);
            p.ptMinTrackSize = POINT {
                x: (160. * s.dpi) as i32,
                y: (112. * s.dpi) as i32,
            };
            p.ptMaxTrackSize = POINT {
                x: (1800. * s.dpi) as i32,
                y: (1100. * s.dpi) as i32,
            };
            0
        }
        WM_NCLBUTTONDOWN => {
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
            let p = point(lp);
            s.pressed = s.hit(p);
            if let Some(Action::Scroll(horizontal)) = s.pressed {
                if let Some((track, thumb, max, _)) = s.scrollbar() {
                    let pos = if horizontal {
                        p.x as f32 / s.dpi
                    } else {
                        p.y as f32 / s.dpi
                    };
                    let start = if horizontal { thumb[0] } else { thumb[1] };
                    let len = if horizontal {
                        thumb[2] - thumb[0]
                    } else {
                        thumb[3] - thumb[1]
                    };
                    let begin = if horizontal { track[0] } else { track[1] };
                    let total = if horizontal {
                        track[2] - track[0]
                    } else {
                        track[3] - track[1]
                    };
                    let grip = if pos >= start && pos <= start + len {
                        pos - start
                    } else {
                        len / 2.
                    };
                    s.scroll_drag = Some((horizontal, grip));
                    s.scroll = ((pos - begin - grip) / (total - len).max(1.) * max).clamp(0., max);
                    s.scroll_target = s.scroll;
                }
            } else if let Some(Action::Move(i)) = s.pressed {
                s.moving = Some((i, p, i, false));
            } else if let Some(Action::Slider(i)) = s.pressed {
                let (w, h) = {
                    let z = s.size();
                    (z[0], z[1])
                };
                let (t, _, _) = s.tiles(w, h, s.scroll);
                if let Some(tile) = t.iter().find(|t| t.index == i) {
                    let v = ((p.x as f32 / s.dpi - tile.rect[0] - 14.)
                        / (tile.rect[2] - tile.rect[0] - 28.)
                        * 100.)
                        .clamp(0., 100.);
                    s.slider = Some((i, v));
                }
            } else if s.pressed.is_none() {
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
            if let Some((horizontal, grip)) = s.scroll_drag {
                if let Some((track, thumb, max, _)) = s.scrollbar() {
                    let pos = if horizontal {
                        p.x as f32 / s.dpi
                    } else {
                        p.y as f32 / s.dpi
                    };
                    let begin = if horizontal { track[0] } else { track[1] };
                    let total = if horizontal {
                        track[2] - track[0]
                    } else {
                        track[3] - track[1]
                    };
                    let len = if horizontal {
                        thumb[2] - thumb[0]
                    } else {
                        thumb[3] - thumb[1]
                    };
                    s.scroll = ((pos - begin - grip) / (total - len).max(1.) * max).clamp(0., max);
                    s.scroll_target = s.scroll;
                    InvalidateRect(hwnd, null(), 0);
                }
            } else if let Some((from, start, mut to, started)) = s.moving {
                let active =
                    started || (p.x - start.x).abs() + (p.y - start.y).abs() > (5. * s.dpi) as i32;
                if active {
                    let pos = [p.x as f32 / s.dpi, p.y as f32 / s.dpi];
                    let z = s.size();
                    let (tiles, _, row) = s.tiles(z[0], z[1], s.scroll);
                    if let Some(tile) = tiles.iter().find(|t| inside(t.rect, pos)) {
                        to = tile.index;
                    }
                    if pos[1] < 70.
                        || (!row && pos[1] > z[1] - 28.)
                        || (row && (pos[0] < 26. || pos[0] > z[0] - 26.))
                    {
                        s.scroll_target += if pos[1] < 70. || (row && pos[0] < 26.) {
                            -24.
                        } else {
                            24.
                        };
                        SetTimer(hwnd, 2, 16, None);
                    }
                }
                s.moving = Some((from, start, to, active));
                InvalidateRect(hwnd, null(), 0);
            } else if let Some(d) = &s.drag {
                let mut pos = zeroed();
                GetCursorPos(&mut pos);
                let dx = pos.x - d.start.x;
                let dy = pos.y - d.start.y;
                let edge = d.edge;
                let min = [(160. * s.dpi) as i32, (112. * s.dpi) as i32];
                let mut r = if edge == HTCAPTION {
                    RECT {
                        left: d.bounds.left + dx,
                        top: d.bounds.top + dy,
                        right: d.bounds.right + dx,
                        bottom: d.bounds.bottom + dy,
                    }
                } else {
                    crate::geometry::resized(d.bounds, edge, dx, dy, min)
                };
                if edge != HTCAPTION {
                    crate::fusion::snap_resize_rect(hwnd, &mut r, edge, min);
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
                InvalidateRect(hwnd, null(), 0);
            } else if let Some((i, _)) = s.slider {
                let z = s.size();
                let (t, _, _) = s.tiles(z[0], z[1], s.scroll);
                if let Some(tile) = t.iter().find(|t| t.index == i) {
                    s.slider = Some((
                        i,
                        ((p.x as f32 / s.dpi - tile.rect[0] - 14.)
                            / (tile.rect[2] - tile.rect[0] - 28.)
                            * 100.)
                            .clamp(0., 100.),
                    ));
                }
                InvalidateRect(hwnd, null(), 0);
            } else {
                let hover = s.hit(p);
                if hover != s.hover {
                    s.hover = hover;
                    InvalidateRect(hwnd, null(), 0);
                }
                let mut t = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                TrackMouseEvent(&mut t);
            }
            0
        }
        WM_LBUTTONUP => {
            s.scroll_drag = None;
            let slider = s.slider.take();
            let drag = s.drag.take().is_some();
            let action = s.pressed.take();
            let moving = s.moving.take();
            ReleaseCapture();
            if let Some((from, _, to, true)) = moving {
                if from != to && from < s.config.controls.len() && to < s.config.controls.len() {
                    let c = s.config.controls.remove(from);
                    s.config.controls.insert(to, c);
                    s.save();
                }
            } else if drag {
                s.key.clear();
                s.bounds();
            } else if let Some((i, value)) = slider {
                if let Some(c) = s.config.controls.get(i).cloned() {
                    s.command(c, value);
                }
            } else if let Some(a) = action {
                if s.hit(point(lp)) == Some(a) {
                    s.activate(a);
                }
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_CAPTURECHANGED | WM_CANCELMODE => {
            s.scroll_drag = None;
            s.moving = None;
            s.slider = None;
            s.pressed = None;
            if s.drag.take().is_some() {
                s.key.clear();
                s.bounds();
            }
            0
        }
        WM_MOUSEWHEEL => {
            s.scroll_target -= ((wp >> 16) as i16 as f32 / 120.) * 60.;
            SetTimer(hwnd, 2, 16, None);
            0
        }
        WM_CHAR if s.manager && s.search_focus => {
            if wp == 8 {
                s.manager_query.pop();
            } else if wp >= 32 {
                if let Some(c) = char::from_u32(wp as u32) {
                    if s.manager_query.len() < 120 {
                        s.manager_query.push(c);
                    }
                }
            }
            s.scroll = 0.;
            s.scroll_target = 0.;
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_KEYDOWN if wp == VK_ESCAPE as usize => {
            s.moving = None;
            s.slider = None;
            s.pressed = None;
            ReleaseCapture();
            if s.manager && !s.manager_parent.is_null() {
                s.hide();
            } else if s.manager {
                s.manager = false;
                s.scroll = 0.;
                s.scroll_target = 0.;
            } else if s.editing {
                s.open_editor();
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_MOUSELEAVE => {
            s.hover = None;
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_TIMER if wp == 2 => {
            let z = s.size();
            let (_, max, _) = s.tiles(z[0], z[1], 0.);
            let max = if s.manager {
                s.scrollbar().map_or(0., |(_, _, max, _)| max)
            } else {
                max
            };
            let max = max.max(0.);
            s.scroll_target = s.scroll_target.clamp(0., max);
            s.scroll += (s.scroll_target - s.scroll) * 0.24;
            if (s.scroll_target - s.scroll).abs() < 0.2 {
                s.scroll = s.scroll_target;
                KillTimer(hwnd, 2);
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_TIMER if wp == 1 => {
            let mut result = None;
            if let Some(worker) = &s.worker {
                while let Ok(value) = worker.rx.try_recv() {
                    result = Some(value);
                }
            }
            if let Some(result) = result {
                s.pending = false;
                s.error = result.errors.first().cloned().unwrap_or_default();
                let mut result = result;
                if let Some(next) = s.queued.take() {
                    if next.control.kind.is_input() {
                        result.mic_volume = Some(next.value);
                    } else {
                        result.volume = Some(next.value);
                    }
                    s.command(next.control, next.value);
                }
                if s.snapshot != result {
                    s.snapshot = result;
                    s.candidates = s.catalog();
                    if let Some(ui) = &s.manager_window {
                        PostMessageW(ui.hwnd(), MANAGER_SYNC, 0, 0);
                    }
                    InvalidateRect(hwnd, null(), 0);
                }
            }
            if s.last_poll.elapsed() > Duration::from_secs(5) && s.slider.is_none() {
                s.read();
            }
            SetTimer(hwnd, 1, if s.pending { 100 } else { 5000 }, None);
            if !s.inspect && !s.config.floating {
                drop(s);
                crate::window::pin_to_desktop(hwnd);
            }
            0
        }
        WM_SIZE => {
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
            0
        }
        WM_DEVICECHANGE => {
            s.read();
            0
        }
        WM_SETTINGCHANGE | WM_DISPLAYCHANGE => {
            s.glass.borrow_mut().refresh();
            s.key.clear();
            s.read();
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
    fn tile_glass_stays_opaque_when_only_widget_background_is_faded() {
        let _lock = crate::native_ui_test_lock();
        let _runtime = render::Runtime::start().unwrap();
        let root =
            std::env::temp_dir().join(format!("lumen-control-opacity-{}", std::process::id()));
        unsafe {
            let shared = Rc::new(RefCell::new(Glass::new(0., 1.)));
            let host = Host::create(&root, shared, true, true).unwrap();
            for design in ["lumen", "cloud", "soft-violet", "liquid-pastel"] {
                let mut expected = None;
                for opacity in [100., 50., 0.] {
                    let mut s = host.state.borrow_mut();
                    s.config.design = design.into();
                    s.config.opacity = opacity;
                    s.key.clear();
                    s.paint();
                    let r = s
                        .hits
                        .iter()
                        .find(|(a, _)| matches!(a, Action::Slider(_) | Action::Control(_)))
                        .unwrap()
                        .1;
                    let f = s.frame.as_ref().unwrap();
                    let pixels = f.snapshot();
                    let x = ((r[0] + r[2]) * 0.5 * s.dpi) as usize;
                    let y = ((r[1] + r[3]) * 0.5 * s.dpi) as usize;
                    let i = (y * f.width as usize + x) * 4;
                    let value = <&[u8; 4]>::try_from(&pixels[i..i + 4]).unwrap();
                    assert_eq!(
                        value[3], 255,
                        "{design} opacity={opacity}: the tile retains its own backing"
                    );
                    if let Some(reference) = expected {
                        let reference: [u8; 4] = reference;
                        assert!(
                            value
                                .iter()
                                .zip(reference)
                                .all(|(a, b)| (*a as i16 - b as i16).abs() <= 2),
                            "{design}: colors match the opaque reference"
                        );
                    } else {
                        expected = Some(*value);
                    }
                    if opacity == 0. {
                        let j = ((f.height - 20) * f.width + f.width / 2) as usize * 4;
                        assert!(pixels[j + 3] <= 1, "empty chrome remains transparent");
                    }
                }
            }
        }
    }
    #[test]
    fn wallpaper_blur_strength_changes_pixels_in_every_design() {
        let _lock = crate::native_ui_test_lock();
        let _runtime = render::Runtime::start().unwrap();
        let root =
            std::env::temp_dir().join(format!("lumen-control-materials-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("checker.png");
        image::RgbImage::from_fn(1920, 1080, |x, y| {
            image::Rgb(if (x / 8 + y / 8) % 2 == 0 {
                [45, 85, 150]
            } else {
                [235, 215, 175]
            })
        })
        .save(&path)
        .unwrap();
        unsafe {
            let shared = Rc::new(RefCell::new(Glass::new(13., 1.)));
            crate::fusion::init(shared.clone());
            let host = Host::create(&root, shared, true, true).unwrap();
            SetWindowPos(
                host.hwnd(),
                null_mut(),
                80,
                80,
                620,
                430,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            for design in ["lumen", "cloud", "soft-violet", "liquid-pastel"] {
                let mut energies = vec![];
                let mut results = vec![];
                for blur in [0., 60.] {
                    let mut s = host.state.borrow_mut();
                    s.dpi = 1.;
                    s.config.design = design.into();
                    s.config.frosted = true;
                    s.config.frost_strength = 100.;
                    s.config.tint = 0.;
                    s.config.blur = blur;
                    s.snapshot.volume = Some(47.);
                    s.snapshot.mic_volume = Some(73.);
                    s.glass = Rc::new(RefCell::new(Glass::fixture_file(&path, blur, 1.)));
                    s.key.clear();
                    s.backdrop = None;
                    s.paint();
                    let frame = s.frame.as_ref().unwrap();
                    let pixels = frame.snapshot();
                    let mut energy = 0u64;
                    for y in 260..380usize {
                        for x in 30..580usize {
                            let i = (y * frame.width as usize + x) * 4;
                            for ch in 0..3 {
                                energy += (pixels[i + ch] as i32 - pixels[i + 4 + ch] as i32)
                                    .unsigned_abs()
                                    as u64;
                            }
                        }
                    }
                    energies.push(energy);
                    results.push(pixels.clone());
                    if let Ok(folder) = std::env::var("LUMEN_CONTROL_SHOTS") {
                        let folder = PathBuf::from(folder);
                        std::fs::create_dir_all(&folder).unwrap();
                        let mut rgba = pixels;
                        for p in rgba.chunks_exact_mut(4) {
                            p.swap(0, 2);
                            let a = p[3] as u32;
                            if a > 0 {
                                for v in &mut p[..3] {
                                    *v = ((*v as u32 * 255 + a / 2) / a).min(255) as u8;
                                }
                            }
                        }
                        image::RgbaImage::from_raw(frame.width as u32, frame.height as u32, rgba)
                            .unwrap()
                            .save(folder.join(format!("{design}-blur-{blur}.png")))
                            .unwrap();
                    }
                }
                assert_ne!(
                    results[0], results[1],
                    "{design}: blur slider must change pixels"
                );
                assert!(
                    energies[1] * 2 < energies[0],
                    "{design}: blur should suppress wallpaper detail: {energies:?}"
                );
            }
            drop(host);
            crate::fusion::shutdown();
        }
        assert!(!root.join("config/native-controls.json").exists());
    }
    #[test]
    fn inline_edit_manager_and_pointer_sorting_preserve_controls_without_hardware_commands() {
        let _lock = crate::native_ui_test_lock();
        let _runtime = render::Runtime::start().unwrap();
        let root =
            std::env::temp_dir().join(format!("lumen-controls-inline-{}", std::process::id()));
        unsafe {
            let shared = Rc::new(RefCell::new(Glass::new(13., 1.)));
            crate::fusion::init(shared.clone());
            let host = Host::create(&root, shared, true, true).unwrap();
            SetWindowPos(
                host.hwnd(),
                null_mut(),
                -3000,
                -3000,
                520,
                420,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let capture = |s: &State, name: &str| {
                if let Ok(folder) = std::env::var("LUMEN_CONTROL_SHOTS") {
                    let folder = PathBuf::from(folder);
                    std::fs::create_dir_all(&folder).unwrap();
                    let f = s.frame.as_ref().unwrap();
                    let mut rgba = f.snapshot();
                    for p in rgba.chunks_exact_mut(4) {
                        p.swap(0, 2);
                        let a = p[3] as u32;
                        if a > 0 {
                            for v in &mut p[..3] {
                                *v = ((*v as u32 * 255 + a / 2) / a).min(255) as u8;
                            }
                        }
                    }
                    image::RgbaImage::from_raw(f.width as u32, f.height as u32, rgba)
                        .unwrap()
                        .save(folder.join(format!("controls-native-{name}.png")))
                        .unwrap();
                }
            };
            let original;
            let (from, to);
            {
                let mut s = host.state.borrow_mut();
                s.dpi = 1.;
                s.config.design = "soft-violet".into();
                s.snapshot.volume = Some(47.);
                s.snapshot.mic_volume = Some(73.);
                s.snapshot.outputs = vec![backend::Device {
                    id: "speaker".into(),
                    name: "Speakers".into(),
                    selected: true,
                }];
                s.config
                    .add(Kind::MicVolume, String::new(), "Microphone volume".into());
                s.config
                    .add(Kind::MicMute, String::new(), "Microphone mute".into());
                original = s.config.controls.clone();
                s.paint();
                capture(&s, "compact");
                s.activate(Action::Edit);
                assert!(s.editing);
                assert!(s.editor.is_none());
                s.hover = Some(Action::Move(0));
                s.paint();
                assert!(s.hits.iter().any(|(a, _)| *a == Action::Remove(0)));
                assert!(s.hits.iter().any(|(a, _)| *a == Action::Rename(0)));
                capture(&s, "edit");
                let (tiles, _, _) = model::layout(&s.config.controls, 520., 420., 0.);
                let center = |r: [f32; 4]| POINT {
                    x: ((r[0] + r[2]) / 2.) as i32,
                    y: ((r[1] + r[3]) / 2.) as i32,
                };
                from = center(tiles[0].rect);
                to = center(tiles[2].rect);
            }
            let lp = |p: POINT| ((p.x as u16 as u32) | ((p.y as u16 as u32) << 16)) as isize;
            SendMessageW(host.hwnd(), WM_LBUTTONDOWN, 1, lp(from));
            SendMessageW(host.hwnd(), WM_MOUSEMOVE, 1, lp(to));
            SendMessageW(host.hwnd(), WM_LBUTTONUP, 0, lp(to));
            {
                let mut s = host.state.borrow_mut();
                assert_eq!(s.config.controls[2].id, original[0].id);
                assert!(s.worker.is_none());
                assert!(!s.pending);
                s.activate(Action::Add);
                assert!(!s.manager, "Widget remains in its original layout");
                let child = s.manager_window.as_ref().unwrap();
                {
                    let mut manager = child.state.borrow_mut();
                    assert!(manager.manager);
                    manager.paint();
                    capture(&manager, "manager");
                    assert!(manager.scrollbar().is_some());
                    manager.manager_query = "microphone".into();
                    manager.paint();
                    for (action, _) in &manager.hits {
                        if let Action::Candidate(i) = action {
                            assert!(manager.candidates[*i].kind.is_input());
                        }
                    }
                    manager.manager_query.clear();
                }
                let mut bounds: RECT = zeroed();
                GetWindowRect(s.hwnd, &mut bounds);
                assert_eq!(
                    (bounds.right - bounds.left, bounds.bottom - bounds.top),
                    (520, 420)
                );
                let candidate = s
                    .candidates
                    .iter()
                    .position(|c| {
                        !s.config
                            .controls
                            .iter()
                            .any(|x| x.kind == c.kind && x.target == c.target)
                    })
                    .unwrap();
                let n = s.config.controls.len();
                s.activate(Action::Candidate(candidate));
                assert_eq!(s.config.controls.len(), n + 1);
                s.activate(Action::Candidate(candidate));
                assert_eq!(
                    s.config.controls.len(),
                    n,
                    "The manager switch removes an existing control"
                );
                assert!(!s.manager && s.editing);
                assert_eq!(s.config.controls.len(), n);
                s.activate(Action::Edit);
                assert!(!s.editing);
                assert!(s
                    .config
                    .controls
                    .iter()
                    .all(|c| original.iter().any(|x| x == c)));
            }
            drop(host);
            crate::fusion::shutdown();
        }
        assert!(!root.join("config/native-controls.json").exists());
    }
    #[test]
    fn device_hotplug_updates_group_rows_and_volume_release_keeps_the_requested_value() {
        let _lock = crate::native_ui_test_lock();
        let _runtime = render::Runtime::start().unwrap();
        let root =
            std::env::temp_dir().join(format!("lumen-grouped-controls-{}", std::process::id()));
        unsafe {
            let glass = Rc::new(RefCell::new(Glass::new(13., 1.)));
            crate::fusion::init(glass.clone());
            let host = Host::create(&root, glass, true, true).unwrap();
            {
                let mut s = host.state.borrow_mut();
                s.config.design = "soft-violet".into();
                s.config.controls.clear();
                s.config
                    .add(Kind::Display, String::new(), "Displays".into());
                s.snapshot.displays = vec![backend::Display {
                    id: "main".into(),
                    name: "DELL".into(),
                    primary: true,
                    enabled: true,
                    hdr_supported: true,
                    hdr_enabled: false,
                }];
                s.paint();
                assert!(!s.hits.iter().any(|(a, _)| *a == Action::Device(0, 0)));
                assert!(s.devices(Kind::Display)[0].1.contains("Main Display"));
                let before = s.tiles(420., 560., 0.).0[0].rect;
                s.snapshot.displays.push(backend::Display {
                    id: "new".into(),
                    name: "New monitor".into(),
                    primary: false,
                    enabled: false,
                    hdr_supported: false,
                    hdr_enabled: false,
                });
                s.paint();
                assert!(s.hits.iter().any(|(a, _)| *a == Action::Device(0, 1)));
                let after = s.tiles(420., 560., 0.).0[0].rect;
                assert_eq!(after[3] - before[3], 34.);
                s.snapshot.displays.pop();
                s.paint();
                assert!(!s.hits.iter().any(|(a, _)| *a == Action::Device(0, 1)));
                if let Ok(folder) = std::env::var("LUMEN_CONTROL_SHOTS") {
                    s.config
                        .add(Kind::Audio, String::new(), "Audio output".into());
                    s.config
                        .add(Kind::Microphone, String::new(), "Microphone".into());
                    s.config.add(Kind::Volume, String::new(), "Volume".into());
                    s.snapshot.volume = Some(64.);
                    s.snapshot.outputs = vec![
                        backend::Device {
                            id: "speaker".into(),
                            name: "Beoplay Portal".into(),
                            selected: true,
                        },
                        backend::Device {
                            id: "usb".into(),
                            name: "USB DAC".into(),
                            selected: false,
                        },
                    ];
                    s.snapshot.inputs = vec![backend::Device {
                        id: "mic".into(),
                        name: "USB Microphone".into(),
                        selected: true,
                    }];
                    for (name, width, height) in [("groups", 520, 560), ("row", 760, 120)] {
                        let dpi = s.dpi;
                        s.dpi = 1.;
                        SetWindowPos(
                            s.hwnd,
                            null_mut(),
                            -3000,
                            -3000,
                            width,
                            height,
                            SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                        s.key.clear();
                        s.paint();
                        if name == "row" {
                            let (track, thumb, max, horizontal) = s.scrollbar().unwrap();
                            assert!(horizontal && max > 0.);
                            assert!(thumb[0] >= track[0] && thumb[2] <= track[2]);
                            assert!(s.hits.iter().any(|(a, _)| *a == Action::Scroll(true)));
                        }
                        let f = s.frame.as_ref().unwrap();
                        let mut pixels = f.snapshot();
                        for p in pixels.chunks_exact_mut(4) {
                            p.swap(0, 2);
                            let a = p[3] as u32;
                            if a > 0 {
                                for c in &mut p[..3] {
                                    *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                                }
                            }
                        }
                        std::fs::create_dir_all(&folder).unwrap();
                        image::RgbaImage::from_raw(f.width as u32, f.height as u32, pixels)
                            .unwrap()
                            .save(PathBuf::from(&folder).join(format!("devices-{name}.png")))
                            .unwrap();
                        s.dpi = dpi;
                    }
                }
                s.pending = true;
                let c = Control {
                    id: "volume".into(),
                    kind: Kind::Volume,
                    target: String::new(),
                    label: "Volume".into(),
                };
                s.command(c.clone(), 64.);
                assert_eq!(s.snapshot.volume, Some(64.));
                assert_eq!(s.queued.as_ref().unwrap().value, 64.);
                s.command(c, 81.);
                assert_eq!(s.snapshot.volume, Some(81.));
                assert_eq!(s.queued.as_ref().unwrap().value, 81.);
                assert!(s.worker.is_none());
            }
            drop(host);
            crate::fusion::shutdown();
        }
    }
    #[test]
    fn all_native_control_designs_render_compact_and_large_without_edge_collisions() {
        let _lock = crate::native_ui_test_lock();
        let _runtime = render::Runtime::start().unwrap();
        let root =
            std::env::temp_dir().join(format!("lumen-controls-render-{}", std::process::id()));
        unsafe {
            let shared = Rc::new(RefCell::new(Glass::new(13., 1.)));
            crate::fusion::init(shared.clone());
            let mut host = Host::create(&root, shared.clone(), true, true).unwrap();
            let hit_dpi = host.state.borrow().dpi;
            crate::geometry::assert_widget_corner_hits(host.hwnd(), hit_dpi);
            assert!(!Rc::ptr_eq(&host.state.borrow().glass, &shared));
            for design in ["lumen", "cloud", "soft-violet", "liquid-pastel"] {
                for dpi in [1., 1.25, 1.5, 2.] {
                    for [w, h] in [[160, 112], [220, 120], [420, 560], [760, 120], [960, 360]] {
                        {
                            let mut s = host.state.borrow_mut();
                            s.config.design = design.into();
                            s.dpi = dpi;
                            s.key.clear();
                            s.frame = None;
                            s.backdrop = None;
                        }
                        SetWindowPos(
                            host.hwnd(),
                            null_mut(),
                            -3000,
                            -3000,
                            (w as f32 * dpi) as i32,
                            (h as f32 * dpi) as i32,
                            SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                        let mut s = host.state.borrow_mut();
                        s.paint();
                        let f = s.frame.as_ref().unwrap();
                        assert_eq!(
                            [f.width, f.height],
                            [(w as f32 * dpi) as i32, (h as f32 * dpi) as i32]
                        );
                        let pixels = f.snapshot();
                        assert!(pixels.chunks_exact(4).any(|p| p[3] > 200));
                        assert!(s.hits.iter().all(|(_, r)| r[1] >= 54.));
                    }
                }
            }
            host.show_settings();
            let settings = host.state.borrow().settings.as_ref().unwrap().hwnd();
            assert!(IsWindow(settings) != 0);
            host.hide();
            assert!(!host.is_visible());
            assert!(host.state.borrow().frame.is_none());
            drop(host);
            crate::fusion::shutdown();
        }
        assert!(!root.join("config/native-controls.json").exists());
    }
}
