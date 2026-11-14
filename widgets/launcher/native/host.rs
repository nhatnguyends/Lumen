//! Native Win32 host. Dock/static widgets have no browser or WebView; Music
//! explicitly delegates playback to its existing Node/browser adapter. No global
//! input hooks or screen capture are used by this host.
//!
//! FFI safety: handles are used only on their creating UI thread. The window's
//! user-data pointer refers to a boxed RefCell which outlives DestroyWindow.
//! try_borrow_mut protects against synchronous Win32 callback reentrancy.
//! GDI objects are restored before deletion; cached shell icons are destroyed
//! when App drops. Windows consumes string/struct pointers synchronously.

use crate::glass::Glass;
use crate::render::{self, Canvas};
#[path = "group_selection.rs"]
mod group_selection;
#[path = "interactions.rs"]
mod interactions;
#[path = "popup.rs"]
mod popup;
#[path = "settings.rs"]
mod settings;
use interactions::{CARET_TIMER, LAUNCH_TIMER, MOTION_TIMER};

#[derive(Clone, Copy)]
struct ResizeDrag {
    window: HWND,
    edge: u32,
    start: (i32, i32),
    bounds: RECT,
}
use crate::model::{self, wide, Dock, Item};
use std::{
    cell::RefCell,
    collections::HashMap,
    mem::{size_of, zeroed},
    path::PathBuf,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::DwmSetWindowAttribute, Gdi::*},
    System::{
        ApplicationInstallationAndServicing::*,
        LibraryLoader::GetModuleHandleW,
        Threading::{CreateMutexW, GetCurrentProcess, GetProcessTimes},
    },
    UI::{
        Controls::{Dialogs::*, EM_GETSEL, EM_LIMITTEXT, EM_SETCUEBANNER, WM_MOUSELEAVE},
        HiDpi::*,
        Input::KeyboardAndMouse::*,
        Shell::*,
        WindowsAndMessaging::*,
    },
};

const TITLE: &str = "Lumen · Native App Dock";
const KEY_FROM_SEARCH: u32 = WM_APP + 1;
const WIDGET_POINTER: u32 = WM_APP + 3;
const WIDGET_REDRAW: u32 = WM_APP + 4;
const MOTION_FRAME: u32 = WM_APP + 7;
const WALLPAPER_READY: u32 = WM_APP + 20;
const SEARCH_ID: usize = 100;
const PIN_TIMER: usize = 1;
const SMOKE_TIMER: usize = 2;
const CMD_ADD: u32 = 10;
const CMD_PC: u32 = 11;
const CMD_BIN: u32 = 12;
const CMD_THEME: u32 = 13;
const CMD_FLOAT: u32 = 14;
const CMD_QUIT: u32 = 15;
const CMD_REMOVE: u32 = 20;
const CMD_LEFT: u32 = 21;
const CMD_RIGHT: u32 = 22;
const CMD_REVEAL: u32 = 23;

#[derive(Clone, Copy)]
struct Palette {
    background: u32,
    card: u32,
    text: u32,
    muted: u32,
    line: u32,
}
fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | ((g as u32) << 8) | ((b as u32) << 16)
}
fn palette(light: bool) -> Palette {
    if light {
        Palette {
            background: rgb(240, 242, 245),
            card: rgb(250, 251, 253),
            text: rgb(29, 39, 53),
            muted: rgb(98, 112, 130),
            line: rgb(216, 222, 230),
        }
    } else {
        Palette {
            background: rgb(22, 27, 35),
            card: rgb(32, 39, 49),
            text: rgb(235, 241, 248),
            muted: rgb(183, 183, 195),
            line: rgb(105, 107, 116),
        }
    }
}

