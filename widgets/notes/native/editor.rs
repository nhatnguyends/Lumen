//! A real RichEdit control owns selection, clipboard, undo and IME composition.
//! Child controls stay inside the card. Their glyphs/selection are composited
//! onto the glass; native controls retain focus, selection, clipboard and IME.
use super::*;
use windows_sys::Win32::System::LibraryLoader::LoadLibraryW;
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};

pub enum Event {
    Note(model::Note),
    Search(String),
    Closed,
}
pub type Events = Rc<RefCell<Vec<Event>>>;
pub struct Editor {
    state: Box<RefCell<State>>,
}
struct State {
    hwnd: HWND,
    edit: HWND,
    board: HWND,
    root: PathBuf,
    events: Events,
    note: Option<model::Note>,
    query: String,
    font: HFONT,
    dpi: f32,
    initial_dpi: f32,
    body_size: f32,
    leading: f32,
    pressed: Option<usize>,
    hover: Option<usize>,
    loading: bool,
    changed: bool,
    rect: [f32; 4],
    text_rect: [f32; 4],
    snapshot: Option<(u32, crate::render::Image)>,
    caret_on: bool,
}
const CLASS: &str = "LumenNativeNotesEditor";
const LABELS: [&str; 6] = ["B", "I", "U", "S", "•", "1."];
const CARET: usize = 110;
const PASTE_IMAGE: u32 = WM_APP + 106;
unsafe fn blink_ms() -> u32 {
    let ms = GetCaretBlinkTime();
    if ms == 0 || ms > 2000 {
        530
    } else {
        ms.max(100)
    }
}
thread_local! {static PRINTING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };}
#[repr(C, packed(4))]
struct Stream {
    cookie: usize,
    error: u32,
    callback: Option<unsafe extern "system" fn(usize, *mut u8, i32, *mut i32) -> u32>,
}
struct Bytes {
    data: Vec<u8>,
    offset: usize,
}
unsafe extern "system" fn read(cookie: usize, p: *mut u8, len: i32, done: *mut i32) -> u32 {
    let b = &mut *(cookie as *mut Bytes);
    let n = (len.max(0) as usize).min(b.data.len() - b.offset);
    std::ptr::copy_nonoverlapping(b.data[b.offset..].as_ptr(), p, n);
    b.offset += n;
    *done = n as i32;
    0
}
unsafe extern "system" fn write(cookie: usize, p: *mut u8, len: i32, done: *mut i32) -> u32 {
    let b = &mut *(cookie as *mut Bytes);
    if len < 0 || b.data.len() + len as usize > 8_000_000 {
        *done = 0;
        return 1;
    }
    b.data
        .extend_from_slice(std::slice::from_raw_parts(p, len as usize));
    *done = len;
    0
}
#[repr(C)]
struct CharFormat {
    size: u32,
    mask: u32,
    effects: u32,
    height: i32,
    offset: i32,
    color: u32,
    charset: u8,
    pitch: u8,
    face: [u16; 32],
}
#[repr(C)]
struct ParaFormat {
    size: u32,
    mask: u32,
    numbering: u16,
    reserved: u16,
    start: i32,
    right: i32,
    offset: i32,
    alignment: u16,
    tab_count: i16,
    tabs: [i32; 32],
}
#[repr(C)]
struct ParaFormat2 {
    base: ParaFormat,
    space_before: i32,
    space_after: i32,
    line_spacing: i32,
    style: i16,
    line_spacing_rule: u8,
    outline_level: u8,
    shading_weight: u16,
    shading_style: u16,
    numbering_start: u16,
    numbering_style: u16,
    numbering_tab: u16,
    border_space: u16,
    border_width: u16,
    borders: u16,
}
impl Editor {
    pub unsafe fn create(
        board: HWND,
        root: PathBuf,
        events: Events,
        note: Option<model::Note>,
        query: String,
        _floating: bool,
    ) -> Result<Self, String> {
        static RICH_EDIT: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
        let module =
            *RICH_EDIT.get_or_init(|| LoadLibraryW(wide("Msftedit.dll").as_ptr()) as usize);
        if module == 0 {
            return Err("Windows RichEdit could not be loaded".into());
        }
        // Msftedit remains loaded for the lifetime of this process (window class owner).
        register(CLASS, Some(proc));
        let mut state = Box::new(RefCell::new(State {
            hwnd: null_mut(),
            edit: null_mut(),
            board,
            root,
            events,
            note,
            query,
            font: null_mut(),
            dpi: GetDpiForWindow(board) as f32 / 96.,
            initial_dpi: GetDpiForWindow(board) as f32 / 96.,
            body_size: 14.,
            leading: 20.,
            pressed: None,
            hover: None,
            loading: true,
            changed: false,
            rect: [0.; 4],
            text_rect: [0.; 4],
            snapshot: None,
            caret_on: true,
        }));
        let search = state.borrow().note.is_none();
        let hwnd = CreateWindowExW(
            WS_EX_TRANSPARENT,
            wide(CLASS).as_ptr(),
            wide("").as_ptr(),
            WS_CHILD,
            0,
            0,
            1,
            1,
            board,
            null_mut(),
            GetModuleHandleW(null()),
            state.as_mut() as *mut _ as _,
        );
        if hwnd.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        {
            let mut s = state.borrow_mut();
            s.hwnd = hwnd;
            s.edit = CreateWindowExW(
                0,
                wide("RICHEDIT50W").as_ptr(),
                wide("").as_ptr(),
                WS_CHILD
                    | WS_VISIBLE
                    | WS_TABSTOP
                    | ES_AUTOVSCROLL as u32
                    | if search {
                        ES_AUTOHSCROLL as u32
                    } else {
                        ES_MULTILINE as u32 | ES_WANTRETURN as u32
                    },
                0,
                0,
                100,
                100,
                hwnd,
                100usize as _,
                GetModuleHandleW(null()),
                null(),
            );
            if s.edit.is_null() {
                DestroyWindow(hwnd);
                return Err("Cannot create native notes editor".into());
            }
            SetWindowSubclass(s.edit, Some(input_proc), 1, board as usize);
            let dpi = s.dpi;
            s.font = CreateFontW(
                -(14.0 * dpi).round() as i32,
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
            SendMessageW(s.edit, WM_SETFONT, s.font as usize, 1);
            let left_margin = if search {
                0
            } else {
                (2. * dpi).round().max(1.) as u16
            };
            SendMessageW(s.edit, 0x00D3, 3, left_margin as isize);
            SendMessageW(s.edit, WM_USER + 67, 0, rgb(245, 242, 233) as isize);
            SendMessageW(s.edit, WM_USER + 69, 0, 1); // ENM_CHANGE
            SendMessageW(
                s.edit,
                windows_sys::Win32::UI::Controls::EM_LIMITTEXT,
                200_000,
                0,
            );
            if let Some(note) = &s.note {
                let rtf = note
                    .rich_text()
                    .replace("Segoe UI;", "Segoe UI Variable Text;")
                    .replace("\\fs24", "\\fs21")
                    .replace("\\fs22", "\\fs21");
                let mut bytes = Bytes {
                    data: rtf.into_bytes(),
                    offset: 0,
                };
                let mut stream = Stream {
                    cookie: &mut bytes as *mut _ as usize,
                    error: 0,
                    callback: Some(read),
                };
                SendMessageW(s.edit, WM_USER + 73, 2, &mut stream as *mut _ as isize);
                if stream.error != 0 {
                    DestroyWindow(hwnd);
                    return Err("Cannot read note formatting; data left untouched".into());
                }
                s.apply_line_spacing();
            } else {
                SetWindowTextW(s.edit, wide(&s.query).as_ptr());
            }
            s.changed = false;
            s.loading = false;
        }

        Ok(Self { state })
    }
    pub unsafe fn flush(&self) {
        self.state.borrow_mut().changed();
    }
    pub unsafe fn floating(&self, _on: bool) {} // Child follows the board's z-order.
    pub fn note_id(&self) -> Option<String> {
        self.state.borrow().note.as_ref().map(|n| n.id.clone())
    }
    #[cfg(test)]
    pub(super) fn input_hwnd(&self) -> HWND {
        self.state.borrow().edit
    }
    pub unsafe fn focus(&self) {
        let (hwnd, edit) = {
            let mut s = self.state.borrow_mut();
            s.caret_on = true;
            (s.hwnd, s.edit)
        };
        SetFocus(edit);
        if GetFocus() == edit {
            SetTimer(hwnd, CARET, blink_ms(), None);
        }
    }
    pub unsafe fn place(
        &self,
        rect: [f32; 4],
        clip: [f32; 2],
        dpi: f32,
        type_scale: typography::Metrics,
        images_h: f32,
    ) {
        let mut s = self.state.borrow_mut();
        let size = [rect[2] - rect[0], rect[3] - rect[1]];
        let inset = if s.note.is_some() {
            type_scale.left
        } else {
            34.
        };
        let target_body = if s.note.is_some() {
            type_scale.body
        } else {
            14.
        };
        let target_leading = if s.note.is_some() {
            type_scale.leading
        } else {
            20.
        };
        let desired_text_rect = if s.note.is_some() {
            let mut tr = type_scale.text_rect(size);
            tr[3] = (tr[3] - images_h).max(tr[1] + target_leading);
            tr
        } else {
            [inset, 5., size[0] - 36., size[1] - 4.]
        };
        let scale_changed = (s.body_size - target_body).abs() > 0.01
            || (s.leading - target_leading).abs() > 0.01
            || (s.dpi - dpi).abs() > 0.001;
        if s.rect != rect || scale_changed || s.text_rect != desired_text_rect {
            s.dpi = dpi;
            s.body_size = target_body;
            s.leading = target_leading;
            if scale_changed {
                s.apply_line_spacing();
            }
            let mut numerator = (dpi * 96.).round().max(1.) as usize;
            let mut denominator = (s.initial_dpi * 96.).round().max(1.) as usize;
            let (mut a, mut b) = (numerator, denominator);
            while b != 0 {
                let remainder = a % b;
                a = b;
                b = remainder;
            }
            numerator /= a;
            denominator /= a;
            SendMessageW(s.edit, WM_USER + 225, numerator, denominator as isize);
            let old_size = [s.rect[2] - s.rect[0], s.rect[3] - s.rect[1]];
            if old_size != size || scale_changed || s.text_rect != desired_text_rect {
                s.snapshot = None;
            }
            s.rect = rect;
            SetWindowPos(
                s.hwnd,
                null_mut(),
                (rect[0] * dpi).round() as i32,
                (rect[1] * dpi).round() as i32,
                (size[0] * dpi).round().max(1.) as i32,
                (size[1] * dpi).round().max(1.) as i32,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            s.text_rect = desired_text_rect;
            s.layout();
        }
        let top = (clip[0] - rect[1]).max(0.).min(size[1]);
        let bottom = (clip[1] - rect[1]).max(0.).min(size[1]);
        let region = CreateRectRgn(
            0,
            (top * dpi).round() as i32,
            (size[0] * dpi).round() as i32,
            (bottom * dpi).round() as i32,
        );
        if SetWindowRgn(s.hwnd, region, 0) == 0 {
            DeleteObject(region);
        }
        ShowWindow(
            s.hwnd,
            if bottom > top {
                SW_SHOWNOACTIVATE
            } else {
                SW_HIDE
            },
        );
    }
    pub unsafe fn draw(&self, dc: HDC, ink: u32) {
        let mut s = self.state.borrow_mut();
        let d = s.dpi;
        let r = s.rect;
        let tr = s.text_rect;
        let oy = if s.note.is_some() { -1. } else { 0. };
        let width = ((tr[2] - tr[0]) * d).round() as i32;
        let height = ((tr[3] - tr[1]) * d).round() as i32;
        if width <= 0 || height <= 0 {
            return;
        }
        if s.snapshot.as_ref().is_none_or(|(old, _)| *old != ink) {
            let mut mattes = Vec::new();
            PRINTING.with(|p| p.set(true));
            for white in [false, true] {
                if let Some(buffer) = Buffer::plain(width, height, 1.) {
                    PatBlt(
                        buffer.dc,
                        0,
                        0,
                        width,
                        height,
                        if white { WHITENESS } else { BLACKNESS },
                    );
                    SendMessageW(s.edit, WM_USER + 67, 0, if white { 0xffffff } else { 0 });
                    SendMessageW(
                        s.edit,
                        WM_PRINTCLIENT,
                        buffer.dc as usize,
                        (PRF_CLIENT | PRF_ERASEBKGND) as isize,
                    );
                    mattes.push(buffer.snapshot());
                }
            }
            PRINTING.with(|p| p.set(false));
            if mattes.len() == 2 {
                let mut pixels = crate::render::recover_icon_alpha(&mattes[0], &mattes[1]);
                for (p, w) in pixels.chunks_exact_mut(4).zip(mattes[1].chunks_exact(4)) {
                    // Native automatic black glyphs adopt the card ink using
                    // the white-matte grayscale coverage so color and opacity
                    // match out-of-focus GDI+ text; retain colored selection highlights.
                    if p[..3].iter().all(|v| *v < 24) {
                        let cov = (1. - w[1] as f32 / 255.).clamp(0., 1.);
                        let a = (cov.powf(0.58) * 255.).round().clamp(0., 255.) as u32;
                        p[3] = a as u8;
                        p[0] = ((ink & 255) * a / 255) as u8;
                        p[1] = (((ink >> 8) & 255) * a / 255) as u8;
                        p[2] = (((ink >> 16) & 255) * a / 255) as u8;
                    }
                }
                s.snapshot = crate::render::Image::from_rgba(width as u32, height as u32, pixels)
                    .map(|image| (ink, image));
            }
        }
        let c = Canvas::new(dc, d);
        if let Some((_, image)) = &s.snapshot {
            let px = ((r[0] + tr[0]) * d).round();
            let py = ((r[1] + tr[1] + oy) * d).round();
            c.image(
                image,
                [
                    px / d,
                    py / d,
                    (px + width as f32) / d,
                    (py + height as f32) / d,
                ],
            );
        }
        if s.note.is_some() && !typography::compact_card([r[2] - r[0], r[3] - r[1]]) {
            let count = LABELS.len() as f32;
            let bw = ((r[2] - r[0] - 24.) / count).min(25.);
            let left = r[2] - 12. - bw * count;
            if left - r[0] > 54. {
                c.text(
                    &s.note
                        .as_ref()
                        .map(|n| icons::note_day(n.updated_at))
                        .unwrap_or_default(),
                    [r[0] + 14., r[3] - 26., left - 4., r[3] - 7.],
                    11.,
                    false,
                    ink,
                    0,
                );
            }
            let active = s.active_formats();
            let light = crate::contrast::luminance(ink) < 0.5;
            for (i, label) in LABELS.iter().enumerate() {
                let x = left + i as f32 * bw;
                let y = r[3] - 32.;
                let btn = [x, y, x + bw - 3., y + 25.];
                let is_hover = s.hover == Some(i);
                let is_active = active[i];
                let is_down = s.pressed == Some(i) && is_hover;
                if is_active {
                    c.round(
                        btn,
                        7.,
                        ink,
                        if is_hover { 60 } else { 44 },
                        false,
                        false,
                    );
                    c.hover_surface(btn, 7., if is_hover { 0.95 } else { 0.78 }, light);
                    c.round(
                        btn,
                        7.,
                        ink,
                        if is_hover { 195 } else { 145 },
                        true,
                        false,
                    );
                } else if is_hover {
                    c.round(
                        btn,
                        7.,
                        ink,
                        if is_down { 38 } else { 24 },
                        false,
                        false,
                    );
                    c.hover_surface(btn, 7., if is_down { 0.85 } else { 0.72 }, light);
                } else {
                    c.hover_surface(btn, 7., 0.25, light);
                }
                c.text_alpha(
                    label,
                    [x, y + 2., x + bw - 3., y + 24.],
                    12.,
                    i < 4 || is_active,
                    ink,
                    1,
                    if is_active || is_hover { 255 } else { 178 },
                );
            }
        }
        if GetFocus() == s.edit && s.caret_on {
            let mut info: GUITHREADINFO = zeroed();
            info.cbSize = size_of::<GUITHREADINFO>() as u32;
            let cr = if GetGUIThreadInfo(0, &mut info) != 0
                && info.hwndCaret == s.edit
                && info.rcCaret.bottom > info.rcCaret.top
            {
                Some(info.rcCaret)
            } else {
                let mut sel_start = 0u32;
                let mut sel_end = 0u32;
                SendMessageW(
                    s.edit,
                    0x00B0,
                    &mut sel_start as *mut _ as usize,
                    &mut sel_end as *mut _ as isize,
                );
                let mut pt = POINT { x: 0, y: 0 };
                SendMessageW(
                    s.edit,
                    WM_USER + 214,
                    &mut pt as *mut _ as usize,
                    sel_end as isize,
                );
                let h = (s.leading * d).round().max(12.) as i32;
                Some(RECT {
                    left: pt.x,
                    top: pt.y,
                    right: pt.x + 1,
                    bottom: pt.y + h,
                })
            };
            if let Some(cr) = cr {
                let x = r[0] + tr[0] + cr.left as f32 / d;
                let y = r[1] + tr[1] + oy + cr.top as f32 / d;
                let ch = ((cr.bottom - cr.top) as f32 / d).max(1.);
                let top = y.max(r[1] + tr[1]);
                let bottom = (y + ch).min(r[1] + tr[3] + oy);
                if bottom > top && x >= r[0] + tr[0] - 1. && x <= r[0] + tr[2] + 1. {
                    c.line(&[(x, top), (x, bottom)], ink, 255, 1.);
                }
            }
        }
    }
}
unsafe fn png_clipboard_format() -> u32 {
    windows_sys::Win32::System::DataExchange::RegisterClipboardFormatW(wide("PNG").as_ptr())
}
fn is_image_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif"
            )
        })
}
unsafe fn clipboard_has_image_drop() -> bool {
    use windows_sys::Win32::System::{
        DataExchange::{CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard},
        Memory::{GlobalLock, GlobalSize, GlobalUnlock},
    };
    if IsClipboardFormatAvailable(15) == 0 || OpenClipboard(null_mut()) == 0 {
        return false;
    }
    let found = (|| -> Option<bool> {
        let handle = GetClipboardData(15); // CF_HDROP
        if handle.is_null() {
            return None;
        }
        let size = GlobalSize(handle);
        if size < 20 {
            return None;
        }
        let ptr = GlobalLock(handle) as *const u8;
        if ptr.is_null() {
            return None;
        }
        let data = std::slice::from_raw_parts(ptr, size);
        let paths = parse_hdrop_paths(data);
        GlobalUnlock(handle);
        Some(paths.iter().any(|p| is_image_path(std::path::Path::new(p))))
    })();
    CloseClipboard();
    found.unwrap_or(false)
}
fn parse_hdrop_paths(data: &[u8]) -> Vec<String> {
    if data.len() < 20 {
        return Vec::new();
    }
    let offset = u32::from_le_bytes(data[0..4].try_into().unwrap_or([0; 4])) as usize;
    let wide_flag = u32::from_le_bytes(data[16..20].try_into().unwrap_or([0; 4])) != 0;
    if offset >= data.len() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let tail = &data[offset..];
    if wide_flag {
        let units: Vec<u16> = tail
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let mut i = 0;
        while i < units.len() {
            let start = i;
            while i < units.len() && units[i] != 0 {
                i += 1;
            }
            if i == start {
                break;
            }
            out.push(String::from_utf16_lossy(&units[start..i]));
            i += 1;
        }
    } else {
        let mut i = 0;
        while i < tail.len() {
            let start = i;
            while i < tail.len() && tail[i] != 0 {
                i += 1;
            }
            if i == start {
                break;
            }
            out.push(String::from_utf8_lossy(&tail[start..i]).into_owned());
            i += 1;
        }
    }
    out
}
unsafe fn paste_from_clipboard(hwnd: HWND, parent: HWND) -> LRESULT {
    use windows_sys::Win32::System::DataExchange::IsClipboardFormatAvailable;
    let png_fmt = png_clipboard_format();
    let has_text = IsClipboardFormatAvailable(13) != 0;
    let has_dib = IsClipboardFormatAvailable(8) != 0
        || IsClipboardFormatAvailable(17) != 0
        || (png_fmt != 0 && IsClipboardFormatAvailable(png_fmt) != 0);
    if clipboard_has_image_drop() || (has_dib && !has_text) {
        PostMessageW(parent, PASTE_IMAGE, 0, 0);
        return 0;
    }
    SendMessageW(hwnd, WM_USER + 64, 13, 0) // EM_PASTESPECIAL / CF_UNICODETEXT
}
// Match the old editor's text-only paste. RichEdit still supplies native Unicode
// clipboard, selection and undo; HTML/OLE objects are not added to note storage.
unsafe extern "system" fn input_proc(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    let parent = GetParent(hwnd);
    let board = data as HWND;
    if msg == WM_SETFOCUS {
        SetTimer(parent, CARET, blink_ms(), None);
        if !board.is_null() {
            PostMessageW(board, super::EDITOR_FOCUS, 0, 0);
        }
    }
    if msg == WM_KILLFOCUS {
        KillTimer(parent, CARET);
    }
    if msg == WM_MOUSEMOVE {
        let mut track = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: hwnd,
            dwHoverTime: 0,
        };
        TrackMouseEvent(&mut track);
        if !board.is_null() {
            PostMessageW(board, super::EDITOR_HOVER, 1, 0);
        }
    }
    if msg == WM_MOUSELEAVE && !board.is_null() {
        PostMessageW(board, super::EDITOR_HOVER, 0, 0);
    }
    if !PRINTING.with(|p| p.get())
        && matches!(
            msg,
            WM_PAINT
                | WM_KEYDOWN
                | WM_KEYUP
                | WM_CHAR
                | WM_LBUTTONDOWN
                | WM_LBUTTONUP
                | WM_LBUTTONDBLCLK
                | WM_RBUTTONDOWN
                | WM_MOUSEMOVE
                | WM_MOUSEWHEEL
                | WM_SETFOCUS
                | WM_KILLFOCUS
                | WM_IME_COMPOSITION
                | WM_IME_ENDCOMPOSITION
        )
    {
        PostMessageW(parent, REDRAW, 0, 0);
    }
    if msg == WM_MOUSEWHEEL && !board.is_null() {
        return SendMessageW(board, WM_MOUSEWHEEL, w, l);
    }
    if msg == WM_KEYDOWN {
        let ctrl = GetKeyState(0x11) < 0;
        let shift = GetKeyState(0x10) < 0;
        let alt = GetKeyState(0x12) < 0;
        if (ctrl && !alt && !shift && w == b'V' as usize)
            || (shift && !ctrl && !alt && w == 0x2D)
        {
            return paste_from_clipboard(hwnd, parent);
        }
    }
    if msg == WM_PASTE {
        return paste_from_clipboard(hwnd, parent);
    }
    if msg == WM_NCDESTROY {
        RemoveWindowSubclass(hwnd, Some(input_proc), 1);
    }
    DefSubclassProc(hwnd, msg, w, l)
}
impl Drop for Editor {
    fn drop(&mut self) {
        unsafe {
            self.flush();
            let (hwnd, board) = {
                let s = self.state.borrow();
                (s.hwnd, s.board)
            };
            if !board.is_null() {
                PostMessageW(board, super::EDITOR_HOVER, 0, 0);
            }
            DestroyWindow(hwnd);
            let font = self.state.borrow().font;
            if !font.is_null() {
                DeleteObject(font);
            }
        }
    }
}
impl State {
    unsafe fn apply_line_spacing(&mut self) {
        if self.edit.is_null() || self.note.is_none() {
            return;
        }
        let was_loading = self.loading;
        let was_changed = self.changed;
        self.loading = true;
        let left_margin = (2. * self.dpi).round().max(1.) as u16;
        SendMessageW(self.edit, 0x00D3, 3, left_margin as isize);
        let mut cf: CharFormat = zeroed();
        cf.size = size_of::<CharFormat>() as u32;
        cf.mask = 0x2000_0000 | 0x8000_0000; // CFM_FACE | CFM_SIZE
        cf.height = (self.body_size * 15.).round() as i32;
        let face = wide("Segoe UI Variable Text");
        let len = face.len().min(cf.face.len());
        cf.face[..len].copy_from_slice(&face[..len]);
        SendMessageW(self.edit, WM_USER + 68, 4, &cf as *const _ as isize); // SCF_ALL
        SendMessageW(self.edit, WM_USER + 68, 0, &cf as *const _ as isize); // SCF_DEFAULT
        let mut pf2: ParaFormat2 = zeroed();
        pf2.base.size = size_of::<ParaFormat2>() as u32;
        pf2.base.mask = 0x0000_0100; // PFM_LINESPACING
        pf2.line_spacing_rule = 4; // tomLineSpaceExactly (in twips)
        pf2.line_spacing = (self.leading * 15.).round() as i32;
        SendMessageW(self.edit, WM_USER + 71, 0, &pf2 as *const _ as isize);
        self.changed = was_changed;
        self.loading = was_loading;
    }
    unsafe fn active_formats(&self) -> [bool; 6] {
        if self.note.is_none() || self.edit.is_null() {
            return [false; 6];
        }
        let mut cf: CharFormat = zeroed();
        cf.size = size_of::<CharFormat>() as u32;
        SendMessageW(self.edit, WM_USER + 58, 1, &mut cf as *mut _ as isize);
        let mut pf: ParaFormat = zeroed();
        pf.size = size_of::<ParaFormat>() as u32;
        SendMessageW(self.edit, WM_USER + 61, 0, &mut pf as *mut _ as isize);

        let mut bullet = pf.numbering == 1;
        let mut numbered = pf.numbering > 1;
        if !bullet && !numbered {
            let mut sel_start = 0u32;
            SendMessageW(self.edit, 0x00B0, &mut sel_start as *mut _ as usize, 0);
            let text = window_text(self.edit);
            let mut utf16_pos = 0usize;
            for line in text.split('\n') {
                let line_clean = line.trim_end_matches('\r');
                let next_pos = utf16_pos + line.encode_utf16().count() + 1;
                if (sel_start as usize) < next_pos {
                    let trimmed = line_clean.trim_start();
                    if trimmed.starts_with("• ") || trimmed.starts_with("- ") {
                        bullet = true;
                    } else if let Some(dot) = trimmed.find(". ") {
                        if dot > 0 && trimmed[..dot].chars().all(|c| c.is_ascii_digit()) {
                            numbered = true;
                        }
                    }
                    break;
                }
                utf16_pos = next_pos;
            }
        }
        [
            cf.effects & 1 != 0,
            cf.effects & 2 != 0,
            cf.effects & 4 != 0,
            cf.effects & 8 != 0,
            bullet,
            numbered,
        ]
    }
    unsafe fn close(&mut self) {
        self.changed();
        ShowWindow(self.hwnd, SW_HIDE);
        self.events.borrow_mut().push(Event::Closed);
        PostMessageW(self.board, EVENTS, 0, 0);
    }
    unsafe fn layout(&self) {
        if self.edit.is_null() {
            return;
        }
        let r = self.text_rect;
        let d = self.dpi;
        MoveWindow(
            self.edit,
            (r[0] * d).round() as i32,
            (r[1] * d).round() as i32,
            ((r[2] - r[0]) * d).round().max(1.) as i32,
            ((r[3] - r[1]) * d).round().max(1.) as i32,
            1,
        );
    }
    unsafe fn changed(&mut self) {
        if self.loading || self.edit.is_null() || !self.changed {
            return;
        }
        let text = window_text(self.edit);
        let event = if let Some(note) = &mut self.note {
            let mut bytes = Bytes {
                data: vec![],
                offset: 0,
            };
            let mut stream = Stream {
                cookie: &mut bytes as *mut _ as usize,
                error: 0,
                callback: Some(write),
            };
            SendMessageW(self.edit, WM_USER + 74, 2, &mut stream as *mut _ as isize);
            if stream.error != 0 {
                return;
            }
            let rtf = String::from_utf8_lossy(&bytes.data)
                .trim_end_matches('\0')
                .to_string();
            if note.body == text && note.rtf.as_ref() == Some(&rtf) {
                return;
            }
            note.body = text;
            note.rtf = Some(rtf);
            model::extract_rtf_pictures(note, &self.root);
            note.updated_at = model::now();
            Event::Note(note.clone())
        } else {
            self.query = text.clone();
            Event::Search(text)
        };
        self.changed = false;
        self.snapshot = None;
        self.events.borrow_mut().push(event);
        PostMessageW(self.board, EVENTS, 0, 0);
    }
    unsafe fn format(&mut self, index: usize) {
        if self.note.is_none() {
            return;
        }
        if index < 4 {
            let mask = [1, 2, 4, 8][index];
            let mut f: CharFormat = zeroed();
            f.size = size_of::<CharFormat>() as u32;
            SendMessageW(self.edit, WM_USER + 58, 1, &mut f as *mut _ as isize);
            f.effects ^= mask;
            f.mask = mask;
            SendMessageW(self.edit, WM_USER + 68, 1, &f as *const _ as isize);
        } else {
            let mut f: ParaFormat = zeroed();
            f.size = size_of::<ParaFormat>() as u32;
            SendMessageW(self.edit, WM_USER + 61, 0, &mut f as *mut _ as isize);
            let target = if index == 4 { 1 } else { 2 };
            f.numbering = if f.numbering == target { 0 } else { target };
            f.mask = 0x20;
            SendMessageW(self.edit, WM_USER + 71, 0, &f as *const _ as isize);
        }
        SetFocus(self.edit);
        self.changed = true;
        self.changed();
    }
    unsafe fn paste_clipboard_image(&mut self) {
        use windows_sys::Win32::System::{
            DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard},
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
        };
        let Some(note) = &mut self.note else {
            return;
        };
        if OpenClipboard(self.hwnd) == 0 {
            return;
        }
        let saved_name = (|| -> Option<String> {
            let dir = model::images_dir(&self.root);
            std::fs::create_dir_all(&dir).ok()?;
            let name = format!("img-{}-{}.png", model::now(), note.images.len());
            let png_fmt = png_clipboard_format();
            if png_fmt != 0 {
                let handle = GetClipboardData(png_fmt);
                if !handle.is_null() {
                    let size = GlobalSize(handle);
                    let ptr = GlobalLock(handle) as *const u8;
                    if !ptr.is_null() && size > 8 {
                        let data = std::slice::from_raw_parts(ptr, size);
                        let decoded = image::load_from_memory(data).ok();
                        GlobalUnlock(handle);
                        if let Some(img) = decoded {
                            img.into_rgba8().save(dir.join(&name)).ok()?;
                            return Some(name);
                        }
                    } else if !ptr.is_null() {
                        GlobalUnlock(handle);
                    }
                }
            }
            let mut handle = GetClipboardData(8); // CF_DIB
            if handle.is_null() {
                handle = GetClipboardData(17); // CF_DIBV5
            }
            if !handle.is_null() {
                let size = GlobalSize(handle);
                if size >= 40 {
                    let ptr = GlobalLock(handle) as *const u8;
                    if !ptr.is_null() {
                        let data = std::slice::from_raw_parts(ptr, size);
                        let header_size =
                            u32::from_le_bytes(data[0..4].try_into().ok()?) as usize;
                        let w = i32::from_le_bytes(data[4..8].try_into().ok()?);
                        let h_raw = i32::from_le_bytes(data[8..12].try_into().ok()?);
                        let bpp = u16::from_le_bytes(data[14..16].try_into().ok()?) as usize;
                        let comp = u32::from_le_bytes(data[16..20].try_into().ok()?);
                        let clr_used =
                            u32::from_le_bytes(data[32..36].try_into().ok()?) as usize;
                        if header_size >= 40
                            && w > 0
                            && h_raw != 0
                            && w <= 8192
                            && h_raw.abs() <= 8192
                            && matches!(bpp, 24 | 32)
                            && matches!(comp, 0 | 3)
                        {
                            let mask_bytes =
                                if comp == 3 && header_size == 40 { 12 } else { 0 };
                            let palette_bytes = if clr_used > 0 { clr_used * 4 } else { 0 };
                            let pixel_offset = header_size + mask_bytes + palette_bytes;
                            let width = w as usize;
                            let height = h_raw.unsigned_abs() as usize;
                            let stride = ((width * bpp + 31) / 32) * 4;
                            if pixel_offset + stride * height <= data.len() {
                                let pixels = &data[pixel_offset..];
                                let mut rgba = vec![255u8; width * height * 4];
                                let mut any_alpha = false;
                                for y in 0..height {
                                    let src_y = if h_raw > 0 { height - 1 - y } else { y };
                                    let row = &pixels
                                        [src_y * stride..src_y * stride + width * (bpp / 8)];
                                    for x in 0..width {
                                        let s = x * (bpp / 8);
                                        let d = (y * width + x) * 4;
                                        rgba[d] = row[s + 2];
                                        rgba[d + 1] = row[s + 1];
                                        rgba[d + 2] = row[s];
                                        if bpp == 32 {
                                            let a = row[s + 3];
                                            if a != 0 {
                                                any_alpha = true;
                                            }
                                            rgba[d + 3] = a;
                                        }
                                    }
                                }
                                GlobalUnlock(handle);
                                if bpp == 32 && !any_alpha {
                                    for p in rgba.chunks_exact_mut(4) {
                                        p[3] = 255;
                                    }
                                }
                                let img = image::RgbaImage::from_raw(
                                    width as u32,
                                    height as u32,
                                    rgba,
                                )?;
                                img.save(dir.join(&name)).ok()?;
                                return Some(name);
                            }
                        }
                        GlobalUnlock(handle);
                    }
                }
            }
            let drop_handle = GetClipboardData(15); // CF_HDROP
            if !drop_handle.is_null() {
                let size = GlobalSize(drop_handle);
                let ptr = GlobalLock(drop_handle) as *const u8;
                if !ptr.is_null() && size >= 20 {
                    let data = std::slice::from_raw_parts(ptr, size);
                    let paths = parse_hdrop_paths(data);
                    GlobalUnlock(drop_handle);
                    for path_str in paths {
                        let p = std::path::Path::new(&path_str);
                        if is_image_path(p) {
                            if let Ok(img) = image::open(p) {
                                img.into_rgba8().save(dir.join(&name)).ok()?;
                                return Some(name);
                            }
                        }
                    }
                } else if !ptr.is_null() {
                    GlobalUnlock(drop_handle);
                }
            }
            None
        })();
        CloseClipboard();
        if let Some(name) = saved_name {
            note.images.push(name);
            note.updated_at = model::now();
            self.events.borrow_mut().push(Event::Note(note.clone()));
            PostMessageW(self.board, EVENTS, 0, 0);
        }
    }
    fn hit(&self, p: [f32; 2]) -> Option<usize> {
        let width = self.rect[2] - self.rect[0];
        let height = self.rect[3] - self.rect[1];
        if self.note.is_none()
            || typography::compact_card([width, height])
            || p[1] < height - 32.
            || p[1] > height - 7.
        {
            return None;
        }
        let count = LABELS.len() as f32;
        let bw = ((width - 24.) / count).min(25.);
        let left = width - 12. - bw * count;
        (0..LABELS.len()).find(|&i| {
            let x = left + i as f32 * bw;
            p[0] >= x && p[0] < x + bw - 3.
        })
    }
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = &*(l as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<State>;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, w, l);
    }
    let Ok(mut s) = (*ptr).try_borrow_mut() else {
        return DefWindowProcW(hwnd, msg, w, l);
    };
    match msg {
        WM_COMMAND if (w >> 16) as u32 == EN_CHANGE => {
            if !s.loading {
                s.changed = true;
                s.changed();
            }
            0
        }
        WM_PAINT => {
            ValidateRect(hwnd, null());
            0
        }
        WM_ERASEBKGND => 1,
        WM_SIZE => {
            let mut client: RECT = zeroed();
            GetClientRect(hwnd, &mut client);
            let old_size = [s.rect[2] - s.rect[0], s.rect[3] - s.rect[1]];
            let size = [client.right as f32 / s.dpi, client.bottom as f32 / s.dpi];
            s.text_rect[2] += size[0] - old_size[0];
            s.text_rect[3] += size[1] - old_size[1];
            s.rect[2] = s.rect[0] + size[0];
            s.rect[3] = s.rect[1] + size[1];
            s.snapshot = None;
            s.layout();
            0
        }
        WM_NCHITTEST => {
            let mut p = POINT {
                x: point(l)[0] as i32,
                y: point(l)[1] as i32,
            };
            ScreenToClient(hwnd, &mut p);
            if s.hit([p.x as f32 / s.dpi, p.y as f32 / s.dpi]).is_some()
                || inside([p.x as f32 / s.dpi, p.y as f32 / s.dpi], s.text_rect)
            {
                HTCLIENT as isize
            } else {
                HTTRANSPARENT as isize
            }
        }
        WM_TIMER if w == CARET => {
            s.caret_on = !s.caret_on;
            PostMessageW(s.board, REDRAW, 0, 0);
            0
        }
        WM_MOUSEMOVE => {
            let p = point(l).map(|v| v / s.dpi);
            let hover = s.hit(p);
            if hover != s.hover {
                s.hover = hover;
                PostMessageW(s.board, REDRAW, 0, 0);
            }
            if !s.board.is_null() {
                PostMessageW(s.board, super::EDITOR_HOVER, 1, 0);
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
            if s.hover.take().is_some() {
                PostMessageW(s.board, REDRAW, 0, 0);
            }
            if !s.board.is_null() {
                PostMessageW(s.board, super::EDITOR_HOVER, 0, 0);
            }
            0
        }
        WM_LBUTTONDOWN => {
            s.pressed = s.hit(point(l).map(|v| v / s.dpi));
            SetCapture(hwnd);
            PostMessageW(s.board, REDRAW, 0, 0);
            0
        }
        WM_LBUTTONUP => {
            ReleaseCapture();
            let hit = s.hit(point(l).map(|v| v / s.dpi));
            if hit.is_some() && hit == s.pressed.take() {
                let i = hit.unwrap();
                s.format(i);
            }
            PostMessageW(s.board, REDRAW, 0, 0);
            0
        }
        WM_CAPTURECHANGED => {
            s.pressed = None;
            0
        }
        KEY => {
            if w == 0x1b || w == 0x57 {
                s.close();
            } else if let Some(i) = [0x42, 0x49, 0x55, 0x53].iter().position(|v| *v == w) {
                s.format(i);
            }
            0
        }
        WM_CLOSE => {
            s.close();
            0
        }
        WM_KILLFOCUS => {
            s.changed();
            0
        }
        REDRAW => {
            s.snapshot = None;
            s.caret_on = true;
            if GetFocus() == s.edit {
                SetTimer(hwnd, CARET, blink_ms(), None);
            }
            PostMessageW(s.board, REDRAW, 0, 0);
            0
        }
        PASTE_IMAGE => {
            s.paste_clipboard_image();
            0
        }
        WM_MOUSEWHEEL => {
            let board = s.board;
            drop(s);
            if !board.is_null() {
                SendMessageW(board, WM_MOUSEWHEEL, w, l)
            } else {
                0
            }
        }
        WM_NCDESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(s);
            DefWindowProcW(hwnd, msg, w, l)
        }
        _ => {
            // Native default dispatch may synchronously send WM_SIZE or enter
            // a move/resize loop; release the guard before either can happen.
            drop(s);
            DefWindowProcW(hwnd, msg, w, l)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rich_edit_round_trips_vietnamese_formatting_and_keeps_unchanged_notes_clean() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            assert_eq!(size_of::<Stream>(), 20);
            let _graphics = crate::render::Runtime::start().unwrap();
            let root = std::env::temp_dir().join(format!(
                "lumen-rich-edit-{}-{}",
                std::process::id(),
                model::now()
            ));
            let glass = Rc::new(RefCell::new(Glass::new(13., 1.)));
            let host = Host::create(&root, glass, true, true).unwrap();
            let board = host.state.borrow().hwnd;
            let mut b = model::Board::default();
            b.create().unwrap();
            let mut note = b.notes.remove(0);
            note.body = "<b>Tiếng Việt 😀</b><br><i>Keep italic</i><ul><li>Bullet</li></ul>".into();
            let events = Rc::new(RefCell::new(vec![]));
            let editor = Editor::create(
                board,
                root.clone(),
                events.clone(),
                Some(note),
                String::new(),
                false,
            )
            .unwrap();
            editor.place(
                [24., 112., 360., 330.],
                [110., 594.],
                1.,
                typography::Metrics::for_theme("lumen"),
                0.,
            );
            assert_eq!(GetParent(editor.state.borrow().hwnd), board);
            assert_eq!(
                GetWindowLongW(editor.state.borrow().hwnd, GWL_STYLE) as u32 & WS_POPUP,
                0
            );
            // DefWindowProc(WM_WINDOWPOSCHANGED) sends WM_SIZE synchronously.
            // Holding the RefMut across default dispatch used to suppress
            // layout and leave the RichEdit control at its previous dimensions.
            let (editor_hwnd, edit_hwnd) = {
                let s = editor.state.borrow();
                (s.hwnd, s.edit)
            };
            let mut before: RECT = zeroed();
            GetWindowRect(edit_hwnd, &mut before);
            let mut outer: RECT = zeroed();
            GetWindowRect(editor_hwnd, &mut outer);
            SetWindowPos(
                editor_hwnd,
                null_mut(),
                0,
                0,
                outer.right - outer.left + 80,
                outer.bottom - outer.top + 60,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let mut after: RECT = zeroed();
            GetWindowRect(edit_hwnd, &mut after);
            assert_eq!(after.right - after.left, before.right - before.left + 80);
            assert_eq!(after.bottom - after.top, before.bottom - before.top + 60);
            editor.flush();
            assert!(
                events.borrow().is_empty(),
                "Opening a note must not rewrite it"
            );
            {
                let mut s = editor.state.borrow_mut();
                assert!(window_text(s.edit).contains("Tiếng Việt 😀"));
                s.changed = true;
                s.changed();
            }
            let note = match events.borrow_mut().pop().unwrap() {
                Event::Note(n) => n,
                _ => panic!(),
            };
            assert!(note.body.contains("Tiếng Việt 😀"));
            let rtf = note.rtf.as_ref().unwrap();
            assert!(rtf.contains("\\b"));
            assert!(rtf.contains("\\i"));
            drop(editor);
            let editor = Editor::create(
                board,
                root.clone(),
                events.clone(),
                Some(note),
                String::new(),
                false,
            )
            .unwrap();
            assert!(window_text(editor.state.borrow().edit).contains("Keep italic"));
            {
                let mut s = editor.state.borrow_mut();
                SetWindowTextW(s.edit, wide("Ghi chú mới – 😀").as_ptr());
                s.changed = true;
                s.changed();
            }
            let note = match events.borrow_mut().pop().unwrap() {
                Event::Note(n) => n,
                _ => panic!(),
            };
            assert_eq!(note.body, "Ghi chú mới – 😀");
            drop(editor);
            drop(host);
        }
    }
}
