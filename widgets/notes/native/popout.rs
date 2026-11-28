//! Always-on-top floating duplicate windows for individual sticky notes.
//! Matches Lumen's opaque sticky-note card visual with a top status/drag bar,
//! 120Hz VSync-paced atomic dragging, smooth unified scrolling, and two-way live sync.
use super::*;
use std::collections::HashMap;
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};

pub(super) const CLASS: &str = "LumenNativeNotesPopout";
const INPUT_CLASS: &str = "LumenNativeNotesPopoutInput";

pub(super) const POPOUT_SYNC: u32 = WM_APP + 108;
const POPOUT_REDRAW: u32 = WM_APP + 109;
const POPOUT_PASTE_IMAGE: u32 = WM_APP + 110;
const CARET_TIMER: usize = 120;
const ANIMATE_TIMER: usize = 121;
const DRAG_TIMER: usize = 122;

const EM_GETLINECOUNT: u32 = 0x00BA;
const EM_EXLINEFROMCHAR: u32 = WM_USER + 54;
const EM_GETSCROLLPOS: u32 = WM_USER + 221;
const EM_SETSCROLLPOS: u32 = WM_USER + 222;

const INSET: f32 = 4.0;
const RADIUS: f32 = 18.0;
const HEADER_H: f32 = 36.0;
const COLOR_BAR_H: f32 = 34.0;
const TOOLBAR_H: f32 = 34.0;
const BODY_SIZE: f32 = 14.0;
const LEADING: f32 = 20.0;
const DEFAULT_W: i32 = 328;
const DEFAULT_H: i32 = 268;

#[derive(Clone, Copy)]
pub(super) enum Anchor {
    Beside([i32; 4]),
    Cascade([i32; 2]),
}