enum Icon {
    Image(render::Image),
    Shell(HICON),
}
struct App {
    frame: Option<crate::compositor::Buffer>,
    window_shadow: Option<crate::compositor::WindowShadow>,
    tray: Option<crate::tray::Tray>,
    taskbar_created: u32,
    background: Option<crate::compositor::Buffer>,
    background_key: String,
    moving_window: bool,
    accent_color: u32,
    overlay: Option<(String, render::Image)>,
    search_frost: Option<(String, render::Image)>,
    search_text: Option<(String, render::Image)>,
    list_base: Option<crate::compositor::Buffer>,
    list_base_key: String,
    pointer: [f32; 2],
    paint_time: f64,
    toolbar_pressed: Option<usize>,
    add_pressed: bool,
    bar_drag: Option<f32>,
    symbols: crate::symbols::Symbols,
    motion: crate::motion::Motion,
    tile_drag: Option<crate::motion::Drag>,
    suppress_click_until: f64,
    last_open: Option<(String, f64)>,
    open_busy_until: f64,
    pending_launch: Option<String>,
    caret_on: bool,
    hwnd: HWND,
    settings: settings::State,
    popup: Option<popup::Popup>,
    group_selection: Option<group_selection::State>,
    clear_pressed: bool,
    catalog_rx: Option<std::sync::mpsc::Receiver<Vec<Item>>>,
    catalog_loaded: f64,
    search_cache: RefCell<Option<(String, usize, Vec<usize>)>>,
    search_ink: u32,
    search_muted: u32,
    resize_drag: Option<ResizeDrag>,
    pending_resize: Option<RECT>,
    root: PathBuf,
    dock: Dock,
    desktop_watch: Option<crate::desktop_sync::Watch>,
    edit: HWND,
    search_font: HFONT,
    search_brush: HBRUSH,
    query: String,
    selected: usize,
    scroll: crate::motion::SmoothScroll,
    frame_queued: bool,
    hover: Option<usize>,
    icons: HashMap<String, Icon>,
    scaled_icons: HashMap<String, ((usize, u32), render::Image)>,
    icon_shadows: HashMap<String, (u32, render::Image)>,
    label_shadows: HashMap<String, (u32, render::Image)>,
    labels: HashMap<String, (String, render::Image)>,
    tile_cache: HashMap<String, (String, f32, render::Image)>,
    profile_counts: [u32; 4],
    label_budget: u8,
    cache_pending: bool,
    status: String,
    status_ticks: u8,
    dpi: f32,
    smoke: bool,
    inspect: bool,
    keyboard_selection: bool,
    glass: std::rc::Rc<RefCell<Glass>>,
    packages: crate::widget_packages::Registry,
    general: Option<crate::general::Host>,
    started: std::time::Instant,
    cpu_start: u64,
}
impl Drop for App {
    fn drop(&mut self) {
        // Every non-null icon in this cache was allocated with SHGFI_ICON.
        unsafe {
            for icon in self.icons.values() {
                match icon {
                    Icon::Shell(handle) => {
                        if !handle.is_null() {
                            DestroyIcon(*handle);
                        }
                    }
                    Icon::Image(_) => {}
                }
            }
            if !self.search_font.is_null() {
                DeleteObject(self.search_font);
            }
            if !self.search_brush.is_null() {
                DeleteObject(self.search_brush);
            }
        }
    }
}
impl App {
    unsafe fn widget_visibility(&self) -> Vec<(String, bool)> {
        self.packages
            .ids()
            .into_iter()
            .map(|id| {
                let visible = if id == "launcher" {
                    IsWindowVisible(self.hwnd) != 0
                } else {
                    self.packages.host(&id).is_some_and(|h| h.visible())
                };
                (id, visible)
            })
            .collect()
    }
    unsafe fn refresh_manager(&mut self) {
        let visible = self.widget_visibility();
        // External package close/crash events also update restart preferences.
        for (id, on) in &visible {
            if self.packages.host(id).is_some() {
                self.packages.remember(id, *on, self.smoke);
            }
        }
        let errors: Vec<_> = self
            .packages
            .ids()
            .iter()
            .filter_map(|id| self.packages.host(id).and_then(|h| h.error()))
            .collect();
        if let Some(host) = &mut self.general {
            host.set_visibility(visible);
            host.set_pinned(self.dock.pinned_widgets.clone());
            if let Some(error) = errors.last() {
                host.report_error(error);
            }
        }
    }
    unsafe fn show_general(&mut self) {
        if self.general.is_none() {
            match crate::general::Host::create(
                self.hwnd,
                &self.root,
                self.glass.clone(),
                self.smoke,
            ) {
                Ok(mut host) => {
                    host.set_packages(self.packages.manifests().to_vec());
                    self.general = Some(host);
                }
                Err(error) => {
                    self.status = error;
                    InvalidateRect(self.hwnd, null(), 0);
                    return;
                }
            }
        }
        self.refresh_manager();
        if let Some(host) = &mut self.general {
            host.show();
        }
    }
    unsafe fn manage(&mut self, action: crate::general::Action) -> Result<(), String> {
        use crate::general::Action::*;
        match &action {
            RefreshGlass => {
                self.notify_wallpaper_changed(self.hwnd);
            }
            RestartUpdate => {
                crate::update::restart(&self.root)?;
                PostMessageW(self.hwnd, WM_CLOSE, 0, 0);
            }
            Pin(id, pinned) => {
                if *pinned {
                    if !self.dock.pinned_widgets.iter().any(|p| p == id) {
                        self.dock.pinned_widgets.push(id.clone());
                    }
                } else {
                    self.dock.pinned_widgets.retain(|p| p != id);
                }
                self.layout_edit(self.hwnd);
                self.settings_changed(true, false);
                self.save();
            }
            Show(id) | Settings(id) => {
                let settings = matches!(&action, Settings(_));
                if id == "launcher" {
                    ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                    apply_window_mode(self.hwnd, self.dock.floating, self.inspect);
                    InvalidateRect(self.hwnd, null(), 0);
                    if settings {
                        self.open_settings(self.hwnd);
                    }
                } else {
                    self.packages
                        .ensure(id, self.glass.clone(), self.inspect, self.smoke)?;
                    if let Some(h) = self.packages.host_mut(id) {
                        if settings {
                            h.settings();
                        } else {
                            h.show();
                        }
                    }
                }
                if !settings {
                    self.packages.remember(id, true, self.smoke);
                }
            }
            Hide(id) => {
                if id == "launcher" {
                    self.cancel_pointer(self.hwnd);
                    ShowWindow(self.hwnd, SW_HIDE);
                    self.window_shadow.take();
                    self.frame = None;
                    self.background = None;
                    self.list_base = None;
                    if !self.settings.hwnd.is_null() {
                        ShowWindow(self.settings.hwnd, SW_HIDE);
                    }
                } else if let Some(h) = self.packages.host_mut(id) {
                    h.hide();
                }
                self.packages.remember(id, false, self.smoke);
            }
        }
        self.refresh_manager();
        if let Some(host) = &mut self.general {
            if matches!(&action, Settings(_)) {
                host.hide();
            } else if IsWindowVisible(host.hwnd()) != 0 {
                SetWindowPos(
                    host.hwnd(),
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
                SetForegroundWindow(host.hwnd());
            }
        }
        if let Settings(id) = action {
            if id == "launcher" && !self.settings.hwnd.is_null() {
                SetForegroundWindow(self.settings.hwnd);
            }
        }
        Ok(())
    }
    unsafe fn show_package(&mut self, id: &str) {
        if let Err(error) = self.manage(crate::general::Action::Show(id.into())) {
            eprintln!("widget-package: {error}");
            self.status = error;
        }
    }
    unsafe fn toggle_package(&mut self, id: &str) {
        let visible = if id == "launcher" {
            IsWindowVisible(self.hwnd) != 0
        } else {
            self.packages.host(id).is_some_and(|h| h.visible())
        };
        let action = if visible {
            crate::general::Action::Hide(id.into())
        } else {
            crate::general::Action::Show(id.into())
        };
        if let Err(error) = self.manage(action) {
            eprintln!("widget-package: {error}");
            self.status = error;
        }
        InvalidateRect(self.hwnd, null(), 0);
    }
    unsafe fn show_notes(&mut self) {
        self.show_package("notes");
    }
    unsafe fn show_calendar(&mut self) {
        self.show_package("calendar");
    }
    unsafe fn show_music(&mut self) {
        self.show_package("music");
    }
    unsafe fn show_clock(&mut self) {
        self.show_package("clock");
    }

    fn columns(&self, hwnd: HWND) -> usize {
        crate::geometry::columns(
            unsafe { self.dimensions(hwnd).0 },
            self.dock.columns,
            self.dock.icon_size,
        )
    }
    fn search_visible(&self) -> bool {
        let (w, h) = unsafe { self.dimensions(self.hwnd) };
        self.dock.show_search && h >= if w < 440 { 220 } else { 180 }
    }
    fn labels_visible(&self) -> bool {
        self.dock.show_labels && unsafe { self.dimensions(self.hwnd).1 } >= 150
    }
    fn grid_top(&self) -> i32 {
        if self.group_selection.is_some() && unsafe { self.dimensions(self.hwnd).0 } < 440 {
            return 72;
        }
        if self.search_visible() || self.group_selection.is_some() {
            self.header_layout(unsafe { self.dimensions(self.hwnd).0 })
                .grid_top
        } else {
            24
        }
    }
    fn row_height(&self) -> i32 {
        self.dock.icon_size + if self.labels_visible() { 42 } else { 22 }
    }
    unsafe fn start_resize(&mut self, hwnd: HWND, edge: u32, lparam: LPARAM) {
        self.cancel_pointer(hwnd);
        let mut bounds: RECT = zeroed();
        GetWindowRect(hwnd, &mut bounds);
        self.resize_drag = Some(ResizeDrag {
            window: hwnd,
            edge,
            start: xy(lparam),
            bounds,
        });
        SetCapture(hwnd);
    }
    unsafe fn move_resize(&mut self, hwnd: HWND, lparam: LPARAM) {
        let Some(drag) = self.resize_drag else {
            return;
        };
        if drag.window != hwnd {
            return;
        }
        let (x, y) = xy(lparam);
        let mut cursor = POINT { x, y };
        ClientToScreen(hwnd, &mut cursor);
        let scale = if hwnd == self.settings.hwnd {
            self.settings.dpi
        } else {
            self.dpi
        };
        let min = if hwnd == self.settings.hwnd {
            [420, 420]
        } else {
            crate::geometry::DOCK_MIN
        };
        let mut r = crate::geometry::resized(
            drag.bounds,
            drag.edge,
            cursor.x - drag.start.0,
            cursor.y - drag.start.1,
            [
                (min[0] as f32 * scale) as i32,
                (min[1] as f32 * scale) as i32,
            ],
        );
        if hwnd == self.hwnd {
            crate::fusion::snap_resize_rect(
                hwnd,
                &mut r,
                drag.edge,
                [
                    (min[0] as f32 * scale) as i32,
                    (min[1] as f32 * scale) as i32,
                ],
            );
        }
        self.pending_resize = Some(r);
        self.arm_motion(self.hwnd);
    }
    pub(super) unsafe fn apply_resize(&mut self) -> bool {
        let (Some(r), Some(drag)) = (self.pending_resize.take(), self.resize_drag) else {
            return false;
        };
        SetWindowPos(
            drag.window,
            null_mut(),
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW,
        );
        if drag.window == self.hwnd {
            self.layout_edit(self.hwnd);
        }
        InvalidateRect(drag.window, null(), 0);
        drag.window == self.hwnd
    }
    unsafe fn finish_resize(&mut self, hwnd: HWND) {
        self.apply_resize();
        self.resize_drag = None;
        InvalidateRect(hwnd, null(), 0);
        ReleaseCapture();
        if hwnd == self.hwnd {
            let mut r: RECT = zeroed();
            GetWindowRect(hwnd, &mut r);
            self.dock.position = Some([r.left, r.top]);
            self.dock.size = Some([
                ((r.right - r.left) as f32 / self.dpi).round() as i32,
                ((r.bottom - r.top) as f32 / self.dpi).round() as i32,
            ]);
            self.save();
        }
    }
    fn placeholder_opacity(&self) -> u8 {
        (148. + self.motion.search_focus.sample(self.now()) * 18.).round() as u8
    }
    fn px(&self, n: i32) -> i32 {
        (n as f32 * self.dpi).round() as i32
    }
    fn save(&mut self) {
        self.search_cache.get_mut().take();
        if let Err(error) = self.dock.save(&self.root) {
            self.status = format!("Save failed: {error}");
        }
    }
    fn visible(&self, hwnd: HWND) -> Vec<usize> {
        if let Some(drag) = self.tile_drag.as_ref().filter(|d| d.active) {
            return drag
                .preview
                .iter()
                .filter_map(|id| self.dock.items.iter().position(|i| &i.id == id))
                .collect();
        }
        if !self.query.is_empty() {
            let mut cache = self.search_cache.borrow_mut();
            if let Some((q, count, results)) = &*cache {
                if *q == self.query && *count == self.dock.items.len() {
                    return results.clone();
                }
            }
            let results = self.dock.matches(&self.query);
            *cache = Some((self.query.clone(), self.dock.items.len(), results.clone()));
            return results;
        }
        self.dock.layout_matches(&self.query, self.columns(hwnd))
    }
    unsafe fn dimensions(&self, hwnd: HWND) -> (i32, i32) {
        let mut rect: RECT = zeroed();
        GetClientRect(hwnd, &mut rect);
        (
            (rect.right as f32 / self.dpi) as i32,
            (rect.bottom as f32 / self.dpi) as i32,
        )
    }
    unsafe fn page_rows(&self, hwnd: HWND) -> usize {
        ((self.dimensions(hwnd).1 - self.grid_top() - 16) / self.row_height()).max(1) as usize
    }
    unsafe fn hit(&self, hwnd: HWND, x: i32, y: i32) -> Option<usize> {
        let (width, height) = self.dimensions(hwnd);
        if x < 16
            || x >= width - 16
            || (self.scroll.max > 0. && x >= width - 18)
            || y < self.grid_top() - 14
            || y >= height - 16
        {
            return None;
        }
        let columns = self.columns(hwnd);
        let cell = (width - 26) / columns as i32;
        let col = ((x - 16) / cell) as usize;
        let content_y = y as f32 - self.grid_top() as f32 + self.scroll.value(self.now());
        if content_y < 0. {
            return None;
        }
        let row = (content_y / self.row_height() as f32) as usize;
        let index = row * columns + col;
        (col < columns
            && index < self.visible(hwnd).len() + usize::from(self.query.trim().is_empty()))
        .then_some(index)
    }
    fn toolbar_items(&self) -> Vec<Option<String>> {
        let mut items = vec![None];
        for id in &self.dock.pinned_widgets {
            if self.packages.manifests().iter().any(|m| m.id == *id)
                && !items.iter().any(|existing| existing.as_deref() == Some(id.as_str()))
            {
                items.push(Some(id.clone()));
            }
        }
        items.push(None);
        items
    }
    fn header_layout(&self, w: i32) -> crate::geometry::DockHeader {
        crate::geometry::dock_header_with_toolbar(w, !self.query.is_empty(), self.toolbar_items().len())
    }
    fn toolbar_hit(&self, w: i32, p: [f32; 2]) -> Option<usize> {
        let count = self.toolbar_items().len();
        let header = self.header_layout(w);
        if !self.search_visible() || (p[1] - header.toolbar_y).abs() > 17. {
            return None;
        }
        let radius = if header.toolbar_step < 39. { 12. } else { 17. };
        (0..count)
            .find(|i| (p[0] - (header.toolbar_x + *i as f32 * header.toolbar_step)).abs() < radius)
    }
    unsafe fn layout_edit(&mut self, hwnd: HWND) {
        let (width, _) = self.dimensions(hwnd);
        ShowWindow(
            self.edit,
            if self.search_visible() && self.group_selection.is_none() {
                SW_SHOW
            } else {
                SW_HIDE
            },
        );
        MoveWindow(
            self.edit,
            self.px(58),
            self.px(26),
            self.px(self.header_layout(width).edit_width),
            self.px(23),
            0,
        );
    }
    unsafe fn rebuild_style(&mut self, hwnd: HWND) {
        let old_font = self.search_font;
        self.search_font = font(self.px(14), false);
        SendMessageW(self.edit, WM_SETFONT, self.search_font as usize, 1);
        if !old_font.is_null() {
            DeleteObject(old_font);
        }
        if !self.search_brush.is_null() {
            DeleteObject(self.search_brush);
        }
        self.search_brush = CreateSolidBrush(palette(self.dock.light).card);
        self.layout_edit(hwnd);
        InvalidateRect(hwnd, null(), 0);
    }
    unsafe fn cache_icon(&mut self, item: &Item) {
        if self.icons.contains_key(&item.id) {
            return;
        }
        self.profile_counts[0] += 1;
        let mut legacy_icon = None;
        if let Some(name) = &item.icon {
            // Imported Electron cache names must remain inside config/icons.
            let path = std::path::Path::new(name);
            if path.components().count() == 1
                && matches!(
                    path.components().next(),
                    Some(std::path::Component::Normal(_))
                )
            {
                if let Some(icon) = load_png(&self.root.join("config/icons").join(path)) {
                    if name.starts_with("native-custom-") {
                        self.icons.insert(item.id.clone(), icon);
                        return;
                    }
                    legacy_icon = Some(icon);
                }
            }
        }
        let source = item
            .icon_source
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(&item.target);
        let key = if item.kind == "aumid" && !source.starts_with("shell:") {
            format!("shell:AppsFolder\\{}", item.target)
        } else {
            source.into()
        };
        let _com = ComGuard::new();
        let lower = key.to_ascii_lowercase();
        let is_shortcut = lower.ends_with(".lnk") || lower.ends_with(".url");
        let is_exe = matches!(
            std::path::Path::new(&lower)
                .extension()
                .and_then(|e| e.to_str()),
            Some("exe" | "bat" | "cmd" | "com" | "msi" | "scr")
        );
        let mut image = if is_shortcut {
            system_image_list_icon(&key).or_else(|| shell_item_image(&key, 0x04 | 0x01))
        } else if key.starts_with("shell:") {
            shell_item_image(&key, 0x04 | 0x01).or_else(|| system_image_list_icon(&key))
        } else if is_exe {
            private_extract_icon(&key)
                .or_else(|| shell_item_image(&key, 0x04 | 0x01))
                .or_else(|| system_image_list_icon(&key))
        } else {
            shell_item_image(&key, 0x01).or_else(|| system_image_list_icon(&key))
        };
        let low_res = image
            .as_ref()
            .is_none_or(|img| {
                let [mx, my] = img.max_opaque_extent();
                mx <= 64 && my <= 64
            });
        if low_res && !item.target.is_empty() && item.target != key {
            let target_lower = item.target.to_ascii_lowercase();
            let target_is_exe = matches!(
                std::path::Path::new(&target_lower)
                    .extension()
                    .and_then(|e| e.to_str()),
                Some("exe" | "ico" | "dll")
            );
            let better = if target_is_exe {
                private_extract_icon(&item.target)
                    .or_else(|| shell_item_image(&item.target, 0x04 | 0x01))
            } else {
                shell_item_image(&item.target, 0x04 | 0x01)
            };
            if let Some(candidate) = better {
                let [cx, cy] = candidate.max_opaque_extent();
                let [ox, oy] = image
                    .as_ref()
                    .map_or([0, 0], |img| img.max_opaque_extent());
                if cx.max(cy) > ox.max(oy) {
                    image = Some(candidate);
                }
            }
        }
        if let Some(image) = image {
            self.icons.insert(item.id.clone(), Icon::Image(image));
            return;
        }
        if let Some(icon) = legacy_icon {
            self.icons.insert(item.id.clone(), icon);
            return;
        }
        let mut info: SHFILEINFOW = zeroed();
        if key.starts_with("shell:") {
            let mut pidl = null_mut();
            if SHParseDisplayName(wide(&key).as_ptr(), null_mut(), &mut pidl, 0, null_mut()) >= 0
                && !pidl.is_null()
            {
                SHGetFileInfoW(
                    pidl as _,
                    0,
                    &mut info,
                    size_of::<SHFILEINFOW>() as u32,
                    SHGFI_ICON | SHGFI_LARGEICON | SHGFI_PIDL,
                );
                windows_sys::Win32::System::Com::CoTaskMemFree(pidl as _);
            }
        } else {
            SHGetFileInfoW(
                wide(&key).as_ptr(),
                0,
                &mut info,
                size_of::<SHFILEINFOW>() as u32,
                SHGFI_ICON | SHGFI_LARGEICON,
            );
        }
        let icon = if !info.hIcon.is_null() {
            if let Some(image) = render::Image::from_icon(info.hIcon) {
                DestroyIcon(info.hIcon);
                Icon::Image(image)
            } else {
                Icon::Shell(info.hIcon)
            }
        } else {
            Icon::Shell(info.hIcon)
        };
        self.icons.insert(item.id.clone(), icon);
    }
    unsafe fn notify_wallpaper_changed(&mut self, hwnd: HWND) {
        if let Some(host) = &mut self.general {
            host.refresh();
        }
        self.packages.refresh();
        crate::fusion::refresh();
        let color = self.glass.borrow().wallpaper_accent();
        self.set_accent(color);
        self.background_key.clear();
        InvalidateRect(hwnd, null(), 0);
        if !self.settings.hwnd.is_null() {
            InvalidateRect(self.settings.hwnd, null(), 0);
        }
    }
    unsafe fn open_item(&mut self, hwnd: HWND, index: usize) {
        let Some(item) = self.dock.items.get(index) else {
            return;
        };
        let target = launch_target(item);
        let parameters = item.args.as_ref().map(wide);
        let cwd = item.cwd.as_ref().map(wide);
        // Open a user-selected catalog item through the Windows shell; no command
        // interpreter or string-built command line is introduced by the host.
        let result = ShellExecuteW(
            hwnd,
            wide("open").as_ptr(),
            wide(&target).as_ptr(),
            parameters.as_ref().map_or(null(), |s| s.as_ptr()),
            cwd.as_ref().map_or(null(), |s| s.as_ptr()),
            SW_SHOWNORMAL,
        ) as isize;
        self.status = if result <= 32 {
            format!("Could not open {} (Windows error {result})", item.name)
        } else {
            format!("Opened {}", item.name)
        };
        InvalidateRect(hwnd, null(), 0);
    }
    unsafe fn add_files(&mut self, hwnd: HWND) {
        let mut buffer = vec![0u16; 32768];
        let filter = wide(
            "All files, apps and shortcuts\0*.*\0Applications and shortcuts\0*.exe;*.lnk;*.url\0",
        );
        let mut dialog: OPENFILENAMEW = zeroed();
        dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
        dialog.hwndOwner = hwnd;
        dialog.lpstrFilter = filter.as_ptr();
        dialog.lpstrFile = buffer.as_mut_ptr();
        dialog.nMaxFile = buffer.len() as u32;
        // Keep the title allocated while the synchronous common dialog is open.
        let title = wide("Add files, applications or shortcuts");
        dialog.lpstrTitle = title.as_ptr();
        dialog.Flags = OFN_NODEREFERENCELINKS
            | OFN_EXPLORER
            | OFN_FILEMUSTEXIST
            | OFN_PATHMUSTEXIST
            | OFN_NOCHANGEDIR
            | OFN_ALLOWMULTISELECT;
        if GetOpenFileNameW(&mut dialog) == 0 {
            let error = CommDlgExtendedError();
            if error != 0 {
                self.status = format!("File picker failed ({error})");
            }
            return;
        }
        let parts: Vec<String> = buffer
            .split(|c| *c == 0)
            .take_while(|s| !s.is_empty())
            .map(String::from_utf16_lossy)
            .collect();
        let paths = if parts.len() <= 1 {
            parts
        } else {
            parts[1..]
                .iter()
                .map(|p| {
                    PathBuf::from(&parts[0])
                        .join(p)
                        .to_string_lossy()
                        .into_owned()
                })
                .collect()
        };
        let mut count = 0;
        for path in paths {
            count += usize::from(self.dock.add(&path));
        }
        self.status = format!("Added {count} item(s)");
        self.save();
        InvalidateRect(hwnd, null(), 0);
    }
    unsafe fn add_anything(&mut self, hwnd: HWND) {
        let mut point: POINT = zeroed();
        GetCursorPos(&mut point);
        ScreenToClient(hwnd, &mut point);
        self.open_popup(
            hwnd,
            popup::Kind::Add,
            [point.x as f32 / self.dpi, point.y as f32 / self.dpi],
        );
    }
    unsafe fn add_folder(&mut self, hwnd: HWND) {
        use windows_sys::Win32::System::Com::*;
        let initialized = CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32) >= 0;
        let title = wide("Choose any folder to add to the dock");
        let mut display = [0u16; 260];
        let mut browse: BROWSEINFOW = zeroed();
        browse.hwndOwner = hwnd;
        browse.pszDisplayName = display.as_mut_ptr();
        browse.lpszTitle = title.as_ptr();
        browse.ulFlags = BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE;
        let selected = SHBrowseForFolderW(&browse);
        if !selected.is_null() {
            let mut path = [0u16; 260];
            if SHGetPathFromIDListW(selected, path.as_mut_ptr()) != 0 {
                let len = path.iter().position(|c| *c == 0).unwrap_or(path.len());
                if self.dock.add(&String::from_utf16_lossy(&path[..len])) {
                    self.save();
                    self.status = "Added folder".into();
                }
            }
            CoTaskMemFree(selected as _);
        }
        if initialized {
            CoUninitialize();
        }
        InvalidateRect(hwnd, null(), 0);
    }
    unsafe fn activate_toolbar(&mut self, hwnd: HWND, button: usize) {
        let items = self.toolbar_items();
        if button == 0 {
            if let Some(host) = &mut self.general {
                if IsWindowVisible(host.hwnd()) != 0 {
                    host.hide();
                    InvalidateRect(hwnd, null(), 0);
                    return;
                }
            }
            self.show_general();
            return;
        }
        if button + 1 == items.len() {
            if !self.settings.hwnd.is_null() && IsWindowVisible(self.settings.hwnd) != 0 {
                ShowWindow(self.settings.hwnd, SW_HIDE);
                InvalidateRect(hwnd, null(), 0);
                return;
            }
            let mut point = POINT {
                x: self.px(self.dimensions(hwnd).0 - 40),
                y: self.px(62),
            };
            ClientToScreen(hwnd, &mut point);
            self.context_menu(hwnd, point.x, point.y, None);
            return;
        }
        if let Some(Some(id)) = items.get(button) {
            self.toggle_package(id);
        }
    }
    unsafe fn context_menu(&mut self, hwnd: HWND, x: i32, y: i32, item: Option<usize>) {
        if item.is_none() {
            self.open_settings(hwnd);
            return;
        }
        let mut point = POINT { x, y };
        ScreenToClient(hwnd, &mut point);
        if let Some(index) = item {
            if let Some(item) = self.dock.items.get(index) {
                self.open_popup(
                    hwnd,
                    popup::Kind::Item(item.id.clone()),
                    [point.x as f32 / self.dpi, point.y as f32 / self.dpi],
                );
            }
        }
    }
    unsafe fn context_command(&mut self, hwnd: HWND, result: u32, item: Option<usize>) {
        match result as u32 {
            CMD_ADD => self.add_anything(hwnd),
            CMD_PC | CMD_BIN => {
                let (target, name) = if result as u32 == CMD_PC {
                    ("shell:MyComputerFolder", "This PC")
                } else {
                    ("shell:RecycleBinFolder", "Recycle Bin")
                };
                if self.dock.add(target) {
                    let i = self.dock.items.last_mut().unwrap();
                    i.name = name.into();
                    i.kind = "shell".into();
                }
                self.save();
            }
            CMD_THEME => {
                self.dock.light = !self.dock.light;
                self.rebuild_style(hwnd);
                self.save();
            }
            CMD_FLOAT => {
                self.dock.floating = !self.dock.floating;
                apply_window_mode(hwnd, self.dock.floating, self.inspect);
                self.save();
            }
            CMD_QUIT => {
                PostMessageW(hwnd, WM_CLOSE, 0, 0);
            }
            CMD_REMOVE => {
                if let Some(index) = item {
                    self.dock.items.remove(index);
                    self.selected = 0;
                    self.save();
                }
            }
            CMD_LEFT => {
                if let Some(index) = item {
                    if index > 0 {
                        self.dock.items.swap(index, index - 1);
                        self.save();
                    }
                }
            }
            CMD_RIGHT => {
                if let Some(index) = item {
                    if index + 1 < self.dock.items.len() {
                        self.dock.items.swap(index, index + 1);
                        self.save();
                    }
                }
            }
            CMD_REVEAL => {
                if let Some(index) = item {
                    let i = &self.dock.items[index];
                    if std::path::Path::new(&i.target).exists() {
                        ShellExecuteW(
                            hwnd,
                            wide("open").as_ptr(),
                            wide("explorer.exe").as_ptr(),
                            wide(format!("/select,\"{}\"", i.target)).as_ptr(),
                            null(),
                            SW_SHOWNORMAL,
                        );
                    } else {
                        self.status = "This item has no local file to reveal".into();
                    }
                }
            }
            n @ 1003..=1008 => {
                self.dock.columns = (n - 1000) as usize;
                self.scroll.snap(0.);
                self.save();
            }
            _ => {}
        }
        InvalidateRect(hwnd, null(), 0);
    }
    unsafe fn key(&mut self, hwnd: HWND, key: u32) {
        if self.group_selection_key(hwnd, key) {
            return;
        }
        if self.popup_key(hwnd, key) {
            return;
        }
        if key == 0x1b && self.tile_drag.is_some() {
            self.cancel_pointer(hwnd);
            return;
        }
        self.keyboard_selection = true;
        let columns = self.columns(hwnd);
        let visible = self.visible(hwnd);
        match key {
            0x0D => {
                if let Some(index) = visible.get(self.selected) {
                    self.launch(hwnd, *index);
                }
            }
            0x1B => {
                self.keyboard_selection = false;
                if !self.query.is_empty() {
                    SetWindowTextW(self.edit, wide("").as_ptr());
                    self.query.clear();
                    self.selected = 0;
                    self.scroll.snap(0.);
                } else {
                    SetFocus(self.edit);
                }
            }
            0x25 => self.selected = self.selected.saturating_sub(1),
            0x27 => self.selected = (self.selected + 1).min(visible.len().saturating_sub(1)),
            0x26 => self.selected = self.selected.saturating_sub(columns),
            0x28 => self.selected = (self.selected + columns).min(visible.len().saturating_sub(1)),
            _ => return,
        }
        let y =
            self.grid_top() as f32 + (self.selected / columns) as f32 * self.row_height() as f32;
        let offset = self.scroll.target();
        let bottom = self.dimensions(hwnd).1 as f32 - 40.;
        if y - offset < self.grid_top() as f32 {
            self.scroll.to(y - self.grid_top() as f32, self.now());
        } else if y + self.row_height() as f32 - offset > bottom {
            self.scroll
                .to(y + self.row_height() as f32 - bottom, self.now());
        }
        self.arm_motion(hwnd);
        InvalidateRect(hwnd, null(), 0);
    }
    unsafe fn paint_translucent_search_text(
        &mut self,
        dc: HDC,
        hwnd: HWND,
        width: i32,
        height: i32,
    ) {
        let mut start = 0u32;
        let mut end = 0u32;
        SendMessageW(
            self.edit,
            EM_GETSEL,
            &mut start as *mut _ as usize,
            &mut end as *mut _ as isize,
        );
        let first = SendMessageW(
            self.edit,
            windows_sys::Win32::UI::Controls::EM_GETFIRSTVISIBLELINE,
            0,
            0,
        );
        let key = format!(
            "{}:{start}:{end}:{first}:{width}:{height}:{}:{}:{}",
            self.query,
            self.dpi,
            self.search_ink,
            GetFocus() == self.edit
        );
        if self.search_text.as_ref().is_none_or(|(old, _)| *old != key) {
            let mut mattes = Vec::new();
            for white in [false, true] {
                let Some(buffer) = crate::compositor::Buffer::plain(width, height, 1.) else {
                    return;
                };
                PatBlt(
                    buffer.dc,
                    0,
                    0,
                    width,
                    height,
                    if white { WHITENESS } else { BLACKNESS },
                );
                let brush = CreateSolidBrush(if white { 0xffffff } else { 0 });
                SEARCH_PALETTE.with(|p| p.set(Some((hwnd, self.edit, brush, self.search_ink))));
                SendMessageW(
                    self.edit,
                    WM_PRINTCLIENT,
                    buffer.dc as usize,
                    (PRF_CLIENT | PRF_ERASEBKGND) as isize,
                );
                SEARCH_PALETTE.with(|p| p.set(None));
                DeleteObject(brush);
                mattes.push(buffer.snapshot());
            }
            self.search_text = render::Image::from_rgba(
                width as u32,
                height as u32,
                render::recover_icon_alpha(&mattes[0], &mattes[1]),
            )
            .map(|image| (key, image));
        }
        if let Some((_, image)) = &self.search_text {
            Canvas::new(dc, self.dpi).image(
                image,
                [
                    58.,
                    26.,
                    58. + width as f32 / self.dpi,
                    26. + height as f32 / self.dpi,
                ],
            );
        }
    }
    unsafe fn paint(&mut self, hwnd: HWND) {
        let frame_started = std::time::Instant::now();
        let counts_before = self.profile_counts;
        let now = self.now();
        self.label_budget = if self.scroll.moving(now) { 2 } else { 64 };
        self.cache_pending = false;
        render::take_cost();
        let mut ps: PAINTSTRUCT = zeroed();
        BeginPaint(hwnd, &mut ps);
        let mut client: RECT = zeroed();
        GetClientRect(hwnd, &mut client);
        let mut frame = self
            .frame
            .take()
            .filter(|f| f.width == client.right && f.height == client.bottom && f.dpi == self.dpi)
            .or_else(|| crate::compositor::Buffer::new(client.right, client.bottom, self.dpi, 34.))
            .expect("native paint buffer");
        let dc = frame.dc;
        let mut p = palette(self.dock.light);
        self.paint_time = now;
        self.motion.search_focus.target(
            if GetFocus() == self.edit { 1. } else { 0. },
            now,
            200.,
            [0.25, 0.1, 0.25, 1.],
        );
        let (w, h) = self.dimensions(hwnd);
        let mut window: RECT = zeroed();
        GetWindowRect(hwnd, &mut window);
        let key = format!(
            "{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
            window.left,
            window.top,
            client.right,
            client.bottom,
            self.dpi,
            self.dock.theme,
            self.dock.frosted,
            self.dock.tint,
            self.dock.frost_strength,
            self.dock.light,
            self.glass.borrow().revision(),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            crate::fusion::revision(hwnd)
        );
        let bg_cached = key == self.background_key && self.background.is_some();
        let own_alpha = (self.dock.opacity * 2.55).round() as u8;
        let group_op = crate::fusion::group_opacities(hwnd, own_alpha);
        let toolbar_items = self.toolbar_items();
        let fast_header_key = if bg_cached
            && self.resize_drag.is_none()
            && self.group_selection.is_none()
            && !self.background_key.is_empty()
        {
            let mut hk = format!(
                "{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
                self.background_key,
                self.search_visible(),
                self.query,
                GetFocus() == self.edit && self.caret_on,
                (self.motion.search_clear.sample(now) * 1000.).round() as i32,
                self.dock.opacity,
                (self.motion.search_focus.sample(now) * 1000.).round() as i32,
                self.placeholder_opacity(),
                self.search_ink,
                self.search_muted,
                w,
                self.dock.pinned_widgets.join(",")
            );
            if let Some((_, ops)) = &group_op {
                for o in ops {
                    hk.push_str(&format!(":{o}"));
                }
            }
            for i in 0..toolbar_items.len() {
                let a = self.motion.toolbar.get(&i).map_or(0., |t| t.sample(now));
                let r = self
                    .motion
                    .toolbar_rotation
                    .get(&i)
                    .map_or(0., |t| t.sample(now));
                let p = self
                    .motion
                    .toolbar_press
                    .get(&i)
                    .map_or(1., |t| t.sample(now));
                hk.push_str(&format!(
                    ":{}:{}:{}",
                    (a * 1000.).round() as i32,
                    (r * 100.).round() as i32,
                    (p * 1000.).round() as i32
                ));
            }
            hk
        } else {
            String::new()
        };
        let fast_reused_header = !fast_header_key.is_empty()
            && fast_header_key == self.list_base_key
            && self
                .list_base
                .as_ref()
                .is_some_and(|b| b.width == frame.width && b.height == frame.height && b.dpi == self.dpi);
        if fast_reused_header {
            // Skip copying self.background because self.list_base will overwrite the frame.
        } else if bg_cached {
            frame.copy_from(self.background.as_ref().unwrap().dc);
        } else {
            if std::env::var_os("LUMEN_TRACE_FRAMES").is_some() {
                eprintln!("native-backdrop-rebuild moving={}", self.moving_window);
            }
            let brush = CreateSolidBrush(p.background);
            FillRect(dc, &client, brush);
            DeleteObject(brush);
            self.glass.borrow().draw_blended(
                dc,
                hwnd,
                client.right,
                client.bottom,
                if self.dock.frosted {
                    (self.dock.frost_strength * 255. / 100.).round() as u8
                } else {
                    0
                },
            );
            let tint = if self.dock.frosted {
                self.dock.tint
            } else {
                100.
            };
            let overlay_key = format!(
                "{}:{}:{}:{}:{}:{}",
                client.right,
                client.bottom,
                self.dpi,
                self.theme_color(),
                tint,
                self.dock.light
            );
            if (self.resize_drag.is_none() || self.overlay.is_none())
                && self
                    .overlay
                    .as_ref()
                    .is_none_or(|(key, _)| *key != overlay_key)
            {
                self.overlay = Canvas::pane_overlay(
                    [client.right as u32, client.bottom as u32],
                    self.dpi,
                    tint / 100.,
                    self.dock.light,
                )
                .map(|image| (overlay_key, image));
            }
            let fused = crate::fusion::fused(hwnd);
            let pane = if fused {
                [0., 0., w as f32, h as f32]
            } else {
                [4., 4., w as f32 - 4., h as f32 - 4.]
            };
            let radius = if fused { 0. } else { 34. };
            {
                let c = Canvas::new(dc, self.dpi);
                if crate::surface::managed(&self.dock.theme) {
                    crate::surface::paint(&c, pane, &self.dock.theme, tint / 100.);
                } else {
                    c.round(
                        pane,
                        radius,
                        self.theme_color(),
                        (tint * 2.55).round() as u8,
                        false,
                        false,
                    );
                }
                c.coating(pane, radius);
            }
            if let Some((_, image)) = &self.overlay {
                Canvas::new(dc, self.dpi).image(
                    image,
                    [
                        0.,
                        0.,
                        client.right as f32 / self.dpi,
                        client.bottom as f32 / self.dpi,
                    ],
                );
            }
            // Build the nested search material before the decorative glow.
            // Reuse it while resizing, but never bake the glow into its frost.
            let prefix = format!("dpi{}:", self.dpi);
            let live_reuse = self.resize_drag.is_some()
                && self
                    .search_frost
                    .as_ref()
                    .is_some_and(|(old, _)| old.starts_with(&prefix));
            if !live_reuse {
                self.search_frost = frame
                    .frosted_piece_opacity(
                        [16., 14., w as f32 - 16., 60.],
                        23.,
                        8.,
                        [-2., -1.],
                        self.theme_color(),
                        0.25,
                    )
                    .map(|image| (format!("{prefix}{key}"), image));
            }
            Canvas::new(dc, self.dpi).widget_corner_reflection_for(
                hwnd,
                pane,
                34.,
                self.dock.light,
            );
            frame.bevel_for(hwnd);
            let cache = self
                .background
                .take()
                .filter(|f| {
                    f.width == client.right && f.height == client.bottom && f.dpi == self.dpi
                })
                .or_else(|| crate::compositor::Buffer::plain(client.right, client.bottom, self.dpi))
                .unwrap();
            cache.copy_from(dc);
            self.background = Some(cache);
            self.background_key = if self.resize_drag.is_some() {
                String::new()
            } else {
                key
            };
        }
        let bg_ms = frame_started.elapsed().as_secs_f64() * 1000.;
        if !fast_reused_header {
            let luminance =
                crate::contrast::sample(dc, self.dpi, [16., 14., (w - 16) as f32, (h - 14) as f32]);
            let (ink, muted) = crate::contrast::inks(luminance, Some(self.search_ink));
            self.search_ink = ink;
            self.search_muted = muted;
        }
        p.text = self.search_ink;
        p.muted = self.search_muted;
        let needs_alpha = self.dock.opacity < 100. || group_op.is_some();
        let alpha_batch = if needs_alpha {
            render::AlphaBatch::new(dc, self.dpi)
        } else {
            None
        };
        if alpha_batch.is_some() && !fast_reused_header {
            frame.background_alpha_for(self.hwnd, own_alpha);
        }
        SetBkMode(dc, TRANSPARENT as i32);
        let header_key = if !fast_header_key.is_empty() {
            fast_header_key
        } else if self.resize_drag.is_none()
            && self.group_selection.is_none()
            && !self.background_key.is_empty()
        {
            let mut hk = format!(
                "{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
                self.background_key,
                self.search_visible(),
                self.query,
                GetFocus() == self.edit && self.caret_on,
                (self.motion.search_clear.sample(now) * 1000.).round() as i32,
                self.dock.opacity,
                (self.motion.search_focus.sample(now) * 1000.).round() as i32,
                self.placeholder_opacity(),
                self.search_ink,
                self.search_muted,
                w,
                self.dock.pinned_widgets.join(",")
            );
            if let Some((_, ops)) = &group_op {
                for o in ops {
                    hk.push_str(&format!(":{o}"));
                }
            }
            for i in 0..toolbar_items.len() {
                let a = self.motion.toolbar.get(&i).map_or(0., |t| t.sample(now));
                let r = self
                    .motion
                    .toolbar_rotation
                    .get(&i)
                    .map_or(0., |t| t.sample(now));
                let p = self
                    .motion
                    .toolbar_press
                    .get(&i)
                    .map_or(1., |t| t.sample(now));
                hk.push_str(&format!(
                    ":{}:{}:{}",
                    (a * 1000.).round() as i32,
                    (r * 100.).round() as i32,
                    (p * 1000.).round() as i32
                ));
            }
            hk
        } else {
            String::new()
        };
        let reused_header = fast_reused_header
            || (!header_key.is_empty()
                && header_key == self.list_base_key
                && self
                    .list_base
                    .as_ref()
                    .is_some_and(|b| b.width == frame.width && b.height == frame.height && b.dpi == self.dpi));
        if reused_header {
            frame.copy_from(self.list_base.as_ref().unwrap().dc);
        } else {
        if self.search_visible() || self.group_selection.is_some() {
            let rect = [16., 14., w as f32 - 16., 60.];
            let prefix = format!("dpi{}:", self.dpi);
            let live_reuse = self.resize_drag.is_some()
                && self
                    .search_frost
                    .as_ref()
                    .is_some_and(|(old, _)| old.starts_with(&prefix));
            if let Some((_, image)) = &self.search_frost {
                let c = Canvas::new(dc, self.dpi);
                if live_reuse {
                    c.image_horizontal_alpha(image, rect, (23. * self.dpi).round() as u32, 1.);
                } else {
                    c.image(image, rect);
                }
            }
        }
        if self.search_visible() && self.group_selection.is_none() {
            Canvas::new(dc, self.dpi).focus_shadow(
                [16., 14., w as f32 - 16., 60.],
                23.,
                self.motion.search_focus.sample(now),
            );
            Canvas::new(dc, self.dpi).search_surface(
                [16., 14., w as f32 - 16., 60.],
                23.,
                self.theme_color(),
                self.dock.light,
            );
            let (ink, muted) = self.adaptive_ink(
                dc,
                [
                    58.,
                    26.,
                    (58. + (self.query.chars().count().max(13) as f32 * 7.2)
                        .min((w - 300).max(40) as f32)),
                    49.,
                ],
                "search",
            );
            self.search_ink = ink;
            self.search_muted = muted;
            p.text = ink;
            p.muted = muted;
            self.symbols
                .draw_rotated_alpha(dc, self.dpi, 0, 32., 28., 16., p.muted, 0., 0.62);
            let header = self.header_layout(w);
            let toolbar_dy = header.toolbar_y - 37.;
            if !header.compact {
                let divider = (header.toolbar_x - 24.).round() as i32;
                self.stroke(dc, &[(divider, 28), (divider, 46)], p.line, 1);
            }
            for (i, item) in toolbar_items.iter().enumerate() {
                let amount = self
                    .motion
                    .toolbar
                    .get(&i)
                    .map_or(0., |t| t.sample(now));
                let x = header.toolbar_x + i as f32 * header.toolbar_step;
                if amount > 0.001 {
                    Canvas::new(dc, self.dpi).round(
                        [x - 15., 21. + toolbar_dy, x + 15., 51. + toolbar_dy],
                        15.,
                        0xffffff,
                        (amount
                            * if crate::contrast::luminance(p.text) < 0.5 {
                                128.
                            } else {
                                36.
                            }) as u8,
                        false,
                        false,
                    );
                }
                let (toolbar_ink, _toolbar_muted) = self.adaptive_ink(
                    dc,
                    [x - 14., 22. + toolbar_dy, x + 14., 50. + toolbar_dy],
                    &format!("toolbar-{i}"),
                );
                let color = toolbar_ink;
                let angle = self
                    .motion
                    .toolbar_rotation
                    .get(&i)
                    .map_or(0., |t| t.sample(now));
                let press_scale = self
                    .motion
                    .toolbar_press
                    .get(&i)
                    .map_or(1., |t| t.sample(now));
                let symbol = if i == 0 {
                    1
                } else if i + 1 == toolbar_items.len() {
                    5
                } else if let Some(id) = item {
                    crate::symbols::widget_symbol(id)
                } else {
                    1
                };
                let size = 16. * press_scale;
                self.symbols.draw_rotated_alpha(
                    dc,
                    self.dpi,
                    symbol,
                    x - size * 0.5,
                    36. + toolbar_dy - size * 0.5,
                    size,
                    color,
                    angle,
                    0.85 + amount * 0.15,
                );
            }
            if !self.query.is_empty() {
                let x = header.clear_x;
                let hover = self.motion.search_clear.sample(now);
                let canvas = Canvas::new(dc, self.dpi);
                canvas.round(
                    [x - 11., 26., x + 11., 48.],
                    11.,
                    p.text,
                    (17. + hover * 13.).round() as u8,
                    false,
                    false,
                );
                canvas.line(&[(x - 3., 34.), (x + 3., 40.)], p.text, 220, 1.2);
                canvas.line(&[(x + 3., 34.), (x - 3., 40.)], p.text, 220, 1.2);
            }
            // The Edit uses the exact glass pixels beneath it as its background.
            render::flush(dc);
            let ew = self.px(header.edit_width);
            let eh = self.px(23);
            let edit_dc = CreateCompatibleDC(dc);
            let edit_bitmap = CreateCompatibleBitmap(dc, ew, eh);
            let old = SelectObject(edit_dc, edit_bitmap);
            BitBlt(edit_dc, 0, 0, ew, eh, dc, self.px(58), self.px(26), SRCCOPY);
            if !self.search_brush.is_null() {
                DeleteObject(self.search_brush);
            }
            self.search_brush = CreatePatternBrush(edit_bitmap);
            SEARCH_PALETTE
                .with(|p| p.set(Some((hwnd, self.edit, self.search_brush, self.search_ink))));
            if alpha_batch.is_some() {
                if !self.query.is_empty() {
                    self.paint_translucent_search_text(dc, hwnd, ew, eh);
                }
            } else {
                if !self.query.is_empty() {
                    SendMessageW(
                        self.edit,
                        WM_PRINTCLIENT,
                        edit_dc as usize,
                        (PRF_CLIENT | PRF_ERASEBKGND) as isize,
                    );
                }
                BitBlt(dc, self.px(58), self.px(26), ew, eh, edit_dc, 0, 0, SRCCOPY);
            }
            SEARCH_PALETTE.with(|p| p.set(None));
            if self.query.trim().is_empty() {
                Canvas::new(dc, self.dpi).text_alpha(
                    "Search app...",
                    [58., 26., header.placeholder_right, 49.],
                    13.5,
                    false,
                    p.muted,
                    0,
                    self.placeholder_opacity(),
                );
            }
            if GetFocus() == self.edit && self.caret_on {
                let mut a = 0u32;
                let mut b = 0u32;
                SendMessageW(
                    self.edit,
                    EM_GETSEL,
                    &mut a as *mut _ as usize,
                    &mut b as *mut _ as isize,
                );
                if a == b {
                    let mut point: POINT = zeroed();
                    if GetCaretPos(&mut point) != 0 {
                        let x = 58. + point.x as f32 / self.dpi;
                        Canvas::new(dc, self.dpi).line(&[(x, 28.), (x, 45.)], p.text, 230, 0.8);
                    }
                }
            }

            SelectObject(edit_dc, old);
            DeleteObject(edit_bitmap);
            DeleteDC(edit_dc);
        }
        self.paint_group_selection_header(dc, w);
        render::flush(dc);
        }
        let header_ms = frame_started.elapsed().as_secs_f64() * 1000.;
        let content_count = self.visible(hwnd).len()
            + usize::from(self.query.trim().is_empty() && self.group_selection.is_none());
        let needs_fade = self.grid_top()
            + content_count.div_ceil(self.columns(hwnd)) as i32 * self.row_height()
            > h - 14;
        let base = if needs_fade || !header_key.is_empty() {
            let base = self
                .list_base
                .take()
                .filter(|b| b.width == frame.width && b.height == frame.height && b.dpi == self.dpi)
                .or_else(|| crate::compositor::Buffer::plain(frame.width, frame.height, self.dpi))
                .unwrap();
            if !reused_header {
                base.copy_from(dc);
                self.list_base_key = header_key;
            }
            Some(base)
        } else {
            self.list_base = None;
            self.list_base_key.clear();
            None
        };
        let clip = SaveDC(dc);
        IntersectClipRect(
            dc,
            self.px(14),
            self.px(self.grid_top() - 14),
            self.px(w - 14),
            self.px(h - 14),
        );
        self.paint_tiles(dc, hwnd, w, h);
        render::flush(dc);
        let tiles_ms = frame_started.elapsed().as_secs_f64() * 1000.;
        RestoreDC(dc, clip);
        if let Some(base) = base.as_ref().filter(|_| self.scroll.max > 0.) {
            frame.fade_list(base, (self.grid_top() - 14) as f32, (h - 14) as f32);
        }
        self.list_base = base;
        self.paint_floating(dc, hwnd, w);
        if let Some(bar) = crate::geometry::scrollbar(
            w,
            h,
            self.grid_top(),
            self.scroll.max,
            self.scroll.value(now),
        ) {
            let (bar_ink, _) = self.adaptive_ink(dc, bar, "scrollbar");
            frame.fill_round(
                bar,
                1.5,
                bar_ink,
                (90. * self.motion.scrollbar.sample(now)) as u8,
            );
        }
        self.motion.status_visible(!self.status.is_empty(), now);
        let status_a = self.motion.status_alpha.sample(now);
        if !self.status.is_empty() && status_a > 0.01 {
            let dy = ((1. - status_a) * 6.).round() as i32;
            self.surface(
                dc,
                [w / 2 - 160, h - 38 + dy, w / 2 + 160, h - 10 + dy],
                14,
                (40. * status_a).round() as u8,
                false,
            );
            self.text(
                dc,
                &self.status,
                [w / 2 - 150, h - 38 + dy, w / 2 + 150, h - 10 + dy],
                10,
                false,
                p.text,
                DT_CENTER | DT_END_ELLIPSIS,
            );
        }
        self.paint_popup(dc, hwnd);
        if self.cache_pending {
            self.ensure_motion_timer(hwnd);
        }
        let cpu_ms = frame_started.elapsed().as_secs_f64() * 1000.;
        let preserve_alpha = needs_alpha;
        drop(alpha_batch);
        frame.present_with_alpha(hwnd, preserve_alpha);
        if self.window_shadow.is_none() {
            self.window_shadow = crate::compositor::WindowShadow::new(hwnd);
        }
        if let Some(shadow) = &mut self.window_shadow {
            shadow.sync(hwnd, self.dpi, (self.dock.opacity * 2.55).round() as u8);
        }
        let present_ms = frame_started.elapsed().as_secs_f64() * 1000.;
        self.frame = Some(frame);
        EndPaint(hwnd, &ps);
        if self.motion.timer_running {
            crate::motion::pace_vsync(hwnd, frame_started, &mut self.frame_queued, MOTION_FRAME);
        }
        if std::env::var_os("LUMEN_TRACE_FRAMES").is_some() {
            eprintln!(
                "native-frame ms={:.2} cpu={:.2} present={:.2} bg={:.2} header={:.2} tiles={:.2} regen={:?} draw_ns={:?} at={:.1}",
                frame_started.elapsed().as_secs_f64() * 1000.,
                cpu_ms,
                present_ms,
                bg_ms, header_ms, tiles_ms,
                std::array::from_fn::<u32, 4, _>(|i| self.profile_counts[i] - counts_before[i]),
                render::take_cost(),
                self.now()
            );
        }
    }
    unsafe fn surface(&self, dc: HDC, rect: [i32; 4], radius: i32, alpha: u8, outline: bool) {
        Canvas::new(dc, self.dpi).round(
            rect.map(|n| n as f32),
            radius as f32,
            rgb(255, 255, 255),
            alpha,
            outline,
            false,
        );
    }
    unsafe fn stroke(&self, dc: HDC, points: &[(i32, i32)], color: u32, width: i32) {
        Canvas::new(dc, self.dpi).line(
            &points
                .iter()
                .map(|(x, y)| (*x as f32, *y as f32))
                .collect::<Vec<_>>(),
            color,
            255,
            width as f32,
        );
    }
    unsafe fn text(
        &self,
        dc: HDC,
        text: &str,
        r: [i32; 4],
        height: i32,
        bold: bool,
        color: u32,
        align: u32,
    ) {
        Canvas::new(dc, self.dpi).text(
            text,
            r.map(|n| n as f32),
            height as f32,
            bold,
            color,
            if align & DT_CENTER != 0 {
                1
            } else if align & DT_RIGHT != 0 {
                2
            } else {
                0
            },
        );
    }
}

