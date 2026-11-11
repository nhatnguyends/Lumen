//! Shared native settings shell and controls, extracted from Dock settings.rs.
//! Values belong to callers. Changes are posted asynchronously to the owner.
use crate::{
    compositor::{Buffer, WindowShadow},
    glass::Glass,
    model::wide,
    render::Canvas,
};
use std::{
    cell::RefCell,
    collections::VecDeque,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    rc::Rc,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::*,
        Input::KeyboardAndMouse::{GetCapture, GetKeyState, ReleaseCapture, SetCapture},
        WindowsAndMessaging::*,
    },
};

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Number(f32),
    Choice(String),
    Action,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub id: u32,
    pub value: Value,
}
#[derive(Clone, Debug)]
pub struct Choice {
    pub key: String,
    pub label: String,
    pub color: Option<u32>,
}
impl Choice {
    pub fn new(key: impl Into<String>, label: impl Into<String>, color: Option<u32>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            color,
        }
    }
}
#[derive(Clone, Debug)]
pub enum Row {
    Section(String),
    Widget {
        id: u32,
        settings_id: u32,
        label: String,
        hint: String,
        value: bool,
    },
    Toggle {
        id: u32,
        label: String,
        hint: String,
        value: bool,
    },
    Slider {
        id: u32,
        label: String,
        value: f32,
        min: f32,
        max: f32,
        suffix: String,
    },
    Choices {
        id: u32,
        label: String,
        selected: String,
        options: Vec<Choice>,
    },
    Button {
        id: u32,
        label: String,
    },
    Note(String),
}
impl Row {
    pub fn section(label: impl Into<String>) -> Self {
        Self::Section(label.into())
    }
    pub fn toggle(id: u32, label: impl Into<String>, hint: impl Into<String>, value: bool) -> Self {
        Self::Toggle {
            id,
            label: label.into(),
            hint: hint.into(),
            value,
        }
    }
    pub fn slider(
        id: u32,
        label: impl Into<String>,
        value: f32,
        min: f32,
        max: f32,
        suffix: impl Into<String>,
    ) -> Self {
        Self::Slider {
            id,
            label: label.into(),
            value,
            min,
            max,
            suffix: suffix.into(),
        }
    }
    pub fn choices(
        id: u32,
        label: impl Into<String>,
        selected: impl Into<String>,
        options: Vec<Choice>,
    ) -> Self {
        Self::Choices {
            id,
            label: label.into(),
            selected: selected.into(),
            options,
        }
    }
    pub fn button(id: u32, label: impl Into<String>) -> Self {
        Self::Button {
            id,
            label: label.into(),
        }
    }
    pub fn note(text: impl Into<String>) -> Self {
        Self::Note(text.into())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Close,
    Scroll,
    Toggle(u32),
    Slider(u32),
    Choice(u32, usize),
    Button(u32),
}
#[derive(Clone, Copy)]
struct Hit {
    rect: [f32; 4],
    target: Target,
}
struct Drag {
    edge: u32,
    start: POINT,
    bounds: RECT,
}
struct State {
    hwnd: HWND,
    owner: HWND,
    title: String,
    subtitle: String,
    glass: Rc<RefCell<Glass>>,
    rows: Vec<Row>,
    message: u32,
    dpi: f32,
    scroll: f32,
    hits: Vec<Hit>,
    pressed: Option<Target>,
    bar_grab: f32,
    focus: Option<Target>,
    drag: Option<Drag>,
    changes: VecDeque<Change>,
    frame: Option<Buffer>,
    backdrop: Option<Buffer>,
    key: String,
    shadow: Option<WindowShadow>,
    surface: u32,
    light: bool,
}
pub struct Host {
    state: Box<RefCell<State>>,
}
impl Host {
    pub unsafe fn create(
        owner: HWND,
        title: &str,
        subtitle: &str,
        glass: Rc<RefCell<Glass>>,
        rows: Vec<Row>,
        notify_message: u32,
    ) -> Result<Self, String> {
        let class = wide("LumenNativeWidgetSettings");
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
        let dpi = if owner.is_null() {
            GetDpiForSystem()
        } else {
            GetDpiForWindow(owner)
        } as f32
            / 96.;
        let mut info: MONITORINFO = zeroed();
        info.cbSize = size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(
            MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST),
            &mut info,
        );
        let mut r: RECT = zeroed();
        GetWindowRect(owner, &mut r);
        let w = (568. * dpi)
            .min((info.rcWork.right - info.rcWork.left) as f32)
            .max(420. * dpi) as i32;
        let h = (668. * dpi)
            .min((info.rcWork.bottom - info.rcWork.top) as f32)
            .max(360. * dpi) as i32;
        let x = (r.left + (r.right - r.left - w) / 2).clamp(
            info.rcWork.left,
            (info.rcWork.right - w).max(info.rcWork.left),
        );
        let y = (r.top - 80).clamp(
            info.rcWork.top,
            (info.rcWork.bottom - h).max(info.rcWork.top),
        );
        let surface = glass.borrow().wallpaper_accent();
        let light = crate::contrast::luminance(surface) > 0.42;
        let mut state = Box::new(RefCell::new(State {
            hwnd: null_mut(),
            owner,
            title: title.into(),
            subtitle: subtitle.into(),
            glass,
            rows,
            message: notify_message,
            dpi,
            scroll: 0.,
            hits: vec![],
            pressed: None,
            bar_grab: 0.,
            focus: None,
            drag: None,
            changes: VecDeque::new(),
            frame: None,
            backdrop: None,
            key: String::new(),
            shadow: None,
            surface,
            light,
        }));
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW,
            class.as_ptr(),
            wide(title).as_ptr(),
            WS_POPUP,
            x,
            y,
            w,
            h,
            owner,
            null_mut(),
            instance,
            &mut *state as *mut _ as _,
        );
        if hwnd.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        SetWindowTextW(hwnd, wide(title).as_ptr());
        state.borrow_mut().hwnd = hwnd;
        state.borrow_mut().dpi = GetDpiForWindow(hwnd) as f32 / 96.;
        Ok(Self { state })
    }
    pub fn hwnd(&self) -> HWND {
        self.state.borrow().hwnd
    }
    pub fn is_visible(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd()) != 0 }
    }
    pub unsafe fn show(&mut self) {
        let hwnd = self.hwnd();
        {
            let mut s = self.state.borrow_mut();
            if s.shadow.is_none() {
                s.shadow = WindowShadow::new(hwnd);
            }
        }
        ShowWindow(hwnd, SW_SHOW);
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
        );
        SetForegroundWindow(hwnd);
        self.state.borrow_mut().paint();
    }
    pub unsafe fn hide(&mut self) {
        self.state.borrow_mut().hide();
    }
    pub fn set_rows(&mut self, rows: Vec<Row>) {
        let mut s = self.state.borrow_mut();
        s.rows = rows;
        unsafe {
            if IsWindowVisible(s.hwnd) != 0 {
                InvalidateRect(s.hwnd, null(), 0);
            }
        }
    }
    pub fn set_surface(&mut self, color: u32, light: bool) {
        let mut s = self.state.borrow_mut();
        s.surface = color;
        s.light = light;
        s.key.clear();
        unsafe {
            if IsWindowVisible(s.hwnd) != 0 {
                InvalidateRect(s.hwnd, null(), 0);
            }
        }
    }
    pub fn take_changes(&mut self) -> Vec<Change> {
        self.state.borrow_mut().changes.drain(..).collect()
    }
    pub fn refresh(&mut self) {
        let mut s = self.state.borrow_mut();
        s.key.clear();
        unsafe {
            if IsWindowVisible(s.hwnd) != 0 {
                InvalidateRect(s.hwnd, null(), 0);
            }
        }
    }
    #[cfg(test)]
    pub unsafe fn render_for_test(&mut self) -> (u32, u32, Vec<u8>) {
        let mut s = self.state.borrow_mut();
        s.paint();
        let f = s.frame.as_ref().unwrap();
        (f.width as u32, f.height as u32, f.snapshot())
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        unsafe {
            let hwnd = self.hwnd();
            if IsWindow(hwnd) != 0 {
                DestroyWindow(hwnd);
            }
        }
    }
}
fn point(lp: LPARAM) -> POINT {
    POINT {
        x: lp as i16 as i32,
        y: (lp >> 16) as i16 as i32,
    }
}
fn inside(r: [f32; 4], p: [f32; 2]) -> bool {
    p[0] >= r[0] && p[0] < r[2] && p[1] >= r[1] && p[1] < r[3]
}
fn height(row: &Row, columns: usize) -> f32 {
    match row {
        Row::Section(_) => 18.,
        Row::Toggle { .. } => 48.,
        Row::Widget { .. } => 116.,
        Row::Slider { .. } => 55.,
        Row::Choices { options, .. } => 34. + options.len().div_ceil(columns) as f32 * 36.,
        Row::Button { .. } => 44.,
        Row::Note(_) => 36.,
    }
}
impl State {
    unsafe fn size(&self) -> [f32; 2] {
        let mut r: RECT = zeroed();
        GetClientRect(self.hwnd, &mut r);
        [r.right as f32 / self.dpi, r.bottom as f32 / self.dpi]
    }
    fn content_height(&self, w: f32) -> f32 {
        100. + self
            .rows
            .iter()
            .map(|r| height(r, if w < 480. { 2 } else { 3 }))
            .sum::<f32>()
    }
    unsafe fn emit(&mut self, id: u32, value: Value) {
        self.changes.retain(|c| c.id != id);
        self.changes.push_back(Change { id, value });
        PostMessageW(self.owner, self.message, 0, 0);
    }
    unsafe fn hide(&mut self) {
        self.pressed = None;
        self.drag = None;
        if GetCapture() == self.hwnd {
            ReleaseCapture();
        }
        if GetCapture() == self.hwnd {
            ReleaseCapture();
        }
        ShowWindow(self.hwnd, SW_HIDE);
        self.frame = None;
        self.backdrop = None;
        self.shadow = None;
        self.key.clear();
    }
    unsafe fn scrollbar(&self) -> Option<[f32; 4]> {
        let [w, h] = self.size();
        let content = self.content_height(w);
        if content <= h {
            return None;
        }
        let track = h - 84.;
        let thumb = (track * track / content).max(24.);
        let y = 64. + (track - thumb) * self.scroll / (content - h + 18.).max(1.);
        Some([w - 15., y, w - 3., y + thumb])
    }
    unsafe fn scroll_to(&mut self, y: f32) {
        let [w, h] = self.size();
        let content = self.content_height(w);
        let track = h - 84.;
        let thumb = (track * track / content).max(24.);
        self.scroll = ((y - self.bar_grab - 64.) / (track - thumb).max(1.)).clamp(0., 1.)
            * (content - h + 18.).max(0.);
        InvalidateRect(self.hwnd, null(), 0);
    }
    unsafe fn activate(&mut self, target: Target) {
        if let Target::Choice(id, index) = target {
            let initial = self.rows.iter().find_map(|row| match row {
                Row::Choices {
                    id: key,
                    selected,
                    options,
                    ..
                } if *key == id && options.get(index).is_some_and(|o| o.key == "custom") => {
                    Some(crate::surface::color(selected, self.surface))
                }
                _ => None,
            });
            if let Some(initial) = initial {
                if let Some(color) = crate::color_picker::choose(self.hwnd, initial) {
                    let key = crate::surface::custom_key(color);
                    for row in &mut self.rows {
                        if let Row::Choices {
                            id: rid, selected, ..
                        } = row
                        {
                            if *rid == id {
                                *selected = key.clone();
                            }
                        }
                    }
                    self.emit(id, Value::Choice(key));
                }
                InvalidateRect(self.hwnd, null(), 0);
                return;
            }
        }
        if target == Target::Close {
            self.hide();
            return;
        }
        let mut change = None;
        for row in &mut self.rows {
            match (target, row) {
                (
                    Target::Toggle(id),
                    Row::Toggle { id: key, value, .. } | Row::Widget { id: key, value, .. },
                ) if id == *key => {
                    *value = !*value;
                    change = Some((id, Value::Bool(*value)));
                }
                (
                    Target::Choice(id, index),
                    Row::Choices {
                        id: key,
                        selected,
                        options,
                        ..
                    },
                ) if id == *key => {
                    if let Some(choice) = options.get(index) {
                        *selected = choice.key.clone();
                        change = Some((id, Value::Choice(selected.clone())));
                    }
                }
                (Target::Button(id), Row::Button { id: key, .. }) if id == *key => {
                    change = Some((id, Value::Action))
                }
                (Target::Button(id), Row::Widget { settings_id, .. }) if id == *settings_id => {
                    change = Some((id, Value::Action))
                }
                _ => {}
            }
        }
        if let Some((id, value)) = change {
            self.emit(id, value);
        }
        InvalidateRect(self.hwnd, null(), 0);
    }
    unsafe fn slide(&mut self, id: u32, x: f32) {
        let w = self.size()[0];
        let ratio = ((x - 28.) / (w - 92.)).clamp(0., 1.);
        let mut change = None;
        for row in &mut self.rows {
            if let Row::Slider {
                id: key,
                value,
                min,
                max,
                ..
            } = row
            {
                if *key == id && min.is_finite() && max.is_finite() && *max > *min {
                    *value = (*min + ratio * (*max - *min)).round().clamp(*min, *max);
                    change = Some(*value);
                }
            }
        }
        if let Some(value) = change {
            self.emit(id, Value::Number(value));
        }
        InvalidateRect(self.hwnd, null(), 0);
    }
    unsafe fn hit(&self, p: POINT) -> Option<Target> {
        let p = [p.x as f32 / self.dpi, p.y as f32 / self.dpi];
        self.hits
            .iter()
            .find(|h| inside(h.rect, p))
            .map(|h| h.target)
    }
    unsafe fn paint(&mut self) {
        let mut ps = zeroed();
        BeginPaint(self.hwnd, &mut ps);
        let mut rect: RECT = zeroed();
        GetClientRect(self.hwnd, &mut rect);
        let [w, h] = self.size();
        let Some(mut frame) = self
            .frame
            .take()
            .filter(|f| f.width == rect.right && f.height == rect.bottom && f.dpi == self.dpi)
            .or_else(|| Buffer::new(rect.right, rect.bottom, self.dpi, 30.))
        else {
            EndPaint(self.hwnd, &ps);
            return;
        };
        let dc = frame.dc;
        let mut pos: RECT = zeroed();
        GetWindowRect(self.hwnd, &mut pos);
        let key = format!(
            "{}:{}:{}:{}:{}:{}:{}:{}",
            pos.left,
            pos.top,
            rect.right,
            rect.bottom,
            self.dpi,
            self.surface,
            self.light,
            self.glass.borrow().revision()
        );
        let frozen = self.drag.is_some()
            && self.backdrop.as_ref().is_some_and(|b| {
                b.width == rect.right && b.height == rect.bottom && b.dpi == self.dpi
            });
        if self.backdrop.is_none() || self.key != key && !frozen {
            let brush = CreateSolidBrush(0x2e2419);
            FillRect(dc, &rect, brush);
            DeleteObject(brush);
            self.glass
                .borrow()
                .draw(dc, self.hwnd, rect.right, rect.bottom);
            let c = Canvas::new(dc, self.dpi);
            c.round([0., 0., w, h], 30., self.surface, 205, false, false);
            c.coating([4., 4., w - 4., h - 4.], 30.);
            drop(c);
            frame.bevel();
            if let Some(back) = Buffer::plain(rect.right, rect.bottom, self.dpi) {
                back.copy_from(dc);
                self.backdrop = Some(back);
            }
            self.key = key;
        } else if let Some(back) = &self.backdrop {
            frame.copy_from(back.dc);
        }
        self.scroll = self
            .scroll
            .clamp(0., (self.content_height(w) - h + 18.).max(0.));
        let white = if self.light { 0x312720 } else { 0xf8f2f1 };
        let dim = if self.light { 0x706358 } else { 0xc7bcac };
        let c = Canvas::new(dc, self.dpi);
        c.round([17., 16., 45., 44.], 14., white, 14, false, false);
        c.round([17., 16., 45., 44.], 14., white, 24, true, false);
        c.line(&[(33., 23.), (27., 30.), (33., 37.)], white, 200, 1.3);
        c.text(&self.title, [54., 14., w - 58., 46.], 13., true, white, 0);
        c.line(&[(w - 37., 25.), (w - 29., 33.)], dim, 210, 1.2);
        c.line(&[(w - 29., 25.), (w - 37., 33.)], dim, 210, 1.2);
        drop(c);
        self.hits = vec![
            Hit {
                rect: [16., 14., 46., 46.],
                target: Target::Close,
            },
            Hit {
                rect: [w - 46., 14., w - 16., 46.],
                target: Target::Close,
            },
        ];
        let saved = SaveDC(dc);
        IntersectClipRect(
            dc,
            0,
            (58. * self.dpi) as i32,
            rect.right,
            rect.bottom - (12. * self.dpi) as i32,
        );
        let c = Canvas::new(dc, self.dpi);
        let shift = |r: [f32; 4]| [r[0], r[1] - self.scroll, r[2], r[3] - self.scroll];
        c.text(
            &self.subtitle,
            shift([24., 62., w - 24., 87.]),
            11.,
            false,
            dim,
            0,
        );
        let cols = if w < 480. { 2 } else { 3 };
        let mut y = 94.;
        // Reuse Dock's grouped 16-DIP card/rim around each section.
        for (i, row) in self.rows.iter().enumerate() {
            if matches!(row, Row::Section(_)) {
                let end = y
                    + 26.
                    + self.rows[i + 1..]
                        .iter()
                        .take_while(|r| !matches!(r, Row::Section(_)))
                        .map(|r| height(r, cols))
                        .sum::<f32>();
                c.round(
                    shift([16., y + 26., w - 16., end]),
                    16.,
                    white,
                    10,
                    false,
                    false,
                );
                c.round(
                    shift([16., y + 26., w - 16., end]),
                    16.,
                    white,
                    35,
                    true,
                    false,
                );
            }
            y += height(row, cols);
        }
        y = 94.;
        let mut hits = vec![];
        for row in &self.rows {
            let rh = height(row, cols);
            match row {
                Row::Section(label) => c.text(
                    &label
                        .to_uppercase()
                        .chars()
                        .map(|c| c.to_string())
                        .collect::<Vec<_>>()
                        .join(" "),
                    shift([25., y + 4., w - 25., y + 20.]),
                    9.,
                    true,
                    dim,
                    0,
                ),
                Row::Widget {
                    id,
                    settings_id,
                    label,
                    hint,
                    value,
                } => {
                    c.text(
                        label,
                        shift([158., y + 5., w - 85., y + 27.]),
                        12.,
                        true,
                        white,
                        0,
                    );
                    c.text(
                        hint,
                        shift([158., y + 28., w - 85., y + 66.]),
                        9.5,
                        false,
                        dim,
                        0,
                    );
                    c.round(
                        shift([18., y, w - 18., y + 108.]),
                        16.,
                        white,
                        12,
                        false,
                        false,
                    );
                    let preview = shift([28., y + 10., 140., y + 98.]);
                    c.round(preview, 14., self.surface, 220, false, false);
                    c.round(preview, 14., white, 65, true, false);
                    widget_preview(&c, preview, label, self.dpi, white);
                    let settings = shift([158., y + 74., 238., y + 100.]);
                    let hover = self.pressed == Some(Target::Button(*settings_id));
                    c.round(
                        settings,
                        14.,
                        white,
                        if hover { 30 } else { 12 },
                        false,
                        false,
                    );
                    c.text("Settings", settings, 10., false, white, 1);
                    hits.push(Hit {
                        rect: settings,
                        target: Target::Button(*settings_id),
                    });
                    let r = shift([w - 61., y + 12., w - 27., y + 32.]);
                    c.round(r, 10., white, if *value { 80 } else { 20 }, false, false);
                    c.round(r, 10., white, 60, true, false);
                    let x = if *value { w - 37. } else { w - 51. };
                    c.round(
                        shift([x - 7., y + 15., x + 7., y + 29.]),
                        7.,
                        white,
                        255,
                        false,
                        false,
                    );
                    hits.push(Hit {
                        rect: shift([w - 68., y, w - 20., y + 46.]),
                        target: Target::Toggle(*id),
                    });
                }
                Row::Toggle {
                    id,
                    label,
                    hint,
                    value,
                } => {
                    c.text(
                        label,
                        shift([28., y + 5., w - 90., y + 27.]),
                        12.,
                        true,
                        white,
                        0,
                    );
                    c.text(
                        hint,
                        shift([28., y + 26., w - 90., y + 42.]),
                        9.5,
                        false,
                        dim,
                        0,
                    );
                    let r = shift([w - 61., y + 10., w - 27., y + 30.]);
                    c.round(r, 10., white, if *value { 58 } else { 20 }, false, false);
                    c.round(r, 10., white, 60, true, false);
                    let x = if *value { w - 37. } else { w - 51. };
                    c.round(
                        shift([x - 7., y + 13., x + 7., y + 27.]),
                        7.,
                        white,
                        255,
                        false,
                        false,
                    );
                    hits.push(Hit {
                        rect: shift([w - 90., y, w - 20., y + 46.]),
                        target: Target::Toggle(*id),
                    });
                }
                Row::Slider {
                    id,
                    label,
                    value,
                    min,
                    max,
                    suffix,
                } => {
                    c.text(
                        label,
                        shift([28., y + 6., w - 80., y + 28.]),
                        11.5,
                        false,
                        white,
                        0,
                    );
                    c.text(
                        &format!("{}{suffix}", value.round()),
                        shift([w - 82., y + 6., w - 28., y + 28.]),
                        10.,
                        false,
                        dim,
                        2,
                    );
                    let ratio = if *max > *min {
                        ((*value - *min) / (*max - *min)).clamp(0., 1.)
                    } else {
                        0.
                    };
                    let fill = 28. + (w - 92.) * ratio;
                    c.round(
                        shift([28., y + 35., w - 64., y + 39.]),
                        2.,
                        dim,
                        65,
                        false,
                        false,
                    );
                    c.round(
                        shift([28., y + 35., fill.max(28.1), y + 39.]),
                        2.,
                        white,
                        190,
                        false,
                        false,
                    );
                    c.round(
                        shift([fill - 6.5, y + 30.5, fill + 6.5, y + 43.5]),
                        6.5,
                        white,
                        255,
                        false,
                        false,
                    );
                    c.text(
                        &value.round().to_string(),
                        shift([w - 59., y + 24., w - 26., y + 49.]),
                        12.,
                        false,
                        white,
                        2,
                    );
                    hits.push(Hit {
                        rect: shift([22., y + 25., w - 60., y + 52.]),
                        target: Target::Slider(*id),
                    });
                }
                Row::Choices {
                    id,
                    label,
                    selected,
                    options,
                } => {
                    c.text(
                        label,
                        shift([28., y + 3., w - 28., y + 27.]),
                        12.,
                        false,
                        dim,
                        0,
                    );
                    let cell = (w - 56.) / cols as f32;
                    for (i, option) in options.iter().enumerate() {
                        let x = 28. + (i % cols) as f32 * cell;
                        let yy = y + 32. + (i / cols) as f32 * 36.;
                        let r = shift([x, yy, x + cell - 8., yy + 29.]);
                        let color = option.color.unwrap_or(self.surface);
                        let ink = if crate::contrast::luminance(color) > 0.42 {
                            0x271e18
                        } else {
                            0xfdfaf8
                        };
                        c.round(r, 9., color, 255, false, false);
                        crate::surface::swatch(&c, r, 9., &option.key, self.dpi);
                        if option.key == *selected
                            || (option.key == "custom"
                                && crate::surface::custom_color(selected).is_some())
                        {
                            c.round(r, 9., ink, 180, true, false);
                        }
                        c.text(&option.label, r, 10., option.key == *selected, ink, 1);
                        hits.push(Hit {
                            rect: r,
                            target: Target::Choice(*id, i),
                        });
                    }
                }
                Row::Button { id, label } => {
                    let r = shift([28., y + 5., w - 28., y + 37.]);
                    c.round(r, 9., white, 14, false, false);
                    c.round(r, 9., white, 35, true, false);
                    c.text(label, r, 11., false, white, 1);
                    hits.push(Hit {
                        rect: r,
                        target: Target::Button(*id),
                    });
                }
                Row::Note(text) => c.text(
                    text,
                    shift([28., y + 4., w - 28., y + 30.]),
                    9.5,
                    false,
                    dim,
                    0,
                ),
            }
            if !matches!(row, Row::Section(_) | Row::Choices { .. }) {
                c.line(
                    &[(16., y + rh - self.scroll), (w - 16., y + rh - self.scroll)],
                    white,
                    20,
                    0.5,
                );
            }
            y += rh;
        }
        for mut hit in hits {
            hit.rect[1] = hit.rect[1].max(58.);
            hit.rect[3] = hit.rect[3].min(h - 12.);
            if hit.rect[3] > hit.rect[1] {
                if self.focus == Some(hit.target) {
                    c.round(hit.rect, 7., white, 130, true, false);
                }
                self.hits.push(hit);
            }
        }
        if self.content_height(w) > h {
            let track = h - 84.;
            let thumb = (track * track / self.content_height(w)).max(24.);
            let yy =
                64. + (track - thumb) * self.scroll / (self.content_height(w) - h + 18.).max(1.);
            c.round(
                [w - 10., yy, w - 7., yy + thumb],
                1.5,
                white,
                100,
                false,
                false,
            );
            if let Some(rect) = self.scrollbar() {
                self.hits.push(Hit {
                    rect,
                    target: Target::Scroll,
                });
            }
        }
        drop(c);
        RestoreDC(dc, saved);
        frame.present(self.hwnd);
        self.frame = Some(frame);
        if let Some(shadow) = &mut self.shadow {
            shadow.sync(self.hwnd, self.dpi, 255);
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

    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<State>;
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
        WM_GETMINMAXINFO => {
            let p = &mut *(lp as *mut MINMAXINFO);
            p.ptMinTrackSize = POINT {
                x: (420. * s.dpi) as i32,
                y: (360. * s.dpi) as i32,
            };
            0
        }
        WM_NCHITTEST => {
            let mut p = point(lp);
            ScreenToClient(hwnd, &mut p);
            let [w, h] = s.size();
            crate::geometry::edge(
                (p.x as f32 / s.dpi) as i32,
                (p.y as f32 / s.dpi) as i32,
                w as i32,
                h as i32,
                8,
                20,
            ) as isize
        }
        WM_NCLBUTTONDOWN if (HTLEFT..=HTBOTTOMRIGHT).contains(&(wp as u32)) => {
            let mut r = zeroed();
            GetWindowRect(hwnd, &mut r);
            let mut p = zeroed();
            GetCursorPos(&mut p);
            s.drag = Some(Drag {
                edge: wp as u32,
                start: p,
                bounds: r,
            });
            drop(s);
            SetCapture(hwnd);
            0
        }
        WM_LBUTTONDOWN => {
            let p = point(lp);
            s.pressed = s.hit(p);
            s.focus = s.pressed;
            if let Some(Target::Slider(id)) = s.pressed {
                let x = p.x as f32 / s.dpi;
                s.slide(id, x);
            } else if s.pressed == Some(Target::Scroll) {
                if let Some(bar) = s.scrollbar() {
                    s.bar_grab = p.y as f32 / s.dpi - bar[1];
                }
            } else if s.pressed.is_none() && p.y as f32 / s.dpi < 58. {
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
            drop(s);
            SetCapture(hwnd);
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_MOUSEMOVE => {
            if let Some(d) = &s.drag {
                let mut p = zeroed();
                GetCursorPos(&mut p);
                let dx = p.x - d.start.x;
                let dy = p.y - d.start.y;
                let r = if d.edge == HTCAPTION {
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
                        [(420. * s.dpi) as i32, (360. * s.dpi) as i32],
                    )
                };
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
            } else if let Some(Target::Slider(id)) = s.pressed {
                let x = point(lp).x as f32 / s.dpi;
                s.slide(id, x);
            } else if s.pressed == Some(Target::Scroll) {
                let y = point(lp).y as f32 / s.dpi;
                s.scroll_to(y);
            }
            0
        }
        WM_LBUTTONUP => {
            let pressed = s.pressed.take();
            let moved = s.drag.take().is_some();
            s.key.clear();
            let hit = s.hit(point(lp));
            drop(s);
            ReleaseCapture();
            if !moved {
                if let Some(target) = pressed {
                    if Some(target) == hit && !matches!(target, Target::Slider(_)) {
                        if let Ok(mut s) = (*ptr).try_borrow_mut() {
                            s.activate(target);
                        }
                    }
                }
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_CAPTURECHANGED | WM_CANCELMODE => {
            s.drag = None;
            s.pressed = None;
            s.key.clear();
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_MOUSEWHEEL => {
            s.scroll -= (wp >> 16) as i16 as f32 / 120. * 48.;
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_KEYDOWN if wp == 0x1B => {
            s.hide();
            drop(s);
            ReleaseCapture();
            0
        }
        WM_KEYDOWN if wp == 0x09 => {
            let mut list: Vec<_> = s.hits.iter().map(|h| h.target).collect();
            list.dedup();
            if !list.is_empty() {
                let reverse = GetKeyState(0x10) < 0;
                let i = s.focus.and_then(|f| list.iter().position(|t| *t == f));
                let n = match i {
                    Some(i) if reverse => (i + list.len() - 1) % list.len(),
                    Some(i) => (i + 1) % list.len(),
                    None => 0,
                };
                s.focus = Some(list[n]);
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_KEYDOWN if wp == 0x0D || wp == 0x20 => {
            if let Some(f) = s.focus {
                s.activate(f);
            }
            0
        }
        WM_KEYDOWN if wp == 0x25 || wp == 0x27 => {
            if let Some(Target::Slider(id)) = s.focus {
                let mut change = None;
                for row in &mut s.rows {
                    if let Row::Slider {
                        id: key,
                        value,
                        min,
                        max,
                        ..
                    } = row
                    {
                        if *key == id {
                            *value = (*value + if wp == 0x25 { -1. } else { 1. }).clamp(*min, *max);
                            change = Some(*value);
                        }
                    }
                }
                if let Some(v) = change {
                    s.emit(id, Value::Number(v));
                }
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_KEYDOWN if wp == 0x21 || wp == 0x22 => {
            s.scroll += if wp == 0x21 { -240. } else { 240. };
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_SIZE => {
            s.key.clear();
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_DPICHANGED => {
            s.dpi = wp as u16 as f32 / 96.;
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

fn widget_preview(c: &Canvas, r: [f32; 4], label: &str, dpi: f32, ink: u32) {
    let name = label.to_lowercase();
    let key = if name.contains("clock") {
        "gradient:afterglow"
    } else {
        "gradient:aurora"
    };
    crate::surface::swatch(c, r, 14., key, dpi);
    let at = |x: f32, y: f32, w: f32, h: f32| [r[0] + x, r[1] + y, r[0] + x + w, r[1] + y + h];
    if name.contains("clock") {
        c.text(
            "MAKE TIME FOR TODAY",
            at(9., 10., 94., 10.),
            4.5,
            true,
            ink,
            0,
        );
        c.text("10:08", at(10., 24., 92., 36.), 27., false, ink, 0);
        c.text(
            "Monday, November 9",
            at(10., 65., 92., 12.),
            6.,
            true,
            ink,
            0,
        );
    } else if name.contains("calendar") {
        c.text("November 2026", at(10., 8., 92., 13.), 7., true, ink, 0);
        for i in 0..28 {
            let x = 10. + (i % 7) as f32 * 13.;
            let y = 26. + (i / 7) as f32 * 13.;
            if i == 8 {
                c.round(at(x, y, 11., 12.), 4., ink, 55, false, false);
            }
            c.text(&(i + 1).to_string(), at(x, y, 11., 12.), 6., i == 8, ink, 1);
        }
    } else if name.contains("music") {
        for (x, y, size, color) in [
            (12., 21., 34., 0x728dba),
            (67., 21., 34., 0x8465ad),
            (33., 13., 46., 0x8f83c4),
        ] {
            c.round(at(x, y, size, size), 7., color, 255, false, false);
        }
        c.text("♪", at(33., 13., 46., 46.), 25., false, ink, 1);
        c.round(at(10., 68., 92., 13.), 6., ink, 35, false, false);
        c.text("‹     ▶     ›", at(10., 68., 92., 13.), 9., true, ink, 1);
    } else if name.contains("note") {
        c.round(at(9., 9., 94., 11.), 5., ink, 20, false, false);
        for (y, color) in [(27., 0xc99365), (56., 0x9b83c4)] {
            c.round(at(9., y, 94., 25.), 7., color, 150, false, false);
            c.round(at(14., y + 5., 4., 4.), 2., ink, 180, false, false);
            c.line(
                &[(r[0] + 23., r[1] + y + 8.), (r[0] + 83., r[1] + y + 8.)],
                ink,
                120,
                1.,
            );
        }
    } else if name.contains("control") {
        for i in 0..4 {
            let x = 9. + (i % 2) as f32 * 49.;
            let y = 9. + (i / 2) as f32 * 35.;
            c.round(at(x, y, 45., 30.), 9., ink, 35, false, false);
            c.round(at(x + 7., y + 6., 12., 12.), 6., ink, 130, false, false);
            c.line(
                &[
                    (r[0] + x + 8., r[1] + y + 24.),
                    (r[0] + x + 36., r[1] + y + 24.),
                ],
                ink,
                180,
                2.,
            );
        }
    } else {
        c.round(at(8., 8., 96., 14.), 7., 0, 50, false, false);
        c.text("Search apps…", at(14., 8., 83., 14.), 5.5, false, ink, 0);
        for i in 0..8 {
            let x = 12. + (i % 4) as f32 * 24.;
            let y = 30. + (i / 4) as f32 * 26.;
            let color = [0xd3b244, 0x9060de, 0xd68c40, 0xa0bb54][i % 4];
            c.round(at(x, y, 16., 16.), 4., color, 255, false, false);
            c.line(
                &[(r[0] + x, r[1] + y + 21.), (r[0] + x + 16., r[1] + y + 21.)],
                ink,
                110,
                1.,
            );
        }
    }
}

pub const WIDGET_HEADER_TOP: f32 = 8.;
pub const WIDGET_HEADER_HEIGHT: f32 = 40.;
pub const WIDGET_TITLE_SIZE: f32 = 12.;
pub fn widget_title_visible(w: f32, h: f32) -> bool {
    w >= 420. && h >= 360.
}
pub fn widget_content_top(w: f32, h: f32) -> f32 {
    if widget_title_visible(w, h) {
        54.
    } else {
        14.
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderAction {
    Settings,
    Hide,
}
pub fn widget_header_hits(w: f32) -> [(HeaderAction, [f32; 4]); 2] {
    [
        (HeaderAction::Settings, [w - 84., 12., w - 52., 44.]),
        (HeaderAction::Hide, [w - 48., 12., w - 16., 44.]),
    ]
}
pub fn palette_choices() -> Vec<Choice> {
    crate::surface::PRESETS
        .iter()
        .map(|(k, l, c)| Choice::new(*k, *l, Some(*c)))
        .collect()
}
const SETTINGS_SVG: &str = include_str!("../assets/toolbar/5.svg");
const CLOSE_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"><path d="m6 6 12 12M18 6 6 18"/></svg>"##;
fn header_icon(kind: usize, dpi: f32, ink: u32) -> Option<crate::render::Image> {
    let px = (16. * dpi * 2.).ceil().max(1.) as u32;
    thread_local! {static PIXELS:RefCell<Vec<(usize,u32,u32,Vec<u8>)>>=const {RefCell::new(Vec::new())};}
    if let Some(pixels) = PIXELS.with(|cache| {
        cache
            .borrow()
            .iter()
            .find(|e| e.0 == kind && e.1 == px && e.2 == ink)
            .map(|e| e.3.clone())
    }) {
        return crate::render::Image::from_rgba(px, px, pixels);
    }
    let color = format!(
        "#{:02x}{:02x}{:02x}",
        ink & 255,
        (ink >> 8) & 255,
        (ink >> 16) & 255
    );
    let svg = (if kind == 0 { SETTINGS_SVG } else { CLOSE_SVG }).replace("currentColor", &color);
    let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(px, px)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(px as f32 / 24., px as f32 / 24.),
        &mut pixmap.as_mut(),
    );
    let pixels = pixmap.data().to_vec();
    PIXELS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() >= 8 {
            cache.remove(0);
        }
        cache.push((kind, px, ink, pixels.clone()));
    });
    crate::render::Image::from_rgba(px, px, pixels)
}
pub unsafe fn draw_widget_header(
    dc: HDC,
    hwnd: HWND,
    dpi: f32,
    w: f32,
    title: &str,
    ink: u32,
    _hover: Option<HeaderAction>,
    pressed: Option<HeaderAction>,
) {
    let c = Canvas::new(dc, dpi);
    let mut r: RECT = zeroed();
    GetClientRect(hwnd, &mut r);
    if widget_title_visible(
        w,
        if hwnd.is_null() {
            f32::INFINITY
        } else {
            r.bottom as f32 / dpi
        },
    ) {
        c.text(title, [20., 8., w - 100., 48.], 12., true, ink, 0);
    }
    let motion = crate::widget_header::visual(hwnd);
    for (i, (action, r)) in widget_header_hits(w).into_iter().enumerate() {
        if motion.alpha < 0.001 {
            continue;
        }
        let amount = motion.hover[i];
        let light = crate::contrast::luminance(ink) < 0.5;
        c.round(
            [r[0] + 1., r[1] + 1., r[2] - 1., r[3] - 1.],
            15.,
            0xffffff,
            (motion.alpha * amount * if light { 128. } else { 19. }).round() as u8,
            false,
            false,
        );
        if pressed == Some(action) {
            c.round(
                r,
                16.,
                ink,
                (26. * motion.alpha).round() as u8,
                false,
                false,
            );
        }
        if let Some(image) = header_icon(i, dpi, ink) {
            let icon = Canvas::new(dc, dpi);
            icon.transform_about([r[0] + 16., r[1] + 16.], 1., motion.rotation[i]);
            icon.image_alpha(
                &image,
                [r[0] + 8., r[1] + 8., r[0] + 24., r[1] + 24.],
                motion.alpha * (0.62 + amount * 0.28),
            );
        }
    }
}
pub unsafe fn retain_header_backing(frame: &mut Buffer, backing: &Buffer, hwnd: HWND, w: f32) {
    let motion = crate::widget_header::visual(hwnd);
    for (i, (_, r)) in widget_header_hits(w).into_iter().enumerate() {
        frame.retain_control_backing(
            backing,
            [r[0] + 1., r[1] + 1., r[2] - 1., r[3] - 1.],
            15.,
            motion.alpha * motion.hover[i],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rows() -> Vec<Row> {
        vec![
            Row::section("Background"),
            Row::toggle(1, "Frosted glass", "Blend with wallpaper", true),
            Row::slider(2, "Glass tint", 18., 0., 100., "%"),
            Row::choices(
                3,
                "Theme",
                "lumen",
                vec![
                    Choice::new("lumen", "Lumen", None),
                    Choice::new("paper", "Paper", None),
                ],
            ),
            Row::section("Window"),
            Row::toggle(4, "Example toggle", "Example description", false),
        ]
    }
    #[test]
    fn dock_metrics_and_header_use_the_outline_gear() {
        assert_eq!(height(&Row::section("Background"), 3), 18.);
        assert!(SETTINGS_SVG.contains("cx=\"12\" cy=\"12\" r=\"3\""));
        for (_, r) in widget_header_hits(240.) {
            assert_eq!(r[2] - r[0], 32.);
            assert_eq!(r[3] - r[1], 32.);
            assert!(r[1] >= WIDGET_HEADER_TOP && r[3] <= WIDGET_HEADER_TOP + WIDGET_HEADER_HEIGHT);
        }
    }
    #[test]
    fn hidden_settings_render_scroll_dispatch_and_release_buffers_at_multiple_dpi() {
        let _lock = crate::native_ui_test_lock();
        let _runtime = crate::render::Runtime::start().unwrap();
        unsafe {
            let glass = Rc::new(RefCell::new(Glass::new(13., 1.)));
            let revision = glass.borrow().revision();
            let mut host = Host::create(
                null_mut(),
                "Test settings",
                "Changes apply only to Test.",
                glass.clone(),
                rows(),
                WM_APP + 84,
            )
            .unwrap();
            let hwnd = host.hwnd();
            for dpi in [1., 1.25, 1.5, 2.] {
                host.state.borrow_mut().dpi = dpi;
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    0,
                    0,
                    (420. * dpi) as i32,
                    (360. * dpi) as i32,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
                let mut s = host.state.borrow_mut();
                s.scroll = 10000.;
                s.paint();
                assert!(s.scroll > 0.);
                assert!(s
                    .hits
                    .iter()
                    .all(|h| h.target == Target::Close || h.rect[1] >= 58. && h.rect[3] <= 348.));
                assert!(s
                    .frame
                    .as_ref()
                    .unwrap()
                    .snapshot()
                    .chunks_exact(4)
                    .any(|p| p[3] > 200));
            }
            {
                let mut s = host.state.borrow_mut();
                s.activate(Target::Toggle(1));
                s.slide(2, 28.);
                s.slide(2, 10000.);
                s.activate(Target::Choice(3, 1));
            }
            let changes = host.take_changes();
            assert_eq!(changes.len(), 3);
            assert!(changes.contains(&Change {
                id: 1,
                value: Value::Bool(false)
            }));
            assert!(changes.contains(&Change {
                id: 2,
                value: Value::Number(100.)
            }));
            assert!(changes.contains(&Change {
                id: 3,
                value: Value::Choice("paper".into())
            }));
            assert_eq!(
                glass.borrow().revision(),
                revision,
                "UI controls must not mutate the shared wallpaper/blur source"
            );
            host.hide();
            {
                let s = host.state.borrow();
                assert!(s.frame.is_none() && s.backdrop.is_none() && s.shadow.is_none());
            }
            host.set_rows(rows());
            host.render_for_test();
            assert_eq!(host.hwnd(), hwnd);
            assert!(!host.is_visible());
        }
    }
    #[test]
    fn shared_header_cache_survives_successive_renderer_sessions() {
        let _lock = crate::native_ui_test_lock();
        for _ in 0..2 {
            let _runtime = crate::render::Runtime::start().unwrap();
            unsafe {
                let buffer = Buffer::plain(240, 56, 1.).unwrap();
                let icon = header_icon(0, 1., 0xffffff).unwrap();
                let canvas = Canvas::new(buffer.dc, 1.);
                canvas.image_alpha(&icon, [8., 8., 24., 24.], 1.);
                drop(canvas);
                assert!(buffer.snapshot().chunks_exact(4).any(|p| p[0] > 0));
            }
        }
    }
}