#[derive(Clone)]
pub(super) enum Command {
    UpdateNote(model::Note),
    UpdateBounds {
        id: String,
        bounds: model::PopoutBounds,
    },
    CreateNew {
        anchor_rect: [i32; 4],
    },
    Closed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HitTarget {
    ColorBead,
    DragBar,
    New,
    Close,
    Swatch(usize),
    Format(usize),
    Scrollbar,
    ImageDelete,
    Image(usize),
    Body,
}

#[derive(Clone, Copy)]
struct DragMove {
    start_cursor: POINT,
    start_pos: [i32; 2],
    applied_pos: [i32; 2],
}

struct PopoutState {
    hwnd: HWND,
    input_host: HWND,
    edit: HWND,
    root: PathBuf,
    note: model::Note,
    dpi: f32,
    initial_dpi: f32,
    font: HFONT,
    frame: Option<Buffer>,
    backdrop: Option<Buffer>,
    backdrop_key: String,
    shadow: Option<WindowShadow>,
    snapshot: Option<(u32, i32, i32, crate::render::Image)>,
    typography: typography::Typography,
    image_cache: HashMap<String, (u32, u32, crate::render::Image)>,
    selected_image: Option<usize>,
    color_menu_open: bool,
    hover: Option<HitTarget>,
    pressed: Option<HitTarget>,
    caret_on: bool,
    loading: bool,
    closing: bool,
    moving: bool,
    resizing: bool,
    scroll: crate::motion::SmoothScroll,
    scroll_clock: Option<crate::motion::FrameClock>,
    frame_queued: bool,
    started: std::time::Instant,
    scrollbar_drag: Option<f32>,
    scrollbar_hover: bool,
    last_applied_scroll: f32,
    edit_bounds: [i32; 4],
    drag_move: Option<DragMove>,
    drag_frame_queued: bool,
    drag_dirty: bool,
}

struct PopoutWindow {
    hwnd: HWND,
    root: PathBuf,
    state: Box<RefCell<PopoutState>>,
}

impl Drop for PopoutWindow {
    fn drop(&mut self) {
        unsafe {
            if !self.hwnd.is_null() && IsWindow(self.hwnd) != 0 {
                DestroyWindow(self.hwnd);
            }
        }
    }
}

#[derive(Default)]
struct Manager {
    windows: HashMap<String, PopoutWindow>,
    boards: Vec<(HWND, PathBuf)>,
    queues: HashMap<usize, Vec<Command>>,
}

impl Manager {
    fn emit(&mut self, root: &Path, cmd: Command) {
        unsafe {
            self.boards
                .retain(|(hwnd, _)| !hwnd.is_null() && IsWindow(*hwnd) != 0);
            for (hwnd, b_root) in &self.boards {
                if b_root == root {
                    self.queues
                        .entry(*hwnd as usize)
                        .or_default()
                        .push(cmd.clone());
                    PostMessageW(*hwnd, POPOUT_SYNC, 0, 0);
                }
            }
        }
    }
}

thread_local! {
    static MANAGER: RefCell<Manager> = RefCell::new(Manager::default());
}

pub(super) unsafe fn register_board(hwnd: HWND, root: PathBuf) {
    MANAGER.with(|m| {
        let mut m = m.borrow_mut();
        if !m.boards.iter().any(|(h, _)| *h == hwnd) {
            m.boards.push((hwnd, root));
        }
    });
}

pub(super) unsafe fn unregister_board(hwnd: HWND, root: &Path, smoke: bool) {
    let to_drop = MANAGER.with(|m| {
        let mut m = m.borrow_mut();
        m.boards.retain(|(h, _)| *h != hwnd && IsWindow(*h) != 0);
        m.queues.remove(&(hwnd as usize));
        let has_peer = m.boards.iter().any(|(_, r)| r == root);
        if smoke || !has_peer {
            let ids: Vec<String> = m
                .windows
                .iter()
                .filter(|(_, w)| w.root == root)
                .map(|(id, _)| id.clone())
                .collect();
            ids.into_iter()
                .filter_map(|id| m.windows.remove(&id))
                .collect()
        } else {
            Vec::new()
        }
    });
    drop(to_drop);
}

pub(super) fn take_commands(hwnd: HWND) -> Vec<Command> {
    MANAGER.with(|m| {
        m.borrow_mut()
            .queues
            .remove(&(hwnd as usize))
            .unwrap_or_default()
    })
}

pub(super) fn is_open(id: &str) -> bool {
    MANAGER.with(|m| {
        m.borrow()
            .windows
            .get(id)
            .is_some_and(|w| unsafe { IsWindow(w.hwnd) != 0 })
    })
}

pub(super) unsafe fn open(
    root: &Path,
    note: model::Note,
    anchor: Option<Anchor>,
    inspect: bool,
    _smoke: bool,
) -> Result<HWND, String> {
    if let Some(existing_hwnd) = MANAGER.with(|m| {
        m.borrow()
            .windows
            .get(&note.id)
            .filter(|w| IsWindow(w.hwnd) != 0)
            .map(|w| w.hwnd)
    }) {
        ensure_topmost(existing_hwnd);
        ShowWindow(existing_hwnd, SW_SHOW);
        SetForegroundWindow(existing_hwnd);
        MANAGER.with(|m| {
            if let Some(w) = m.borrow().windows.get(&note.id) {
                if let Ok(s) = w.state.try_borrow() {
                    SetFocus(s.edit);
                }
            }
        });
        return Ok(existing_hwnd);
    }

    editor::ensure_richedit_loaded()?;
    register(CLASS, Some(popout_proc));
    register(INPUT_CLASS, Some(input_host_proc));

    let open_count = MANAGER.with(|m| m.borrow().windows.len()) as i32;
    let saved = note.popout.clone();
    let (pos, dip_size) = if let Some(p) = &saved {
        (
            [p.x, p.y],
            [
                p.width.clamp(model::POPOUT_MIN_W, 1600),
                p.height.clamp(model::POPOUT_MIN_H, 1200),
            ],
        )
    } else {
        match anchor {
            Some(Anchor::Beside(a)) => {
                let offset = (open_count % 6) * 28;
                ([a[2] + 16 + offset, a[1] + 24 + offset], [DEFAULT_W, DEFAULT_H])
            }
            Some(Anchor::Cascade(p)) => ([p[0] + 28, p[1] + 28], [DEFAULT_W, DEFAULT_H]),
            None => {
                let offset = (open_count % 6) * 28;
                ([220 + offset, 180 + offset], [DEFAULT_W, DEFAULT_H])
            }
        }
    };

    let sys_dpi = (GetDpiForSystem() as f32 / 96.).max(1.);
    let probe = RECT {
        left: pos[0],
        top: pos[1],
        right: pos[0].saturating_add((dip_size[0] as f32 * sys_dpi).round() as i32),
        bottom: pos[1].saturating_add((dip_size[1] as f32 * sys_dpi).round() as i32),
    };
    let monitor = MonitorFromRect(&probe, MONITOR_DEFAULTTONEAREST);
    let mut info: MONITORINFO = zeroed();
    info.cbSize = size_of::<MONITORINFO>() as u32;
    if GetMonitorInfoW(monitor, &mut info) == 0
        || info.rcWork.right <= info.rcWork.left
        || info.rcWork.bottom <= info.rcWork.top
    {
        return Err("Cannot resolve an active monitor work area for floating note".into());
    }
    let mon_dpi = crate::geometry::monitor_dpi(monitor, null_mut()).max(1.);
    let w_px = (dip_size[0] as f32 * mon_dpi).round() as i32;
    let h_px = (dip_size[1] as f32 * mon_dpi).round() as i32;
    let mut adjusted_pos = pos;
    if saved.is_none() {
        if let Some(Anchor::Beside(a)) = anchor {
            if adjusted_pos[0] + w_px > info.rcWork.right && a[0] - 16 - w_px >= info.rcWork.left {
                adjusted_pos[0] = a[0] - 16 - w_px;
            }
        }
    }
    let r = restored_bounds(adjusted_pos, [w_px, h_px], info.rcWork);
    let win_w = r.right - r.left;
    let win_h = r.bottom - r.top;

    let note_id = note.id.clone();
    let mut state = Box::new(RefCell::new(PopoutState {
        hwnd: null_mut(),
        input_host: null_mut(),
        edit: null_mut(),
        root: root.to_path_buf(),
        note,
        dpi: mon_dpi,
        initial_dpi: mon_dpi,
        font: null_mut(),
        frame: None,
        backdrop: None,
        backdrop_key: String::new(),
        shadow: None,
        snapshot: None,
        typography: Default::default(),
        image_cache: Default::default(),
        selected_image: None,
        color_menu_open: false,
        hover: None,
        pressed: None,
        caret_on: true,
        loading: true,
        closing: false,
        moving: false,
        resizing: false,
        scroll: Default::default(),
        scroll_clock: None,
        frame_queued: false,
        started: std::time::Instant::now(),
        scrollbar_drag: None,
        scrollbar_hover: false,
        last_applied_scroll: 0.,
        edit_bounds: [0; 4],
        drag_move: None,
        drag_frame_queued: false,
        drag_dirty: false,
    }));

    let ex_style = crate::gpu::window_ex_style()
        | WS_EX_TOPMOST
        | if inspect {
            WS_EX_APPWINDOW
        } else {
            WS_EX_TOOLWINDOW
        };
    let hwnd = CreateWindowExW(
        ex_style,
        wide(CLASS).as_ptr(),
        wide("Lumen · Sticky Note").as_ptr(),
        WS_POPUP | WS_THICKFRAME,
        r.left,
        r.top,
        win_w,
        win_h,
        null_mut(),
        null_mut(),
        GetModuleHandleW(null()),
        state.as_mut() as *mut _ as _,
    );
    if hwnd.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    SetWindowTextW(hwnd, wide("Lumen · Sticky Note").as_ptr());
    windows_sys::Win32::UI::Shell::DragAcceptFiles(hwnd, 1);

    let (live_dpi, _) = crate::geometry::clamp_window_to_work(
        hwnd,
        [model::POPOUT_MIN_W, model::POPOUT_MIN_H],
        mon_dpi,
    );

    let input_host = CreateWindowExW(
        WS_EX_TRANSPARENT,
        wide(INPUT_CLASS).as_ptr(),
        wide("").as_ptr(),
        WS_CHILD | WS_VISIBLE,
        0,
        0,
        1,
        1,
        hwnd,
        null_mut(),
        GetModuleHandleW(null()),
        &*state as *const _ as _,
    );
    if input_host.is_null() {
        DestroyWindow(hwnd);
        return Err(std::io::Error::last_os_error().to_string());
    }
    let edit = CreateWindowExW(
        0,
        wide("RICHEDIT50W").as_ptr(),
        wide("").as_ptr(),
        WS_CHILD
            | WS_VISIBLE
            | WS_TABSTOP
            | ES_AUTOVSCROLL as u32
            | ES_MULTILINE as u32
            | ES_WANTRETURN as u32,
        0,
        0,
        100,
        100,
        input_host,
        100usize as _,
        GetModuleHandleW(null()),
        null(),
    );
    if edit.is_null() {
        DestroyWindow(hwnd);
        return Err("Cannot create floating note RichEdit control".into());
    }
    SetWindowSubclass(edit, Some(popout_input_proc), 1, hwnd as usize);

    {
        let mut s = state.borrow_mut();
        s.hwnd = hwnd;
        s.input_host = input_host;
        s.edit = edit;
        s.dpi = live_dpi;
        s.initial_dpi = live_dpi;
        s.font = CreateFontW(
            -(BODY_SIZE * live_dpi).round() as i32,
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            ANTIALIASED_QUALITY as u32,
            0,
            wide("Segoe UI Variable Text").as_ptr(),
        );
        SendMessageW(edit, WM_SETFONT, s.font as usize, 1);
        let left_margin = (2. * live_dpi).round().max(1.) as u16;
        SendMessageW(edit, 0x00D3, 3, left_margin as isize);
        SendMessageW(edit, WM_USER + 67, 0, rgb(36, 40, 46) as isize);
        SendMessageW(edit, WM_USER + 69, 0, 1); // ENM_CHANGE
        SendMessageW(
            edit,
            windows_sys::Win32::UI::Controls::EM_LIMITTEXT,
            200_000,
            0,
        );
        editor::set_edit_default_char_format(edit, BODY_SIZE, false);
        if !editor::stream_note_rtf(edit, &s.note) {
            drop(s);
            DestroyWindow(hwnd);
            return Err("Cannot read floating note formatting".into());
        }
        editor::apply_edit_line_spacing(edit, live_dpi, LEADING, false);
        s.loading = false;
        s.layout_edit();
        s.shadow = WindowShadow::new(hwnd);
    }

    ensure_topmost(hwnd);
    ShowWindow(hwnd, SW_SHOW);
    SetForegroundWindow(hwnd);
    {
        let mut s = state.borrow_mut();
        s.paint();
        SetFocus(s.edit);
        SetTimer(s.hwnd, CARET_TIMER, editor::blink_ms(), None);
        let bounds = s.current_bounds(true);
        let root_clone = s.root.clone();
        let id_clone = s.note.id.clone();
        MANAGER.with(|m| {
            let mut m = m.borrow_mut();
            m.emit(
                &root_clone,
                Command::UpdateBounds {
                    id: id_clone,
                    bounds,
                },
            );
        });
    }

    MANAGER.with(|m| {
        m.borrow_mut().windows.insert(
            note_id,
            PopoutWindow {
                hwnd,
                root: root.to_path_buf(),
                state,
            },
        );
    });
    Ok(hwnd)
}

pub(super) unsafe fn sync_note(note: &model::Note) {
    let target = MANAGER.with(|m| {
        let m = m.borrow();
        m.windows
            .get(&note.id)
            .filter(|w| IsWindow(w.hwnd) != 0)
            .map(|w| (w.hwnd, &*w.state as *const RefCell<PopoutState>))
    });
    let Some((hwnd, state_ptr)) = target else {
        return;
    };
    let Ok(mut s) = (*state_ptr).try_borrow_mut() else {
        return;
    };
    let content_same = s.note.body == note.body && s.note.rtf == note.rtf;
    let color_same = s.note.color == note.color;
    let images_same = s.note.images == note.images;
    if content_same && color_same && images_same {
        return;
    }
    s.note.color = note.color.clone();
    s.note.pinned = note.pinned;
    s.note.updated_at = note.updated_at;
    if !color_same {
        s.backdrop_key.clear();
    }
    if !images_same {
        s.note.images = note.images.clone();
        if s.selected_image
            .is_some_and(|idx| idx >= s.note.images.len())
        {
            s.selected_image = None;
        }
        s.layout_edit();
    }
    if !content_same && GetFocus() != s.edit {
        s.note.body = note.body.clone();
        s.note.rtf = note.rtf.clone();
        s.loading = true;
        SetWindowTextW(s.edit, wide("").as_ptr());
        editor::set_edit_default_char_format(s.edit, BODY_SIZE, true);
        let _ = editor::stream_note_rtf(s.edit, &s.note);
        editor::apply_edit_line_spacing(s.edit, s.dpi, LEADING, false);
        s.loading = false;
        s.snapshot = None;
        s.layout_edit();
    }
    drop(s);
    InvalidateRect(hwnd, null(), 0);
}

pub(super) unsafe fn close(id: &str) {
    let entry = MANAGER.with(|m| m.borrow_mut().windows.remove(id));
    if let Some(w) = entry {
        if let Ok(mut s) = w.state.try_borrow_mut() {
            s.closing = true;
            s.flush_edit();
            let bounds = s.current_bounds(false);
            let root = s.root.clone();
            drop(s);
            MANAGER.with(|m| {
                let mut m = m.borrow_mut();
                m.emit(
                    &root,
                    Command::UpdateBounds {
                        id: id.to_string(),
                        bounds,
                    },
                );
                m.emit(&root, Command::Closed(id.to_string()));
            });
        }
        drop(w);
    }
}

pub(super) unsafe fn handle_key(hwnd: HWND, vk: usize) {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<PopoutState>;
    if ptr.is_null() {
        return;
    }
    let Ok(mut s) = (*ptr).try_borrow_mut() else {
        return;
    };
    match vk {
        0x42 | 0x49 | 0x55 | 0x53 => {
            let idx = match vk {
                0x42 => 0,
                0x49 => 1,
                0x55 => 2,
                _ => 3,
            };
            s.format(idx);
            InvalidateRect(hwnd, null(), 0);
        }
        0x57 => {
            drop(s);
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
        }
        0x4e => {
            s.emit_create_new();
        }
        0x1b => {
            if s.color_menu_open {
                s.color_menu_open = false;
                s.backdrop_key.clear();
                s.layout_edit();
                InvalidateRect(hwnd, null(), 0);
            } else if s.selected_image.take().is_some() {
                InvalidateRect(hwnd, null(), 0);
            }
        }
        _ => {}
    }
}

unsafe fn ensure_topmost(hwnd: HWND) {
    SetWindowPos(
        hwnd,
        HWND_TOPMOST,
        0,
        0,
        0,
        0,
        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
    );
}

unsafe fn finish_close(id: &str, bounds: model::PopoutBounds) {
    let removed = MANAGER.with(|m| {
        let mut m = m.borrow_mut();
        let entry = m.windows.remove(id);
        if let Some(root) = entry.as_ref().map(|e| e.root.clone()) {
            m.emit(
                &root,
                Command::UpdateBounds {
                    id: id.to_string(),
                    bounds,
                },
            );
            m.emit(&root, Command::Closed(id.to_string()));
        }
        entry
    });
    drop(removed);
}

impl PopoutState {
    fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1000.
    }