fn launch_target(item: &Item) -> String {
    if item.kind == "aumid" {
        format!("shell:AppsFolder\\{}", item.target)
    } else if item.kind == "url" && !item.target.contains(':') {
        format!("https://{}", item.target)
    } else {
        item.target.clone()
    }
}
/// Cache high-resolution source icons without resizing them before rendering.
unsafe fn load_png(path: &std::path::Path) -> Option<Icon> {
    if std::fs::metadata(path).ok()?.len() > 16 * 1024 * 1024 {
        return None;
    }
    render::Image::load(path).map(Icon::Image)
}

struct ComGuard(bool);
impl ComGuard {
    unsafe fn new() -> Self {
        use windows_sys::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
        Self(CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32) >= 0)
    }
}
impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                windows_sys::Win32::System::Com::CoUninitialize();
            }
        }
    }
}

struct ComPtr(*mut std::ffi::c_void);
impl Drop for ComPtr {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let release: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32 =
                    std::mem::transmute(com_slot(self.0, 2));
                release(self.0);
            }
        }
    }
}

unsafe fn com_slot(object: *mut std::ffi::c_void, index: usize) -> *const std::ffi::c_void {
    let vtable = *(object as *const *const *const std::ffi::c_void);
    *vtable.add(index)
}

unsafe fn private_extract_icon(key: &str) -> Option<render::Image> {
    let mut hicon: HICON = null_mut();
    let mut icon_id: u32 = 0;
    let count = PrivateExtractIconsW(
        wide(key).as_ptr(),
        0,
        256,
        256,
        &mut hicon,
        &mut icon_id,
        1,
        0,
    );
    if count == 0 || count == u32::MAX || hicon.is_null() {
        return None;
    }
    let image = render::Image::from_icon(hicon);
    DestroyIcon(hicon);
    image
}

