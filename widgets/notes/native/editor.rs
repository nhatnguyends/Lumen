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
    events: Events,
    note: Option<model::Note>,
    query: String,
    font: HFONT,
    dpi: f32,
    initial_dpi: f32,
    pressed: Option<usize>,
    loading: bool,
    changed: bool,
    rect: [f32; 4],
    text_rect: [f32; 4],
    snapshot: Option<(u32, crate::render::Image)>,
    caret_on: bool,
}
const CLASS: &str = "LumenNativeNotesEditor";
const LABELS: [&str; 7] = ["B", "I", "U", "S", "•", "1.", "✓"];
const CARET: usize = 110;
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
impl Editor {
    pub unsafe fn create(
        board: HWND,
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
            events,
            note,
            query,
            font: null_mut(),
            dpi: GetDpiForWindow(board) as f32 / 96.,
            initial_dpi: GetDpiForWindow(board) as f32 / 96.,
            pressed: None,
            loading: true,
            changed: false,
            rect: [0.; 4],
            text_rect: [0.; 4],
            snapshot: None,
            caret_on: true,
        }));
        let dpi = state.borrow().dpi;
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
            s.font = CreateFontW(
                -(if search { 14. } else { 16. } * dpi).round() as i32,
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
                CLEARTYPE_QUALITY as u32,
                0,
                wide("Segoe UI").as_ptr(),
            );
            SendMessageW(s.edit, WM_SETFONT, s.font as usize, 1);
            SendMessageW(s.edit, WM_USER + 67, 0, rgb(245, 242, 233) as isize);
            SendMessageW(s.edit, WM_USER + 69, 0, 1); // ENM_CHANGE
            SendMessageW(
                s.edit,
                windows_sys::Win32::UI::Controls::EM_LIMITTEXT,
                200_000,
                0,
            );
            if let Some(note) = &s.note {
                let mut bytes = Bytes {
                    data: note.rich_text().into_bytes(),
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
            } else {
                SetWindowTextW(s.edit, wide(&s.query).as_ptr());
            }
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
        let edit = self.state.borrow().edit;
        SetFocus(edit);
    }
    pub unsafe fn place(
        &self,
        rect: [f32; 4],
        clip: [f32; 2],
        dpi: f32,
        type_scale: typography::Metrics,
    ) {
        let mut s = self.state.borrow_mut();
        let size = [rect[2] - rect[0], rect[3] - rect[1]];
        let inset = if s.note.is_some() {
            type_scale.left
        } else {
            34.
        };
        if s.rect != rect || s.dpi != dpi || s.text_rect[0] != inset {
            // Scale the view, not stored RTF font sizes. Existing rich notes
            // therefore shrink with previews without rewriting formatting.
            let mut numerator =
                (dpi * 96.).round().max(1.) as usize * if s.note.is_some() { 7 } else { 8 };
            let mut denominator = (s.initial_dpi * 96.).round().max(1.) as usize * 8;
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
            if old_size != size || s.dpi != dpi || s.text_rect[0] != inset {
                s.snapshot = None;
            }
            s.rect = rect;
            s.dpi = dpi;
            SetWindowPos(
                s.hwnd,
                null_mut(),
                (rect[0] * dpi).round() as i32,
                (rect[1] * dpi).round() as i32,
                (size[0] * dpi).round().max(1.) as i32,
                (size[1] * dpi).round().max(1.) as i32,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            s.text_rect = if s.note.is_some() {
                type_scale.text_rect(size)
            } else {
                [inset, 5., size[0] - 36., size[1] - 4.]
            };
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
                for p in pixels.chunks_exact_mut(4) {
                    // Native automatic black glyphs adopt the card ink; retain
                    // colored selection highlights and selected white glyphs.
                    if p[..3].iter().all(|v| *v < 24) {
                        let a = p[3] as u32;
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
            c.image(
                image,
                [r[0] + tr[0], r[1] + tr[1], r[0] + tr[2], r[1] + tr[3]],
            );
        }
        if s.note.is_some() && !typography::compact_card([r[2] - r[0], r[3] - r[1]]) {
            let bw = ((r[2] - r[0] - 24.) / 7.).min(24.);
            let left = r[2] - 12. - bw * 7.;
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
            for (i, label) in LABELS.iter().enumerate() {
                let x = left + i as f32 * bw;
                let y = r[3] - 32.;
                c.hover_surface(
                    [x, y, x + bw - 3., y + 25.],
                    7.,
                    0.35,
                    crate::contrast::luminance(ink) < 0.5,
                );
                c.text(label, [x, y + 2., x + bw - 3., y + 24.], 12., i < 4, ink, 1);
            }
        }
        if GetFocus() == s.edit && s.caret_on {
            let mut info: GUITHREADINFO = zeroed();
            info.cbSize = size_of::<GUITHREADINFO>() as u32;
            if GetGUIThreadInfo(0, &mut info) != 0 && info.hwndCaret == s.edit {
                let cr = info.rcCaret;
                let x = r[0] + tr[0] + cr.left as f32 / d;
                let y = r[1] + tr[1] + cr.top as f32 / d;
                if y >= r[1] + tr[1] && y < r[1] + tr[3] {
                    c.line(
                        &[
                            (x, y),
                            (x, (y + (cr.bottom - cr.top) as f32 / d).min(r[1] + tr[3])),
                        ],
                        ink,
                        255,
                        1.,
                    );
                }
            }
        }
    }
}
// Match the old editor's text-only paste. RichEdit still supplies native Unicode
// clipboard, selection and undo; HTML/OLE objects are not added to note storage.
unsafe extern "system" fn input_proc(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    let parent = GetParent(hwnd);
    if msg == WM_SETFOCUS {
        SetTimer(parent, CARET, GetCaretBlinkTime().max(100), None);
    }
    if msg == WM_KILLFOCUS {
        KillTimer(parent, CARET);
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
    if msg == WM_PASTE {
        return SendMessageW(hwnd, WM_USER + 64, 13, 0);
    } // EM_PASTESPECIAL / CF_UNICODETEXT
    if msg == WM_NCDESTROY {
        RemoveWindowSubclass(hwnd, Some(input_proc), 1);
    }
    DefSubclassProc(hwnd, msg, w, l)
}
impl Drop for Editor {
    fn drop(&mut self) {
        unsafe {
            self.flush();
            let hwnd = self.state.borrow().hwnd;
            DestroyWindow(hwnd);
            let font = self.state.borrow().font;
            if !font.is_null() {
                DeleteObject(font);
            }
        }
    }
}
impl State {
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
        let bw = ((width - 24.) / 7.).min(24.);
        let left = width - 12. - bw * 7.;
        (0..7).find(|&i| {
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
            s.changed = true;
            s.changed();
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
        WM_LBUTTONDOWN => {
            s.pressed = s.hit(point(l).map(|v| v / s.dpi));
            SetCapture(hwnd);
            0
        }
        WM_LBUTTONUP => {
            ReleaseCapture();
            let hit = s.hit(point(l).map(|v| v / s.dpi));
            if hit.is_some() && hit == s.pressed.take() {
                let i = hit.unwrap();
                if i == 6 {
                    s.close();
                } else {
                    s.format(i);
                }
            }
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
            PostMessageW(s.board, REDRAW, 0, 0);
            0
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
            let editor =
                Editor::create(board, events.clone(), Some(note), String::new(), false).unwrap();
            editor.place(
                [24., 112., 360., 330.],
                [110., 594.],
                1.,
                typography::Metrics::for_theme("lumen"),
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
            let editor =
                Editor::create(board, events.clone(), Some(note), String::new(), false).unwrap();
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