    unsafe fn dimensions(&self) -> [f32; 2] {
        let mut r: RECT = zeroed();
        GetClientRect(self.hwnd, &mut r);
        [r.right as f32 / self.dpi, r.bottom as f32 / self.dpi]
    }

    unsafe fn card_rect(&self) -> [f32; 4] {
        let [w, h] = self.dimensions();
        [
            INSET,
            INSET,
            (w - INSET).max(INSET + 1.),
            (h - INSET).max(INSET + 1.),
        ]
    }

    unsafe fn header_rect(&self) -> [f32; 4] {
        let c = self.card_rect();
        [c[0], c[1], c[2], c[1] + HEADER_H]
    }

    unsafe fn bead_btn_rect(&self) -> [f32; 4] {
        let c = self.card_rect();
        [c[0] + 8., c[1] + 4., c[0] + 36., c[1] + 32.]
    }

    unsafe fn new_btn_rect(&self) -> [f32; 4] {
        let c = self.card_rect();
        [c[2] - 66., c[1] + 4., c[2] - 38., c[1] + 32.]
    }

    unsafe fn close_btn_rect(&self) -> [f32; 4] {
        let c = self.card_rect();
        [c[2] - 36., c[1] + 4., c[2] - 8., c[1] + 32.]
    }

    unsafe fn color_bar_rect(&self) -> Option<[f32; 4]> {
        if !self.color_menu_open {
            return None;
        }
        let c = self.card_rect();
        Some([c[0], c[1] + HEADER_H, c[2], c[1] + HEADER_H + COLOR_BAR_H])
    }

    fn swatch_rect(bar: [f32; 4], idx: usize) -> [f32; 4] {
        let count = model::COLORS.len() as f32;
        let step = ((bar[2] - bar[0] - 20.) / count).min(28.);
        let total = step * count;
        let start_x = (bar[0] + bar[2]) * 0.5 - total * 0.5 + (step - 16.) * 0.5;
        let x = start_x + idx as f32 * step;
        let y = (bar[1] + bar[3]) * 0.5 - 8.;
        [x, y, x + 16., y + 16.]
    }

    unsafe fn toolbar_rect(&self) -> [f32; 4] {
        let c = self.card_rect();
        [c[0], c[3] - TOOLBAR_H, c[2], c[3]]
    }

    unsafe fn format_btn_rect(&self, idx: usize) -> [f32; 4] {
        let c = self.card_rect();
        let count = editor::TOOLBAR_LABELS.len() as f32;
        let bw = ((c[2] - c[0] - 24.) / count).min(25.);
        let left = c[2] - 12. - bw * count;
        let x = left + idx as f32 * bw;
        let y = c[3] - 30.;
        [x, y, x + bw - 3., y + 25.]
    }