unsafe fn shell_item_image(key: &str, flags: u32) -> Option<render::Image> {
    let iid = windows_sys::core::GUID::from_u128(0xbcc18b79_ba16_442f_80c4_8a59c30c463b);
    let mut factory = ComPtr(null_mut());
    if SHCreateItemFromParsingName(wide(key).as_ptr(), null_mut(), &iid, &mut factory.0) < 0
        || factory.0.is_null()
    {
        return None;
    }
    let get_image: unsafe extern "system" fn(
        *mut std::ffi::c_void,
        SIZE,
        u32,
        *mut HBITMAP,
    ) -> i32 = std::mem::transmute(com_slot(factory.0, 3));
    let mut hbitmap: HBITMAP = null_mut();
    if get_image(factory.0, SIZE { cx: 256, cy: 256 }, flags, &mut hbitmap) < 0
        || hbitmap.is_null()
    {
        return None;
    }
    let image = render::Image::from_hbitmap(hbitmap);
    DeleteObject(hbitmap as _);
    image
}

unsafe fn system_image_list_icon(key: &str) -> Option<render::Image> {
    let mut info: SHFILEINFOW = zeroed();
    let ok = if key.starts_with("shell:") {
        let mut pidl = null_mut();
        let res =
            if SHParseDisplayName(wide(key).as_ptr(), null_mut(), &mut pidl, 0, null_mut()) >= 0
                && !pidl.is_null()
            {
                let r = SHGetFileInfoW(
                    pidl as _,
                    0,
                    &mut info,
                    size_of::<SHFILEINFOW>() as u32,
                    SHGFI_SYSICONINDEX | SHGFI_PIDL,
                );
                windows_sys::Win32::System::Com::CoTaskMemFree(pidl as _);
                r
            } else {
                0
            };
        res != 0
    } else {
        SHGetFileInfoW(
            wide(key).as_ptr(),
            0,
            &mut info,
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_SYSICONINDEX,
        ) != 0
    };
    if !ok {
        return None;
    }
    let iid = windows_sys::core::GUID::from_u128(0x46eb5926_582e_4017_9fdf_e8998daa0950);
    for (list_id, min_extent) in [(SHIL_JUMBO, 56u32), (SHIL_EXTRALARGE, 36u32), (SHIL_LARGE, 0u32)]
    {
        let mut list = ComPtr(null_mut());
        if SHGetImageList(list_id as i32, &iid, &mut list.0) < 0 || list.0.is_null() {
            continue;
        }
        let get_icon: unsafe extern "system" fn(
            *mut std::ffi::c_void,
            i32,
            u32,
            *mut HICON,
        ) -> i32 = std::mem::transmute(com_slot(list.0, 10));
        let mut hicon: HICON = null_mut();
        if get_icon(
            list.0,
            info.iIcon,
            windows_sys::Win32::UI::Controls::ILD_TRANSPARENT,
            &mut hicon,
        ) < 0
            || hicon.is_null()
        {
            continue;
        }
        let image = render::Image::from_icon(hicon);
        DestroyIcon(hicon);
        if let Some(image) = image {
            let [mx, my] = image.max_opaque_extent();
            if min_extent > 0 && mx <= min_extent && my <= min_extent {
                continue;
            }
            return Some(image);
        }
    }
    None
}

