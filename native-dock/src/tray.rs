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
    icon_normal: HICON,
    icon_badge: HICON,
    update_available: bool,
    registered: bool,
    use_guid: bool,
}
unsafe fn create_hicon(rgba: &[u8]) -> Option<HICON> {
    let mut pixels = rgba.to_vec();
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
        None
    } else {
        Some(icon)
    }
}
fn add_red_badge(rgba: &[u8]) -> Vec<u8> {
    let mut out = rgba.to_vec();
    let cx = 25.0_f32;
    let cy = 7.0_f32;
    let r_outer = 6.2_f32;
    let r_inner = 5.0_f32;
    for y in 0..32 {
        for x in 0..32 {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            let outer_a = (r_outer - dist + 0.5).clamp(0.0, 1.0);
            if outer_a <= 0.0 {
                continue;
            }
            let inner_a = (r_inner - dist + 0.5).clamp(0.0, 1.0);
            // Dark outline ring around a bright red dot for contrast on any taskbar theme.
            let (sr, sg, sb) = (
                20.0 + (239.0 - 20.0) * inner_a,
                20.0 + (58.0 - 20.0) * inner_a,
                24.0 + (68.0 - 24.0) * inner_a,
            );
            let idx = (y * 32 + x) * 4;
            let da = out[idx + 3] as f32 / 255.0;
            let out_a = outer_a + da * (1.0 - outer_a);
            if out_a > 0.0 {
                let dst = [out[idx] as f32, out[idx + 1] as f32, out[idx + 2] as f32];
                let src = [sr, sg, sb];
                for c in 0..3 {
                    out[idx + c] =
                        ((src[c] * outer_a + dst[c] * da * (1.0 - outer_a)) / out_a)
                            .round()
                            .clamp(0.0, 255.0) as u8;
                }
                out[idx + 3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}
impl Tray {
    pub unsafe fn new(hwnd: HWND, root: &Path) -> Option<Self> {
        let rgba = image::open(root.join("resources/tray.png"))
            .ok()?
            .into_rgba8();
        let image = image::imageops::resize(&rgba, 32, 32, image::imageops::FilterType::Lanczos3);
        let base_pixels = image.into_raw();
        let icon_normal = create_hicon(&base_pixels)?;
        let icon_badge = match create_hicon(&add_red_badge(&base_pixels)) {
            Some(icon) => icon,
            None => {
                DestroyIcon(icon_normal);
                return None;
            }
        };
        let mut data: NOTIFYICONDATAW = zeroed();
        data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = hwnd;
        data.uID = 1;
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_GUID;
        data.uCallbackMessage = MESSAGE;
        data.hIcon = icon_normal;
        data.guidItem = GUID::from_u128(0xc7c6f41d_f516_4c86_9f3e_8db41829601a);
        let tip = crate::model::wide("Lumen");
        data.szTip[..tip.len().min(127)].copy_from_slice(&tip[..tip.len().min(127)]);
        let mut tray = Self {
            data,
            icon_normal,
            icon_badge,
            update_available: false,
            registered: false,
            use_guid: true,
        };
        SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, icon_normal as isize);
        SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, icon_normal as isize);
        let _ = tray.add();
        Some(tray)
    }
    pub fn is_registered(&self) -> bool {
        self.registered
    }
    fn base_flags(&self) -> u32 {
        if self.use_guid {
            NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_GUID
        } else {
            NIF_MESSAGE | NIF_ICON | NIF_TIP
        }
    }
    pub unsafe fn notify(&mut self, title: &str, body: &str) {
        if !self.registered {
            let _ = self.add();
        }
        self.data.uFlags = self.base_flags() | NIF_INFO;
        self.data.dwInfoFlags = NIIF_INFO;
        self.data.szInfoTitle.fill(0);
        let t = crate::model::wide(title);
        let t_len = t.len().min(self.data.szInfoTitle.len() - 1);
        self.data.szInfoTitle[..t_len].copy_from_slice(&t[..t_len]);
        self.data.szInfo.fill(0);
        let b = crate::model::wide(body);
        let b_len = b.len().min(self.data.szInfo.len() - 1);
        self.data.szInfo[..b_len].copy_from_slice(&b[..b_len]);
        if Shell_NotifyIconW(NIM_MODIFY, &self.data) == 0 {
            self.registered = false;
            let _ = self.add();
        }
        self.data.uFlags = self.base_flags();
    }
    pub unsafe fn set_update_available(&mut self, available: bool) {
        if self.update_available == available {
            return;
        }
        if !self.registered {
            let _ = self.add();
        }
        self.update_available = available;
        self.data.hIcon = if available {
            self.icon_badge
        } else {
            self.icon_normal
        };
        self.data.szTip.fill(0);
        let tip = crate::model::wide(if available { "Update available" } else { "Lumen" });
        self.data.szTip[..tip.len().min(127)].copy_from_slice(&tip[..tip.len().min(127)]);
        self.data.uFlags = self.base_flags();
        if Shell_NotifyIconW(NIM_MODIFY, &self.data) == 0 {
            self.registered = false;
            let _ = self.add();
        }
    }
    pub unsafe fn add(&mut self) -> bool {
        self.data.uFlags = self.base_flags();
        let mut added = Shell_NotifyIconW(NIM_ADD, &self.data) != 0;
        if !added {
            Shell_NotifyIconW(NIM_DELETE, &self.data);
            added = Shell_NotifyIconW(NIM_ADD, &self.data) != 0;
        }
        if !added && self.use_guid {
            let err = GetLastError();
            crate::logger::warn(
                "tray",
                format!("tray NIM_ADD with NIF_GUID failed (err={err}); falling back to uID"),
            );
            self.use_guid = false;
            self.data.uFlags = self.base_flags();
            self.data.guidItem = zeroed();
            added = Shell_NotifyIconW(NIM_ADD, &self.data) != 0;
            if !added {
                Shell_NotifyIconW(NIM_DELETE, &self.data);
                added = Shell_NotifyIconW(NIM_ADD, &self.data) != 0;
            }
        }
        self.registered = added;
        if added {
            crate::logger::info(
                "tray",
                format!("tray icon registered={added} use_guid={}", self.use_guid),
            );
        } else {
            let err = GetLastError();
            crate::logger::warn(
                "tray",
                format!(
                    "tray icon registered={added} use_guid={} err={err}",
                    self.use_guid
                ),
            );
        }
        eprintln!("native-tray registered={added}");
        added
    }
    pub unsafe fn menu(&self) -> u32 {
        let menu = CreatePopupMenu();
        let update_label = if self.update_available {
            "Update now"
        } else {
            "Check for updates"
        };
        for (id, label) in [
            (8, "General Settings / Widget Manager"),
            (11, update_label),
            (10, "Open Log"),
            (3, "Quit Lumen"),
        ] {
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
            self.data.uFlags = self.base_flags();
            Shell_NotifyIconW(NIM_DELETE, &self.data);
            DestroyIcon(self.icon_normal);
            DestroyIcon(self.icon_badge);
        }
    }
}