    unsafe fn content_bounds(&self) -> [f32; 4] {
        let c = self.card_rect();
        let top = c[1] + HEADER_H + if self.color_menu_open { COLOR_BAR_H } else { 0. };
        let bottom = c[3] - TOOLBAR_H;
        [
            c[0] + 14.,
            top + 8.,
            (c[2] - 18.).max(c[0] + 32.),
            (bottom - 4.).max(top + 24.),
        ]
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

    fn images_height(&mut self, width: f32) -> f32 {
        let names = self.note.images.clone();
        let mut total = 0f32;
        for name in &names {
            if let Some((w, h)) = self.load_image(name) {
                if w > 0 {
                    let sh = (h as f32 / w as f32) * width;
                    if total > 0. {
                        total += 6.;
                    }
                    total += sh;
                }
            }
        }
        total
    }

    unsafe fn doc_metrics(&mut self) -> ([f32; 4], f32, f32, f32) {
        let cb = self.content_bounds();
        let text_w = (cb[2] - cb[0]).max(BODY_SIZE);
        let viewport_h = (cb[3] - cb[1]).max(LEADING);
        let edit_lines = if self.edit.is_null() {
            1
        } else {
            (SendMessageW(self.edit, EM_GETLINECOUNT, 0, 0) as usize).max(1)
        };
        let (text_empty, wrapped) = richtext::wrap_note(
            &self.note,
            text_w,
            self.dpi,
            BODY_SIZE,
            &mut self.typography,
        );
        let wrapped_lines = if text_empty { 1 } else { wrapped.len().max(1) };
        let line_count = edit_lines.max(wrapped_lines);
        let text_total_h = (line_count as f32 * LEADING + 8.).max(LEADING);
        let img_total_h = self.images_height(text_w);
        let total_doc_h = text_total_h
            + if img_total_h > 0. {
                6. + img_total_h
            } else {
                0.
            };
        let max_scroll = (total_doc_h - viewport_h).max(0.);
        (cb, text_total_h, viewport_h, max_scroll)
    }

    unsafe fn visible_rects_at(
        &mut self,
        scroll_y: f32,
    ) -> ([f32; 4], [f32; 4], Vec<[f32; 4]>, f32) {
        let (cb, text_total_h, viewport_h, max_scroll) = self.doc_metrics();
        let text_w = (cb[2] - cb[0]).max(BODY_SIZE);
        if self.note.images.is_empty() {
            return (cb, [0.; 4], vec![], max_scroll);
        }
        let vis_text_h = (text_total_h - scroll_y).clamp(0., viewport_h);
        let tr = [cb[0], cb[1], cb[2], cb[1] + vis_text_h];
        let mut y = cb[1] + text_total_h + 6. - scroll_y;
        let names = self.note.images.clone();
        let mut rects = Vec::with_capacity(names.len());
        for name in &names {
            if let Some((iw, ih)) = self.load_image(name) {
                if iw > 0 {
                    let sh = (ih as f32 / iw as f32) * text_w;
                    rects.push([cb[0], y, cb[0] + text_w, y + sh]);
                    y += sh + 6.;
                    continue;
                }
            }
            rects.push([0.; 4]);
        }
        (tr, cb, rects, max_scroll)
    }

    unsafe fn apply_scroll_position(&mut self, scroll_y: f32) {
        if self.input_host.is_null() || self.edit.is_null() {
            return;
        }
        let (tr, _, _, _) = self.visible_rects_at(scroll_y);
        let d = self.dpi;
        let x = (tr[0] * d).round() as i32;
        let y = (tr[1] * d).round() as i32;
        let w = ((tr[2] - tr[0]) * d).round().max(1.) as i32;
        let vis_h_dip = (tr[3] - tr[1]).max(0.);
        let h = (vis_h_dip * d).round().max(1.) as i32;
        if !self.note.images.is_empty() && vis_h_dip < 1. {
            ShowWindow(self.input_host, SW_HIDE);
            self.edit_bounds = [0; 4];
        } else {
            let next = [x, y, w, h];
            if self.edit_bounds != next || IsWindowVisible(self.input_host) == 0 {
                SetWindowPos(
                    self.input_host,
                    null_mut(),
                    x,
                    y,
                    w,
                    h,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                MoveWindow(self.edit, 0, 0, w, h, 0);
                if IsWindowVisible(self.input_host) == 0 {
                    ShowWindow(self.input_host, SW_SHOWNOACTIVATE);
                }
                self.edit_bounds = next;
            }
        }
        let pt = POINT {
            x: 0,
            y: (scroll_y * d).round().max(0.) as i32,
        };
        SendMessageW(self.edit, EM_SETSCROLLPOS, 0, &pt as *const _ as isize);
        if (scroll_y - self.last_applied_scroll).abs() > 0.01 {
            self.last_applied_scroll = scroll_y;
            self.snapshot = None;
        }
    }

    unsafe fn layout_edit(&mut self) {
        if self.input_host.is_null() || self.edit.is_null() {
            return;
        }
        let now = self.now();
        let (cb, _, _, max_scroll) = self.doc_metrics();
        self.scroll.bounds(max_scroll, now);
        let d = self.dpi;
        let w = ((cb[2] - cb[0]) * d).round().max(1.) as i32;
        let h = ((cb[3] - cb[1]) * d).round().max(1.) as i32;
        SetWindowPos(
            self.input_host,
            null_mut(),
            (cb[0] * d).round() as i32,
            (cb[1] * d).round() as i32,
            w,
            h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        MoveWindow(self.edit, 0, 0, w, h, 1);
        // Re-query line count after RichEdit wraps to the new width.
        let (_, _, _, updated_max) = self.doc_metrics();
        self.scroll.bounds(updated_max, now);
        self.edit_bounds = [0; 4];
        let scroll_y = self.scroll.value(now);
        self.apply_scroll_position(scroll_y);
        self.snapshot = None;
    }

    unsafe fn sync_scroll_to_caret(&mut self) {
        if self.input_host.is_null() || self.edit.is_null() {
            return;
        }
        let now = self.now();
        let mut pt = POINT { x: 0, y: 0 };
        SendMessageW(self.edit, EM_GETSCROLLPOS, 0, &mut pt as *mut _ as isize);
        let rich_y = (pt.y as f32 / self.dpi).max(0.);
        let (_, _, viewport_h, max_scroll) = self.doc_metrics();
        let bound = max_scroll.max(rich_y);
        self.scroll.bounds(bound, now);

        let mut sel_start: u32 = 0;
        let mut sel_end: u32 = 0;
        SendMessageW(
            self.edit,
            0x00B0, // EM_GETSEL
            &mut sel_start as *mut u32 as isize as usize,
            &mut sel_end as *mut u32 as isize,
        );
        let caret_line =
            (SendMessageW(self.edit, EM_EXLINEFROMCHAR, 0, sel_end as isize) as i32).max(0)
                as usize;
        let caret_top = caret_line as f32 * LEADING;
        let caret_bottom = (caret_line + 1) as f32 * LEADING + 4.;

        let cur = if (rich_y - self.last_applied_scroll).abs() > 0.5 {
            rich_y
        } else {
            self.scroll.value(now)
        };
        let mut target = cur;
        if caret_bottom > target + viewport_h {
            target = (caret_bottom - viewport_h).max(target);
        }
        if caret_top < target {
            target = caret_top;
        }
        target = target.clamp(0., bound);

        if (target - self.scroll.value(now)).abs() > 0.25
            || (target - self.last_applied_scroll).abs() > 0.25
            || (rich_y - target).abs() > 0.5
            || !self.note.images.is_empty()
        {
            self.scroll.snap(target);
            let applied = self.scroll.value(now);
            self.last_applied_scroll = applied;
            self.apply_scroll_position(applied);
            self.snapshot = None;
        }
    }

    unsafe fn scrollbar_rect(&self, scroll_y: f32) -> Option<[f32; 4]> {
        if self.scroll.max <= 0. {
            return None;
        }
        let c = self.card_rect();
        let cb = self.content_bounds();
        let track_top = cb[1] + 2.;
        let track_bottom = cb[3] - 2.;
        let track_h = (track_bottom - track_top).max(18.);
        let viewport_h = (cb[3] - cb[1]).max(LEADING);
        let thumb_h = (track_h * viewport_h / (viewport_h + self.scroll.max)).clamp(20., track_h);
        let thumb_y =
            track_top + (track_h - thumb_h) * (scroll_y / self.scroll.max).clamp(0., 1.);
        Some([c[2] - 9., thumb_y, c[2] - 5., thumb_y + thumb_h])
    }

    unsafe fn scroll_drag_to(&mut self, pointer_y: f32, grab: f32) {
        if self.scroll.max <= 0. {
            return;
        }
        let cb = self.content_bounds();
        let track_top = cb[1] + 2.;
        let track_bottom = cb[3] - 2.;
        let track_h = (track_bottom - track_top).max(18.);
        let viewport_h = (cb[3] - cb[1]).max(LEADING);
        let thumb_h = (track_h * viewport_h / (viewport_h + self.scroll.max)).clamp(20., track_h);
        if track_h <= thumb_h {
            self.scroll.snap(0.);
            return;
        }
        let frac = ((pointer_y - grab - track_top) / (track_h - thumb_h)).clamp(0., 1.);
        self.scroll.snap(frac * self.scroll.max);
    }

    unsafe fn current_bounds(&self, open: bool) -> model::PopoutBounds {
        let mut r: RECT = zeroed();
        GetWindowRect(self.hwnd, &mut r);
        model::PopoutBounds {
            x: r.left,
            y: r.top,
            width: ((r.right - r.left) as f32 / self.dpi)
                .round()
                .clamp(model::POPOUT_MIN_W as f32, 1600.) as i32,
            height: ((r.bottom - r.top) as f32 / self.dpi)
                .round()
                .clamp(model::POPOUT_MIN_H as f32, 1200.) as i32,
            open,
        }
    }

    unsafe fn emit_note(&self) {
        let root = self.root.clone();
        let note = self.note.clone();
        MANAGER.with(|m| {
            m.borrow_mut().emit(&root, Command::UpdateNote(note));
        });
    }

    unsafe fn emit_bounds(&self, open: bool) {
        let bounds = self.current_bounds(open);
        let root = self.root.clone();
        let id = self.note.id.clone();
        MANAGER.with(|m| {
            m.borrow_mut()
                .emit(&root, Command::UpdateBounds { id, bounds });
        });
    }

    unsafe fn emit_create_new(&self) {
        let mut r: RECT = zeroed();
        GetWindowRect(self.hwnd, &mut r);
        let root = self.root.clone();
        MANAGER.with(|m| {
            m.borrow_mut().emit(
                &root,
                Command::CreateNew {
                    anchor_rect: [r.left, r.top, r.right, r.bottom],
                },
            );
        });
    }

    unsafe fn flush_edit(&mut self) {
        if self.loading || self.edit.is_null() {
            return;
        }
        let text = window_text(self.edit);
        let Some(rtf) = editor::read_richedit_rtf(self.edit) else {
            return;
        };
        if self.note.body == text && self.note.rtf.as_ref() == Some(&rtf) {
            return;
        }
        self.note.body = text;
        self.note.rtf = Some(rtf);
        model::extract_rtf_pictures(&mut self.note, &self.root);
        self.note.updated_at = model::now();
        self.snapshot = None;
        self.emit_note();
    }

    unsafe fn format(&mut self, index: usize) {
        editor::toggle_edit_format(self.edit, index);
        SetFocus(self.edit);
        self.snapshot = None;
        self.flush_edit();
    }

    unsafe fn delete_selected_image(&mut self) {
        let Some(idx) = self.selected_image.take() else {
            return;
        };
        if idx >= self.note.images.len() {
            return;
        }
        self.note.images.remove(idx);
        self.note.updated_at = model::now();
        self.layout_edit();
        self.emit_note();
        InvalidateRect(self.hwnd, null(), 0);
    }

    unsafe fn paste_clipboard_image(&mut self) {
        let saved = editor::save_clipboard_images(self.hwnd, &self.root, self.note.images.len());
        if saved.is_empty() {
            return;
        }
        self.note.images.extend(saved);
        self.note.updated_at = model::now();
        self.layout_edit();
        self.emit_note();
        InvalidateRect(self.hwnd, null(), 0);
    }

    unsafe fn drop_files(&mut self, hdrop: windows_sys::Win32::UI::Shell::HDROP) {
        use windows_sys::Win32::UI::Shell::{DragFinish, DragQueryFileW};
        let count = DragQueryFileW(hdrop, u32::MAX, null_mut(), 0);
        let mut paths = Vec::new();
        for idx in 0..count {
            let len = DragQueryFileW(hdrop, idx, null_mut(), 0);
            if len == 0 {
                continue;
            }
            let mut buf = vec![0u16; (len + 1) as usize];
            if DragQueryFileW(hdrop, idx, buf.as_mut_ptr(), len + 1) > 0 {
                let nul = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                let path = PathBuf::from(String::from_utf16_lossy(&buf[..nul]));
                if editor::is_image_path(&path) {
                    paths.push(path);
                }
            }
        }
        DragFinish(hdrop);
        if paths.is_empty() {
            return;
        }
        let base_idx = self.note.images.len();
        let mut added = Vec::new();
        for (i, path) in paths.iter().enumerate() {
            if let Some(name) = editor::save_image_file(&self.root, path, base_idx + i) {
                added.push(name);
            }
        }
        if added.is_empty() {
            return;
        }
        self.note.images.extend(added);
        self.note.updated_at = model::now();
        self.layout_edit();
        self.emit_note();
        InvalidateRect(self.hwnd, null(), 0);
    }

    unsafe fn hit(&mut self, p: [f32; 2]) -> Option<HitTarget> {
        if inside(p, self.bead_btn_rect()) {
            return Some(HitTarget::ColorBead);
        }
        if inside(p, self.new_btn_rect()) {
            return Some(HitTarget::New);
        }
        if inside(p, self.close_btn_rect()) {
            return Some(HitTarget::Close);
        }
        if inside(p, self.header_rect()) {
            return Some(HitTarget::DragBar);
        }
        if let Some(bar) = self.color_bar_rect() {
            if inside(p, bar) {
                for i in 0..model::COLORS.len() {
                    let sr = Self::swatch_rect(bar, i);
                    if inside(p, [sr[0] - 4., sr[1] - 4., sr[2] + 4., sr[3] + 4.]) {
                        return Some(HitTarget::Swatch(i));
                    }
                }
                return Some(HitTarget::Body);
            }
        }
        let scroll_y = self.scroll.value(self.now());
        if let Some(bar) = self.scrollbar_rect(scroll_y) {
            if inside(p, [bar[0] - 6., bar[1] - 4., bar[2] + 6., bar[3] + 4.]) {
                return Some(HitTarget::Scrollbar);
            }
        }
        for i in 0..editor::TOOLBAR_LABELS.len() {
            if inside(p, self.format_btn_rect(i)) {
                return Some(HitTarget::Format(i));
            }
        }
        if !self.note.images.is_empty() {
            let (_, img_area, rects, _) = self.visible_rects_at(scroll_y);
            if inside(p, img_area) {
                if let Some(sel) = self.selected_image {
                    if let Some(&ir) = rects.get(sel) {
                        if ir[2] > ir[0] && inside(p, image_delete_rect(ir)) {
                            return Some(HitTarget::ImageDelete);
                        }
                    }
                }
                for (i, &ir) in rects.iter().enumerate() {
                    if ir[2] > ir[0] && ir[3] > ir[1] && inside(p, ir) {
                        return Some(HitTarget::Image(i));
                    }
                }
            }
        }
        if inside(p, self.card_rect()) && p[1] >= self.header_rect()[3] {
            return Some(HitTarget::Body);
        }
        None
    }

    unsafe fn handle_action(&mut self, target: HitTarget) {
        match target {
            HitTarget::ColorBead => {
                self.color_menu_open = !self.color_menu_open;
                self.backdrop_key.clear();
                self.layout_edit();
                InvalidateRect(self.hwnd, null(), 0);
            }
            HitTarget::DragBar | HitTarget::Scrollbar => {}
            HitTarget::New => {
                self.flush_edit();
                self.emit_create_new();
            }
            HitTarget::Close => {
                PostMessageW(self.hwnd, WM_CLOSE, 0, 0);
            }
            HitTarget::Swatch(i) => {
                if let Some(color) = model::COLORS.get(i) {
                    if self.note.color != *color {
                        self.note.color = (*color).into();
                        self.note.updated_at = model::now();
                        self.backdrop_key.clear();
                        self.emit_note();
                    }
                    self.color_menu_open = false;
                    self.backdrop_key.clear();
                    self.layout_edit();
                    InvalidateRect(self.hwnd, null(), 0);
                }
            }
            HitTarget::Format(i) => {
                self.format(i);
                InvalidateRect(self.hwnd, null(), 0);
            }
            HitTarget::ImageDelete => {
                self.delete_selected_image();
            }
            HitTarget::Image(i) => {
                self.selected_image = Some(i);
                SetFocus(self.hwnd);
                InvalidateRect(self.hwnd, null(), 0);
            }
            HitTarget::Body => {
                self.selected_image = None;
                SetFocus(self.edit);
                InvalidateRect(self.hwnd, null(), 0);
            }
        }
    }

    unsafe fn arm_scroll(&mut self, now: f64) {
        if self.scroll.moving(now) {
            if self.scroll_clock.is_none() {
                self.scroll_clock = Some(crate::motion::FrameClock::start());
            }
            SetTimer(
                self.hwnd,
                ANIMATE_TIMER,
                crate::motion::frame_interval_ms(),
                None,
            );
            if !self.frame_queued {
                InvalidateRect(self.hwnd, null(), 0);
            }
        }
    }

    unsafe fn paint(&mut self) {
        let frame_started = std::time::Instant::now();
        let mut ps: PAINTSTRUCT = zeroed();
        BeginPaint(self.hwnd, &mut ps);
        let mut r: RECT = zeroed();
        GetClientRect(self.hwnd, &mut r);
        let Some(mut frame) = self
            .frame
            .take()
            .filter(|f| f.matches_geometry(r.right, r.bottom, self.dpi, RADIUS))
            .or_else(|| Buffer::new(r.right, r.bottom, self.dpi, RADIUS))
        else {
            EndPaint(self.hwnd, &ps);
            return;
        };

        let [w, _h] = self.dimensions();
        let card = self.card_rect();
        let accent = super::State::color(&self.note);
        let body_ink = rgb(242, 244, 247);
        let muted_ink = rgb(196, 200, 208);

        let bg_key = format!(
            "{}:{}:{}:{}:{}",
            r.right,
            r.bottom,
            self.dpi.to_bits(),
            accent,
            self.color_menu_open
        );
        let bg_cached = self
            .backdrop
            .as_ref()
            .is_some_and(|b| b.width == r.right && b.height == r.bottom && b.dpi == self.dpi)
            && self.backdrop_key == bg_key;

        if !bg_cached {
            let Some(bg) = self
                .backdrop
                .take()
                .filter(|b| b.width == r.right && b.height == r.bottom && b.dpi == self.dpi)
                .or_else(|| Buffer::plain(r.right, r.bottom, self.dpi))
            else {
                EndPaint(self.hwnd, &ps);
                return;
            };
            // Opaque dark base + widget card tint (78/255) + Lumen glass_surface + coating.
            let base_brush = CreateSolidBrush(rgb(36, 40, 46));
            FillRect(bg.dc, &r, base_brush);
            DeleteObject(base_brush);
            {
                let c = Canvas::new(bg.dc, self.dpi);
                c.round(card, RADIUS, accent, 78, false, false);
                c.round(
                    [0., 0., w, card[1] + HEADER_H],
                    0.,
                    rgb(0, 0, 0),
                    26,
                    false,
                    false,
                );
                c.line(
                    &[
                        (card[0] + 1., card[1] + HEADER_H),
                        (card[2] - 1., card[1] + HEADER_H),
                    ],
                    0xffffff,
                    20,
                    1.,
                );
                c.glass_surface(card, RADIUS, 20, false);
                c.coating(card, RADIUS);

                if self.color_menu_open {
                    let bar = [card[0], card[1] + HEADER_H, card[2], card[1] + HEADER_H + COLOR_BAR_H];
                    c.round(bar, 0., rgb(18, 20, 24), 125, false, false);
                    c.line(&[(bar[0], bar[3]), (bar[2], bar[3])], 0xffffff, 20, 1.);
                }
            }
            crate::render::flush(bg.dc);
            self.backdrop = Some(bg);
            self.backdrop_key = bg_key;
        }

        frame.copy_from(self.backdrop.as_ref().unwrap().dc);

        let now = self.now();
        let scroll_y = self.scroll.value(now);
        self.apply_scroll_position(scroll_y);

        let (text_rect, img_area, img_rects, _) = self.visible_rects_at(scroll_y);
        let new_btn = self.new_btn_rect();
        let close_btn = self.close_btn_rect();
        let color_bar = self.color_bar_rect();
        let scrolling = self.scroll.moving(now);

        {
            let c = Canvas::new(frame.dc, self.dpi);

            // Top status bar: color bead, subtle centered drag handle, +, and x.
            icons::bead(&c, card[0] + 14., card[1] + 10., accent);
            if self.hover == Some(HitTarget::ColorBead) || self.color_menu_open {
                c.round(
                    [card[0] + 11., card[1] + 7., card[0] + 33., card[1] + 29.],
                    11.,
                    0xffffff,
                    if self.color_menu_open { 200 } else { 115 },
                    true,
                    false,
                );
            }

            let mid_x = (card[0] + card[2]) * 0.5;
            let mid_y = card[1] + HEADER_H * 0.5;
            if mid_x - 16. > card[0] + 42. && mid_x + 16. < card[2] - 72. {
                let drag_hot = self.drag_move.is_some()
                    || self.hover == Some(HitTarget::DragBar);
                c.round(
                    [mid_x - 16., mid_y - 1.5, mid_x + 16., mid_y + 1.5],
                    1.5,
                    0xffffff,
                    if drag_hot { 96 } else { 44 },
                    false,
                    false,
                );
            }

            for (target, br) in [
                (HitTarget::New, new_btn),
                (HitTarget::Close, close_btn),
            ] {
                let is_hover = self.hover == Some(target);
                let is_down = is_hover && self.pressed == Some(target);
                if is_hover {
                    c.round(
                        br,
                        7.,
                        0xffffff,
                        if is_down { 42 } else { 24 },
                        false,
                        false,
                    );
                }
            }

            let nx = (new_btn[0] + new_btn[2]) * 0.5;
            let ny = (new_btn[1] + new_btn[3]) * 0.5;
            c.line(&[(nx - 5., ny), (nx + 5., ny)], body_ink, 210, 1.35);
            c.line(&[(nx, ny - 5.), (nx, ny + 5.)], body_ink, 210, 1.35);

            let cx = (close_btn[0] + close_btn[2]) * 0.5;
            let cy = (close_btn[1] + close_btn[3]) * 0.5;
            c.line(
                &[(cx - 4.2, cy - 4.2), (cx + 4.2, cy + 4.2)],
                body_ink,
                210,
                1.35,
            );
            c.line(
                &[(cx + 4.2, cy - 4.2), (cx - 4.2, cy + 4.2)],
                body_ink,
                210,
                1.35,
            );

            // Color picker beads
            if let Some(bar) = color_bar {
                for (i, name) in model::COLORS.iter().enumerate() {
                    let sr = Self::swatch_rect(bar, i);
                    let mut dummy = self.note.clone();
                    dummy.color = (*name).into();
                    let swatch_color = super::State::color(&dummy);
                    icons::bead(&c, sr[0], sr[1], swatch_color);
                    let selected = self.note.color == *name;
                    let hovered = self.hover == Some(HitTarget::Swatch(i));
                    if selected || hovered {
                        c.round(
                            [sr[0] - 3., sr[1] - 3., sr[2] + 3., sr[3] + 3.],
                            11.,
                            0xffffff,
                            if selected { 235 } else { 130 },
                            true,
                            false,
                        );
                    }
                }
            }

            // Footer matching widget card editor (day label + right-aligned format pills)
            let count = editor::TOOLBAR_LABELS.len() as f32;
            let bw = ((card[2] - card[0] - 24.) / count).min(25.);
            let left = card[2] - 12. - bw * count;
            if left - card[0] > 54. {
                c.text_alpha(
                    &icons::note_day(self.note.updated_at),
                    [card[0] + 14., card[3] - 26., left - 4., card[3] - 7.],
                    11.,
                    false,
                    muted_ink,
                    0,
                    185,
                );
            }
            let active = editor::active_edit_formats(self.edit);
            for (i, label) in editor::TOOLBAR_LABELS.iter().enumerate() {
                let btn = self.format_btn_rect(i);
                let is_hover = self.hover == Some(HitTarget::Format(i));
                let is_down = is_hover && self.pressed == Some(HitTarget::Format(i));
                let is_active = active[i];
                if is_active {
                    c.round(
                        btn,
                        7.,
                        body_ink,
                        if is_hover { 60 } else { 44 },
                        false,
                        false,
                    );
                    c.hover_surface(btn, 7., if is_hover { 0.95 } else { 0.78 }, false);
                    c.round(
                        btn,
                        7.,
                        body_ink,
                        if is_hover { 195 } else { 145 },
                        true,
                        false,
                    );
                } else if is_hover {
                    c.round(
                        btn,
                        7.,
                        body_ink,
                        if is_down { 38 } else { 24 },
                        false,
                        false,
                    );
                    c.hover_surface(btn, 7., if is_down { 0.85 } else { 0.72 }, false);
                } else {
                    c.hover_surface(btn, 7., 0.25, false);
                }
                c.text_alpha(
                    label,
                    [btn[0], btn[1] + 2., btn[2], btn[3] - 1.],
                    12.,
                    i < 4 || is_active,
                    body_ink,
                    1,
                    if is_active || is_hover { 255 } else { 178 },
                );
            }
        }

        // RichEdit text snapshot + caret
        let tw_px = ((text_rect[2] - text_rect[0]) * self.dpi).round() as i32;
        let th_px = ((text_rect[3] - text_rect[1]) * self.dpi).round() as i32;
        if tw_px > 0 && th_px > 0 && !self.edit.is_null() {
            if self
                .snapshot
                .as_ref()
                .is_none_or(|(ink, sw, sh, _)| *ink != body_ink || *sw != tw_px || *sh != th_px)
            {
                self.snapshot = editor::print_richedit_to_image(self.edit, tw_px, th_px, body_ink)
                    .map(|img| (body_ink, tw_px, th_px, img));
            }
            let c = Canvas::new(frame.dc, self.dpi);
            if let Some((_, _, _, image)) = &self.snapshot {
                let px = (text_rect[0] * self.dpi).round();
                let py = (text_rect[1] * self.dpi).round();
                c.image(
                    image,
                    [
                        px / self.dpi,
                        py / self.dpi,
                        (px + tw_px as f32) / self.dpi,
                        (py + th_px as f32) / self.dpi,
                    ],
                );
            }
            if GetFocus() == self.edit && self.caret_on {
                editor::draw_richedit_caret(
                    &c,
                    self.edit,
                    [text_rect[0], text_rect[1]],
                    text_rect,
                    LEADING,
                    self.dpi,
                    body_ink,
                );
            }
        }

        // Attached images clipped to content bounds
        if !self.note.images.is_empty() && img_area[3] > img_area[1] {
            let saved = SaveDC(frame.dc);
            IntersectClipRect(
                frame.dc,
                (img_area[0] * self.dpi).floor() as i32,
                (img_area[1] * self.dpi).floor() as i32,
                (img_area[2] * self.dpi).ceil() as i32,
                (img_area[3] * self.dpi).ceil() as i32,
            );
            let c = Canvas::new(frame.dc, self.dpi);
            for (idx, (name, ir)) in self.note.images.iter().zip(img_rects.iter()).enumerate() {
                if ir[2] <= ir[0] || ir[3] <= ir[1] || ir[3] < img_area[1] || ir[1] > img_area[3] {
                    continue;
                }
                if let Some((_, _, img)) = self.image_cache.get(name) {
                    c.image(img, *ir);
                }
                if self.selected_image == Some(idx) {
                    c.round(
                        [ir[0] - 1., ir[1] - 1., ir[2] + 1., ir[3] + 1.],
                        6.,
                        rgb(96, 165, 250),
                        0,
                        false,
                        true,
                    );
                    let dr = image_delete_rect(*ir);
                    c.round(dr, 10., rgb(24, 24, 28), 210, false, true);
                    c.text("×", dr, 13., true, rgb(255, 255, 255), 1);
                }
            }
            drop(c);
            RestoreDC(frame.dc, saved);
        }

        // Software SDF scrollbar (zero GDI+ context overhead on scroll)
        if let Some(bar) = self.scrollbar_rect(scroll_y) {
            let alpha = if self.scrollbar_drag.is_some() || self.scrollbar_hover {
                175
            } else if scrolling {
                135
            } else {
                95
            };
            frame.fill_round(bar, 2., body_ink, alpha);
        }

        frame.present_with_alpha(self.hwnd, false);
        self.frame = Some(frame);
        EndPaint(self.hwnd, &ps);

        if self.resizing {
            if let Some(shadow) = &self.shadow {
                shadow.hide();
            }
        } else if self.drag_move.is_none() {
            if let Some(shadow) = &mut self.shadow {
                shadow.sync_with_radius(self.hwnd, self.dpi, 255, RADIUS);
            }
        }

        if IsWindowVisible(self.hwnd) != 0 && scrolling && self.drag_move.is_none() {
            crate::motion::pace_vsync(
                self.hwnd,
                frame_started,
                &mut self.frame_queued,
                crate::motion::VSYNC_FRAME,
            );
        }
    }
}

unsafe fn step_drag_move(hwnd: HWND, ptr: *const RefCell<PopoutState>) -> bool {
    let Ok(mut s) = (*ptr).try_borrow_mut() else {
        return false;
    };
    let Some(d) = s.drag_move.as_mut() else {
        return false;
    };
    let mut cur: POINT = zeroed();
    if GetCursorPos(&mut cur) == 0 {
        return false;
    }
    let nx = d.start_pos[0] + (cur.x - d.start_cursor.x);
    let ny = d.start_pos[1] + (cur.y - d.start_cursor.y);
    if [nx, ny] == d.applied_pos {
        return false;
    }
    d.applied_pos = [nx, ny];
    let dpi = s.dpi;
    let mut shadow = s.shadow.take();
    // Release RefMut before EndDeferWindowPos so synchronous WM_WINDOWPOSCHANGING /
    // WM_DPICHANGED callbacks can borrow PopoutState safely.
    drop(s);
    if let Some(sh) = shadow.as_mut() {
        sh.move_with_owner(hwnd, nx, ny, dpi);
    } else {
        SetWindowPos(
            hwnd,
            null_mut(),
            nx,
            ny,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
    if let Ok(mut s) = (*ptr).try_borrow_mut() {
        if s.shadow.is_none() {
            s.shadow = shadow;
        }
    }
    true
}

unsafe fn finish_drag_move(hwnd: HWND, ptr: *const RefCell<PopoutState>) {
    let _ = step_drag_move(hwnd, ptr);
    let Ok(mut s) = (*ptr).try_borrow_mut() else {
        return;
    };
    if s.drag_move.take().is_none() {
        return;
    }
    s.drag_frame_queued = false;
    s.drag_dirty = false;
    KillTimer(hwnd, DRAG_TIMER);
    if !s.scroll.moving(s.now()) {
        s.scroll_clock = None;
    }
    if GetFocus() == s.edit {
        SetTimer(hwnd, CARET_TIMER, editor::blink_ms(), None);
    }
    let dpi = s.dpi;
    if let Some(shadow) = &mut s.shadow {
        shadow.sync_with_radius(hwnd, dpi, 255, RADIUS);
    }
    s.emit_bounds(true);
    InvalidateRect(hwnd, null(), 0);
}

unsafe extern "system" fn input_host_proc(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
) -> LRESULT {
    if msg == WM_PAINT {
        ValidateRect(hwnd, null());
        return 0;
    }
    if msg == WM_ERASEBKGND {
        return 1;
    }
    if msg == WM_COMMAND {
        let parent = GetParent(hwnd);
        if !parent.is_null() {
            return SendMessageW(parent, msg, w, l);
        }
    }
    DefWindowProcW(hwnd, msg, w, l)
}

unsafe extern "system" fn popout_input_proc(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    let popout = data as HWND;
    if msg == WM_SETFOCUS {
        SetTimer(popout, CARET_TIMER, editor::blink_ms(), None);
    }
    if msg == WM_KILLFOCUS {
        KillTimer(popout, CARET_TIMER);
    }
    if msg == WM_MOUSEWHEEL && !popout.is_null() {
        return SendMessageW(popout, WM_MOUSEWHEEL, w, l);
    }
    if !editor::is_printing() {
        let key_scroll = matches!(
            msg,
            WM_KEYDOWN | WM_CHAR | WM_IME_COMPOSITION | WM_IME_ENDCOMPOSITION
        );
        if key_scroll
            || matches!(
                msg,
                WM_PAINT
                    | WM_KEYUP
                    | WM_LBUTTONDOWN
                    | WM_LBUTTONUP
                    | WM_LBUTTONDBLCLK
                    | WM_RBUTTONDOWN
                    | WM_VSCROLL
                    | WM_SETFOCUS
                    | WM_KILLFOCUS
            )
            || (msg == WM_MOUSEMOVE && (w & 1) != 0)
        {
            PostMessageW(popout, POPOUT_REDRAW, if key_scroll { 1 } else { 0 }, 0);
        }
    }
    if msg == WM_KEYDOWN {
        let ctrl = GetKeyState(0x11) < 0;
        let shift = GetKeyState(0x10) < 0;
        let alt = GetKeyState(0x12) < 0;
        if (ctrl && !alt && !shift && w == b'V' as usize)
            || (shift && !ctrl && !alt && w == 0x2D)
        {
            return editor::paste_clipboard_or_notify_image(hwnd, popout, POPOUT_PASTE_IMAGE);
        }
    }
    if msg == WM_CHAR && w == 0x16 {
        return 0;
    }
    if msg == WM_PASTE {
        return editor::paste_clipboard_or_notify_image(hwnd, popout, POPOUT_PASTE_IMAGE);
    }
    if msg == WM_NCDESTROY {
        RemoveWindowSubclass(hwnd, Some(popout_input_proc), 1);
    }
    DefSubclassProc(hwnd, msg, w, l)
}

unsafe extern "system" fn popout_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
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
        crate::gpu::remove_surface(hwnd);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        return DefWindowProcW(hwnd, msg, w, l);
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<PopoutState>;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, w, l);
    }
    let Ok(mut s) = (*ptr).try_borrow_mut() else {
        return DefWindowProcW(hwnd, msg, w, l);
    };
    match msg {
        WM_DESTROY => {
            KillTimer(hwnd, CARET_TIMER);
            KillTimer(hwnd, ANIMATE_TIMER);
            KillTimer(hwnd, DRAG_TIMER);
            s.scroll_clock = None;
            s.shadow.take();
            if !s.font.is_null() {
                DeleteObject(s.font);
                s.font = null_mut();
            }
            0
        }
        WM_CLOSE => {
            s.closing = true;
            s.flush_edit();
            let id = s.note.id.clone();
            let bounds = s.current_bounds(false);
            drop(s);
            finish_close(&id, bounds);
            0
        }
        WM_PAINT => {
            s.paint();
            0
        }
        WM_COMMAND if (w >> 16) as u32 == EN_CHANGE => {
            if !s.loading {
                s.flush_edit();
                s.sync_scroll_to_caret();
                s.snapshot = None;
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_NCHITTEST => {
            let mut p = POINT {
                x: point(l)[0] as i32,
                y: point(l)[1] as i32,
            };
            ScreenToClient(hwnd, &mut p);
            let size = s.dimensions();
            let pt = [p.x as f32 / s.dpi, p.y as f32 / s.dpi];
            let edge = crate::geometry::edge(
                pt[0].floor() as i32,
                pt[1].floor() as i32,
                size[0].round() as i32,
                size[1].round() as i32,
                9,
                18,
            );
            let in_header_btn = inside(pt, s.bead_btn_rect())
                || inside(pt, s.new_btn_rect())
                || inside(pt, s.close_btn_rect());
            if in_header_btn
                && pt[0] > INSET + 2.
                && pt[0] < size[0] - INSET - 2.
                && pt[1] > INSET + 2.
            {
                return HTCLIENT as isize;
            }
            if edge != HTCLIENT {
                return edge as isize;
            }
            HTCLIENT as isize
        }
        WM_GETMINMAXINFO => {
            let m = &mut *(l as *mut MINMAXINFO);
            m.ptMinTrackSize = POINT {
                x: (model::POPOUT_MIN_W as f32 * s.dpi) as i32,
                y: (model::POPOUT_MIN_H as f32 * s.dpi) as i32,
            };
            m.ptMaxTrackSize = POINT {
                x: (1600. * s.dpi) as i32,
                y: (1200. * s.dpi) as i32,
            };
            0
        }
        WM_LBUTTONDOWN => {
            let p = point(l).map(|v| v / s.dpi);
            s.scrollbar_drag = None;
            let hit = s.hit(p);
            s.pressed = hit;
            if hit == Some(HitTarget::DragBar) {
                let mut cur: POINT = zeroed();
                GetCursorPos(&mut cur);
                let mut wr: RECT = zeroed();
                GetWindowRect(hwnd, &mut wr);
                s.drag_move = Some(DragMove {
                    start_cursor: cur,
                    start_pos: [wr.left, wr.top],
                    applied_pos: [wr.left, wr.top],
                });
                s.drag_frame_queued = false;
                s.drag_dirty = false;
                KillTimer(hwnd, CARET_TIMER);
                if s.scroll_clock.is_none() {
                    s.scroll_clock = Some(crate::motion::FrameClock::start());
                }
                SetCapture(hwnd);
                InvalidateRect(hwnd, null(), 0);
                return 0;
            }
            if hit == Some(HitTarget::Scrollbar) {
                let scroll_y = s.scroll.value(s.now());
                if let Some(bar) = s.scrollbar_rect(scroll_y) {
                    s.scrollbar_drag = Some(p[1] - bar[1]);
                }
                SetCapture(hwnd);
                InvalidateRect(hwnd, null(), 0);
                return 0;
            }
            // Clicking along the scrollbar track outside the thumb jumps immediately.
            let c = s.card_rect();
            let cb = s.content_bounds();
            if s.scroll.max > 0. && p[0] >= c[2] - 14. && p[0] <= c[2] && p[1] >= cb[1] && p[1] <= cb[3]
            {
                let track_h = (cb[3] - cb[1] - 4.).max(18.);
                let viewport_h = (cb[3] - cb[1]).max(LEADING);
                let thumb_h =
                    (track_h * viewport_h / (viewport_h + s.scroll.max)).clamp(20., track_h);
                s.scroll_drag_to(p[1], thumb_h * 0.5);
                s.scrollbar_drag = Some(thumb_h * 0.5);
                SetCapture(hwnd);
                InvalidateRect(hwnd, null(), 0);
                return 0;
            }
            SetCapture(hwnd);
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_LBUTTONUP => {
            ReleaseCapture();
            if s.drag_move.is_some() {
                s.pressed = None;
                drop(s);
                finish_drag_move(hwnd, ptr);
                return 0;
            }
            if s.scrollbar_drag.take().is_some() {
                s.pressed = None;
                InvalidateRect(hwnd, null(), 0);
                return 0;
            }
            let p = point(l).map(|v| v / s.dpi);
            let pressed = s.pressed.take();
            let hit = s.hit(p);
            if pressed.is_some() && pressed == hit {
                s.handle_action(pressed.unwrap());
            } else {
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_MOUSEMOVE => {
            if s.drag_move.is_some() {
                if s.drag_frame_queued {
                    s.drag_dirty = true;
                    return 0;
                }
                let started = std::time::Instant::now();
                s.drag_dirty = false;
                drop(s);
                let _ = step_drag_move(hwnd, ptr);
                if let Ok(mut s) = (*ptr).try_borrow_mut() {
                    if s.drag_move.is_some() {
                        SetTimer(hwnd, DRAG_TIMER, crate::motion::frame_interval_ms(), None);
                        crate::motion::pace_vsync(
                            hwnd,
                            started,
                            &mut s.drag_frame_queued,
                            crate::motion::VSYNC_FRAME,
                        );
                    }
                }
                return 0;
            }
            let p = point(l).map(|v| v / s.dpi);
            if let Some(grab) = s.scrollbar_drag {
                s.scroll_drag_to(p[1], grab);
                InvalidateRect(hwnd, null(), 0);
                return 0;
            }
            let hover = s.hit(p);
            let sb_hover = hover == Some(HitTarget::Scrollbar);
            if hover != s.hover || sb_hover != s.scrollbar_hover {
                s.hover = hover;
                s.scrollbar_hover = sb_hover;
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
            if s.hover.take().is_some() || s.scrollbar_hover {
                s.scrollbar_hover = false;
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_CAPTURECHANGED | WM_CANCELMODE => {
            s.pressed = None;
            s.scrollbar_drag = None;
            if s.drag_move.is_some() {
                drop(s);
                finish_drag_move(hwnd, ptr);
                if msg == WM_CANCELMODE {
                    ReleaseCapture();
                    return DefWindowProcW(hwnd, msg, w, l);
                }
                return 0;
            }
            if s.moving || s.resizing {
                let st = &mut *s;
                crate::widget_interaction::exit_size_move(hwnd, &mut st.moving, &mut st.resizing);
            }
            if msg == WM_CANCELMODE {
                ReleaseCapture();
                InvalidateRect(hwnd, null(), 0);
                drop(s);
                return DefWindowProcW(hwnd, msg, w, l);
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_MOUSEWHEEL => {
            if s.scroll.max > 0. {
                let now = s.now();
                let delta = -((w >> 16) as u16 as i16) as f32 / 120. * 72.;
                s.scroll.by(delta, now);
                s.arm_scroll(now);
            }
            0
        }
        WM_TIMER if w == ANIMATE_TIMER => {
            if !s.frame_queued {
                InvalidateRect(hwnd, null(), 0);
            }
            if !s.scroll.moving(s.now()) {
                KillTimer(hwnd, ANIMATE_TIMER);
                if s.drag_move.is_none() {
                    s.scroll_clock = None;
                }
            }
            0
        }
        WM_TIMER if w == DRAG_TIMER => {
            if s.drag_move.is_some() && !s.drag_frame_queued && s.drag_dirty {
                s.drag_dirty = false;
                drop(s);
                let _ = step_drag_move(hwnd, ptr);
            } else if s.drag_move.is_none() {
                KillTimer(hwnd, DRAG_TIMER);
            }
            0
        }
        m if m == crate::motion::VSYNC_FRAME => {
            if s.drag_move.is_some() {
                s.drag_frame_queued = false;
                if s.drag_dirty {
                    let started = std::time::Instant::now();
                    s.drag_dirty = false;
                    drop(s);
                    let _ = step_drag_move(hwnd, ptr);
                    if let Ok(mut s) = (*ptr).try_borrow_mut() {
                        if s.drag_move.is_some() {
                            crate::motion::pace_vsync(
                                hwnd,
                                started,
                                &mut s.drag_frame_queued,
                                crate::motion::VSYNC_FRAME,
                            );
                        }
                    }
                }
                return 0;
            }
            s.frame_queued = false;
            if s.scroll.moving(s.now()) {
                InvalidateRect(hwnd, null(), 0);
            } else {
                KillTimer(hwnd, ANIMATE_TIMER);
                s.scroll_clock = None;
            }
            0
        }
        WM_KEYDOWN => {
            if (w == VK_DELETE as usize || w == VK_BACK as usize) && s.selected_image.is_some() {
                s.delete_selected_image();
                return 0;
            }
            if w == VK_ESCAPE as usize && s.selected_image.take().is_some() {
                InvalidateRect(hwnd, null(), 0);
                return 0;
            }
            drop(s);
            DefWindowProcW(hwnd, msg, w, l)
        }
        WM_DROPFILES => {
            s.drop_files(w as windows_sys::Win32::UI::Shell::HDROP);
            0
        }
        POPOUT_PASTE_IMAGE => {
            s.paste_clipboard_image();
            0
        }
        POPOUT_REDRAW => {
            if s.drag_move.is_some() {
                return 0;
            }
            s.snapshot = None;
            s.caret_on = true;
            if GetFocus() == s.edit {
                SetTimer(hwnd, CARET_TIMER, editor::blink_ms(), None);
            }
            if w != 0 && !s.scroll.moving(s.now()) {
                s.sync_scroll_to_caret();
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_TIMER if w == CARET_TIMER => {
            if s.drag_move.is_some() {
                return 0;
            }
            s.caret_on = !s.caret_on;
            InvalidateRect(hwnd, null(), 0);
            0
        }
        KEY => {
            drop(s);
            handle_key(hwnd, w);
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
            DefWindowProcW(hwnd, msg, w, l)
        }
        WM_EXITSIZEMOVE => {
            {
                let s = &mut *s;
                crate::widget_interaction::exit_size_move(hwnd, &mut s.moving, &mut s.resizing);
            }
            s.emit_bounds(true);
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_SIZE => {
            let width = (l as u32 & 0xffff) as i32;
            let height = ((l as u32 >> 16) & 0xffff) as i32;
            if s.moving {
                let s = &mut *s;
                crate::widget_interaction::start_resize(hwnd, &mut s.resizing, s.shadow.as_ref());
            }
            if width > 0 && height > 0 {
                s.backdrop_key.clear();
                s.layout_edit();
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_WINDOWPOSCHANGING => {
            if s.drag_move.is_none() {
                let dpi = s.dpi;
                let resizing = s.resizing;
                crate::widget_interaction::on_window_pos_changed(
                    &*(l as *const WINDOWPOS),
                    resizing,
                    &mut s.shadow,
                    dpi,
                );
            }
            drop(s);
            DefWindowProcW(hwnd, msg, w, l)
        }
        WM_DPICHANGED => {
            let new_dpi = ((w as u16) as f32 / 96.).max(1.);
            s.dpi = new_dpi;
            s.backdrop_key.clear();
            editor::set_edit_zoom(s.edit, new_dpi, s.initial_dpi);
            editor::apply_edit_line_spacing(s.edit, new_dpi, LEADING, true);
            s.snapshot = None;
            let r = crate::geometry::clamp_dpi_rect(
                *(l as *const RECT),
                [model::POPOUT_MIN_W, model::POPOUT_MIN_H],
                new_dpi,
            );
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
        WM_ACTIVATE if (w as u16) as u32 == WA_INACTIVE => {
            if !s.closing {
                s.flush_edit();
                s.emit_bounds(true);
            }
            0
        }
        _ => {
            drop(s);
            DefWindowProcW(hwnd, msg, w, l)
        }
    }
}

#[cfg(test)]
pub(super) unsafe fn test_paint_and_inspect(
    id: &str,
) -> Option<(f32, f32, Option<[f32; 4]>, i32, [u8; 4], bool)> {
    MANAGER.with(|m| {
        let m = m.borrow();
        let w = m.windows.get(id)?;
        let mut s = w.state.borrow_mut();
        s.layout_edit();
        s.paint();
        let now = s.now();
        let val = s.scroll.value(now);
        let max = s.scroll.max;
        let bar = s.scrollbar_rect(val);
        let mut pt = POINT { x: 0, y: 0 };
        SendMessageW(s.edit, EM_GETSCROLLPOS, 0, &mut pt as *mut _ as isize);
        let frame = s.frame.as_ref()?;
        let px = frame.snapshot();
        let cx = (frame.width / 2) as usize;
        let cy = (frame.height / 2) as usize;
        let idx = (cy * frame.width as usize + cx) * 4;
        let bgra = [px[idx], px[idx + 1], px[idx + 2], px[idx + 3]];
        let dragging = s.drag_move.is_some();
        Some((val, max, bar, pt.y, bgra, dragging))
    })
}

#[cfg(test)]
pub(super) fn test_is_dragging(id: &str) -> bool {
    MANAGER.with(|m| {
        m.borrow()
            .windows
            .get(id)
            .is_some_and(|w| w.state.borrow().drag_move.is_some())
    })
}

#[cfg(test)]
pub(super) unsafe fn test_scroll_by(id: &str, delta: f32) {
    MANAGER.with(|m| {
        let m = m.borrow();
        if let Some(w) = m.windows.get(id) {
            let mut s = w.state.borrow_mut();
            let now = s.now();
            let target = (s.scroll.value(now) + delta).clamp(0., s.scroll.max);
            s.scroll.snap(target);
            s.paint();
        }
    });
}

#[cfg(test)]
pub(super) fn test_edit_hwnd(id: &str) -> Option<HWND> {
    MANAGER.with(|m| m.borrow().windows.get(id).map(|w| w.state.borrow().edit))
}
