//! Windows notification-area integration. No startup registration.
use std::{
    mem::{size_of, zeroed},
    path::Path,
    ptr::{null, null_mut},
};
use windows_sys::{
    core::GUID,
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        UI::{Shell::*, WindowsAndMessaging::*},
    },
};
pub const MESSAGE: u32 = WM_APP + 9;
pub struct Tray {
    data: NOTIFYICONDATAW,
    icon: HICON,
}
impl Tray {
    pub unsafe fn new(hwnd: HWND, root: &Path) -> Option<Self> {
        let rgba = image::open(root.join("resources/tray.png"))
            .ok()?
            .into_rgba8();
        let image = image::imageops::resize(&rgba, 32, 32, image::imageops::FilterType::Lanczos3);
        let mut pixels = image.into_raw();
        let mut mask = vec![0u8; 32 * 4];
        for (i, p) in pixels.chunks_exact_mut(4).enumerate() {
            for c in 0..3 {
                p[c] = ((p[c] as u32 * p[3] as u32 + 127) / 255) as u8;
            }
            p.swap(0, 2);
            if p[3] == 0 {
                mask[(i / 32) * 4 + (i % 32) / 8] |= 0x80 >> ((i % 32) % 8);
            }
        }
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = 32;
        info.bmiHeader.biHeight = -32;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        let mut bits = null_mut();
        let color = CreateDIBSection(null_mut(), &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if color.is_null() {
            return None;
        }
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u8, pixels.len());
        let mask = CreateBitmap(32, 32, 1, 1, mask.as_ptr() as _);
        let icon_info = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&icon_info);
        DeleteObject(color);
        DeleteObject(mask);
        if icon.is_null() {
            return None;
        }
        let mut data: NOTIFYICONDATAW = zeroed();
        data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = hwnd;
        data.uID = 1;
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_GUID;
        data.uCallbackMessage = MESSAGE;
        data.hIcon = icon;
        data.guidItem = GUID::from_u128(0xc7c6f41d_f516_4c86_9f3e_8db41829601a);
        let tip = crate::model::wide("Lumen · Native App Dock");
        data.szTip[..tip.len().min(127)].copy_from_slice(&tip[..tip.len().min(127)]);
        let tray = Self { data, icon };
        if !tray.add() {
            return None;
        }
        SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, icon as isize);
        SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, icon as isize);
        Some(tray)
    }
    pub unsafe fn add(&self) -> bool {
        let added = Shell_NotifyIconW(NIM_ADD, &self.data) != 0;
        eprintln!("native-tray registered={added}");
        added
    }
    pub unsafe fn menu(&self) -> u32 {
        let menu = CreatePopupMenu();
        for (id, label) in [(8, "General Settings / Widget Manager"), (3, "Quit Lumen")] {
            AppendMenuW(menu, MF_STRING, id, crate::model::wide(label).as_ptr());
        }
        let mut point: POINT = zeroed();
        GetCursorPos(&mut point);
        SetForegroundWindow(self.data.hWnd);
        let result = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            self.data.hWnd,
            null(),
        );
        DestroyMenu(menu);
        PostMessageW(self.data.hWnd, WM_NULL, 0, 0);
        result as u32
    }
}
impl Drop for Tray {
    fn drop(&mut self) {
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &self.data);
            DestroyIcon(self.icon);
        }
    }
}
