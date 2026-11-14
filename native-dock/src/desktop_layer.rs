//! Keep widget and fused decoration HWNDs below application windows immediately,
//! including activation/drag requests; a periodic pin alone permits visible jumps.
use std::ptr::{null, null_mut};
use windows_sys::Win32::{
    Foundation::*,
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};
const ID: usize = 0x4c44534b;
fn property() -> Vec<u16> {
    crate::model::wide("Lumen.DesktopLayer")
}
pub unsafe fn contains(hwnd: HWND) -> bool {
    GetPropW(hwnd, property().as_ptr()) as usize == 1
}
pub unsafe fn register(hwnd: HWND, enabled: bool) {
    if contains(hwnd)==enabled {return;}
    if enabled {
        SetPropW(hwnd, property().as_ptr(), 1usize as HANDLE);
        SetWindowSubclass(hwnd, Some(proc), ID, 0);
    } else {
        RemovePropW(hwnd, property().as_ptr());
        RemoveWindowSubclass(hwnd, Some(proc), ID);
    }
}
unsafe fn anchor(hwnd: HWND) -> HWND {
    let mut current = GetTopWindow(null_mut());
    let mut last = HWND_BOTTOM;
    for _ in 0..4096 {
        if current.is_null() {
            break;
        }
        let mut name = [0u16; 128];
        let len = GetClassNameW(current, name.as_mut_ptr(), 128).max(0) as usize;
        let class = String::from_utf16_lossy(&name[..len]);
        if current != hwnd
            && IsWindowVisible(current) != 0
            && !contains(current)
            && !matches!(
                class.as_str(),
                "Progman" | "WorkerW" | "LumenNativeGlassShadow" | "LumenNativeFusionSurface"
            )
            && GetWindowLongPtrW(current, GWL_EXSTYLE) & WS_EX_TOPMOST as isize == 0
        {
            last = current;
        }
        current = GetWindow(current, GW_HWNDNEXT);
    }
    last
}
pub unsafe fn pin(hwnd: HWND) {
    SetWindowPos(
        hwnd,
        anchor(hwnd),
        0,
        0,
        0,
        0,
        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
    );
}
unsafe extern "system" fn proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match msg {
        WM_WINDOWPOSCHANGING => {
            let position = &mut *(lp as *mut WINDOWPOS);
            if position.flags & SWP_NOZORDER == 0 {
                // A bridge can deliberately sit behind its own member. Preserve
                // that ordering, while rejecting TOP/TOPMOST/application anchors.
                if position.hwndInsertAfter == hwnd || !contains(position.hwndInsertAfter) {
                    position.hwndInsertAfter = anchor(hwnd);
                }
                position.flags |= SWP_NOACTIVATE | SWP_NOOWNERZORDER;
            }
        }
        WM_NCDESTROY => {
            register(hwnd, false);
        }
        _ => {}
    }
    DefSubclassProc(hwnd, msg, wp, lp)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn activation_and_topmost_requests_keep_widgets_below_applications() {
        let _lock = crate::native_ui_test_lock();
        unsafe {
            let make = || {
                CreateWindowExW(
                    WS_EX_TOOLWINDOW,
                    crate::model::wide("STATIC").as_ptr(),
                    crate::model::wide("Lumen desktop-layer test").as_ptr(),
                    WS_POPUP,
                    -20000,
                    -20000,
                    20,
                    20,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    null(),
                )
            };
            let app = make();
            let widget = make();
            let bridge = make();
            assert!(!app.is_null() && !widget.is_null() && !bridge.is_null());
            for h in [app, widget, bridge] {
                ShowWindow(h, SW_SHOWNOACTIVATE);
            }
            register(widget, true);
            register(bridge, true);
            pin(widget);
            SetWindowPos(widget, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
            assert_eq!(
                GetWindowLongPtrW(widget, GWL_EXSTYLE) & WS_EX_TOPMOST as isize,
                0
            );
            let mut cursor = GetWindow(app, GW_HWNDNEXT);
            let mut below = false;
            for _ in 0..4096 {
                if cursor.is_null() {
                    break;
                }
                if cursor == widget {
                    below = true;
                    break;
                }
                cursor = GetWindow(cursor, GW_HWNDNEXT);
            }
            assert!(below, "widget must remain below a normal application");
            SetWindowPos(
                bridge,
                widget,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            assert_eq!(GetWindow(bridge, GW_HWNDPREV), widget);
            // Keep keyboard focus possible for Dock search; constrain stacking,
            // rather than making all widget input non-activating.
            for h in [bridge, widget, app] {
                DestroyWindow(h);
            }
        }
    }
}