unsafe fn font(height: i32, bold: bool) -> HFONT {
    CreateFontW(
        -height,
        0,
        0,
        0,
        if bold { 500 } else { 400 },
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        OUT_DEFAULT_PRECIS as u32,
        CLIP_DEFAULT_PRECIS as u32,
        ANTIALIASED_QUALITY as u32,
        DEFAULT_PITCH as u32,
        wide("Segoe UI Variable Text").as_ptr(),
    )
}
pub(crate) unsafe fn apply_window_mode(hwnd: HWND, _floating: bool, inspect: bool) {
    crate::desktop_layer::register(hwnd, !inspect);
    // Legacy saved flags remain readable, but widgets always belong to the desktop.
    let floating = false;
    let owner = if !floating && !inspect {
        FindWindowW(wide("Progman").as_ptr(), null())
    } else {
        null_mut()
    };
    SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, owner as isize);
    SetWindowPos(
        hwnd,
        if floating {
            HWND_TOPMOST
        } else {
            HWND_NOTOPMOST
        },
        0,
        0,
        0,
        0,
        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
    );
    if !floating && !inspect {
        pin_to_desktop(hwnd);
    }
}
/// Same z-order strategy as src/main/win32.ts: immediately ABOVE the highest
/// desktop surface, below ordinary applications. Never reparents into Explorer.
pub(crate) unsafe fn pin_to_desktop(hwnd: HWND) {
    crate::desktop_layer::pin(hwnd);
}
fn xy(lparam: LPARAM) -> (i32, i32) {
    (
        (lparam as u16 as i16) as i32,
        ((lparam >> 16) as u16 as i16) as i32,
    )
}
unsafe fn round_window(hwnd: HWND, _scale: f32, _radius: i32) {
    SetWindowRgn(hwnd, null_mut(), 0);
}
thread_local! {static SEARCH_PALETTE:std::cell::Cell<Option<(HWND,HWND,HBRUSH,u32)>>=const{std::cell::Cell::new(None)};}
// This subclass paints only our own search control's empty-state hint. It is
// local to this Edit HWND, with no keyboard hooks or global input observation.
unsafe extern "system" fn search_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    parent: usize,
) -> LRESULT {
    if message == WM_PAINT && GetWindowTextLengthW(hwnd) == 0 {
        let pointer = GetWindowLongPtrW(parent as HWND, GWLP_USERDATA) as *const RefCell<App>;
        if !pointer.is_null() {
            if let Ok(app) = (*pointer).try_borrow() {
                let mut ps: PAINTSTRUCT = zeroed();
                let dc = BeginPaint(hwnd, &mut ps);
                let mut rect: RECT = zeroed();
                GetClientRect(hwnd, &mut rect);
                FillRect(dc, &rect, app.search_brush);
                Canvas::new(dc, app.dpi).text_alpha(
                    "Search app...",
                    [
                        0.,
                        0.,
                        rect.right as f32 / app.dpi,
                        rect.bottom as f32 / app.dpi,
                    ],
                    13.5,
                    false,
                    app.search_muted,
                    0,
                    app.placeholder_opacity(),
                );
                EndPaint(hwnd, &ps);
                return 0;
            }
        }
    }
    let result = DefSubclassProc(hwnd, message, wparam, lparam);
    if matches!(
        message,
        WM_KEYUP
            | WM_CHAR
            | WM_LBUTTONDOWN
            | WM_LBUTTONUP
            | WM_MOUSEMOVE
            | WM_SETFOCUS
            | WM_KILLFOCUS
    ) {
        if message == WM_SETFOCUS {
            SetTimer(parent as HWND, CARET_TIMER, 500, None);
        }
        if message == WM_KILLFOCUS {
            KillTimer(parent as HWND, CARET_TIMER);
        }
        if message == WM_MOUSEMOVE {
            PostMessageW(parent as HWND, WIDGET_POINTER, 0, 0);
            let mut tracking = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            TrackMouseEvent(&mut tracking);
        } else {
            PostMessageW(parent as HWND, WIDGET_REDRAW, 0, 0);
        }
    }
    if message == WM_MOUSELEAVE {
        PostMessageW(parent as HWND, WIDGET_POINTER, 0, 0);
    }
    result
}

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_CTLCOLOREDIT {
        let override_brush = SEARCH_PALETTE.with(|p| p.get());
        if let Some((parent, edit, brush, color)) = override_brush {
            if hwnd == parent && lparam as HWND == edit {
                SetTextColor(wparam as HDC, color);
                SetBkMode(wparam as HDC, TRANSPARENT as i32);
                SetBrushOrgEx(wparam as HDC, 0, 0, null_mut());
                return brush as isize;
            }
        }
    }
    // Region changes and move/resize callbacks can reenter while App is borrowed.
    // Keep non-client sizing independent so Windows cannot restore a frame.
    if message == WM_NCCALCSIZE {
        return 0;
    }
    if message == WM_NCPAINT {
        return 0;
    }
    if message == WM_NCACTIVATE {
        return 1;
    }
    if message == WM_ERASEBKGND {
        return 1;
    }
    if message == WM_NCCREATE {
        let create = &*(lparam as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<App>;
    if pointer.is_null() {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
    let Ok(mut app) = (*pointer).try_borrow_mut() else {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    };
    match message {
        WM_CREATE => {
            app.hwnd = hwnd;
            app.dpi = GetDpiForWindow(hwnd) as f32 / 96.0;
            if std::env::args().any(|a| a == "--smoke-test") {
                app.glass.borrow_mut().refresh();
            } else {
                app.glass.borrow_mut().refresh_async(hwnd, WALLPAPER_READY);
            }
            let accent = app.glass.borrow().wallpaper_accent();
            app.accent_color = accent;
            if app.dock.theme == crate::accent::THEME {
                app.dock.light = crate::accent::light(app.accent_color);
            }
            app.edit = CreateWindowExW(
                0,
                wide("EDIT").as_ptr(),
                wide("").as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL as u32,
                0,
                0,
                10,
                10,
                hwnd,
                SEARCH_ID as HMENU,
                GetModuleHandleW(null()),
                null(),
            );
            if app.edit.is_null() {
                return -1;
            }
            SendMessageW(
                app.edit,
                EM_SETCUEBANNER,
                1,
                wide("Search app...").as_ptr() as isize,
            );
            SendMessageW(app.edit, EM_LIMITTEXT, 256, 0);
            SetWindowSubclass(app.edit, Some(search_proc), 1, hwnd as usize);
            app.rebuild_style(hwnd);
            ShowWindow(
                app.edit,
                if app.search_visible() {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
            DragAcceptFiles(hwnd, 1);
            app.tray = crate::tray::Tray::new(hwnd, &app.root);
            let yes = 1u32;
            DwmSetWindowAttribute(hwnd, 11, &yes as *const _ as _, 4);
            DwmSetWindowAttribute(hwnd, 12, &yes as *const _ as _, 4);
            let border = 0xfffffffeu32;
            DwmSetWindowAttribute(hwnd, 34, &border as *const _ as _, 4);
            let corners = 1u32;
            DwmSetWindowAttribute(hwnd, 33, &corners as *const _ as _, 4);
            apply_window_mode(hwnd, app.dock.floating, app.inspect);
            SetTimer(hwnd, PIN_TIMER, 1000, None);
            if app.smoke {
                SetTimer(hwnd, SMOKE_TIMER, 5000, None);
            }
            0
        }
        message if message == app.taskbar_created && message != 0 => {
            if let Some(tray) = &app.tray {
                tray.add();
            }
            0
        }
        message if message == WM_APP + 31 => {
            app.show_clock();
            0
        }
        message if message == WM_APP + 32 => {
            use crate::general::Action;
            let widget = match wparam {
                1 => "music",
                2 => "notes",
                3 => "calendar",
                4 => "clock",
                _ => "launcher",
            };
            if let Err(error) = app.manage(Action::Show(widget.into())) {
                app.status = error;
            }
            0
        }
        message if message == crate::general::ACTION_MESSAGE => {
            let actions = app
                .general
                .as_mut()
                .map(|h| h.take_actions())
                .unwrap_or_default();
            for action in actions {
                if let Err(error) = app.manage(action) {
                    eprintln!("native-general: {error}");
                    if let Some(host) = &mut app.general {
                        host.report_error(&error);
                    }
                }
            }
            0
        }
        crate::tray::MESSAGE => {
            let action = match lparam as u32 {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => 1,
                WM_RBUTTONUP => app.tray.as_ref().map_or(0, |t| t.menu()),
                _ => 0,
            };
            if action == 1 || action == 2 {
                ShowWindow(hwnd, SW_SHOWNORMAL);
                SetForegroundWindow(hwnd);
                SetWindowPos(
                    hwnd,
                    HWND_NOTOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
                InvalidateRect(hwnd, null(), 0);
            }
            if action == 2 {
                app.open_settings(hwnd);
            }
            if action == 5 {
                app.show_music();
            }
            if action == 4 {
                app.show_clock();
            }
            if action == 7 {
                app.show_notes();
            }
            if action == 6 {
                app.show_calendar();
            }
            if action == 8 || action == 9 {
                app.show_general();
            }
            if action == 3 {
                PostMessageW(hwnd, WM_CLOSE, 0, 0);
            }
            0
        }
        WM_PAINT => {
            app.paint(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_CTLCOLOREDIT => {
            if let Some(popup) = app.popup.as_ref().filter(|p| p.edit == lparam as HWND) {
                SetTextColor(wparam as HDC, rgb(232, 230, 240));
                SetBkColor(wparam as HDC, rgb(28, 27, 39));
                return popup.brush as isize;
            }
            SetTextColor(wparam as HDC, app.search_ink);
            SetBkMode(wparam as HDC, TRANSPARENT as i32);
            SetBrushOrgEx(wparam as HDC, 0, 0, null_mut());
            app.search_brush as isize
        }
        WM_DISPLAYCHANGE => {
            app.background_key.clear();
            InvalidateRect(hwnd, null(), 0);
            if !app.settings.hwnd.is_null() {
                InvalidateRect(app.settings.hwnd, null(), 0);
            }
            0
        }
        WM_SETTINGCHANGE | WM_THEMECHANGED => {
            let refreshed = app.glass.borrow_mut().refresh();
            if refreshed {
                app.notify_wallpaper_changed(hwnd);
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_SIZE => {
            app.popup = None;
            app.layout_edit(hwnd);

            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_WINDOWPOSCHANGING => {
            let dpi = app.dpi;
            if let Some(shadow) = &mut app.window_shadow {
                shadow.follow_position(&*(lparam as *const WINDOWPOS), dpi);
            }
            drop(app);
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_WINDOWPOSCHANGED => {
            let dpi = app.dpi;
            let opacity = (app.dock.opacity * 2.55).round() as u8;
            if let Some(shadow) = &mut app.window_shadow {
                shadow.sync(hwnd, dpi, opacity);
            }
            drop(app);
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_MOVE => {
            if (!app.moving_window || app.dock.frosted || crate::glass::liquid_glass())
                && crate::glass::move_paint_due(hwnd)
            {
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_NCLBUTTONDOWN if (HTLEFT..=HTBOTTOMRIGHT).contains(&(wparam as u32)) => {
            app.start_resize(hwnd, wparam as u32, lparam);
            0
        }
        WM_LBUTTONUP => {
            let (x, y) = xy(lparam);
            let p = [x as f32 / app.dpi, y as f32 / app.dpi];
            if app.popup_up(hwnd, p) {
                return 0;
            }
            if app.group_selection_up(hwnd, p) {
                return 0;
            }
            if app.clear_pressed {
                app.clear_pressed = false;
                ReleaseCapture();
                if app.clear_hit(hwnd, p) {
                    app.clear_search(hwnd);
                }
                return 0;
            }
            if app.bar_drag.take().is_some() {
                ReleaseCapture();
            } else if let Some(button) = app.toolbar_pressed.take() {
                let count = app.toolbar_items().len();
                let now = app.now();
                app.motion.toolbar_press(None, count, now);
                app.arm_motion(hwnd);
                ReleaseCapture();
                if app.toolbar_hit(app.dimensions(hwnd).0, p) == Some(button) {
                    app.activate_toolbar(hwnd, button);
                }
            } else if app.add_pressed {
                app.add_pressed = false;
                let now = app.now();
                app.motion.press(None, now);
                app.arm_motion(hwnd);
                ReleaseCapture();
                if app.hit(hwnd, p[0] as i32, p[1] as i32) == Some(app.visible(hwnd).len()) {
                    app.add_anything(hwnd);
                }
            } else if app.resize_drag.is_some() {
                app.move_resize(hwnd, lparam);
                app.finish_resize(hwnd);
            } else {
                if app.tile_drag.as_ref().is_some_and(|d| d.active) {
                    app.pointer_move(hwnd, p);
                }
                app.pointer_up(hwnd);
            }
            0
        }
        WM_CAPTURECHANGED => {
            app.resize_drag = None;
            app.pending_resize = None;
            app.clear_pressed = false;
            app.toolbar_pressed = None;
            app.add_pressed = false;
            app.bar_drag = None;
            if let Some(selection) = &mut app.group_selection {
                selection.pressed = None;
            }
            if let Some(popup) = &mut app.popup {
                popup.pressed = None;
            }
            if app.tile_drag.is_some() {
                app.cancel_pointer(hwnd);
            }
            0
        }
        WM_CANCELMODE | WM_KILLFOCUS => {
            if let Some(selection) = &mut app.group_selection {
                selection.pressed = None;
            }
            if !app.popup.as_ref().is_some_and(|p| p.edit == wparam as HWND) {
                app.popup = None;
            }
            app.cancel_pointer(hwnd);
            0
        }
        WM_GETMINMAXINFO => {
            let info = &mut *(lparam as *mut MINMAXINFO);
            info.ptMinTrackSize = POINT {
                x: app.px(crate::geometry::DOCK_MIN[0]),
                y: app.px(crate::geometry::DOCK_MIN[1]),
            };
            0
        }
        WM_NCHITTEST => {
            let (x, y) = xy(lparam);
            let mut rect: RECT = zeroed();
            GetWindowRect(hwnd, &mut rect);
            if crate::compositor::distance(
                (x - rect.left) as f32,
                (y - rect.top) as f32,
                (rect.right - rect.left) as f32,
                (rect.bottom - rect.top) as f32,
                34. * app.dpi,
                4. * app.dpi,
            ) > 0.5
            {
                return HTTRANSPARENT as i32 as isize;
            }
            let edge = crate::geometry::edge(
                x - rect.left,
                y - rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                app.px(10),
                app.px(40),
            );
            if edge != HTCLIENT {
                return edge as isize;
            }
            if y < rect.top + app.px(14)
                || (app.search_visible()
                    && app.dimensions(hwnd).0 >= 440
                    && y >= rect.top + app.px(62)
                    && y < rect.top + app.px(78))
            {
                HTCAPTION as isize
            } else {
                HTCLIENT as isize
            }
        }

        WM_DPICHANGED => {
            app.dpi = (wparam as u16) as f32 / 96.0;
            let rect = &*(lparam as *const RECT);
            SetWindowPos(
                hwnd,
                null_mut(),
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            app.rebuild_style(hwnd);
            0
        }
        WM_ENTERSIZEMOVE => {
            app.moving_window = true;
            if std::env::var_os("LUMEN_TRACE_FRAMES").is_some() {
                eprintln!("native-move freeze=true");
            }
            0
        }
        WM_EXITSIZEMOVE => {
            app.moving_window = false;
            if std::env::var_os("LUMEN_TRACE_FRAMES").is_some() {
                eprintln!("native-move freeze=false");
            }
            let mut rect: RECT = zeroed();
            GetWindowRect(hwnd, &mut rect);
            app.dock.position = Some([rect.left, rect.top]);
            app.dock.size = Some([
                ((rect.right - rect.left) as f32 / app.dpi).round() as i32,
                ((rect.bottom - rect.top) as f32 / app.dpi).round() as i32,
            ]);
            app.save();
            app.background_key.clear();
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_COMMAND if (wparam & 0xffff) == SEARCH_ID && (wparam >> 16) as u32 == EN_CHANGE => {
            let len = GetWindowTextLengthW(app.edit).clamp(0, 256) as usize;
            let mut text = vec![0u16; len + 1];
            GetWindowTextW(app.edit, text.as_mut_ptr(), text.len() as i32);
            app.query = String::from_utf16_lossy(&text[..len]).trim().to_owned();
            app.layout_edit(hwnd);
            app.selected = 0;
            app.scroll.snap(0.);
            let now = app.now();
            let p = app.pointer;
            app.motion.hover(None, p, now);
            app.hover = None;
            app.arm_motion(hwnd);
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_KEYDOWN | KEY_FROM_SEARCH => {
            app.key(hwnd, wparam as u32);
            0
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            let (x, y) = xy(lparam);
            let x = (x as f32 / app.dpi) as i32;
            let y = (y as f32 / app.dpi) as i32;
            let (width, _) = app.dimensions(hwnd);
            let p = [x as f32, y as f32];
            if app.popup_down(hwnd, p) {
                return 0;
            }
            if app.clear_hit(hwnd, p) {
                app.clear_pressed = true;
                SetCapture(hwnd);
                return 0;
            }
            if let Some(bar) = crate::geometry::scrollbar(
                width,
                app.dimensions(hwnd).1,
                app.grid_top(),
                app.scroll.max,
                app.scroll.value(app.now()),
            ) {
                if p[0] >= bar[0] - 7.
                    && p[0] <= bar[2] + 7.
                    && p[1] >= bar[1] - 3.
                    && p[1] <= bar[3] + 3.
                {
                    SetFocus(hwnd);
                    let value = app.scroll.value(app.now());
                    app.scroll.snap(value);
                    app.bar_drag = Some(p[1] - bar[1]);
                    SetCapture(hwnd);
                    return 0;
                }
            }
            if app.group_selection_down(hwnd, p) {
                return 0;
            }
            if let Some(button) = app.toolbar_hit(width, p) {
                SetFocus(hwnd);
                app.toolbar_pressed = Some(button);
                let count = app.toolbar_items().len();
                let now = app.now();
                app.motion.toolbar_press(Some(button), count, now);
                app.arm_motion(hwnd);
                SetCapture(hwnd);
            } else if app.search_visible() && (14..60).contains(&y) {
                SetFocus(app.edit);
            } else if let Some(pos) = app.hit(hwnd, x, y) {
                if pos == app.visible(hwnd).len() {
                    SetFocus(hwnd);
                    app.add_pressed = true;
                    let now = app.now();
                    app.motion.press(Some("__add".into()), now);
                    app.arm_motion(hwnd);
                    SetCapture(hwnd);
                    return 0;
                }
                app.pointer_down(hwnd, pos, [x as f32, y as f32]);
            }
            0
        }
        WM_MOUSEMOVE => {
            if app.resize_drag.is_some() {
                app.move_resize(hwnd, lparam);
                return 0;
            }
            let (x, y) = xy(lparam);
            let dpi = app.dpi;
            app.widget_hover(hwnd, true);
            if app.popup_move(hwnd, [x as f32 / dpi, y as f32 / dpi]) {
                return 0;
            }
            if let Some(grab) = app.bar_drag {
                let value = crate::geometry::scroll_drag(
                    app.dimensions(hwnd).1,
                    app.grid_top(),
                    app.scroll.max,
                    y as f32 / dpi,
                    grab,
                );
                app.scroll.snap(value);
                app.arm_motion(hwnd);
            } else {
                app.pointer_move(hwnd, [x as f32 / dpi, y as f32 / dpi]);
            }
            let mut tracking = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            TrackMouseEvent(&mut tracking);
            0
        }
        WM_MOUSELEAVE => {
            app.check_widget_pointer(hwnd);
            app.popup_leave(hwnd);
            if app.tile_drag.is_none() {
                app.hover_tile(hwnd, [-1000., -1000.]);
            }
            0
        }
        popup::POPUP_KEY => {
            app.popup_key(hwnd, wparam as u32);
            0
        }
        WM_COMMAND if (wparam & 0xffff) == popup::EDIT_ID => {
            app.arm_motion(hwnd);
            0
        }
        WIDGET_REDRAW => {
            app.arm_motion(hwnd);
            0
        }
        WIDGET_POINTER => {
            app.check_widget_pointer(hwnd);
            0
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) as u16 as i16) as i32;
            let now = app.now();
            if app.popup_wheel(hwnd, delta) {
                return 0;
            }
            let pointer = app.pointer;
            app.hover = None;
            app.motion.hover(None, pointer, now);
            app.motion.group_target(None, now);
            app.scroll.by(-(delta as f32) / 120. * 72., now);
            app.arm_motion(hwnd);
            0
        }
        WM_CONTEXTMENU => {
            if app.group_selection.is_some() {
                return 0;
            }
            let (mut x, mut y) = xy(lparam);
            let item = if x == -1 && y == -1 {
                let mut point = POINT {
                    x: app.px(36),
                    y: app.px(150),
                };
                ClientToScreen(hwnd, &mut point);
                x = point.x;
                y = point.y;
                app.visible(hwnd).get(app.selected).copied()
            } else {
                let mut point = POINT { x, y };
                ScreenToClient(hwnd, &mut point);
                app.hit(
                    hwnd,
                    (point.x as f32 / app.dpi) as i32,
                    (point.y as f32 / app.dpi) as i32,
                )
                .and_then(|pos| app.visible(hwnd).get(pos).copied())
            };
            app.context_menu(hwnd, x, y, item);
            0
        }
        WM_DROPFILES => {
            let drop = wparam as HDROP;
            let count = DragQueryFileW(drop, u32::MAX, null_mut(), 0);
            let mut added = 0;
            for n in 0..count {
                let len = DragQueryFileW(drop, n, null_mut(), 0);
                let mut path = vec![0u16; len as usize + 1];
                DragQueryFileW(drop, n, path.as_mut_ptr(), len + 1);
                added += usize::from(
                    app.dock
                        .add(&String::from_utf16_lossy(&path[..len as usize])),
                );
            }
            DragFinish(drop);
            app.status = format!("Added {added} item(s)");
            app.save();
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_TIMER if wparam == MOTION_TIMER => {
            app.motion_tick(hwnd);
            0
        }
        MOTION_FRAME => {
            app.frame_queued = false;
            if app.motion.timer_running {
                app.motion_tick(hwnd);
            }
            0
        }
        WALLPAPER_READY => {
            if app.glass.borrow_mut().finish_async() {
                let accent = app.glass.borrow().wallpaper_accent();
                app.accent_color = accent;
                if app.dock.theme == crate::accent::THEME {
                    app.dock.light = crate::accent::light(app.accent_color);
                    app.rebuild_style(hwnd);
                }
                app.background_key.clear();
                app.packages.refresh();
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_TIMER if wparam == LAUNCH_TIMER => {
            KillTimer(hwnd, LAUNCH_TIMER);
            if let Some(id) = app.pending_launch.take() {
                if let Some(index) = app.dock.items.iter().position(|i| i.id == id) {
                    app.open_item(hwnd, index);
                    let now = app.now();
                    app.motion.launch_finished(now);
                    app.open_busy_until = now + 280.;
                    app.arm_motion(hwnd);
                }
            }
            0
        }
        WM_TIMER if wparam == CARET_TIMER => {
            app.caret_on = !app.caret_on;
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_TIMER if wparam == PIN_TIMER => {
            app.sync_desktop_items(hwnd);
            if let Some(host) = &mut app.general {
                host.poll_update();
            }
            app.refresh_manager();
            app.packages.tick();
            if let Some(items) = app.catalog_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
                app.search_cache.get_mut().take();
                app.catalog_rx = None;
                app.catalog_loaded = app.now();
                app.dock.items.retain(|i| !i.catalog);
                let mut names: std::collections::HashSet<_> = app
                    .dock
                    .items
                    .iter()
                    .map(|i| i.name.to_lowercase())
                    .collect();
                let mut targets: std::collections::HashSet<_> = app
                    .dock
                    .items
                    .iter()
                    .map(|i| i.target.to_lowercase())
                    .collect();
                app.dock.items.extend(items.into_iter().filter(|i| {
                    names.insert(i.name.to_lowercase()) && targets.insert(i.target.to_lowercase())
                }));
                if !app.query.is_empty() {
                    app.arm_motion(hwnd);
                }
            }
            if app.catalog_rx.is_none() && app.now() - app.catalog_loaded > 300000. {
                app.catalog_rx = Some(crate::catalog::start());
            }

            if !app.status.is_empty() {
                app.status_ticks += 1;
                if app.status_ticks >= 3 {
                    app.status.clear();
                    app.status_ticks = 0;
                    InvalidateRect(hwnd, null(), 0);
                }
            }
            let refreshed = app.glass.borrow_mut().refresh();
            if refreshed {
                app.notify_wallpaper_changed(hwnd);
            }
            if !app.dock.floating && !app.inspect {
                pin_to_desktop(hwnd);
            }
            0
        }
        WM_TIMER if wparam == SMOKE_TIMER => {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
            0
        }
        WM_CLOSE => {
            app.tile_drag = None;
            if let Some(host) = &mut app.general {
                host.hide();
            }
            if !app.settings.hwnd.is_null() {
                let settings_hwnd = app.settings.hwnd;
                app.settings.hwnd = null_mut();
                DestroyWindow(settings_hwnd);
            }
            let mut rect: RECT = zeroed();
            GetWindowRect(hwnd, &mut rect);
            app.dock.position = Some([rect.left, rect.top]);
            if !app.smoke {
                app.dock.size = Some([
                    ((rect.right - rect.left) as f32 / app.dpi).round() as i32,
                    ((rect.bottom - rect.top) as f32 / app.dpi).round() as i32,
                ]);
                app.save();
            }
            // Drop mutable borrow before destruction, so WM_DESTROY can borrow.
            drop(app);
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            report_metrics(app.started.elapsed().as_secs_f64(), app.cpu_start);
            KillTimer(hwnd, PIN_TIMER);
            KillTimer(hwnd, SMOKE_TIMER);
            for id in [MOTION_TIMER, LAUNCH_TIMER, CARET_TIMER] {
                KillTimer(hwnd, id);
            }
            PostQuitMessage(0);
            0
        }
        WM_NCDESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        _ => {
            if message >= 0xc000 {
                if message == crate::widget_packages::manager_message() {
                    app.show_general();
                    return 0;
                }
                if let Some(id) = app.packages.message_package(message).map(str::to_owned) {
                    app.show_package(&id);
                    return 0;
                }
            }
            drop(app);
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
    }
}

pub fn run() -> Result<(), String> {
    let _apartment = crate::catalog::Apartment::new();
    let _graphics = render::Runtime::start()?;
    let root = model::root();
    if crate::update::portable(&root) {
        let folder = root.join("config");
        std::fs::create_dir_all(&folder).map_err(|e|format!("Extract Lumen to a writable user folder (not Program Files). Cannot create its config folder: {e}"))?;
        let probe = folder.join(format!(".write-check-{}", std::process::id()));
        let file=std::fs::OpenOptions::new().write(true).create_new(true).open(&probe)
            .map_err(|e|format!("Lumen needs write access to its config folder. Move the extracted folder to Downloads or another writable location: {e}"))?;
        drop(file);
        std::fs::remove_file(probe).map_err(|e| e.to_string())?;
    }
    let mut dock = Dock::load(&root)?;
    let accent_color = crate::accent::vivid(crate::accent::FALLBACK);
    if dock.theme == crate::accent::THEME {
        dock.light = crate::accent::light(accent_color);
    }
    let smoke = std::env::args().any(|a| a == "--smoke-test");
    let inspect = std::env::args().any(|a| a == "--inspect");
    // All UI calls run on this thread; the boxed state outlives the native window.
    unsafe {
        let process_dpi_ok =
            SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let dpi_error = GetLastError();
        let before = GetAwarenessFromDpiAwarenessContext(GetThreadDpiAwarenessContext());
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        if std::env::var_os("LUMEN_TRACE_FRAMES").is_some() {
            eprintln!("native-dpi process_ok={process_dpi_ok} error={dpi_error} previous_awareness={before}");
        }
        let manifest = wide(root.join(if crate::update::portable(&root) {
            "resources/lumen.manifest"
        } else {
            "native-dock/lumen.manifest"
        }));
        let activation = ACTCTXW {
            cbSize: size_of::<ACTCTXW>() as u32,
            lpSource: manifest.as_ptr(),
            ..zeroed()
        };
        let context = CreateActCtxW(&activation);
        let mut activation_cookie = 0;
        if context != INVALID_HANDLE_VALUE {
            ActivateActCtx(context, &mut activation_cookie);
        }
        let mutex_name = if smoke {
            format!("Local\\LumenNativeDockSmoke-{}", std::process::id())
        } else {
            "Local\\LumenNativeDockPrototype".into()
        };
        let mutex = CreateMutexW(null(), 0, wide(mutex_name).as_ptr());
        if mutex.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        // A normal app command for graceful local reloads, preserving all widget
        // state through the same WM_CLOSE path as Quit in the tray.
        if std::env::args().any(|arg| arg == "--quit") {
            let existing = FindWindowW(wide("LumenNativeDockWindow").as_ptr(), null());
            if !existing.is_null() {
                PostMessageW(existing, WM_CLOSE, 0, 0);
            }
            CloseHandle(mutex);
            return Ok(());
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            if !smoke {
                let existing = FindWindowW(wide("LumenNativeDockWindow").as_ptr(), null());
                if !existing.is_null() {
                    let mut sent = false;
                    let packages = crate::widget_packages::Registry::discover(&root);
                    for id in packages.ids() {
                        if std::env::args().any(|a| a == format!("--{id}")) {
                            sent |= PostMessageW(
                                existing,
                                crate::widget_packages::show_message(&id),
                                0,
                                0,
                            ) != 0;
                        }
                    }
                    if std::env::args().any(|a| a == "--manage-widgets") {
                        sent |=
                            PostMessageW(existing, crate::widget_packages::manager_message(), 0, 0)
                                != 0;
                    }
                    if !sent {
                        PostMessageW(existing, WM_APP + 32, 0, 0);
                    }
                    CloseHandle(mutex);
                    return Ok(());
                }
            }
            CloseHandle(mutex);
            return Err(
                "The native dock is already running. Close it from its ··· menu first.".into(),
            );
        }
        let instance = GetModuleHandleW(null());
        let class = wide("LumenNativeDockWindow");
        let wc = WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: class.as_ptr(),
            ..zeroed()
        };
        if RegisterClassW(&wc) == 0 {
            CloseHandle(mutex);
            return Err(std::io::Error::last_os_error().to_string());
        }
        let mut work: RECT = zeroed();
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut work as *mut _ as _, 0);
        let dpi = GetDpiForSystem() as f32 / 96.0;
        let mut app = Box::new(RefCell::new(App {
            frame: None,
            window_shadow: None,
            tray: None,
            taskbar_created: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
            background: None,
            list_base: None,
            list_base_key: String::new(),
            pointer: [-1000.; 2],
            paint_time: 0.,
            toolbar_pressed: None,
            add_pressed: false,
            bar_drag: None,
            background_key: String::new(),
            moving_window: false,
            accent_color,
            overlay: None,
            search_frost: None,
            search_text: None,
            symbols: crate::symbols::Symbols::default(),
            motion: crate::motion::Motion::default(),
            tile_drag: None,
            suppress_click_until: 0.,
            last_open: None,
            open_busy_until: 0.,
            pending_launch: None,
            caret_on: true,
            hwnd: null_mut(),
            settings: settings::State::default(),
            popup: None,
            group_selection: None,
            clear_pressed: false,
            catalog_rx: Some(crate::catalog::start()),
            catalog_loaded: 0.,
            search_cache: RefCell::new(None),
            search_ink: rgb(248, 250, 253),
            search_muted: rgb(193, 202, 215),
            resize_drag: None,
            pending_resize: None,
            glass: std::rc::Rc::new(RefCell::new(Glass::new(dock.blur, dpi))),
            packages: crate::widget_packages::Registry::discover(&root),
            general: None,
            root,
            desktop_watch: None,
            dock,
            edit: null_mut(),
            search_font: null_mut(),
            search_brush: null_mut(),
            query: String::new(),
            selected: 0,
            scroll: Default::default(),
            frame_queued: false,
            hover: None,
            icons: HashMap::new(),
            scaled_icons: HashMap::new(),
            icon_shadows: HashMap::new(),
            label_shadows: HashMap::new(),
            labels: HashMap::new(),
            tile_cache: HashMap::new(),
            profile_counts: [0; 4],
            label_budget: 2,
            cache_pending: false,
            status: String::new(),
            status_ticks: 0,
            dpi: 1.0,
            smoke,
            inspect,
            keyboard_selection: false,
            started: std::time::Instant::now(),
            cpu_start: process_cpu(),
        }));
        let size = app.borrow().dock.size.unwrap_or([746, 370]);
        let width = (size[0] as f32 * dpi).round() as i32;
        let height = (size[1] as f32 * dpi).round() as i32;
        let saved = app
            .borrow()
            .dock
            .position
            .unwrap_or([work.right - width - 36, work.top + 72]);
        // Retain saved positions on any connected monitor, including negative X.
        let mut rect = RECT {
            left: saved[0],
            top: saved[1],
            right: saved[0] + width,
            bottom: saved[1] + height,
        };
        let monitor = MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = zeroed();
        info.cbSize = size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(monitor, &mut info);
        rect.left = rect.left.clamp(
            info.rcWork.left,
            (info.rcWork.right - width).max(info.rcWork.left),
        );
        rect.top = rect.top.clamp(
            info.rcWork.top,
            (info.rcWork.bottom - height).max(info.rcWork.top),
        );
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED
                | if inspect {
                    WS_EX_APPWINDOW
                } else {
                    WS_EX_TOOLWINDOW
                },
            class.as_ptr(),
            wide(TITLE).as_ptr(),
            WS_POPUP | WS_CLIPCHILDREN,
            rect.left,
            rect.top,
            width,
            height,
            null_mut(),
            null_mut(),
            instance,
            &mut *app as *mut _ as _,
        );
        if hwnd.is_null() {
            CloseHandle(mutex);
            return Err(std::io::Error::last_os_error().to_string());
        }
        crate::fusion::init(app.borrow().glass.clone());
        if let Err(error) = crate::general::load_preferences(&app.borrow().root) {
            eprintln!("native-general: {error}");
        }
        ShowWindow(
            hwnd,
            if !inspect && !app.borrow().packages.enabled("launcher", true) {
                SW_HIDE
            } else if inspect {
                SW_SHOW
            } else {
                SW_SHOWNOACTIVATE
            },
        );
        apply_window_mode(hwnd, app.borrow().dock.floating, inspect);
        UpdateWindow(hwnd);
        // A fresh layered window has no backing surface until its first upload.
        // Initialize it explicitly; later frames are driven by WM_PAINT/timers.
        app.borrow_mut().paint(hwnd);
        println!(
            "native-dock-ready startup_ms={} items={} pid={} wallpaper_glass={}",
            app.borrow().started.elapsed().as_millis(),
            app.borrow().dock.items.len(),
            std::process::id(),
            app.borrow().glass.borrow().available()
        );
        let mut startup = app.borrow().packages.startup_ids();
        for arg in std::env::args() {
            if let Some(id) = arg.strip_prefix("--") {
                if app.borrow().packages.ids().iter().any(|p| p == id)
                    && !startup.iter().any(|p| p == id)
                {
                    startup.push(id.into());
                }
            }
        }
        for id in startup {
            app.borrow_mut().show_package(&id);
        }
        if std::env::args().any(|a| a == "--manage-widgets") {
            app.borrow_mut().show_general();
        }
        let mut message: MSG = zeroed();
        loop {
            let result = GetMessageW(&mut message, null_mut(), 0, 0);
            if result <= 0 {
                if result == -1 {
                    DestroyWindow(hwnd);
                    CloseHandle(mutex);
                    return Err(std::io::Error::last_os_error().to_string());
                }
                break;
            }
            // Route keys received by our own search field; no system-wide input.
            let edit = app.borrow().edit;
            if message.hwnd == edit
                && message.message == WM_KEYDOWN
                && matches!(message.wParam as u32, 0x0D | 0x1B | 0x26 | 0x28)
            {
                SendMessageW(hwnd, KEY_FROM_SEARCH, message.wParam, 0);
                continue;
            }
            if crate::notes::translate(&message) {
                continue;
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        crate::fusion::shutdown();
        CloseHandle(mutex);
        app.borrow_mut().general.take();
        app.borrow_mut().packages.shutdown();
        drop(app);
        if context != INVALID_HANDLE_VALUE {
            if activation_cookie != 0 {
                DeactivateActCtx(0, activation_cookie);
            }
            ReleaseActCtx(context);
        }
    }
    Ok(())
}

#[repr(C)]
struct MemoryCounters {
    cb: u32,
    page_faults: u32,
    peak_working_set: usize,
    working_set: usize,
    peak_paged_pool: usize,
    paged_pool: usize,
    peak_nonpaged_pool: usize,
    nonpaged_pool: usize,
    pagefile: usize,
    peak_pagefile: usize,
    private_usage: usize,
}
#[link(name = "psapi")]
extern "system" {
    fn GetProcessMemoryInfo(process: HANDLE, counters: *mut MemoryCounters, size: u32) -> i32;
}
unsafe fn process_cpu() -> u64 {
    let mut created: FILETIME = zeroed();
    let mut exited: FILETIME = zeroed();
    let mut kernel: FILETIME = zeroed();
    let mut user: FILETIME = zeroed();
    GetProcessTimes(
        GetCurrentProcess(),
        &mut created,
        &mut exited,
        &mut kernel,
        &mut user,
    );
    (((kernel.dwHighDateTime as u64) << 32) | kernel.dwLowDateTime as u64)
        + (((user.dwHighDateTime as u64) << 32) | user.dwLowDateTime as u64)
}
unsafe fn report_metrics(seconds: f64, cpu_start: u64) {
    let mut memory: MemoryCounters = zeroed();
    memory.cb = size_of::<MemoryCounters>() as u32;
    GetProcessMemoryInfo(
        GetCurrentProcess(),
        &mut memory,
        size_of::<MemoryCounters>() as u32,
    );
    let cpu = process_cpu().saturating_sub(cpu_start) as f64 / 10_000_000.0;
    println!("native-dock-metrics working_set_mb={:.2} private_mb={:.2} process_cpu_percent={:.3} elapsed_s={:.2}",
        memory.working_set as f64 / 1048576.0,memory.private_usage as f64 / 1048576.0,100.0*cpu/seconds.max(0.001),seconds);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_floating_flags_cannot_make_widgets_topmost() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                wide("STATIC").as_ptr(),
                wide("Lumen mode test").as_ptr(),
                WS_POPUP,
                0,
                0,
                10,
                10,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            );
            assert!(!hwnd.is_null());
            apply_window_mode(hwnd, true, true);
            let floating = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST != 0;
            apply_window_mode(hwnd, false, true);
            let normal = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST == 0;
            DestroyWindow(hwnd);
            assert!(!floating && normal);
        }
    }
    #[test]
    fn catalog_and_web_targets_are_not_shell_command_strings() {
        let mut dock = Dock::default();
        dock.add("Package!App");
        let mut item = dock.items.remove(0);
        item.kind = "aumid".into();
        assert_eq!(launch_target(&item), "shell:AppsFolder\\Package!App");
        item.kind = "url".into();
        item.target = "example.org/a".into();
        assert_eq!(launch_target(&item), "https://example.org/a");
    }
}
