//! Sticky Notes shares the dock's UI thread, wallpaper source and compositor.
mod editor;
mod icons;
mod layout;
mod lenses;
pub mod model;
mod settings;
mod theme;
mod typography;
use crate::{
    compositor::{Buffer, WindowShadow},
    glass::Glass,
    model::wide,
    render::Canvas,
};
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
    UI::{Controls::WM_MOUSELEAVE, HiDpi::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};
fn image_delete_rect(r: [f32; 4]) -> [f32; 4] {
    [r[2] - 24., r[1] + 4., r[2] - 4., r[1] + 24.]
}
unsafe fn compute_image_rects(
    note: &model::Note,
    card_rect: [f32; 4],
    type_scale: &typography::Metrics,
    typography: &mut typography::Typography,
    image_cache: &std::collections::HashMap<String, (u32, u32, crate::render::Image)>,
    dpi: f32,
) -> Vec<[f32; 4]> {
    if note.images.is_empty() {
        return vec![];
    }
    let text = note.plain();
    let content_x = card_rect[0] + type_scale.left;
    let text_w = (card_rect[2] - content_x - 16.).max(type_scale.body);
    let text_lines = if text.trim().is_empty() {
        0
    } else {
        typography.line_count(&text, text_w, dpi, type_scale.body)
    };
    let mut img_y = card_rect[1]
        + type_scale.top
        + (text_lines as f32) * type_scale.leading
        + if text.trim().is_empty() { 0. } else { 6. };
    let mut rects = Vec::with_capacity(note.images.len());
    for name in &note.images {
        if let Some((iw, ih, _)) = image_cache.get(name) {
            if *iw > 0 {
                let scaled_h = (*ih as f32 / *iw as f32) * text_w;
                rects.push([content_x, img_y, content_x + text_w, img_y + scaled_h]);
                img_y += scaled_h + 6.;
                continue;
            }
        }
        rects.push([0., 0., 0., 0.]);
    }
    rects
}
const CLASS: &str = "LumenNativeNotesBoard";
const EVENTS: u32 = WM_APP + 101;
const KEY: u32 = WM_APP + 102;
const REDRAW: u32 = WM_APP + 103;
const EDITOR_HOVER: u32 = WM_APP + 105;
const EDITOR_FOCUS: u32 = WM_APP + 107;
const SAVE: usize = 101;
const ANIMATE: usize = 102;
#[derive(Clone, Debug, PartialEq)]
enum Action {
    New,
    Search,
    Clear,
    Undo,
    Done,
    Settings,
    Hide,
    Folder,
    Card(String),
    Pin(String),
    Color(String),
    Delete(String),
    PickColor(String, usize),
}
struct ImageDrag {
    note_id: String,
    index: usize,
    start: [f32; 2],
    current: [f32; 2],
    active: bool,
}
struct State {
    hwnd: HWND,
    root: PathBuf,
    board: model::Board,
    glass: Rc<RefCell<Glass>>,
    dpi: f32,
    frame: Option<Buffer>,
    background: Option<Buffer>,
    header_base: Option<Buffer>,
    list_base: Option<Buffer>,
    background_key: String,
    header_base_key: String,
    bg_ink: Option<(u32, u32)>,
    paint_scroll: Option<f32>,
    shadow: Option<WindowShadow>,
    overlay: Option<(String, crate::render::Image)>,
    editor: Option<editor::Editor>,
    edit_order: Option<Vec<String>>,
    palette_note: Option<String>,
    events: editor::Events,
    query: String,
    undo: Option<model::Note>,
    settings: Option<crate::settings_ui::Host>,
    local_glass: Option<Rc<RefCell<Glass>>>,
    pressed: Option<Action>,
    hover: Option<Action>,
    card_hovers: std::collections::HashMap<String, crate::motion::Tween>,
    editor_hovered: bool,
    scroll: crate::motion::SmoothScroll,
    scroll_clock: Option<crate::motion::FrameClock>,
    frame_queued: bool,
    started: std::time::Instant,
    moving: bool,
    scrollbar_drag: Option<f32>,
    scrollbar_visible: bool,
    smoke: bool,
    dirty: bool,
    dirty_since: Option<std::time::Instant>,
    status: String,
    ink: Option<u32>,
    typography: typography::Typography,
    button_lenses: lenses::Lenses,
    image_cache: std::collections::HashMap<String, (u32, u32, crate::render::Image)>,
    card_cache: std::collections::HashMap<String, (String, crate::render::Image)>,
    selected_image: Option<(String, usize)>,
    image_drag: Option<ImageDrag>,
}
pub struct Host {
    state: Box<RefCell<State>>,
}
pub fn should_open(root: &Path) -> bool {
    model::Board::load(root).is_ok_and(|b| b.open)
}
pub unsafe fn register(name: &str, proc: WNDPROC) {
    let name = wide(name);
    let class = WNDCLASSW {
        style: CS_DBLCLKS,
        lpfnWndProc: proc,
        hInstance: GetModuleHandleW(null()),
        lpszClassName: name.as_ptr(),
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        ..zeroed()
    };
    RegisterClassW(&class);
}
pub fn point(l: LPARAM) -> [f32; 2] {
    [(l as u16 as i16) as f32, ((l >> 16) as u16 as i16) as f32]
}
/// Clamp only to the selected monitor's work area; negative desktop coordinates
/// are valid. Do not force every restored widget onto the primary display.
fn restored_bounds(position: [i32; 2], size: [i32; 2], work: RECT) -> RECT {
    let width = size[0]
        .max(1)
        .min(work.right.saturating_sub(work.left).max(1));
    let height = size[1]
        .max(1)
        .min(work.bottom.saturating_sub(work.top).max(1));
    let left = position[0].clamp(work.left, work.right.saturating_sub(width).max(work.left));
    let top = position[1].clamp(work.top, work.bottom.saturating_sub(height).max(work.top));
    RECT {
        left,
        top,
        right: left.saturating_add(width),
        bottom: top.saturating_add(height),
    }
}
pub fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | ((g as u32) << 8) | ((b as u32) << 16)
}
pub unsafe fn window_text(hwnd: HWND) -> String {
    let n = GetWindowTextLengthW(hwnd).max(0) as usize;
    let mut text = vec![0u16; n + 1];
    let n = GetWindowTextW(hwnd, text.as_mut_ptr(), text.len() as i32).max(0) as usize;
    String::from_utf16_lossy(&text[..n])
}
/// Only messages already delivered to this process are examined. No input hook.
pub unsafe fn translate(msg: &MSG) -> bool {
    if msg.message != WM_KEYDOWN {
        return false;
    }
    let root = GetAncestor(msg.hwnd, GA_ROOT);
    let mut class = [0u16; 80];
    let n = GetClassNameW(root, class.as_mut_ptr(), 80);
    let class = String::from_utf16_lossy(&class[..n.max(0) as usize]);
    if class != CLASS && class != "LumenNativeNotesEditor" {
        return false;
    }
    let ctrl = GetKeyState(VK_CONTROL as i32) < 0;
    if msg.wParam == VK_ESCAPE as usize
        || ctrl && matches!(msg.wParam, 0x4e | 0x46 | 0x57 | 0x42 | 0x49 | 0x55 | 0x53)
    {
        // New/find always belong to the board, including while editing a note.
        let parent = GetParent(msg.hwnd);
        let mut name = [0u16; 80];
        let len = GetClassNameW(parent, name.as_mut_ptr(), 80).max(0) as usize;
        let inline = String::from_utf16_lossy(&name[..len]) == "LumenNativeNotesEditor";
        let dest = if inline && !matches!(msg.wParam, 0x4e | 0x46) {
            parent
        } else {
            root
        };
        SendMessageW(dest, KEY, msg.wParam, 0);
        return true;
    }
    false
}
impl Host {
    pub unsafe fn create(
        root: &Path,
        glass: Rc<RefCell<Glass>>,
        inspect: bool,
        smoke: bool,
    ) -> Result<Self, String> {
        let board = model::Board::load(root)?;
        register(CLASS, Some(proc));
        let dpi = GetDpiForSystem() as f32 / 96.;
        let mut s = Box::new(RefCell::new(State {
            hwnd: null_mut(),
            root: root.into(),
            board,
            glass,
            dpi,
            frame: None,
            background: None,
            header_base: None,
            list_base: None,
            background_key: String::new(),
            header_base_key: String::new(),
            bg_ink: None,
            paint_scroll: None,
            shadow: None,
            overlay: None,
            editor: None,
            edit_order: None,
            palette_note: None,
            events: Rc::new(RefCell::new(vec![])),
            query: String::new(),
            undo: None,
            settings: None,
            local_glass: None,
            pressed: None,
            hover: None,
            card_hovers: Default::default(),
            editor_hovered: false,
            scroll: Default::default(),
            scroll_clock: None,
            frame_queued: false,
            started: std::time::Instant::now(),
            moving: false,
            scrollbar_drag: None,
            scrollbar_visible: false,
            smoke,
            dirty: false,
            dirty_since: None,
            status: String::new(),
            ink: None,
            typography: Default::default(),
            button_lenses: Default::default(),
            image_cache: Default::default(),
            card_cache: Default::default(),
            selected_image: None,
            image_drag: None,
        }));
        let (size, pos) = {
            let state = s.borrow();
            (state.board.size, state.board.position.unwrap_or([160, 120]))
        };
        let [w, h] = size.map(|v| (v as f32 * dpi).round() as i32);
        let requested = RECT {
            left: pos[0],
            top: pos[1],
            right: pos[0].saturating_add(w),
            bottom: pos[1].saturating_add(h),
        };
        let monitor = MonitorFromRect(&requested, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = zeroed();
        info.cbSize = size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0
            || info.rcWork.right <= info.rcWork.left
            || info.rcWork.bottom <= info.rcWork.top
        {
            return Err("Cannot resolve an active monitor work area for Sticky Notes".into());
        }
        let r = restored_bounds(pos, [w, h], info.rcWork);
        let w = r.right - r.left;
        let h = r.bottom - r.top;
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED
                | if inspect {
                    WS_EX_APPWINDOW
                } else {
                    WS_EX_TOOLWINDOW
                },
            wide(CLASS).as_ptr(),
            wide("Lumen · Sticky Notes").as_ptr(),
            WS_POPUP | WS_THICKFRAME,
            r.left,
            r.top,
            w,
            h,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            s.as_mut() as *mut _ as _,
        );
        if hwnd.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        SetWindowTextW(hwnd, wide("Lumen · Sticky Notes").as_ptr());
        {
            let mut state = s.borrow_mut();
            state.hwnd = hwnd;
            state.dpi = GetDpiForWindow(hwnd) as f32 / 96.;
            // Keep the actual restored geometry for the next normal save.
            state.bounds();
            state.shadow = WindowShadow::new(hwnd);
            state.update_local_blur();
        }
        println!(
            "native-notes-ready notes={} pid={}",
            s.borrow().board.notes.len(),
            std::process::id()
        );
        Ok(Self { state: s })
    }
    pub unsafe fn show(&mut self) {
        let mut s = self.state.borrow_mut();
        if s.shadow.is_none() {
            s.shadow = WindowShadow::new(s.hwnd);
        }
        s.board.open = true;
        ShowWindow(
            s.hwnd,
            if crate::desktop_layer::contains(s.hwnd) {
                SW_SHOWNOACTIVATE
            } else {
                SW_SHOW
            },
        );
        s.apply_floating();
        if !crate::desktop_layer::contains(s.hwnd) {
            SetForegroundWindow(s.hwnd);
        }
        s.paint();
        s.schedule();
    }
    /// Entry points for the shared widget manager; preserve notes and bounds.
    pub fn hwnd(&self) -> HWND {
        self.state.borrow().hwnd
    }
    pub unsafe fn is_visible(&self) -> bool {
        IsWindowVisible(self.hwnd()) != 0
    }
    pub unsafe fn show_settings(&mut self) {
        self.state.borrow_mut().open_settings();
    }
    pub unsafe fn hide(&mut self) {
        self.state.borrow_mut().hide();
    }
    pub unsafe fn refresh(&mut self) {
        let mut s = self.state.borrow_mut();
        s.glass.borrow_mut().refresh();
        if let Some(glass) = &s.local_glass {
            glass.borrow_mut().refresh();
        }
        s.background_key.clear();
        s.overlay = None;
        if IsWindowVisible(s.hwnd) != 0 {
            InvalidateRect(s.hwnd, null(), 0);
        }
        if s.settings.is_some() {
            let color = s.surface_color();
            let light = crate::accent::light(color);
            if let Some(host) = &mut s.settings {
                host.set_surface(color, light);
            }
        }
    }
    pub unsafe fn apply_common(&mut self, opacity: Option<f32>, floating: Option<bool>) {
        let mut s = self.state.borrow_mut();
        if let Some(opacity) = opacity.filter(|v| v.is_finite()) {
            s.board.opacity = opacity.clamp(0., 100.);
        }
        if let Some(floating) = floating {
            s.board.floating = floating;
            s.apply_floating();
        }
        let rows = settings::rows(&s.board);
        if let Some(settings) = &mut s.settings {
            settings.set_rows(rows);
        }
        s.schedule();
        InvalidateRect(s.hwnd, null(), 0);
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        unsafe {
            let hwnd = {
                let mut s = self.state.borrow_mut();
                if let Some(e) = s.editor.take() {
                    e.flush();
                    drop(e);
                }
                s.consume();
                s.flush();
                s.settings.take();
                s.shadow.take();
                s.hwnd
            };
            DestroyWindow(hwnd);
        }
    }
}
impl State {
    fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1000.
    }
    unsafe fn dimensions(&self) -> [f32; 2] {
        let mut r: RECT = zeroed();
        GetClientRect(self.hwnd, &mut r);
        [r.right as f32 / self.dpi, r.bottom as f32 / self.dpi]
    }
    unsafe fn apply_floating(&self) {
        crate::window::apply_window_mode(
            self.hwnd,
            false,
            GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE) as u32 & WS_EX_APPWINDOW != 0,
        );
        if let Some(e) = &self.editor {
            e.floating(false);
        }
    }
    unsafe fn schedule(&mut self) {
        if self.smoke {
            return;
        }
        self.dirty = true;
        let since = self.dirty_since.get_or_insert_with(std::time::Instant::now);
        let remaining = 1200u128.saturating_sub(since.elapsed().as_millis());
        SetTimer(self.hwnd, SAVE, (remaining.min(260).max(1)) as u32, None);
    }
    unsafe fn flush(&mut self) -> bool {
        KillTimer(self.hwnd, SAVE);
        if self.smoke || !self.dirty {
            return true;
        }
        match self.board.save(&self.root) {
            Ok(()) => {
                self.dirty = false;
                self.dirty_since = None;
                self.status.clear();
                true
            }
            Err(e) => {
                self.status = format!("Save failed: {e}");
                SetTimer(self.hwnd, SAVE, 1500, None);
                InvalidateRect(self.hwnd, null(), 0);
                false
            }
        }
    }
    unsafe fn consume(&mut self) {
        let events = std::mem::take(&mut *self.events.borrow_mut());
        for event in events {
            match event {
                editor::Event::Note(note) => {
                    if let Some(old) = self.board.notes.iter_mut().find(|n| n.id == note.id) {
                        *old = note;
                        self.schedule();
                    }
                }
                editor::Event::Search(q) => {
                    self.query = q;
                    self.scroll.snap(0.);
                }
                editor::Event::Closed => {
                    self.editor.take();
                    self.edit_order = None;
                    if IsWindowVisible(self.hwnd) != 0 {
                        SetFocus(self.hwnd);
                    }
                }
            }
        }
        InvalidateRect(self.hwnd, null(), 0);
    }
    unsafe fn finish_editor(&mut self) {
        if let Some(e) = self.editor.take() {
            e.flush();
            drop(e);
        }
        self.consume();
        self.edit_order = None;
    }
    unsafe fn open_editor(&mut self, id: Option<String>) {
        if self.editor.as_ref().is_some_and(|e| e.note_id() == id) {
            return;
        }
        self.finish_editor();
        self.palette_note = None;
        if id.is_some() {
            self.edit_order = Some(
                self.board
                    .visible(&self.query)
                    .iter()
                    .map(|&i| self.board.notes[i].id.clone())
                    .collect(),
            );
        }
        let note = id.and_then(|id| self.board.notes.iter().find(|n| n.id == id).cloned());
        match editor::Editor::create(
            self.hwnd,
            self.root.clone(),
            self.events.clone(),
            note,
            self.query.clone(),
            self.board.floating,
        ) {
            Ok(e) => {
                self.editor = Some(e);
                if let Some(id) = self.editor.as_ref().and_then(|e| e.note_id()) {
                    if let Some((_, r)) = self.layout().into_iter().find(|(n, _)| *n == id) {
                        let viewport = self.viewport();
                        let delta = if r[3] > viewport[1] {
                            r[3] - viewport[1]
                        } else if r[1] < viewport[0] {
                            r[1] - viewport[0]
                        } else {
                            0.
                        };
                        if delta != 0. {
                            let value = self.scroll.value(self.now()) + delta;
                            self.scroll.snap(value);
                        }
                    }
                }
                self.place_editor();
                if let Some(e) = &self.editor {
                    e.focus();
                }
            }
            Err(e) => {
                self.status = e;
            }
        }
    }
    unsafe fn place_editor(&mut self) {
        let Some(id) = self.editor.as_ref().map(|e| e.note_id()) else {
            return;
        };
        let rect = if let Some(id) = id.as_ref() {
            self.layout()
                .into_iter()
                .find(|(n, _)| n == id)
                .map(|(_, r)| r)
        } else {
            Some(self.search_rect())
        };
        let images_h = if let (Some(id), Some(rect)) = (&id, rect) {
            let type_scale = self.type_scale();
            let text_w = (rect[2] - rect[0] - type_scale.left - 16.).max(type_scale.body);
            if let Some(images) = self
                .board
                .notes
                .iter()
                .find(|n| n.id == *id)
                .map(|n| n.images.clone())
            {
                self.images_height(&images, text_w)
            } else {
                0.
            }
        } else {
            0.
        };
        if let (Some(e), Some(rect)) = (&self.editor, rect) {
            e.place(
                rect,
                if e.note_id().is_some() {
                    self.viewport()
                } else {
                    [self.search_rect()[1], self.search_rect()[3]]
                },
                self.dpi,
                self.type_scale(),
                images_h,
            );
        }
    }
    fn load_image(&mut self, name: &str) -> Option<(u32, u32)> {
        if let Some((w, h, _)) = self.image_cache.get(name) {
            return Some((*w, *h));
        }
        let path = model::images_dir(&self.root).join(name);
        let decoded = image::open(path).ok()?.into_rgba8();
        let (w, h) = decoded.dimensions();
        let mut pixels = decoded.into_raw();
        for p in pixels.chunks_exact_mut(4) {
            let a = p[3] as u32;
            p[0] = ((p[0] as u32 * a + 127) / 255) as u8;
            p[1] = ((p[1] as u32 * a + 127) / 255) as u8;
            p[2] = ((p[2] as u32 * a + 127) / 255) as u8;
        }
        let img = crate::render::Image::from_rgba(w, h, pixels)?;
        self.image_cache.insert(name.into(), (w, h, img));
        Some((w, h))
    }
    fn images_height(&mut self, images: &[String], text_w: f32) -> f32 {
        let mut total = 0f32;
        for name in images {
            if let Some((w, h)) = self.load_image(name) {
                if w > 0 {
                    let scaled_h = (h as f32 / w as f32) * text_w;
                    if total > 0. {
                        total += 6.;
                    }
                    total += scaled_h;
                }
            }
        }
        total
    }
    unsafe fn note_image_rects(&mut self, note_id: &str, card_rect: [f32; 4]) -> Vec<[f32; 4]> {
        let Some(note) = self.board.notes.iter().find(|n| n.id == note_id).cloned() else {
            return vec![];
        };
        for name in &note.images {
            let _ = self.load_image(name);
        }
        let type_scale = self.type_scale();
        compute_image_rects(
            &note,
            card_rect,
            &type_scale,
            &mut self.typography,
            &self.image_cache,
            self.dpi,
        )
    }
    unsafe fn hit_image(&mut self, p: [f32; 2]) -> Option<(String, usize, [f32; 4])> {
        if p[1] < self.viewport()[0] || p[1] > self.viewport()[1] {
            return None;
        }
        for (id, r) in self.layout() {
            if !inside(p, r) {
                continue;
            }
            let rects = self.note_image_rects(&id, r);
            for (idx, ir) in rects.into_iter().enumerate() {
                if ir[2] > ir[0] && ir[3] > ir[1] && inside(p, ir) {
                    return Some((id, idx, ir));
                }
            }
        }
        None
    }
    unsafe fn delete_selected_image(&mut self) {
        let Some((note_id, idx)) = self.selected_image.take() else {
            return;
        };
        let removed_name = if let Some(note) = self.board.notes.iter_mut().find(|n| n.id == note_id)
        {
            if idx < note.images.len() {
                let name = note.images.remove(idx);
                note.updated_at = model::now();
                Some(name)
            } else {
                None
            }
        } else {
            None
        };
        if let Some(name) = removed_name {
            let still_used = self
                .board
                .notes
                .iter()
                .any(|n| n.images.iter().any(|img| img == &name));
            if !still_used {
                let _ = std::fs::remove_file(model::images_dir(&self.root).join(&name));
                self.image_cache.remove(&name);
            }
            self.card_cache.remove(&note_id);
            self.place_editor();
            self.schedule();
        }
    }
    unsafe fn move_image(&mut self, from_id: &str, from_idx: usize, p: [f32; 2]) {
        let target_card = self
            .layout()
            .into_iter()
            .find(|(_, r)| inside(p, *r))
            .map(|(id, r)| (id, r));
        let Some((target_id, target_rect)) = target_card else {
            return;
        };
        if target_id == from_id {
            let rects = self.note_image_rects(from_id, target_rect);
            if rects.len() <= 1 || from_idx >= rects.len() {
                return;
            }
            let mut target_idx = rects.len() - 1;
            for (i, r) in rects.iter().enumerate() {
                let mid = (r[1] + r[3]) * 0.5;
                if p[1] < mid {
                    target_idx = i;
                    break;
                }
            }
            if target_idx == from_idx {
                return;
            }
            if let Some(note) = self.board.notes.iter_mut().find(|n| n.id == from_id) {
                let img = note.images.remove(from_idx);
                let insert_at = target_idx.min(note.images.len());
                note.images.insert(insert_at, img);
                note.updated_at = model::now();
                self.selected_image = Some((from_id.to_string(), insert_at));
                self.card_cache.remove(from_id);
                self.place_editor();
                self.schedule();
            }
            return;
        }
        let img_name = if let Some(src) = self.board.notes.iter_mut().find(|n| n.id == from_id) {
            if from_idx < src.images.len() {
                let name = src.images.remove(from_idx);
                src.updated_at = model::now();
                Some(name)
            } else {
                None
            }
        } else {
            None
        };
        if let Some(name) = img_name {
            if let Some(dst) = self.board.notes.iter_mut().find(|n| n.id == target_id) {
                dst.images.push(name);
                dst.updated_at = model::now();
                let new_idx = dst.images.len() - 1;
                self.selected_image = Some((target_id.clone(), new_idx));
                self.card_cache.remove(from_id);
                self.card_cache.remove(&target_id);
                self.place_editor();
                self.schedule();
            }
        }
    }
    unsafe fn bounds(&mut self) {
        let mut r: RECT = zeroed();
        GetWindowRect(self.hwnd, &mut r);
        self.board.position = Some([r.left, r.top]);
        self.board.size = [
            ((r.right - r.left) as f32 / self.dpi).round() as i32,
            ((r.bottom - r.top) as f32 / self.dpi).round() as i32,
        ];
        self.schedule();
    }
    unsafe fn hide(&mut self) {
        self.finish_editor();
        self.palette_note = None;
        if let Some(settings) = &mut self.settings {
            settings.hide();
        }
        self.board.open = false;
        self.schedule();
        self.flush();
        ShowWindow(self.hwnd, SW_HIDE);
        KillTimer(self.hwnd, ANIMATE);
        let offset = self.scroll.value(self.now());
        self.scroll.snap(offset);
        self.shadow.take();
        self.frame.take();
        self.background.take();
        self.header_base.take();
        self.list_base.take();
        self.overlay.take();
        self.typography.clear();
        self.button_lenses.clear();
        self.card_cache.clear();
        self.background_key.clear();
        self.header_base_key.clear();
        self.bg_ink = None;
    }
    fn color(note: &model::Note) -> u32 {
        match note.color.as_str() {
            "sky" => rgb(142, 197, 255),
            "violet" => rgb(192, 166, 255),
            "mint" => rgb(127, 224, 189),
            "rose" => rgb(255, 158, 184),
            "peach" => rgb(255, 176, 138),
            "slate" => rgb(185, 194, 216),
            _ => rgb(255, 211, 110),
        }
    }
    unsafe fn chrome(&self) -> layout::Chrome {
        let [w, h] = self.dimensions();
        layout::Chrome {
            w,
            h,
            search_open: self.editor.as_ref().is_some_and(|e| e.note_id().is_none()),
        }
    }
    unsafe fn type_scale(&self) -> typography::Metrics {
        let [w, h] = self.dimensions();
        typography::Metrics::for_board(&self.board.design, w, h)
    }
    unsafe fn search_rect(&self) -> [f32; 4] {
        self.chrome().search()
    }
    unsafe fn add_rect(&self) -> [f32; 4] {
        self.chrome().add()
    }
    unsafe fn footer_folder(&self) -> [f32; 4] {
        self.chrome().folder()
    }
    unsafe fn footer_undo(&self) -> [f32; 4] {
        self.chrome().undo()
    }
    unsafe fn content_top(&self) -> f32 {
        self.chrome().content_top()
    }
    unsafe fn viewport(&self) -> [f32; 2] {
        self.chrome().viewport()
    }
    fn hovered_card_id(&self) -> Option<String> {
        match &self.hover {
            Some(
                Action::Card(id)
                | Action::Pin(id)
                | Action::Delete(id)
                | Action::Color(id)
                | Action::PickColor(id, _),
            ) => Some(id.clone()),
            _ if self.editor_hovered => self.editor.as_ref().and_then(|e| e.note_id()),
            _ => None,
        }
    }
    fn card_hover(&self, id: &str, now: f64) -> f32 {
        self.card_hovers.get(id).map_or(0., |t| t.sample(now))
    }
    fn animating(&self, now: f64) -> bool {
        self.scroll.moving(now) || self.card_hovers.values().any(|t| t.moving(now))
    }
    unsafe fn sync_hover(&mut self) {
        let now = self.now();
        // Suppress transient hover glow trails while actively scrolling.
        let active_id = if self.scroll.moving(now) {
            None
        } else {
            self.hovered_card_id()
        };
        let ids: Vec<String> = self.board.notes.iter().map(|n| n.id.clone()).collect();
        self.card_hovers
            .retain(|id, t| ids.contains(id) && (t.moving(now) || t.sample(now) > 0.001));
        for id in ids {
            let to = if active_id.as_deref() == Some(id.as_str()) {
                1.
            } else {
                0.
            };
            if let Some(tween) = self.card_hovers.get_mut(&id) {
                tween.target(
                    to,
                    now,
                    if to > 0. { 150. } else { 200. },
                    [0.25, 0.1, 0.25, 1.],
                );
            } else if to > 0. {
                let mut tween = crate::motion::Tween::default();
                tween.target(1., now, 150., [0.25, 0.1, 0.25, 1.]);
                self.card_hovers.insert(id, tween);
            }
        }
        if self.animating(now) {
            if self.scroll_clock.is_none() {
                self.scroll_clock = Some(crate::motion::FrameClock::start());
            }
            SetTimer(self.hwnd, ANIMATE, 10, None);
            InvalidateRect(self.hwnd, null(), 0);
        }
    }
    unsafe fn layout(&mut self) -> Vec<(String, [f32; 4])> {
        let [w, _] = self.dimensions();
        if self.viewport()[1] <= self.viewport()[0] {
            self.scroll.bounds(0., self.now());
            return Vec::new();
        }
        let mut list = self.board.visible(&self.query);
        if let Some(order) = &self.edit_order {
            list.sort_by_key(|&i| {
                order
                    .iter()
                    .position(|id| *id == self.board.notes[i].id)
                    .unwrap_or(usize::MAX)
            });
        }
        let cols = if self.board.design == "lumen" {
            ((w - 42.) / 240.).floor().max(1.) as usize
        } else {
            1
        };
        let chrome = self.chrome();
        let type_scale = self.type_scale();
        let inset = chrome.card_inset(&self.board.design);
        let top = self.content_top();
        let gap = 14.;
        let width = (w - inset * 2. - gap * (cols - 1) as f32) / cols as f32;
        let text_width = (width - type_scale.left - 16.).max(type_scale.body);
        let dpi = self.dpi;
        let notes_meta: Vec<(String, String, Vec<String>, bool)> = list
            .iter()
            .map(|&index| {
                let note = &self.board.notes[index];
                let plain = note.plain();
                let empty = plain.trim().is_empty() && note.images.is_empty();
                (note.id.clone(), plain, note.images.clone(), empty)
            })
            .collect();
        let mut out = Vec::with_capacity(notes_meta.len());
        let mut row_top = top;
        for chunk in notes_meta.chunks(cols) {
            let mut row_height = 0f32;
            for (col, (id, plain, images, empty)) in chunk.iter().enumerate() {
                let img_h = self.images_height(images, text_width);
                let img_gap = if img_h > 0. && !plain.trim().is_empty() {
                    6.
                } else {
                    0.
                };
                let lines = if *empty {
                    1
                } else if plain.trim().is_empty() {
                    0
                } else {
                    self.typography
                        .line_count(plain, text_width, dpi, type_scale.body)
                };
                let needed = type_scale.top
                    + (lines as f32) * type_scale.leading
                    + img_gap
                    + img_h
                    + 34.;
                let raw_h = if *empty {
                    type_scale.card_height(true)
                } else {
                    needed.max(type_scale.height)
                };
                let h = if raw_h <= type_scale.height {
                    chrome.card_height(raw_h)
                } else {
                    raw_h
                };
                row_height = row_height.max(h);
                let x = inset + col as f32 * (width + gap);
                out.push((id.clone(), [x, row_top, x + width, row_top + h]));
            }
            row_top += row_height + gap;
        }
        let content_bottom = if out.is_empty() { top } else { row_top - gap };
        // Keep 14 DIP bottom clearance so the last card's shadow is fully scrollable into view.
        let max = if out.is_empty() {
            0.
        } else {
            (content_bottom + 14. - self.viewport()[1]).max(0.)
        };
        self.scroll.bounds(max, self.now());
        let offset = self
            .paint_scroll
            .unwrap_or_else(|| self.scroll.value(self.now()));
        for (_, r) in &mut out {
            r[1] -= offset;
            r[3] -= offset;
        }
        out
    }
    unsafe fn toolbar(&self) -> Vec<(Action, [f32; 4], &'static str)> {
        let [w, _] = self.dimensions();
        let mut controls = vec![
            (Action::New, self.add_rect(), "+"),
            if self.editor.is_some()
                && (self.chrome().tiny()
                    || self.chrome().card_height(self.type_scale().height) < 112.)
            {
                (Action::Done, self.footer_undo(), "✓")
            } else {
                (Action::Undo, self.footer_undo(), "↶")
            },
        ];
        for (action, rect) in crate::settings_ui::widget_header_hits(w) {
            controls.push((
                match action {
                    crate::settings_ui::HeaderAction::Settings => Action::Settings,
                    crate::settings_ui::HeaderAction::Hide => Action::Hide,
                },
                rect,
                "",
            ));
        }
        controls
    }
    unsafe fn palette_rect(&mut self) -> Option<(String, [f32; 4])> {
        let id = self.palette_note.clone()?;
        let r = self.layout().into_iter().find(|(n, _)| *n == id)?.1;
        let viewport = self.viewport();
        Some((id, layout::palette(r, viewport)))
    }
    unsafe fn hit(&mut self, p: [f32; 2]) -> Option<Action> {
        if let Some((id, r)) = self.palette_rect() {
            if inside(p, r) {
                return layout::palette_hit(r, p).map(|index| Action::PickColor(id, index));
            }
        }
        for (a, r, _) in self.toolbar() {
            if inside(p, r) {
                return Some(a);
            }
        }
        if inside(p, self.footer_folder()) {
            return Some(Action::Folder);
        }
        if inside(p, self.search_rect()) {
            return Some(
                if !self.chrome().search_icon()
                    && p[0] > self.search_rect()[2] - 34.
                    && !self.query.is_empty()
                {
                    Action::Clear
                } else {
                    Action::Search
                },
            );
        }
        if p[1] < self.viewport()[0] || p[1] > self.viewport()[1] {
            return None;
        }
        for (id, r) in self.layout() {
            if inside(p, r) {
                return Some(if p[1] < r[1] + self.type_scale().top.min(34.) {
                    if p[0] < r[0] + 38. {
                        Action::Color(id)
                    } else if p[0] > r[2] - 34. {
                        Action::Delete(id)
                    } else if p[0] > r[2] - 70. {
                        Action::Pin(id)
                    } else {
                        Action::Card(id)
                    }
                } else {
                    Action::Card(id)
                });
            }
        }
        None
    }
    unsafe fn action(&mut self, a: Action) {
        if !matches!(&a, Action::Color(_) | Action::PickColor(..)) {
            self.palette_note = None;
        }
        match a.clone() {
            Action::New => {
                self.finish_editor();
                match self.board.create() {
                    Ok(id) => {
                        self.query.clear();
                        self.scroll.snap(0.);
                        self.schedule();
                        self.open_editor(Some(id));
                    }
                    Err(e) => self.status = e,
                }
            }
            Action::Done => self.finish_editor(),
            Action::Search => self.open_editor(None),
            Action::Clear => {
                self.finish_editor();
                self.query.clear();
                self.scroll.snap(0.);
            }
            Action::Undo => {
                if let Some(n) = self.undo.take() {
                    if self.board.notes.len() < 400 {
                        self.board.notes.push(n);
                        self.schedule();
                    } else {
                        self.undo = Some(n);
                        self.status = "The board already contains 400 notes".into();
                    }
                }
            }
            Action::Settings => {
                if let Some(e) = &self.editor {
                    e.flush();
                }
                self.consume();
                self.open_settings();
            }
            Action::Hide => self.hide(),
            Action::Folder => {
                let folder = self.root.join("config");
                if let Err(e) = std::fs::create_dir_all(&folder) {
                    self.status = e.to_string();
                } else {
                    windows_sys::Win32::UI::Shell::ShellExecuteW(
                        self.hwnd,
                        wide("open").as_ptr(),
                        wide(folder).as_ptr(),
                        null(),
                        null(),
                        SW_SHOWNORMAL,
                    );
                }
            }
            Action::Card(id) => self.open_editor(Some(id)),
            Action::Color(id) => {
                self.finish_editor();
                self.palette_note = if self.palette_note.as_ref() == Some(&id) {
                    None
                } else {
                    Some(id)
                };
            }
            Action::PickColor(id, index) => {
                self.palette_note = None;
                if let Some(note) = self.board.notes.iter_mut().find(|n| n.id == id) {
                    if let Some(color) = model::COLORS.get(index) {
                        note.color = (*color).into();
                        note.updated_at = model::now();
                        self.schedule();
                    }
                }
            }
            Action::Pin(id) | Action::Delete(id) => {
                self.finish_editor();
                if let Some(i) = self.board.notes.iter().position(|n| n.id == id) {
                    match a {
                        Action::Pin(_) => self.board.notes[i].pinned = !self.board.notes[i].pinned,
                        _ => {
                            self.undo = Some(self.board.notes.remove(i));
                        }
                    }
                    if let Some(n) = self.board.notes.iter_mut().find(|n| n.id == id) {
                        n.updated_at = model::now();
                    }
                    self.schedule();
                }
            }
        }
        InvalidateRect(self.hwnd, null(), 0);
    }
    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_card_contents(
        c: &Canvas,
        note: &model::Note,
        rect: [f32; 4],
        color: u32,
        ink: u32,
        muted: u32,
        paper: bool,
        index: bool,
        honey: bool,
        tiny: bool,
        number: Option<usize>,
        hover_t: f32,
        pin_hover: bool,
        delete_hover: bool,
        type_scale: &typography::Metrics,
        typography: &mut typography::Typography,
        image_cache: &std::collections::HashMap<String, (u32, u32, crate::render::Image)>,
        dpi: f32,
        draw_text: bool,
    ) {
        let [x, y, right, bottom] = rect;
        if !paper && !index && !honey {
            // Tint belongs below the reflective finish: painting it last
            // muted the card's upper highlight and directional rim.
            c.round(rect, 18., color, 78, false, false);
            c.glass_surface(rect, 18., 20, false);
            // A second thin coating brings the upper-left sheen close
            // to the legacy card's subtle 9% white highlight. Coating
            // adds no outline, so the existing single rim stays intact.
            c.coating(rect, 18.);
        }
        if paper {
            c.round(rect, 10., rgb(255, 252, 243), 170, false, false);
            c.line(
                &[(x + 16., bottom - 1.), (right - 16., bottom - 1.)],
                rgb(121, 100, 68),
                55,
                0.8,
            );
        }
        if honey {
            c.round(
                [x, y + 2., right, bottom + 2.],
                20.,
                rgb(134, 109, 68),
                30,
                false,
                false,
            );
            c.round(rect, 14., rgb(255, 252, 245), 242, false, false);
            c.line(
                &[(x + 20., bottom - 1.), (right - 20., bottom - 1.)],
                rgb(176, 149, 104),
                65,
                0.8,
            );
        }
        if index {
            c.line(&[(x + 4., y + 1.), (right - 4., y + 1.)], ink, 26, 1.);
        }
        if hover_t > 0.001 {
            if paper || index || honey {
                c.round(
                    rect,
                    10.,
                    ink,
                    (hover_t * 12.).round() as u8,
                    false,
                    false,
                );
            } else {
                c.hover_surface(rect, 18., hover_t * 0.28, false);
            }
        }
        let (card_ink, card_muted) = (ink, muted);
        if let Some(number) = number {
            c.text(
                &format!("{number:02}"),
                [x + 12., y + 42., x + 48., y + 76.],
                16.,
                false,
                card_muted,
                0,
            );
        }
        let icon_y = y + if tiny { 8. } else { 12. };
        icons::bead(c, x + 14., icon_y, color);
        icons::pin(
            c,
            right - 66.,
            icon_y,
            card_ink,
            note.pinned,
            hover_t,
            pin_hover,
        );
        icons::trash(
            c,
            right - 34.,
            icon_y,
            card_ink,
            hover_t,
            delete_hover,
        );
        if paper {
            let mut line = y + type_scale.top + type_scale.leading;
            while line < bottom - 28. {
                c.line(
                    &[(x + type_scale.left, line), (right - 18., line)],
                    card_ink,
                    28,
                    0.8,
                );
                line += type_scale.leading;
            }
            if !tiny {
                c.line(
                    &[(x + 38., y + 36.), (x + 38., bottom - 28.)],
                    rgb(184, 91, 89),
                    90,
                    1.,
                );
            }
        }
        let text = note.plain();
        let content_x = x + type_scale.left;
        let text_w = (right - content_x - 16.).max(type_scale.body);
        let text_lines = if text.trim().is_empty() {
            if note.images.is_empty() { 1 } else { 0 }
        } else {
            typography.line_count(&text, text_w, dpi, type_scale.body)
        };
        if !note.images.is_empty() {
            let mut img_y = y
                + type_scale.top
                + (text_lines as f32) * type_scale.leading
                + if text.trim().is_empty() { 0. } else { 6. };
            for name in &note.images {
                if let Some((iw, ih, image)) = image_cache.get(name) {
                    if *iw > 0 {
                        let scaled_h = (*ih as f32 / *iw as f32) * text_w;
                        c.image(
                            image,
                            [content_x, img_y, content_x + text_w, img_y + scaled_h],
                        );
                        img_y += scaled_h + 6.;
                    }
                }
            }
        }
        if !draw_text {
            return;
        }
        let preview = if text.trim().is_empty() && note.images.is_empty() {
            "Write something…".into()
        } else {
            text.clone()
        };
        let compact_card = typography::compact_card([right - x, bottom - y]);
        for (i, line) in typography
            .lines(&preview, text_w, dpi, type_scale.body, usize::MAX)
            .iter()
            .enumerate()
        {
            let top = y + type_scale.top + i as f32 * type_scale.leading;
            c.text(
                line,
                [content_x, top, right - 16., top + type_scale.leading],
                type_scale.body,
                false,
                if text.trim().is_empty() {
                    card_muted
                } else {
                    card_ink
                },
                0,
            );
        }
        if !compact_card {
            c.text(
                &icons::note_day(note.updated_at),
                [x + 17., bottom - 26., right - 17., bottom - 8.],
                type_scale.caption,
                false,
                card_muted,
                0,
            );
        }
    }
    unsafe fn paint(&mut self) {
        let frame_started = std::time::Instant::now();
        let mut ps: PAINTSTRUCT = zeroed();
        BeginPaint(self.hwnd, &mut ps);
        let mut r: RECT = zeroed();
        GetClientRect(self.hwnd, &mut r);
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
        // Layout, editor placement and palette hit geometry share one scroll
        // position for this frame, even when rendering takes several ms.
        self.paint_scroll = Some(self.scroll.value(self.now()));
        let [w, h] = self.dimensions();
        let mut wr: RECT = zeroed();
        GetWindowRect(self.hwnd, &mut wr);
        let paper = self.board.design == "notebook";
        let index = self.board.design == "index";
        let honey = self.board.design == "honey";
        let type_scale = self.type_scale();
        let base = self.surface_color();
        let source_glass = self.source_glass();
        let key = format!(
            "{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
            wr.left,
            wr.top,
            r.right,
            r.bottom,
            self.dpi,
            self.board.design,
            self.board.palette,
            self.content_top(),
            self.board.frosted,
            self.board.tint,
            self.board.frost_strength,
            source_glass.borrow().revision(),
            crate::fusion::revision(self.hwnd)
        );
        let cached = self
            .background
            .as_ref()
            .is_some_and(|b| b.width == r.right && b.height == r.bottom && b.dpi == self.dpi);
        let bg_reused = cached && self.background_key == key;
        let own_alpha = (self.board.opacity * 2.55) as u8;
        let group_op = crate::fusion::group_opacities(self.hwnd, own_alpha);
        let header_action_key = |a: Option<&Action>| match a {
            Some(Action::Settings) => 1u8,
            Some(Action::Hide) => 2,
            Some(Action::New) => 3,
            Some(Action::Undo) => 4,
            Some(Action::Done) => 5,
            Some(Action::Search) => 6,
            _ => 0,
        };
        let header_key = if bg_reused
            && !self.moving
            && self.editor.as_ref().is_none_or(|e| e.note_id().is_some())
            && !self.background_key.is_empty()
        {
            let hv = crate::widget_header::visual(self.hwnd);
            let mut hk = format!(
                "{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
                self.background_key,
                own_alpha,
                self.chrome().tiny(),
                self.chrome().search_icon(),
                self.query,
                self.undo.is_some(),
                header_action_key(self.hover.as_ref()),
                header_action_key(self.pressed.as_ref()),
                hv.alpha.to_bits(),
                hv.hover[0].to_bits(),
                hv.hover[1].to_bits(),
                hv.rotation[0].to_bits(),
                hv.rotation[1].to_bits(),
            );
            if let Some((_, ops)) = &group_op {
                for o in ops {
                    hk.push_str(&format!(":{o}"));
                }
            }
            hk
        } else {
            String::new()
        };
        let reused_header = !header_key.is_empty()
            && header_key == self.header_base_key
            && self.bg_ink.is_some()
            && self
                .header_base
                .as_ref()
                .is_some_and(|b| b.width == r.right && b.height == r.bottom && b.dpi == self.dpi);
        if reused_header {
            frame.copy_from(self.header_base.as_ref().unwrap().dc);
        } else if bg_reused {
            frame.copy_from(self.background.as_ref().unwrap().dc);
        } else {
            self.bg_ink = None;
            let brush = CreateSolidBrush(base);
            FillRect(frame.dc, &r, brush);
            DeleteObject(brush);
            if theme::solid_material(&self.board.design) {
                theme::backdrop(
                    frame.dc,
                    [r.right, r.bottom],
                    self.dpi,
                    &self.board.design,
                    self.content_top(),
                );
                let c = Canvas::new(frame.dc, self.dpi);
                crate::surface::paint(
                    &c,
                    [0., 0., w, h],
                    &self.board.palette,
                    if self.board.frosted {
                        self.board.tint / 100.
                    } else {
                        1.
                    },
                );
            } else {
                source_glass.borrow().draw_blended(
                    frame.dc,
                    self.hwnd,
                    r.right,
                    r.bottom,
                    if self.board.frosted {
                        (self.board.frost_strength * 2.55) as u8
                    } else {
                        0
                    },
                );
                {
                    let c = Canvas::new(frame.dc, self.dpi);
                    let pane = [0., 0., w, h];
                    if !crate::surface::managed(&self.board.palette) {
                        c.round(
                            pane,
                            30.,
                            base,
                            (if self.board.frosted {
                                self.board.tint
                            } else {
                                100.
                            } * 2.55) as u8,
                            false,
                            false,
                        );
                    }
                    crate::surface::paint(
                        &c,
                        pane,
                        &self.board.palette,
                        if self.board.frosted {
                            self.board.tint / 100.
                        } else {
                            1.
                        },
                    );
                    c.coating(pane, 30.);
                }
                let overlay_key = format!(
                    "{}:{}:{}:{}:{}",
                    r.right, r.bottom, self.dpi, self.board.tint, paper
                );
                if (!self.moving && self.overlay.as_ref().is_none_or(|(k, _)| *k != overlay_key))
                    || self.overlay.is_none()
                {
                    self.overlay = Canvas::pane_overlay(
                        [r.right as u32, r.bottom as u32],
                        self.dpi,
                        self.board.tint / 100.,
                        paper,
                    )
                    .map(|v| (overlay_key, v));
                }
                {
                    let c = Canvas::new(frame.dc, self.dpi);
                    if let Some((_, image)) = &self.overlay {
                        c.image(image, [0., 0., w, h]);
                    }
                    c.widget_corner_reflection_for(self.hwnd, [0., 0., w, h], 30., paper);
                }
            }
            frame.bevel_for(self.hwnd);
            if let Some(b) = self
                .background
                .take()
                .filter(|b| b.width == r.right && b.height == r.bottom && b.dpi == self.dpi)
                .or_else(|| Buffer::plain(r.right, r.bottom, self.dpi))
            {
                b.copy_from(frame.dc);
                self.background = Some(b);
                self.background_key = key;
            }
        }
        let layout = self.layout();
        self.place_editor();
        let palette_rect = self.palette_rect();
        let backing = self.background.as_ref().unwrap();
        let (ink, muted) = if let Some(pair) = self.bg_ink.filter(|_| bg_reused) {
            pair
        } else {
            let lum = crate::contrast::sample(frame.dc, self.dpi, [20., 20., w - 20., h - 20.]);
            let pair = crate::contrast::inks(lum, None);
            self.ink = Some(pair.0);
            self.bg_ink = Some(pair);
            pair
        };
        if !reused_header {
        frame.background_alpha_for(self.hwnd, own_alpha);
        {
            let c = Canvas::new(frame.dc, self.dpi);
            let header_action = |a: Option<&Action>| match a {
                Some(Action::Settings) => Some(crate::settings_ui::HeaderAction::Settings),
                Some(Action::Hide) => Some(crate::settings_ui::HeaderAction::Hide),
                _ => None,
            };
            crate::settings_ui::draw_widget_header(
                frame.dc,
                self.hwnd,
                self.dpi,
                w,
                "Sticky Notes",
                ink,
                header_action(self.hover.as_ref()),
                header_action(self.pressed.as_ref()),
            );
            for (a, rect, label) in self.toolbar() {
                if matches!(a, Action::Settings | Action::Hide) {
                    continue;
                }
                let hover = self.hover.as_ref() == Some(&a);
                if a == Action::New {
                    let down = hover && self.pressed.as_ref() == Some(&a);
                    let radius = (rect[2] - rect[0]) * 0.5;
                    c.soft_shadow(
                        rect,
                        radius,
                        if down {
                            14.
                        } else if hover {
                            28.
                        } else {
                            19.
                        },
                        if self.chrome().tiny() { 4. } else { 6. },
                        if down { 1.5 } else { 2. },
                    );
                    let mut style = lenses::ENABLED | lenses::PRIMARY;
                    if hover {
                        style |= lenses::HOVER;
                    }
                    if down {
                        style |= lenses::PRESSED;
                    }
                    if crate::contrast::luminance(ink) < 0.5 {
                        style |= lenses::LIGHT;
                    }
                    self.button_lenses.draw(&c, self.dpi, rect, base, style);
                    let x = (rect[0] + rect[2]) * 0.5;
                    let y = (rect[1] + rect[3]) * 0.5;
                    let arm = if self.chrome().tiny() { 5. } else { 6. };
                    c.line(&[(x - arm, y), (x + arm, y)], ink, 255, 1.7);
                    c.line(&[(x, y - arm), (x, y + arm)], ink, 255, 1.7);
                    continue;
                } else if hover {
                    c.hover_surface(rect, 16., 0.7, paper);
                }
                if matches!(a, Action::New | Action::Undo | Action::Done) {
                    c.text_alpha(
                        label,
                        rect,
                        if a == Action::New { 23. } else { 16. },
                        false,
                        ink,
                        1,
                        if a == Action::Undo && self.undo.is_none() {
                            60
                        } else {
                            220
                        },
                    );
                }
            }
            let search = self.search_rect();
            if self.board.opacity < 100. {
                backing.restore_glass_card(frame.dc, search, 17.);
            }
            if self.chrome().search_icon() {
                let mut style = lenses::ENABLED;
                if self.hover.as_ref() == Some(&Action::Search) {
                    style |= lenses::HOVER;
                }
                if self.pressed.as_ref() == Some(&Action::Search) {
                    style |= lenses::PRESSED;
                }
                if !self.query.is_empty() {
                    style |= lenses::ACTIVE;
                }
                if crate::contrast::luminance(ink) < 0.5 {
                    style |= lenses::LIGHT;
                }
                self.button_lenses.draw(&c, self.dpi, search, base, style);
                icons::search(
                    &c,
                    self.dpi,
                    (search[0] + search[2]) * 0.5 - 8.,
                    (search[1] + search[3]) * 0.5 - 8.,
                    ink,
                    235,
                );
            } else {
                c.search_surface(search, 16., base, paper);
                c.round(search, 16., base, 35, false, false);
                icons::search(
                    &c,
                    self.dpi,
                    search[0] + 12.,
                    search[1] + 8.,
                    ink,
                    if self.query.is_empty() { 155 } else { 235 },
                );
                if let Some(e) = self.editor.as_ref().filter(|e| e.note_id().is_none()) {
                    e.draw(frame.dc, ink);
                } else if self.query.is_empty() {
                    if search[2] - search[0] >= 136. {
                        c.text_alpha(
                            "Search notes…",
                            [
                                search[0] + 34.,
                                search[1] + 6.,
                                search[2] - 12.,
                                search[3] - 4.,
                            ],
                            14.,
                            false,
                            muted,
                            0,
                            108,
                        );
                    }
                } else {
                    c.text(
                        &self.query,
                        [
                            search[0] + 34.,
                            search[1] + 6.,
                            search[2] - 38.,
                            search[3] - 4.,
                        ],
                        14.,
                        false,
                        ink,
                        0,
                    );
                }
                if !self.query.is_empty() {
                    c.text(
                        "×",
                        [
                            search[2] - 36.,
                            search[1] + 3.,
                            search[2] - 6.,
                            search[3] - 3.,
                        ],
                        18.,
                        false,
                        ink,
                        1,
                    );
                }
            }
        }
        for (a, r, _) in self.toolbar() {
            if a == Action::New
                || (self.hover.as_ref() == Some(&a)
                    && !matches!(a, Action::Settings | Action::Hide))
            {
                frame.retain_control_backing(
                    backing,
                    r,
                    if a == Action::New {
                        (r[2] - r[0]) / 2.
                    } else {
                        16.
                    },
                    1.,
                );
            }
        }
        crate::settings_ui::retain_header_backing(&mut frame, backing, self.hwnd, w);
        if !header_key.is_empty() {
            if let Some(hb) = self
                .header_base
                .take()
                .filter(|b| b.width == r.right && b.height == r.bottom && b.dpi == self.dpi)
                .or_else(|| Buffer::plain(r.right, r.bottom, self.dpi))
            {
                hb.copy_from(frame.dc);
                self.header_base = Some(hb);
                self.header_base_key = header_key;
            }
        } else {
            self.header_base_key.clear();
        }
        }
        let fade_base = if self.scroll.max > 0. {
            let had_valid = reused_header
                && self
                    .list_base
                    .as_ref()
                    .is_some_and(|b| b.width == r.right && b.height == r.bottom && b.dpi == self.dpi);
            let base = self
                .list_base
                .take()
                .filter(|b| b.width == r.right && b.height == r.bottom && b.dpi == self.dpi)
                .or_else(|| Buffer::plain(r.right, r.bottom, self.dpi));
            if let Some(b) = &base {
                if !had_valid {
                    b.copy_from(frame.dc);
                }
            }
            base
        } else {
            None
        };
        // Content is clipped before drawing, not a full-board screenshot buffer.
        let saved = SaveDC(frame.dc);
        IntersectClipRect(
            frame.dc,
            0,
            (self.viewport()[0] * self.dpi) as i32,
            r.right,
            (self.viewport()[1] * self.dpi) as i32,
        );
        let now = self.now();
        let scrolling = self.scroll.moving(now);
        let tiny = self.chrome().tiny();
        if self.card_cache.len() > 128 {
            self.card_cache
                .retain(|id, _| self.board.notes.iter().any(|n| n.id == *id));
        }
        let mut batch = Some(crate::render::Batch::new(frame.dc, self.dpi));
        {
            for (pos, (id, rect)) in layout.iter().enumerate() {
                if rect[3] < self.viewport()[0] || rect[1] > self.viewport()[1] {
                    continue;
                }
                let Some(note) = self.board.notes.iter().find(|n| n.id == *id).cloned() else {
                    continue;
                };
                let [x, y, right, bottom] = *rect;
                let color = Self::color(&note);
                let hover_t = if scrolling { 0. } else { self.card_hover(id, now) };
                let has_editor = self
                    .editor
                    .as_ref()
                    .is_some_and(|e| e.note_id().as_deref() == Some(id.as_str()));
                let pin_hover = !scrolling && self.hover.as_ref() == Some(&Action::Pin(id.clone()));
                let delete_hover =
                    !scrolling && self.hover.as_ref() == Some(&Action::Delete(id.clone()));
                let number = (index && !tiny).then_some(pos + 1);
                if !has_editor && hover_t <= 0.001 && !pin_hover && !delete_hover {
                    use std::hash::{Hash, Hasher};
                    let pad_px = if honey {
                        (3. * self.dpi).ceil() as u32
                    } else {
                        (1. * self.dpi).ceil() as u32
                    };
                    let pad = pad_px as f32 / self.dpi;
                    let cw_px = ((right - x) * self.dpi).round().max(1.) as u32;
                    let ch_px = ((bottom - y) * self.dpi).round().max(1.) as u32;
                    let ax = (x * self.dpi).round() / self.dpi;
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    note.body.hash(&mut hasher);
                    note.rtf.hash(&mut hasher);
                    for name in &note.images {
                        name.hash(&mut hasher);
                        self.image_cache.contains_key(name).hash(&mut hasher);
                    }
                    let day = icons::note_day(note.updated_at);
                    let card_key = format!(
                        "{}:{}:{}:{}:{}:{}:{}:{}:{:?}:{}:{}",
                        cw_px,
                        ch_px,
                        self.dpi.to_bits(),
                        self.board.design,
                        color,
                        note.pinned,
                        ink,
                        muted,
                        number,
                        day,
                        hasher.finish()
                    );
                    if !self.moving && self.card_cache.get(id).is_none_or(|(k, _)| *k != card_key) {
                        let local = [
                            pad,
                            pad,
                            pad + cw_px as f32 / self.dpi,
                            pad + ch_px as f32 / self.dpi,
                        ];
                        let dpi = self.dpi;
                        let typography = &mut self.typography;
                        let image_cache = &self.image_cache;
                        if let Some(sprite) = Canvas::render_sprite(
                            cw_px + pad_px * 2,
                            ch_px + pad_px * 2,
                            dpi,
                            |c| {
                                Self::draw_card_contents(
                                    c,
                                    &note,
                                    local,
                                    color,
                                    ink,
                                    muted,
                                    paper,
                                    index,
                                    honey,
                                    tiny,
                                    number,
                                    0.,
                                    false,
                                    false,
                                    &type_scale,
                                    typography,
                                    image_cache,
                                    dpi,
                                    true,
                                );
                            },
                        ) {
                            self.card_cache.insert(id.clone(), (card_key.clone(), sprite));
                        }
                    }
                    if let (Some(b), Some((_, sprite))) =
                        (&batch, self.card_cache.get(id).filter(|(k, _)| *k == card_key))
                    {
                        if self.board.opacity < 100. {
                            backing.restore_glass_card(
                                frame.dc,
                                *rect,
                                if paper {
                                    14.
                                } else if index {
                                    0.
                                } else {
                                    18.
                                },
                            );
                        }
                        let [sw, sh] = sprite.pixel_size();
                        let sprite_rect = [
                            ax - pad,
                            y - pad,
                            ax - pad + sw as f32 / self.dpi,
                            y - pad + sh as f32 / self.dpi,
                        ];
                        if b.blit(sprite, sprite_rect, !scrolling) {
                            continue;
                        }
                    }
                }
                batch.take();
                // At full background opacity the cached frame already contains
                // this exact backing. Avoid a GDI flush and pixel mask per card.
                if self.board.opacity < 100. {
                    backing.restore_glass_card(
                        frame.dc,
                        *rect,
                        if paper {
                            14.
                        } else if index {
                            0.
                        } else {
                            18.
                        },
                    );
                }
                let c = Canvas::new(frame.dc, self.dpi);
                Self::draw_card_contents(
                    &c,
                    &note,
                    *rect,
                    color,
                    ink,
                    muted,
                    paper,
                    index,
                    honey,
                    tiny,
                    number,
                    hover_t,
                    pin_hover,
                    delete_hover,
                    &type_scale,
                    &mut self.typography,
                    &self.image_cache,
                    self.dpi,
                    !has_editor,
                );
                drop(c);
                if let Some(e) = self
                    .editor
                    .as_ref()
                    .filter(|e| e.note_id().as_deref() == Some(id.as_str()))
                {
                    e.draw(frame.dc, ink);
                }
            }
            if let Some((sel_id, sel_idx)) = self.selected_image.clone() {
                if let Some((_, card_rect)) = layout.iter().find(|(id, _)| *id == sel_id) {
                    if let Some(note) = self.board.notes.iter().find(|n| n.id == sel_id) {
                        let rects = compute_image_rects(
                            note,
                            *card_rect,
                            &type_scale,
                            &mut self.typography,
                            &self.image_cache,
                            self.dpi,
                        );
                        if let Some(&ir) = rects.get(sel_idx) {
                            if ir[2] > ir[0] && ir[3] > ir[1] {
                                let c = Canvas::new(frame.dc, self.dpi);
                                c.round(
                                    [ir[0] - 1., ir[1] - 1., ir[2] + 1., ir[3] + 1.],
                                    6.,
                                    rgb(96, 165, 250),
                                    0,
                                    false,
                                    true,
                                );
                                let dr = image_delete_rect(ir);
                                c.round(dr, 10., rgb(24, 24, 28), 210, false, true);
                                c.text("×", dr, 13., true, rgb(255, 255, 255), 1);
                            }
                        }
                    }
                }
            }
            if let Some(drag) = &self.image_drag {
                if drag.active {
                    if let Some((_, target_rect)) =
                        layout.iter().find(|(_, r)| inside(drag.current, *r))
                    {
                        let c = Canvas::new(frame.dc, self.dpi);
                        c.round(*target_rect, 18., rgb(96, 165, 250), 28, false, true);
                    }
                }
            }
            if layout.is_empty() && self.viewport()[1] > self.viewport()[0] {
                batch.take();
                let c = Canvas::new(frame.dc, self.dpi);
                let top = self.content_top() + if self.chrome().tiny() { 8. } else { 20. };
                c.text(
                    if self.query.is_empty() {
                        "Your thoughts here"
                    } else {
                        "No matching notes"
                    },
                    [16., top, w - 16., top + 24.],
                    if self.chrome().tiny() { 13. } else { 19. },
                    true,
                    ink,
                    1,
                );
                if !self.chrome().tiny() {
                    c.text(
                        "Create a note with + or Ctrl + N",
                        [24., top + 32., w - 24., top + 60.],
                        12.,
                        false,
                        muted,
                        1,
                    );
                }
            }
        }
        drop(batch);
        RestoreDC(frame.dc, saved);
        if let Some(base) = &fade_base {
            let viewport = self.viewport();
            let offset = self
                .paint_scroll
                .unwrap_or_else(|| self.scroll.value(self.now()));
            // Reveal each fade gradually as content enters that edge; full
            // notes at the first/last scroll stop retain their original rim.
            let top = viewport[0] - (14. - offset).max(0.);
            let bottom = viewport[1] + (26. - (self.scroll.max - offset)).max(0.);
            frame.fade_list(base, top, bottom);
        }
        self.list_base = fade_base;
        {
            let c = Canvas::new(frame.dc, self.dpi);
            if let Some((id, r)) = &palette_rect {
                backing.restore_glass_card(frame.dc, *r, 12.);
                c.glass_surface(*r, 12., 100, paper);
                if let Some(note) = self.board.notes.iter().find(|n| n.id == *id) {
                    for (i, color) in model::COLORS.iter().enumerate() {
                        let mut swatch = note.clone();
                        swatch.color = (*color).into();
                        let cols = layout::palette_columns(*r);
                        let x = r[0] + 8. + (i % cols) as f32 * 24.;
                        let y = r[1] + 10. + (i / cols) as f32 * 24.;
                        icons::bead(&c, x, y, Self::color(&swatch));
                        if note.color == *color {
                            c.round(
                                [x - 3., y - 3., x + 19., y + 19.],
                                11.,
                                ink,
                                220,
                                true,
                                false,
                            );
                        }
                    }
                }
            }
        }

        {
            let c = Canvas::new(frame.dc, self.dpi);
            if self.scrollbar_visible {
                if let Some(bar) = crate::geometry::scrollbar(
                    w as i32,
                    (self.viewport()[1] + 40.) as i32,
                    self.viewport()[0] as i32,
                    self.scroll.max,
                    self.scroll.value(self.now()),
                ) {
                    frame.fill_round(bar, 2., ink, 120);
                }
            }
            c.text(
                &if self.status.is_empty() {
                    format!("{} notes", self.board.notes.len())
                } else {
                    self.status.clone()
                },
                if self.chrome().tiny() {
                    [16., h - 32., w - 80., h - 10.]
                } else {
                    [24., h - 41., w - 168., h - 19.]
                },
                11.,
                false,
                muted,
                0,
            );
            if self.chrome().tiny() {
                icons::folder(&c, w - 34., h - 30., muted);
            } else {
                icons::folder(&c, w - 124., h - 40., muted);
                c.text(
                    "Open folder",
                    [w - 104., h - 41., w - 24., h - 19.],
                    11.,
                    false,
                    muted,
                    1,
                );
            }
        }
        self.paint_scroll = None;
        frame.present_with_alpha(self.hwnd, true);
        self.frame = Some(frame);
        EndPaint(self.hwnd, &ps);
        if let Some(shadow) = &mut self.shadow {
            shadow.sync_with_radius(self.hwnd, self.dpi, (self.board.opacity * 2.55) as u8, 30.);
        }
        if IsWindowVisible(self.hwnd) != 0 && self.animating(self.now()) {
            crate::motion::pace_vsync(
                self.hwnd,
                frame_started,
                &mut self.frame_queued,
                crate::motion::VSYNC_FRAME,
            );
        }
    }
}
fn inside(p: [f32; 2], r: [f32; 4]) -> bool {
    p[0] >= r[0] && p[0] < r[2] && p[1] >= r[1] && p[1] < r[3]
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // These responses must also hold during synchronous callbacks from
    // UpdateLayeredWindow/SetWindowPos while paint owns the state borrow.
    // Otherwise default nonclient calculation can shrink a layered HWND to
    // its smaller client dimensions on each successive frame.
    if msg == WM_NCCALCSIZE {
        return 0;
    }
    if msg == WM_ERASEBKGND {
        return 1;
    }
    if msg == WM_NCCREATE {
        let create = &*(l as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        return 1;
    }
    if msg == WM_NCDESTROY {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        return DefWindowProcW(hwnd, msg, w, l);
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<State>;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, w, l);
    }
    let Ok(mut s) = (*ptr).try_borrow_mut() else {
        return DefWindowProcW(hwnd, msg, w, l);
    };
    match msg {
        WM_PAINT => {
            s.paint();
            0
        }
        WM_NCHITTEST => {
            let mut p = POINT {
                x: point(l)[0] as i32,
                y: point(l)[1] as i32,
            };
            ScreenToClient(hwnd, &mut p);
            let size = s.dimensions();
            let p = [p.x as f32 / s.dpi, p.y as f32 / s.dpi];
            let edge = crate::geometry::widget_edge(p[0], p[1], size[0], size[1]);
            let actionable = s.hit(p).is_some();
            // Tiny header/footer buttons sit inside the diagonal corner grip.
            // Their hit targets win there; the outer resize band stays free.
            if edge != HTCLIENT
                && (!actionable
                    || crate::compositor::distance(p[0], p[1], size[0], size[1], 30., 4.) > -4.)
            {
                return edge as isize;
            }
            if p[1] < 58. && !actionable {
                HTCAPTION as isize
            } else {
                HTCLIENT as isize
            }
        }
        WM_GETMINMAXINFO => {
            let m = &mut *(l as *mut MINMAXINFO);
            m.ptMinTrackSize = POINT {
                x: (model::MIN_SIZE as f32 * s.dpi) as i32,
                y: (model::MIN_SIZE as f32 * s.dpi) as i32,
            };
            m.ptMaxTrackSize = POINT {
                x: (1600. * s.dpi) as i32,
                y: (1200. * s.dpi) as i32,
            };
            0
        }
        WM_NCLBUTTONDOWN => {
            if s.editor.is_some() || s.palette_note.is_some() {
                s.finish_editor();
                s.palette_note = None;
                InvalidateRect(hwnd, null(), 0);
            }
            drop(s);
            DefWindowProcW(hwnd, msg, w, l)
        }
        WM_LBUTTONDOWN => {
            let p = point(l).map(|v| v / s.dpi);
            s.scrollbar_drag = None;
            if let Some((sel_id, sel_idx)) = s.selected_image.clone() {
                if let Some((_, card_rect)) = s.layout().into_iter().find(|(id, _)| *id == sel_id)
                {
                    let rects = s.note_image_rects(&sel_id, card_rect);
                    if let Some(&ir) = rects.get(sel_idx) {
                        if inside(p, image_delete_rect(ir)) {
                            s.delete_selected_image();
                            InvalidateRect(hwnd, null(), 0);
                            return 0;
                        }
                    }
                }
            }
            if let Some((id, idx, _)) = s.hit_image(p) {
                if s
                    .editor
                    .as_ref()
                    .and_then(|e| e.note_id())
                    .as_deref()
                    != Some(id.as_str())
                {
                    s.finish_editor();
                }
                s.selected_image = Some((id.clone(), idx));
                s.image_drag = Some(ImageDrag {
                    note_id: id,
                    index: idx,
                    start: p,
                    current: p,
                    active: false,
                });
                SetFocus(hwnd);
                SetCapture(hwnd);
                InvalidateRect(hwnd, null(), 0);
                return 0;
            }
            s.selected_image = None;
            if let Some(bar) = crate::geometry::scrollbar(
                s.dimensions()[0] as i32,
                (s.viewport()[1] + 40.) as i32,
                s.viewport()[0] as i32,
                s.scroll.max,
                s.scroll.value(s.now()),
            ) {
                if inside(p, [bar[0] - 5., bar[1], bar[2] + 5., bar[3]]) {
                    s.scrollbar_drag = Some(p[1] - bar[1]);
                }
            }
            s.pressed = s.hit(p);
            SetCapture(hwnd);
            0
        }
        WM_LBUTTONUP => {
            ReleaseCapture();
            let dpi = s.dpi;
            let p = point(l).map(|v| v / dpi);
            if let Some(drag) = s.image_drag.take() {
                if drag.active {
                    s.move_image(&drag.note_id, drag.index, p);
                }
                InvalidateRect(hwnd, null(), 0);
                return 0;
            }
            if s.scrollbar_drag.take().is_none() {
                let a = s.pressed.take();
                let hit = s.hit(p);
                if a.is_some() && a == hit {
                    s.action(a.unwrap());
                } else if a.is_none()
                    && hit.is_none()
                    && (s.editor.is_some() || s.palette_note.is_some())
                {
                    s.finish_editor();
                    s.palette_note = None;
                    InvalidateRect(hwnd, null(), 0);
                }
            }
            0
        }
        WM_CAPTURECHANGED => {
            s.pressed = None;
            s.scrollbar_drag = None;
            s.image_drag = None;
            0
        }
        WM_LBUTTONDBLCLK => {
            let dpi = s.dpi;
            if s.hit(point(l).map(|v| v / dpi)).is_none() {
                s.action(Action::New);
            }
            0
        }
        WM_MOUSEMOVE => {
            let p = point(l).map(|v| v / s.dpi);
            if let Some(drag) = &mut s.image_drag {
                drag.current = p;
                if (p[0] - drag.start[0]).hypot(p[1] - drag.start[1]) > 4. {
                    drag.active = true;
                }
                InvalidateRect(hwnd, null(), 0);
                return 0;
            }
            if let Some(grab) = s.scrollbar_drag {
                let value = crate::geometry::scroll_drag(
                    (s.viewport()[1] + 40.) as i32,
                    s.viewport()[0] as i32,
                    s.scroll.max,
                    p[1],
                    grab,
                );
                s.scroll.snap(value);
            }
            let hover = s.hit(p);
            if hover != s.hover || !s.scrollbar_visible || s.scrollbar_drag.is_some() {
                s.hover = hover;
                s.scrollbar_visible = true;
                s.sync_hover();
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
            s.scrollbar_visible = false;
            s.sync_hover();
            InvalidateRect(hwnd, null(), 0);
            0
        }
        EDITOR_HOVER => {
            let hovered = w != 0;
            if s.editor_hovered != hovered {
                s.editor_hovered = hovered;
                s.sync_hover();
            }
            0
        }
        EDITOR_FOCUS => {
            if s.selected_image.take().is_some() {
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_KEYDOWN => {
            if (w == VK_DELETE as usize || w == VK_BACK as usize)
                && s.selected_image.is_some()
            {
                s.delete_selected_image();
                InvalidateRect(hwnd, null(), 0);
                return 0;
            }
            if w == VK_ESCAPE as usize && s.selected_image.is_some() {
                s.selected_image = None;
                InvalidateRect(hwnd, null(), 0);
                return 0;
            }
            drop(s);
            DefWindowProcW(hwnd, msg, w, l)
        }
        WM_MOUSEWHEEL => {
            let now = s.now();
            s.scroll
                .by(-((w >> 16) as u16 as i16) as f32 / 120. * 96., now);
            s.scrollbar_visible = true;
            if s.scroll_clock.is_none() {
                s.scroll_clock = Some(crate::motion::FrameClock::start());
            }
            SetTimer(hwnd, ANIMATE, 10, None);
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_TIMER if w == ANIMATE => {
            if !s.frame_queued {
                InvalidateRect(hwnd, null(), 0);
            }
            if !s.animating(s.now()) {
                KillTimer(hwnd, ANIMATE);
                s.scroll_clock = None;
            }
            0
        }
        m if m == crate::motion::VSYNC_FRAME => {
            s.frame_queued = false;
            if s.animating(s.now()) {
                InvalidateRect(hwnd, null(), 0);
            } else {
                KillTimer(hwnd, ANIMATE);
                s.scroll_clock = None;
            }
            0
        }
        WM_TIMER if w == SAVE => {
            s.consume();
            s.flush();
            0
        }
        EVENTS => {
            s.consume();
            0
        }
        settings::MESSAGE => {
            s.settings_changed();
            0
        }
        REDRAW => {
            InvalidateRect(hwnd, null(), 0);
            0
        }
        KEY => {
            match w {
                0x4e => s.action(Action::New),
                0x46 => s.action(Action::Search),
                0x57 => s.hide(),
                0x1b => {
                    s.finish_editor();
                    s.query.clear();
                    InvalidateRect(hwnd, null(), 0);
                }
                0x42 | 0x49 | 0x55 | 0x53 => {}
                _ => {}
            }
            0
        }
        WM_ENTERSIZEMOVE => {
            s.moving = true;
            s.finish_editor();
            0
        }
        WM_EXITSIZEMOVE => {
            s.moving = false;
            s.background_key.clear();
            s.overlay = None;
            s.bounds();
            InvalidateRect(hwnd, null(), 0);
            0
        }
        windows_sys::Win32::UI::WindowsAndMessaging::WM_MOVE => {
            if (s.board.frosted || crate::glass::liquid_glass())
                && crate::glass::move_paint_due(hwnd)
            {
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_SIZE => {
            let width = (l as u32 & 0xffff) as i32;
            let height = ((l as u32 >> 16) & 0xffff) as i32;
            if width > 0
                && height > 0
                && s.frame
                    .as_ref()
                    .is_none_or(|b| !b.matches_geometry(width, height, s.dpi, 30.))
            {
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_WINDOWPOSCHANGING => {
            let dpi = s.dpi;
            if let Some(shadow) = &mut s.shadow {
                shadow.follow_position(&*(l as *const WINDOWPOS), dpi);
            }
            drop(s);
            DefWindowProcW(hwnd, msg, w, l)
        }
        WM_SETTINGCHANGE | WM_DISPLAYCHANGE => {
            s.glass.borrow_mut().refresh();
            if let Some(glass) = &s.local_glass {
                glass.borrow_mut().refresh();
            }
            s.background_key.clear();
            s.overlay = None;
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_DPICHANGED => {
            s.dpi = (w as u16) as f32 / 96.;
            s.update_local_blur();
            s.background_key.clear();
            drop(s);
            let r = &*(l as *const RECT);
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
        WM_ACTIVATE if (w as u16) as u32 == WA_INACTIVE => {
            let target = l as HWND;
            let to_settings = s
                .settings
                .as_ref()
                .is_some_and(|h| !target.is_null() && h.hwnd() == target);
            if !to_settings {
                s.finish_editor();
                s.palette_note = None;
                InvalidateRect(hwnd, null(), 0);
            } else if let Some(e) = &s.editor {
                e.flush();
                s.consume();
            }
            s.flush();
            0
        }
        WM_CLOSE => {
            s.hide();
            0
        }
        _ => {
            // DefWindowProc may run the native move/resize modal loop or send
            // WM_SIZE synchronously. Those callbacks need to borrow our state.
            drop(s);
            DefWindowProcW(hwnd, msg, w, l)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Explicit renderer capture only; hidden HWND, no live app changes"]
    fn capture_refined_themes() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            let _runtime = crate::render::Runtime::start().unwrap();
            let root = std::env::temp_dir().join(format!(
                "lumen-notes-capture-{}-{}",
                std::process::id(),
                model::now()
            ));
            let mut board = model::Board::default();
            board.size = [420, 650];
            board.position = Some([160, 120]);
            for (i,body)in [
                "Today<br>Make room for what matters.<br>A small step is still progress.",
                "Một chút mỗi ngày<br>Ghi lại ý tưởng, rồi quay lại khi cần.<br>Chữ rõ, khoảng cách nhẹ nhàng.",
                "Studio notes<br>Soft light, quiet colours.<br>Keep the useful details close.",
                "Reading list<br>A book for the weekend.<br>A page worth coming back to.",
                "Small idea<br>Give the next good thought<br>a little space to grow.",
                "Later<br>Pick up coffee beans.<br>Take a walk before sunset.",
            ].iter().enumerate(){board.create().unwrap();let n=board.notes.last_mut().unwrap();n.body=(*body).into();n.color=["sky","mint","peach","violet","rose","slate"][i].into();n.updated_at=model::now()-i as u64*3_600_000;n.pinned=i==0;}
            board.save(&root).unwrap();
            let original = std::fs::read(root.join("config/native-notes.json")).unwrap();
            let glass = Rc::new(RefCell::new(Glass::new(13., 1.)));
            glass.borrow_mut().refresh();
            let host = Host::create(&root, glass, true, true).unwrap();
            let output = crate::model::root().join(".scratch/notes-themes-refined");
            std::fs::create_dir_all(&output).unwrap();
            {
                let mut s = host.state.borrow_mut();
                assert_eq!(IsWindowVisible(s.hwnd), 0);
                let save = |s: &State, name: &str| {
                    let frame = s.frame.as_ref().unwrap();
                    let mut pixels = frame.snapshot();
                    for p in pixels.chunks_exact_mut(4) {
                        p.swap(0, 2);
                        let a = p[3] as u32;
                        if a > 0 && a < 255 {
                            for c in &mut p[..3] {
                                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                            }
                        }
                    }
                    image::save_buffer(
                        output.join(name),
                        &pixels,
                        frame.width as u32,
                        frame.height as u32,
                        image::ColorType::Rgba8,
                    )
                    .unwrap();
                };
                for (i, design) in model::DESIGNS.iter().enumerate() {
                    s.board.design = (*design).into();
                    s.background_key.clear();
                    s.scroll.snap(0.);
                    s.paint();
                    assert!(
                        s.add_rect()[3] - s.add_rect()[1] > s.search_rect()[3] - s.search_rect()[1]
                    );
                    save(&s, &format!("{}-{}.png", i + 1, design));
                    let max = s.scroll.max;
                    s.scroll.snap(max * 0.48);
                    s.paint();
                    assert!(s.list_base.is_some());
                    let viewport = s.viewport();
                    let frame = s.frame.as_ref().unwrap();
                    let base = s.list_base.as_ref().unwrap();
                    let pixels = frame.snapshot();
                    let background = base.snapshot();
                    let x = (210. * s.dpi) as usize;
                    for dip in [viewport[0] + 0.25, viewport[1] - 0.25] {
                        let y = (dip * s.dpi) as usize;
                        let offset = (y * frame.width as usize + x) * 4;
                        assert!(
                            (0..3).all(|c| (pixels[offset + c] as i32
                                - background[offset + c] as i32)
                                .abs()
                                <= 4),
                            "Both viewport edges must fade back to the chrome"
                        );
                    }
                    save(&s, &format!("{}-{}-scroll.png", i + 1, design));
                    // Regress compact/large edge spacing against the actual
                    // layout and rendered buffer, without showing the host.
                    for [width, height] in [
                        [160, 160],
                        [160, 320],
                        [320, 160],
                        [240, 240],
                        [300, 650],
                        [880, 540],
                    ] {
                        let hwnd = s.hwnd;
                        let dpi = s.dpi;
                        drop(s); // Native resize callbacks can borrow State.
                        SetWindowPos(
                            hwnd,
                            null_mut(),
                            0,
                            0,
                            (width as f32 * dpi) as i32,
                            (height as f32 * dpi) as i32,
                            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                        s = host.state.borrow_mut();
                        s.background_key.clear();
                        s.scroll.snap(0.);
                        s.paint();
                        let [w, h] = s.dimensions();
                        let add = s.add_rect();
                        let search = s.search_rect();
                        let tiny = s.chrome().tiny();
                        assert!(
                            add[0] >= 12.
                                && add[1] >= 12.
                                && add[2] + if tiny { 4. } else { 12. } <= search[0]
                        );
                        assert!(
                            search[2] > search[0]
                                && search[3] + if tiny { 8. } else { 14. } <= s.content_top()
                        );
                        for (_, r) in s.layout() {
                            let inset = if tiny { 12. } else { 16. };
                            assert!(r[0] >= inset && r[2] <= w - inset && r[2] > r[0]);
                        }
                        assert!(s.footer_folder()[2] <= w - 12. && s.footer_folder()[3] <= h - 8.);
                        save(
                            &s,
                            &format!("{}-{}-{}x{}.png", i + 1, design, width, height),
                        );
                    }
                    let hwnd = s.hwnd;
                    let dpi = s.dpi;
                    drop(s);
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        0,
                        0,
                        (420. * dpi) as i32,
                        (650. * dpi) as i32,
                        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                    s = host.state.borrow_mut();
                    s.background_key.clear();
                    assert_eq!(
                        IsWindowVisible(s.hwnd),
                        0,
                        "Captures must never show a desktop window"
                    );
                }
                let [w, h] = s.dimensions();
                assert!(matches!(s.hit([w - 70., h - 30.]), Some(Action::Folder)));
                s.board.notes.clear();
                s.paint();
                assert!(
                    s.list_base.is_none(),
                    "No overflow buffer is retained for an empty board"
                );
            }
            drop(host);
            assert!(std::fs::read(root.join("config/native-notes.json")).unwrap() == original);
            let shots: Vec<_> = model::DESIGNS
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    image::open(output.join(format!("{}-{}.png", i + 1, name)))
                        .unwrap()
                        .into_rgba8()
                })
                .collect();
            let width = shots[0].width();
            let height = shots[0].height();
            let mut gallery = image::RgbaImage::from_pixel(
                width * 2 + 48,
                height * 2 + 48,
                image::Rgba([23, 24, 36, 255]),
            );
            for (i, shot) in shots.iter().enumerate() {
                image::imageops::overlay(
                    &mut gallery,
                    shot,
                    (16 + (i as u32 % 2) * (width + 16)) as i64,
                    (16 + (i as u32 / 2) * (height + 16)) as i64,
                );
            }
            gallery.save(output.join("notes-themes.png")).unwrap();
            std::fs::remove_file(root.join("config/native-notes.json")).unwrap();
            std::fs::remove_dir(root.join("config")).unwrap();
            std::fs::remove_dir(root).unwrap();
        }
    }
    #[test]
    fn tiny_layout_keeps_native_input_and_corner_button_hits_usable() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            let _runtime = crate::render::Runtime::start().unwrap();
            let root = std::env::temp_dir().join(format!(
                "lumen-tiny-notes-{}-{}",
                std::process::id(),
                model::now()
            ));
            let mut board = model::Board::default();
            board.size = [160, 160];
            board.create().unwrap();
            board.save(&root).unwrap();
            let original = std::fs::read(root.join("config/native-notes.json")).unwrap();
            let glass = Rc::new(RefCell::new(Glass::new(13., 1.)));
            glass.borrow_mut().refresh();
            let host = Host::create(&root, glass, true, true).unwrap();
            let hit_dpi = host.state.borrow().dpi;
            crate::geometry::assert_widget_corner_hits(host.hwnd(), hit_dpi);
            let hwnd = host.hwnd();
            let mut min: MINMAXINFO = zeroed();
            SendMessageW(hwnd, WM_GETMINMAXINFO, 0, &mut min as *mut _ as isize);
            assert_eq!(
                min.ptMinTrackSize.x,
                (160. * host.state.borrow().dpi) as i32
            );
            for design in model::DESIGNS {
                let mut s = host.state.borrow_mut();
                s.board.design = design.into();
                s.background_key.clear();
                s.scroll.snap(0.);
                s.paint();
                let first = s.layout()[0].clone();
                let rect = first.1;
                assert!(rect[0] >= 12. && rect[2] <= 148. && rect[3] <= s.viewport()[1]);
                assert_eq!(s.hit([26., 26.]), Some(Action::New));
                assert_eq!(s.hit([58., 26.]), Some(Action::Search));
                s.action(Action::Card(first.0));
                s.paint();
                let input = s.editor.as_ref().unwrap().input_hwnd();
                let mut input_rect: RECT = zeroed();
                GetClientRect(input, &mut input_rect);
                assert!(input_rect.right >= 80 && input_rect.bottom >= 20);
                assert_eq!(GetAncestor(input, GA_ROOT), hwnd);
                assert!(s.toolbar().iter().any(|(a, _, _)| *a == Action::Done));
                s.action(Action::Done);
                s.action(Action::Search);
                s.paint();
                assert!(
                    s.layout().is_empty(),
                    "Short search view must not overlap content"
                );
                assert!(s.search_rect()[2] - s.search_rect()[0] >= 120.);
                assert!(s.toolbar().iter().any(|(a, _, _)| *a == Action::Done));
                s.action(Action::Done);
                assert_eq!(s.layout().len(), 1);
                assert_eq!(IsWindowVisible(hwnd), 0);
            }
            let mut point = POINT { x: 20, y: 20 };
            let dpi = host.state.borrow().dpi;
            point.x = (20. * dpi) as i32;
            point.y = (20. * dpi) as i32;
            ClientToScreen(hwnd, &mut point);
            let packed = ((point.x as u16 as u32) | ((point.y as u16 as u32) << 16)) as isize;
            assert_eq!(
                SendMessageW(hwnd, WM_NCHITTEST, 0, packed),
                HTCLIENT as isize,
                "Add must win over diagonal resize grip"
            );
            drop(host);
            assert_eq!(
                std::fs::read(root.join("config/native-notes.json")).unwrap(),
                original
            );
            std::fs::remove_file(root.join("config/native-notes.json")).unwrap();
            std::fs::remove_dir(root.join("config")).unwrap();
            std::fs::remove_dir(root).unwrap();
        }
    }
    #[test]
    fn inline_edit_compact_layout_and_detached_settings_preserve_notes_and_bounds() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            let _runtime = crate::render::Runtime::start().unwrap();
            let root = std::env::temp_dir().join(format!(
                "lumen-inline-notes-{}-{}",
                std::process::id(),
                model::now()
            ));
            let glass = Rc::new(RefCell::new(Glass::new(13., 1.)));
            glass.borrow_mut().refresh();
            let mut host = Host::create(&root, glass.clone(), true, true).unwrap();
            {
                let mut s = host.state.borrow_mut();
                for _ in 0..3 {
                    s.board.create().unwrap();
                }
                s.board.notes[0].body = "<b>Original</b>".into();
            }
            host.show();
            let hwnd = host.hwnd();
            SetWindowPos(
                hwnd,
                null_mut(),
                0,
                0,
                240,
                240,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let mut s = host.state.borrow_mut();
            let id = s.board.notes[0].id.clone();
            s.action(Action::Card(id.clone()));
            s.paint();
            let editor = s.editor.as_ref().unwrap();
            let input = editor.input_hwnd();
            assert_eq!(
                GetAncestor(input, GA_ROOT),
                hwnd,
                "Inline editor must belong to the board, not a separate top-level window"
            );
            let mut outer: RECT = zeroed();
            GetWindowRect(hwnd, &mut outer);
            let layout = s.layout();
            let active = layout.iter().find(|(n, _)| *n == id).unwrap().1;
            assert!(
                active[0] >= 0.
                    && active[2] <= 240.
                    && active[1] >= s.viewport()[0]
                    && active[3] <= s.viewport()[1]
            );
            let before_order: Vec<_> = layout.iter().map(|n| n.0.clone()).collect();
            SetWindowTextW(input, wide("Gõ ngay trên thẻ – Tiếng Việt 😀").as_ptr());
            s.consume();
            s.paint();
            assert!(s.board.notes.iter().any(|n| n.id == id
                && n.body == "Gõ ngay trên thẻ – Tiếng Việt 😀"
                && n.rtf.is_some()));
            assert_eq!(
                s.layout().iter().map(|n| n.0.clone()).collect::<Vec<_>>(),
                before_order,
                "Typing must not reorder the active card"
            );
            let frame = s.frame.as_ref().unwrap();
            let mut pixels = frame.snapshot();
            for p in pixels.chunks_exact_mut(4) {
                p.swap(0, 2);
            }
            image::save_buffer(
                crate::model::root().join(".scratch/native-notes-inline.png"),
                &pixels,
                frame.width as u32,
                frame.height as u32,
                image::ColorType::Rgba8,
            )
            .unwrap();
            for design in model::DESIGNS {
                s.board.design = design.into();
                s.paint();
                assert!(s.layout().iter().all(|(_, r)| r[0] >= 0. && r[2] <= 240.));
            }
            let before = (
                s.board.size,
                s.board.position,
                s.board.design.clone(),
                s.board.notes.len(),
            );
            s.open_settings();
            let settings_hwnd = s.settings.as_ref().unwrap().hwnd();
            assert_ne!(settings_hwnd, hwnd);
            assert!(
                s.editor.is_some(),
                "Opening settings must retain the inline editor"
            );
            s.open_settings();
            assert_eq!(
                s.settings.as_ref().unwrap().hwnd(),
                settings_hwnd,
                "Reopening must reuse settings HWND"
            );
            assert_eq!(
                (
                    s.board.size,
                    s.board.position,
                    s.board.design.clone(),
                    s.board.notes.len()
                ),
                before
            );
            assert!(Rc::ptr_eq(&s.glass, &glass));
            settings::apply(
                &mut s.board,
                crate::settings_ui::Change {
                    id: 4,
                    value: crate::settings_ui::Value::Number(24.),
                },
            );
            s.update_local_blur();
            assert!(
                !Rc::ptr_eq(&s.source_glass(), &glass),
                "Notes blur must never mutate the shared Dock Glass"
            );
            s.hide();
            assert!(s.editor.is_none());
            drop(s);
            drop(host);
            assert!(!root.join("config/native-notes.json").exists());
        }
    }
    #[test]
    fn restored_bounds_preserve_negative_monitors_and_recover_disconnected_positions() {
        let left_monitor = RECT {
            left: -1920,
            top: -200,
            right: 0,
            bottom: 880,
        };
        let valid = restored_bounds([-1700, -100], [720, 620], left_monitor);
        assert_eq!(
            [valid.left, valid.top, valid.right, valid.bottom],
            [-1700, -100, -980, 520]
        );
        let primary = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };
        let disconnected = restored_bounds([-3000, -1500], [720, 620], primary);
        assert_eq!(
            [
                disconnected.left,
                disconnected.top,
                disconnected.right,
                disconnected.bottom
            ],
            [0, 0, 720, 620]
        );
        let oversized = restored_bounds([5000, 5000], [2600, 1800], primary);
        assert_eq!(
            [
                oversized.left,
                oversized.top,
                oversized.right,
                oversized.bottom
            ],
            [0, 0, 1920, 1040]
        );
        let right_edge = restored_bounds([1800, 900], [720, 620], primary);
        assert_eq!(
            [
                right_edge.left,
                right_edge.top,
                right_edge.right,
                right_edge.bottom
            ],
            [1200, 420, 1920, 1040]
        );
    }
    #[test]
    fn board_renders_designs_edits_and_restores_deleted_note() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            let _graphics = crate::render::Runtime::start().unwrap();
            let root = std::env::temp_dir().join(format!(
                "lumen-notes-board-{}-{}",
                std::process::id(),
                model::now()
            ));
            let glass = Rc::new(RefCell::new(Glass::new(13., 1.)));
            glass.borrow_mut().refresh();
            let mut host = Host::create(&root, glass, true, true).unwrap();
            assert_eq!(window_text(host.hwnd()), "Lumen · Sticky Notes");
            {
                let mut s = host.state.borrow_mut();
                for body in [
                    "<b>Today</b><br>Review the launch checklist<br>Take a walk and clear your head.",
                    "Small idea<br>A quiet space for thoughts, right on the desktop.",
                    "Tiếng Việt 😀<br>Ghi chú được lưu riêng, giữ nguyên bản cũ.",
                    "<b>Groceries</b><br><ul><li>Coffee beans</li><li>Sourdough</li></ul>",
                ] {
                    s.board.create().unwrap();
                    s.board.notes.last_mut().unwrap().body = body.into();
                }
                s.board.notes[0].pinned = true;
            }
            host.show();
            {
                let mut s = host.state.borrow_mut();
                let mut before: RECT = zeroed();
                GetWindowRect(s.hwnd, &mut before);
                for _ in 0..8 {
                    s.paint();
                    let mut after: RECT = zeroed();
                    GetWindowRect(s.hwnd, &mut after);
                    assert_eq!(
                        [after.left, after.top, after.right, after.bottom],
                        [before.left, before.top, before.right, before.bottom],
                        "Repeated layered paints must not shrink or move the native window"
                    );
                }
                // Stateless NC handling must also work with a live RefMut.
                let mut client = before;
                assert_eq!(
                    SendMessageW(s.hwnd, WM_NCCALCSIZE, 0, &mut client as *mut _ as isize),
                    0
                );
                assert_eq!(
                    [client.left, client.top, client.right, client.bottom],
                    [before.left, before.top, before.right, before.bottom]
                );
                let size = s.board.size;
                let position = s.board.position;
                for design in ["notebook", "index", "lumen"] {
                    assert!(settings::apply(
                        &mut s.board,
                        crate::settings_ui::Change {
                            id: 7,
                            value: crate::settings_ui::Value::Choice(design.into())
                        }
                    ));
                    s.background_key.clear();
                    assert_eq!(s.board.design, design);
                    assert_eq!(s.board.size, size);
                    assert_eq!(s.board.position, position);
                    s.paint();
                    assert!(s.frame.is_some());
                }
                s.board.design = "lumen".into();
                s.paint();
                let frame = s.frame.as_ref().unwrap();
                let mut pixels = frame.snapshot();
                for p in pixels.chunks_exact_mut(4) {
                    p.swap(0, 2);
                }
                let folder = crate::model::root().join(".scratch");
                std::fs::create_dir_all(&folder).unwrap();
                image::save_buffer(
                    folder.join("native-notes-preview.png"),
                    &pixels,
                    frame.width as u32,
                    frame.height as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                let id = s.board.notes[0].id.clone();
                s.action(Action::Delete(id.clone()));
                assert_eq!(s.board.notes.len(), 3);
                s.action(Action::Undo);
                assert_eq!(s.board.notes.len(), 4);
                assert!(s.board.notes.iter().any(|n| n.id == id && n.pinned));
                s.query = "tiếng".into();
                assert_eq!(s.layout().len(), 1);
                s.action(Action::Clear);
                assert_eq!(s.layout().len(), 4);
                s.board.opacity = 0.;
                s.paint();
                let frame = s.frame.as_ref().unwrap();
                let pixels = frame.snapshot();
                let alpha = |x: f32, y: f32| {
                    pixels[(((y * s.dpi) as usize) * frame.width as usize + (x * s.dpi) as usize)
                        * 4
                        + 3]
                };
                assert!(
                    alpha(50., 200.) > 200,
                    "Note backing must survive zero chrome opacity"
                );
                assert!(
                    alpha(300., 10.) <= 1,
                    "Chrome must become transparent while preserving a hit target"
                );
                s.hide();
                assert_eq!(IsWindowVisible(s.hwnd), 0);
                assert!(s.frame.is_none() && s.background.is_none() && s.overlay.is_none());
                assert!(s.editor.is_none() && s.shadow.is_none());
                assert!(!s.scroll.moving(s.now()));
            }
            drop(host);
            assert!(
                !root.join("config/native-notes.json").exists(),
                "Smoke mode must never write live data"
            );
        }
    }
    #[test]
    fn pasted_image_scales_to_note_width_and_preserves_channels() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            let _graphics = crate::render::Runtime::start().unwrap();
            let root = std::env::temp_dir().join(format!(
                "lumen-notes-image-scale-{}-{}",
                std::process::id(),
                model::now()
            ));
            let dir = model::images_dir(&root);
            std::fs::create_dir_all(&dir).unwrap();
            // Solid red 800x400 image (2:1 aspect ratio)
            let mut rgba = vec![0u8; 800 * 400 * 4];
            for p in rgba.chunks_exact_mut(4) {
                p[0] = 240;
                p[1] = 20;
                p[2] = 10;
                p[3] = 255;
            }
            let img = image::RgbaImage::from_raw(800, 400, rgba).unwrap();
            img.save(dir.join("test-wide.png")).unwrap();

            let glass = Rc::new(RefCell::new(Glass::new(13., 1.)));
            glass.borrow_mut().refresh();
            let mut host = Host::create(&root, glass, true, true).unwrap();
            host.show();
            {
                let mut s = host.state.borrow_mut();
                s.board.notes.clear();
                s.board.create().unwrap();
                s.board.notes[0].images.push("test-wide.png".into());

                SetWindowPos(
                    s.hwnd,
                    null_mut(),
                    0,
                    0,
                    (280. * s.dpi).round() as i32,
                    (420. * s.dpi).round() as i32,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
                let layout_narrow = s.layout();
                let narrow_rect = layout_narrow[0].1;
                let narrow_w = narrow_rect[2] - narrow_rect[0];
                let narrow_text_w = (narrow_w - s.type_scale().left - 16.).max(s.type_scale().body);
                let narrow_img_h = s.images_height(&["test-wide.png".into()], narrow_text_w);
                assert!(
                    (narrow_img_h - narrow_text_w * 0.5).abs() < 0.5,
                    "2:1 image should scale to half of note text width: {narrow_img_h} vs {}",
                    narrow_text_w * 0.5
                );
                assert!(
                    (narrow_rect[3] - narrow_rect[1]) >= narrow_img_h,
                    "Card height must expand to fit scaled image"
                );

                SetWindowPos(
                    s.hwnd,
                    null_mut(),
                    0,
                    0,
                    (380. * s.dpi).round() as i32,
                    (520. * s.dpi).round() as i32,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
                let layout_wide = s.layout();
                let wide_rect = layout_wide[0].1;
                let wide_w = wide_rect[2] - wide_rect[0];
                let wide_text_w = (wide_w - s.type_scale().left - 16.).max(s.type_scale().body);
                let wide_img_h = s.images_height(&["test-wide.png".into()], wide_text_w);
                assert!(
                    wide_img_h > narrow_img_h,
                    "Scaled image height must grow when note width grows: {wide_img_h} vs {narrow_img_h}"
                );
                assert!(
                    (wide_img_h - wide_text_w * 0.5).abs() < 0.5,
                    "2:1 image should remain proportional to wider note width"
                );

                SetWindowPos(
                    s.hwnd,
                    null_mut(),
                    0,
                    0,
                    (280. * s.dpi).round() as i32,
                    (420. * s.dpi).round() as i32,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
                s.paint();
                let frame = s.frame.as_ref().unwrap();
                let pixels = frame.snapshot();
                let x = ((narrow_rect[0] + s.type_scale().left + 20.) * s.dpi) as usize;
                let y = ((narrow_rect[1] + s.type_scale().top + 20.) * s.dpi) as usize;
                let idx = (y * frame.width as usize + x) * 4;
                // BGRA in DIBSection: R is at idx + 2, B is at idx + 0
                assert!(
                    pixels[idx + 2] > pixels[idx] + 80,
                    "Red channel must dominate blue for a red image (no R/B swap)"
                );
                s.hide();
            }
            drop(host);
            let _ = std::fs::remove_dir_all(&root);
        }
    }
}

pub(crate) mod package;
