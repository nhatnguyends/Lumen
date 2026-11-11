//! Small app-owned text prompt. Only Save commits; Enter/Escape work in the edit.
use crate::{compositor::Buffer, model::wide, render::Canvas};
use std::{
    mem::zeroed,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::{EM_LIMITTEXT, EM_SETSEL},
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::{EnableWindow, IsWindowEnabled, SetFocus},
        WindowsAndMessaging::*,
    },
};
struct State {
    edit: HWND,
    font: HFONT,
    done: bool,
    result: Option<String>,
    pressed: i32,
    dpi: f32,
    title: String,
    frame: Option<Buffer>,
    brush: HBRUSH,
}
pub unsafe fn choose(owner: HWND, title: &str, initial: &str) -> Option<String> {
    choose_impl(owner, title, initial, None)
}
unsafe fn choose_impl(
    owner: HWND,
    title: &str,
    initial: &str,
    test_action: Option<bool>,
) -> Option<String> {
    let class = wide("LumenNativeTextPrompt");
    let instance = GetModuleHandleW(null());
    let wc = WNDCLASSW {
        lpfnWndProc: Some(proc),
        hInstance: instance,
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        lpszClassName: class.as_ptr(),
        ..zeroed()
    };
    RegisterClassW(&wc);
    let dpi = GetDpiForWindow(owner).max(96) as f32 / 96.;
    let mut r: RECT = zeroed();
    GetWindowRect(owner, &mut r);
    let mut state = Box::new(State {
        edit: null_mut(),
        font: null_mut(),
        done: false,
        result: None,
        pressed: 0,
        dpi,
        title: title.into(),
        frame: None,
        brush: CreateSolidBrush(0x332c29),
    });
    let hwnd = CreateWindowExW(
        WS_EX_TOOLWINDOW,
        class.as_ptr(),
        wide(title).as_ptr(),
        WS_POPUP,
        if test_action.is_some() {
            -20000
        } else {
            r.left + ((r.right - r.left) - (360. * dpi) as i32) / 2
        },
        if test_action.is_some() {
            -20000
        } else {
            r.top + 60
        },
        (360. * dpi) as i32,
        (174. * dpi) as i32,
        owner,
        null_mut(),
        instance,
        &mut *state as *mut _ as _,
    );
    if hwnd.is_null() {
        DeleteObject(state.brush);
        return None;
    }
    // A native EDIT needs a regular composited parent. UpdateLayeredWindow
    // does not compose child HWNDs reliably on every Windows graphics stack.
    SetWindowRgn(
        hwnd,
        CreateRoundRectRgn(
            0,
            0,
            (360. * dpi) as i32 + 1,
            (174. * dpi) as i32 + 1,
            (44. * dpi) as i32,
            (44. * dpi) as i32,
        ),
        0,
    );
    state.font = CreateFontW(
        -(15. * dpi) as i32,
        0,
        0,
        0,
        400,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        OUT_DEFAULT_PRECIS as u32,
        CLIP_DEFAULT_PRECIS as u32,
        ANTIALIASED_QUALITY as u32,
        DEFAULT_PITCH as u32,
        wide("Segoe UI").as_ptr(),
    );
    state.edit = CreateWindowExW(
        0,
        wide("EDIT").as_ptr(),
        wide(initial).as_ptr(),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL as u32,
        (24. * dpi) as i32,
        (64. * dpi) as i32,
        (312. * dpi) as i32,
        (28. * dpi) as i32,
        hwnd,
        null_mut(),
        instance,
        null(),
    );
    SendMessageW(state.edit, WM_SETFONT, state.font as usize, 1);
    SendMessageW(state.edit, EM_LIMITTEXT, 80, 0);
    SendMessageW(state.edit, EM_SETSEL, 0, -1);
    let enabled = IsWindowEnabled(owner) != 0;
    EnableWindow(owner, 0);
    if let Some(apply) = test_action {
        SendMessageW(hwnd, WM_PAINT, 0, 0);
        let frame = state.frame.as_ref().expect("Prompt paints its own chrome");
        assert!(frame.snapshot().chunks_exact(4).any(|p| p[3] > 200));
        if apply {
            commit(&mut state);
        } else {
            state.done = true;
        }
    } else {
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        SetFocus(state.edit);
        InvalidateRect(hwnd, null(), 0);
        UpdateWindow(hwnd);
    }
    let mut msg: MSG = zeroed();
    while !state.done {
        let code = GetMessageW(&mut msg, null_mut(), 0, 0);
        if code <= 0 {
            if code == 0 {
                PostQuitMessage(msg.wParam as i32);
            }
            break;
        }
        if msg.message == WM_KEYDOWN && matches!(msg.wParam as u32, 0x0d | 0x1b) {
            if msg.wParam == 0x0d {
                commit(&mut state);
            } else {
                state.done = true;
            }
            continue;
        }
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    DestroyWindow(hwnd);
    state.frame.take();
    if !state.font.is_null() {
        DeleteObject(state.font);
    }
    DeleteObject(state.brush);
    if enabled && IsWindow(owner) != 0 {
        EnableWindow(owner, 1);
        SetForegroundWindow(owner);
    }
    state.result.take()
}
unsafe fn commit(s: &mut State) {
    let mut text = [0u16; 256];
    let n = GetWindowTextW(s.edit, text.as_mut_ptr(), 256).max(0) as usize;
    let value = String::from_utf16_lossy(&text[..n]).trim().to_string();
    if !value.is_empty() {
        s.result = Some(value);
        s.done = true;
    }
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        SetWindowLongPtrW(
            hwnd,
            GWLP_USERDATA,
            (*(lp as *const CREATESTRUCTW)).lpCreateParams as isize,
        );
        return 1;
    }
    let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
    if p.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let s = &mut *p;
    match msg {
        WM_PAINT => {
            let mut ps = zeroed();
            let target = BeginPaint(hwnd, &mut ps);
            let mut r: RECT = zeroed();
            GetClientRect(hwnd, &mut r);
            if let Some(mut f) = Buffer::new(r.right, r.bottom, s.dpi, 22.) {
                let brush = CreateSolidBrush(0x28232d);
                FillRect(f.dc, &r, brush);
                DeleteObject(brush);
                let c = Canvas::new(f.dc, s.dpi);
                c.round([0., 0., 360., 174.], 22., 0x28232d, 255, false, false);
                c.text(&s.title, [24., 14., 336., 48.], 14., true, 0xf6f3f8, 0);
                for (label, r) in [
                    ("Cancel", [24., 118., 173., 154.]),
                    ("Save", [187., 118., 336., 154.]),
                ] {
                    c.round(r, 16., 0xffffff, 26, false, false);
                    c.text(label, r, 12., true, 0xf6f3f8, 1);
                }
                drop(c);
                f.bevel();
                BitBlt(target, 0, 0, r.right, r.bottom, f.dc, 0, 0, SRCCOPY);
                s.frame = Some(f);
            }
            EndPaint(hwnd, &ps);
            0
        }
        WM_ERASEBKGND => 1,
        WM_CTLCOLOREDIT => {
            SetTextColor(wp as HDC, 0xf6f3f8);
            SetBkColor(wp as HDC, 0x332c29);
            s.brush as isize
        }
        WM_LBUTTONDOWN => {
            let x = lp as i16 as f32 / s.dpi;
            let y = (lp >> 16) as i16 as f32 / s.dpi;
            s.pressed = if (118.0..154.).contains(&y) {
                if (24.0..173.).contains(&x) {
                    1
                } else if (187.0..336.).contains(&x) {
                    2
                } else {
                    0
                }
            } else {
                0
            };
            0
        }
        WM_LBUTTONUP => {
            let x = lp as i16 as f32 / s.dpi;
            let y = (lp >> 16) as i16 as f32 / s.dpi;
            if (118.0..154.).contains(&y) {
                if s.pressed == 1 && (24.0..173.).contains(&x) {
                    s.done = true;
                }
                if s.pressed == 2 && (187.0..336.).contains(&x) {
                    commit(s);
                }
            }
            s.pressed = 0;
            0
        }
        WM_CLOSE => {
            s.done = true;
            0
        }
        WM_NCDESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rename_prompt_renders_and_apply_cancel_restore_owner() {
        let _lock = crate::native_ui_test_lock();
        let _runtime = crate::render::Runtime::start().unwrap();
        unsafe {
            let owner = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                wide("STATIC").as_ptr(),
                wide("Prompt owner").as_ptr(),
                WS_POPUP,
                -20000,
                -20000,
                420,
                560,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            );
            assert!(!owner.is_null());
            assert_eq!(
                choose_impl(owner, "Rename control", "Volume", Some(false)),
                None
            );
            assert!(IsWindowEnabled(owner) != 0);
            assert_eq!(
                choose_impl(owner, "Rename control", "Volume", Some(true)),
                Some("Volume".into())
            );
            assert!(IsWindowEnabled(owner) != 0);
            DestroyWindow(owner);
        }
    }
}
