//! App-owned HSV wheel. Apply commits a COLORREF; Cancel leaves caller state
//! untouched. No persistent graphics handles, external picker or screen capture.
use crate::{
    compositor::{Buffer, WindowShadow},
    model::wide,
    render::{Canvas, Image},
};
use std::{
    cell::RefCell,
    mem::zeroed,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{HiDpi::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};
#[derive(Clone, Copy)]
pub struct Hsv {
    pub h: f32,
    pub s: f32,
    pub v: f32,
}
pub fn rgb(h: Hsv) -> u32 {
    let h = Hsv {
        h: h.h.rem_euclid(360.),
        s: h.s.clamp(0., 1.),
        v: h.v.clamp(0., 1.),
    };
    let c = h.v * h.s;
    let x = c * (1. - ((h.h / 60.) % 2. - 1.).abs());
    let m = h.v - c;
    let c = match (h.h / 60.) as u32 {
        0 => [c, x, 0.],
        1 => [x, c, 0.],
        2 => [0., c, x],
        3 => [0., x, c],
        4 => [x, 0., c],
        _ => [c, 0., x],
    }
    .map(|v| ((v + m) * 255.).round() as u32);
    c[0] | c[1] << 8 | c[2] << 16
}
pub fn hsv(color: u32) -> Hsv {
    let [r, g, b] = [
        (color & 255) as f32 / 255.,
        ((color >> 8) & 255) as f32 / 255.,
        ((color >> 16) & 255) as f32 / 255.,
    ];
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d < 0.00001 {
        0.
    } else if max == r {
        60. * ((g - b) / d).rem_euclid(6.)
    } else if max == g {
        60. * ((b - r) / d + 2.)
    } else {
        60. * ((r - g) / d + 4.)
    };
    Hsv {
        h,
        s: if max == 0. { 0. } else { d / max },
        v: max,
    }
}
struct State {
    hwnd: HWND,
    dpi: f32,
    color: Hsv,
    wheel: Option<Image>,
    square: Option<(u32, Image)>,
    frame: Option<Buffer>,
    shadow: Option<WindowShadow>,
    drag: u8,
    pressed: u8,
    done: bool,
    apply: bool,
}
unsafe fn wheel() -> Option<Image> {
    let size = 256;
    let mut pixels = Vec::with_capacity(size * size * 4);
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 + 0.5 - 128.;
            let dy = y as f32 + 0.5 - 128.;
            let d = dx.hypot(dy);
            let a = ((128. - d).min(d - 99.) + 0.5).clamp(0., 1.);
            let color = rgb(Hsv {
                h: dy.atan2(dx).to_degrees().rem_euclid(360.),
                s: 1.,
                v: 1.,
            });
            let alpha = (a * 255.).round() as u8;
            pixels.extend_from_slice(&[
                ((color & 255) as f32 * a).round() as u8,
                (((color >> 8) & 255) as f32 * a).round() as u8,
                (((color >> 16) & 255) as f32 * a).round() as u8,
                alpha,
            ]);
        }
    }
    Image::from_rgba(256, 256, pixels)
}
impl State {
    unsafe fn paint(&mut self) {
        let mut ps = zeroed();
        BeginPaint(self.hwnd, &mut ps);
        let w = (420. * self.dpi).round() as i32;
        let h = (430. * self.dpi).round() as i32;
        let Some(mut frame) = self
            .frame
            .take()
            .or_else(|| Buffer::new(w, h, self.dpi, 24.))
        else {
            EndPaint(self.hwnd, &ps);
            return;
        };
        let brush = CreateSolidBrush(0x29221f);
        let r = RECT {
            left: 0,
            top: 0,
            right: w,
            bottom: h,
        };
        FillRect(frame.dc, &r, brush);
        DeleteObject(brush);
        {
            let c = Canvas::new(frame.dc, self.dpi);
            c.reflection_gradient([0., 0., 420., 430.], 0.7, false);
            c.coating([4., 4., 416., 426.], 24.);
            c.text(
                "Custom surface color",
                [24., 14., 365., 44.],
                13.,
                true,
                0xf8f4f2,
                0,
            );
            c.line(&[(383., 24.), (393., 34.)], 0xd8d0cb, 230, 1.2);
            c.line(&[(393., 24.), (383., 34.)], 0xd8d0cb, 230, 1.2);
            if let Some(wheel) = &self.wheel {
                c.image(wheel, [90., 65., 330., 305.]);
            }
            let key = (self.color.h * 10.).round() as u32;
            if self.square.as_ref().is_none_or(|(k, _)| *k != key) {
                let mut pixels = Vec::with_capacity(128 * 128 * 4);
                for y in 0..128 {
                    for x in 0..128 {
                        let color = rgb(Hsv {
                            h: self.color.h,
                            s: x as f32 / 127.,
                            v: 1. - y as f32 / 127.,
                        });
                        pixels.extend_from_slice(&[
                            (color & 255) as u8,
                            ((color >> 8) & 255) as u8,
                            ((color >> 16) & 255) as u8,
                            255,
                        ]);
                    }
                }
                self.square = Image::from_rgba(128, 128, pixels).map(|image| (key, image));
            }
            if let Some((_, image)) = &self.square {
                c.image(image, [146., 121., 274., 249.]);
            }
            let hue = self.color.h.to_radians();
            let x = 210. + hue.cos() * 107.;
            let y = 185. + hue.sin() * 107.;
            c.round(
                [x - 7., y - 7., x + 7., y + 7.],
                7.,
                0xffffff,
                255,
                true,
                false,
            );
            let x = 146. + self.color.s * 128.;
            let y = 121. + (1. - self.color.v) * 128.;
            c.round([x - 5., y - 5., x + 5., y + 5.], 5., 0, 220, true, false);
            c.round(
                [x - 6.5, y - 6.5, x + 6.5, y + 6.5],
                6.5,
                0xffffff,
                255,
                true,
                false,
            );
            let color = rgb(self.color);
            c.round([28., 326., 66., 354.], 7., color, 255, false, false);
            c.text(
                &crate::surface::custom_key(color)[7..],
                [80., 326., 390., 354.],
                13.,
                true,
                0xf7f5f4,
                0,
            );
            c.text(
                "Hue · Saturation · Brightness",
                [28., 299., 392., 321.],
                10.,
                false,
                0xbdb5b0,
                1,
            );
            for (r, label, active) in [
                ([24., 376., 194., 412.], "Cancel", self.pressed == 1),
                ([208., 376., 396., 412.], "Apply color", self.pressed == 2),
            ] {
                c.round(r, 12., 0xffffff, if active { 48 } else { 20 }, false, false);
                c.round(r, 12., 0xffffff, 45, true, false);
                c.text(label, r, 12., true, 0xf7f5f4, 1);
            }
        }
        frame.bevel();
        frame.present(self.hwnd);
        if self.shadow.is_none() {
            self.shadow = WindowShadow::new(self.hwnd);
        }
        if let Some(s) = &mut self.shadow {
            s.sync(self.hwnd, self.dpi, 255);
        }
        self.frame = Some(frame);
        EndPaint(self.hwnd, &ps);
    }
    unsafe fn update(&mut self, p: [f32; 2]) {
        if self.drag == 1 {
            self.color.h = (p[1] - 185.)
                .atan2(p[0] - 210.)
                .to_degrees()
                .rem_euclid(360.);
        }
        if self.drag == 2 {
            self.color.s = ((p[0] - 146.) / 128.).clamp(0., 1.);
            self.color.v = (1. - (p[1] - 121.) / 128.).clamp(0., 1.);
        }
        InvalidateRect(self.hwnd, null(), 0);
    }
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let cs = &*(lp as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
        return 1;
    }
    if msg == WM_NCCALCSIZE || msg == WM_ERASEBKGND {
        return if msg == WM_ERASEBKGND { 1 } else { 0 };
    }
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<State>;
    if pointer.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let Ok(mut s) = (*pointer).try_borrow_mut() else {
        return DefWindowProcW(hwnd, msg, wp, lp);
    };
    let p = [lp as i16 as f32 / s.dpi, (lp >> 16) as i16 as f32 / s.dpi];
    match msg {
        WM_PAINT => {
            s.paint();
            0
        }
        WM_CLOSE => {
            s.done = true;
            0
        }
        WM_KEYDOWN => {
            match wp as u32 {
                0x1b => s.done = true,
                0x0d => {
                    s.apply = true;
                    s.done = true
                }
                0x25 => s.color.h = (s.color.h - 1.).rem_euclid(360.),
                0x27 => s.color.h = (s.color.h + 1.).rem_euclid(360.),
                0x26 => s.color.v = (s.color.v + 0.01).min(1.),
                0x28 => s.color.v = (s.color.v - 0.01).max(0.),
                _ => {}
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_NCHITTEST => {
            let mut p = POINT {
                x: lp as i16 as i32,
                y: (lp >> 16) as i16 as i32,
            };
            ScreenToClient(hwnd, &mut p);
            if p.y as f32 / s.dpi < 48. && p.x as f32 / s.dpi < 370. {
                HTCAPTION as isize
            } else {
                HTCLIENT as isize
            }
        }
        WM_LBUTTONDOWN => {
            let d = (p[0] - 210.).hypot(p[1] - 185.);
            s.drag = if (92. ..=122.).contains(&d) {
                1
            } else if p[0] >= 146. && p[0] <= 274. && p[1] >= 121. && p[1] <= 249. {
                2
            } else {
                0
            };
            s.pressed = if p[1] >= 376. && p[1] <= 412. {
                if p[0] >= 24. && p[0] <= 194. {
                    1
                } else if p[0] >= 208. && p[0] <= 396. {
                    2
                } else {
                    0
                }
            } else if p[0] >= 370. && p[1] < 48. {
                3
            } else {
                0
            };
            s.update(p);
            drop(s);
            SetCapture(hwnd);
            0
        }
        WM_MOUSEMOVE => {
            if s.drag != 0 {
                s.update(p);
            }
            0
        }
        WM_LBUTTONUP => {
            let pressed = s.pressed;
            s.pressed = 0;
            s.drag = 0;
            if pressed == 1 && (376. ..=412.).contains(&p[1]) && (24. ..=194.).contains(&p[0]) {
                s.done = true;
            }
            if pressed == 2 && (376. ..=412.).contains(&p[1]) && (208. ..=396.).contains(&p[0]) {
                s.done = true;
                s.apply = true;
            }
            if pressed == 3 && (0. ..48.).contains(&p[1]) && (370. ..=416.).contains(&p[0]) {
                s.done = true;
            }
            drop(s);
            ReleaseCapture();
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_CAPTURECHANGED | WM_CANCELMODE => {
            s.drag = 0;
            s.pressed = 0;
            0
        }
        WM_DPICHANGED => {
            s.dpi = (wp as u16) as f32 / 96.;
            s.frame = None;
            s.square = None;
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
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_NCDESTROY => {
            s.done = true;
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
pub unsafe fn choose(owner: HWND, initial: u32) -> Option<u32> {
    choose_impl(owner, initial, None)
}
unsafe fn choose_impl(owner: HWND, initial: u32, test_action: Option<bool>) -> Option<u32> {
    let class = wide("LumenNativeColorWheel");
    let instance = GetModuleHandleW(null());
    let wc = WNDCLASSW {
        lpfnWndProc: Some(proc),
        hInstance: instance,
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        lpszClassName: class.as_ptr(),
        ..zeroed()
    };
    RegisterClassW(&wc);
    let dpi = (GetDpiForWindow(owner) as f32 / 96.).max(1.);
    let w = (420. * dpi).round() as i32;
    let h = (430. * dpi).round() as i32;
    let mut r: RECT = zeroed();
    GetWindowRect(owner, &mut r);
    let mut info: MONITORINFO = zeroed();
    info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(
        MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST),
        &mut info,
    );
    let x = (r.left + (r.right - r.left - w) / 2).clamp(
        info.rcWork.left,
        (info.rcWork.right - w).max(info.rcWork.left),
    );
    let y = (r.top + (r.bottom - r.top - h) / 2).clamp(
        info.rcWork.top,
        (info.rcWork.bottom - h).max(info.rcWork.top),
    );
    let mut state = Box::new(RefCell::new(State {
        hwnd: null_mut(),
        dpi,
        color: hsv(initial),
        wheel: wheel(),
        square: None,
        frame: None,
        shadow: None,
        drag: 0,
        pressed: 0,
        done: false,
        apply: false,
    }));
    let hwnd = CreateWindowExW(
        WS_EX_LAYERED | WS_EX_TOOLWINDOW,
        class.as_ptr(),
        wide("Custom surface color").as_ptr(),
        WS_POPUP,
        if test_action.is_some() { -20000 } else { x },
        if test_action.is_some() { -20000 } else { y },
        w,
        h,
        owner,
        null_mut(),
        instance,
        &mut *state as *mut _ as _,
    );
    if hwnd.is_null() {
        return None;
    }
    state.borrow_mut().hwnd = hwnd;
    SetWindowTextW(hwnd, wide("Custom surface color").as_ptr());
    let enabled = IsWindowEnabled(owner) != 0;
    EnableWindow(owner, 0);
    if let Some(apply) = test_action {
        SendMessageW(hwnd, WM_PAINT, 0, 0);
        let mut s = state.borrow_mut();
        s.apply = apply;
        s.done = true;
    } else {
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        SetFocus(hwnd);
        InvalidateRect(hwnd, null(), 0);
    }
    let mut msg: MSG = zeroed();
    while !state.borrow().done {
        let result = GetMessageW(&mut msg, null_mut(), 0, 0);
        if result <= 0 {
            if result == 0 {
                PostQuitMessage(msg.wParam as i32);
            }
            break;
        }
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    if IsWindow(hwnd) != 0 {
        DestroyWindow(hwnd);
    }
    if enabled && IsWindow(owner) != 0 {
        EnableWindow(owner, 1);
        if test_action.is_none() {
            SetForegroundWindow(owner);
        }
    }
    let s = state.borrow();
    s.apply.then(|| rgb(s.color))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hidden_picker_cancel_apply_and_graphics_teardown_preserve_its_owner() {
        let _lock = crate::native_ui_test_lock();
        unsafe {
            for _ in 0..2 {
                let _runtime = crate::render::Runtime::start().unwrap();
                let owner = CreateWindowExW(
                    WS_EX_TOOLWINDOW,
                    wide("STATIC").as_ptr(),
                    wide("Color owner test").as_ptr(),
                    WS_POPUP,
                    -20000,
                    -20000,
                    420,
                    400,
                    null_mut(),
                    null_mut(),
                    GetModuleHandleW(null()),
                    null(),
                );
                assert!(!owner.is_null());
                let mut before: RECT = zeroed();
                GetWindowRect(owner, &mut before);
                assert_eq!(choose_impl(owner, 0x8f5a34, Some(false)), None);
                assert!(IsWindowEnabled(owner) != 0);
                assert_eq!(choose_impl(owner, 0x8f5a34, Some(true)), Some(0x8f5a34));
                let mut after: RECT = zeroed();
                GetWindowRect(owner, &mut after);
                assert_eq!(
                    [before.left, before.top, before.right, before.bottom],
                    [after.left, after.top, after.right, after.bottom]
                );
                assert_eq!(IsWindowVisible(owner), 0);
                DestroyWindow(owner);
            }
        }
    }
    #[test]
    fn hue_saturation_value_roundtrip_preserves_selected_colors() {
        for r in [0, 17, 128, 255] {
            for g in [0, 63, 181, 255] {
                for b in [0, 45, 217, 255] {
                    let c = r | g << 8 | b << 16;
                    assert_eq!(rgb(hsv(c)), c);
                }
            }
        }
        assert_eq!(
            rgb(Hsv {
                h: 0.,
                s: 1.,
                v: 1.
            }),
            0xff
        );
        assert_eq!(
            rgb(Hsv {
                h: 120.,
                s: 1.,
                v: 1.
            }),
            0xff00
        );
    }
}
