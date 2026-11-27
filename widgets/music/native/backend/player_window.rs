//! Native Win32 host for the dedicated YouTube Music Chromium window and PNG icon.
use image::{DynamicImage, ImageFormat, RgbaImage};
use std::{
    io::Cursor,
    mem::{size_of, zeroed},
    ptr::null_mut,
    sync::OnceLock,
};
use windows_sys::Win32::{
    Foundation::{BOOL, HWND, LPARAM, RECT},
    Graphics::Gdi::{
        GetMonitorInfoW, MONITORINFO, MonitorFromRect, MonitorFromWindow, MONITOR_DEFAULTTONEAREST,
    },
    UI::WindowsAndMessaging::{
        CreateIconFromResourceEx, EnumWindows, GetWindowPlacement, GetWindowRect, GetWindowTextW,
        GetWindowThreadProcessId, HICON, HWND_TOP, ICON_BIG, ICON_SMALL, SMTO_ABORTIFHUNG,
        SPI_GETWORKAREA, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW,
        SW_HIDE, SW_MINIMIZE, SW_SHOWMINNOACTIVE, SW_SHOWNORMAL, SendMessageTimeoutW,
        SetForegroundWindow, SetWindowPlacement, SetWindowPos, ShowWindow, SystemParametersInfoW,
        WINDOWPLACEMENT, WM_GETICON, WM_SETICON,
    },
};

fn triangle_sdf(
    px: f32,
    py: f32,
    ax: f32,
    ay: f32,
    bx: f32,
    by: f32,
    cx: f32,
    cy: f32,
) -> f32 {
    let edge = |x1: f32, y1: f32, x2: f32, y2: f32| -> (f32, f32) {
        let ex = x2 - x1;
        let ey = y2 - y1;
        let wx = px - x1;
        let wy = py - y1;
        let denom = ex * ex + ey * ey;
        let t = if denom > 0.0 {
            ((wx * ex + wy * ey) / denom).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let dx = wx - ex * t;
        let dy = wy - ey * t;
        (dx * dx + dy * dy, ex * wy - ey * wx)
    };
    let (d1, s1) = edge(ax, ay, bx, by);
    let (d2, s2) = edge(bx, by, cx, cy);
    let (d3, s3) = edge(cx, cy, ax, ay);
    let min_d = d1.min(d2).min(d3).sqrt();
    let inside = (s1 >= 0.0 && s2 >= 0.0 && s3 >= 0.0) || (s1 <= 0.0 && s2 <= 0.0 && s3 <= 0.0);
    if inside {
        -min_d
    } else {
        min_d
    }
}

pub fn render_youtube_music_png(size: u32) -> Option<Vec<u8>> {
    let len = (size as usize).checked_mul(size as usize)?.checked_mul(4)?;
    let mut raw = vec![0u8; len];
    let s = size as f32;
    let c = s * 0.5;
    let outer_r = s * 0.45;
    let ring_r = s * 0.275;
    let ring_half_w = 1.0f32.max(s * 0.024);
    let tri_l = c - s * 0.075;
    let tri_r = c + s * 0.135;
    let tri_h = s * 0.125;

    for y in 0..size {
        for x in 0..size {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let dist = (px - c).hypot(py - c);
            let outer_a = (outer_r - dist + 0.6).clamp(0.0, 1.0);
            if outer_a <= 0.0 {
                continue;
            }
            let ring_dist = (dist - ring_r).abs() - ring_half_w;
            let ring_a = (0.6 - ring_dist).clamp(0.0, 1.0);
            let tri_dist = triangle_sdf(px, py, tri_l, c - tri_h, tri_r, c, tri_l, c + tri_h);
            let tri_a = (0.6 - tri_dist).clamp(0.0, 1.0);
            let white_a = ring_a.max(tri_a);

            let off = ((y * size + x) * 4) as usize;
            raw[off] = 255;
            raw[off + 1] = (white_a * 255.0).round() as u8;
            raw[off + 2] = (white_a * 255.0 + (1.0 - white_a) * 32.0).round() as u8;
            raw[off + 3] = (outer_a * 255.0).round() as u8;
        }
    }

    let img = RgbaImage::from_raw(size, size, raw)?;
    let mut png = Vec::new();
    DynamicImage::ImageRgba8(img)
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .ok()?;
    Some(png)
}

struct CachedIcons {
    big: isize,
    small: isize,
}

static ICONS: OnceLock<Option<CachedIcons>> = OnceLock::new();
static FAVICON_SCRIPT: OnceLock<Option<String>> = OnceLock::new();

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((n >> 6) & 0x3f) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

pub fn youtube_music_favicon_script() -> Option<String> {
    FAVICON_SCRIPT
        .get_or_init(|| {
            let png = render_youtube_music_png(128)?;
            let b64 = encode_base64(&png);
            Some(format!(
                r###"(() => {{
  const href = "data:image/png;base64,{b64}";
  let applying = false;
  const isIconLink = el => {{
    if (!el || el.tagName !== 'LINK') return false;
    const rel = (el.getAttribute('rel') || '').toLowerCase();
    return rel.split(/\s+/).includes('icon');
  }};
  const enforce = () => {{
    if (applying) return;
    applying = true;
    try {{
      const root = document.head || document.documentElement;
      if (!root) return;
      let ours = null;
      for (const link of [...document.querySelectorAll('link[rel]')]) {{
        if (!isIconLink(link)) continue;
        if (link.getAttribute('data-lumen-icon') === '1' && !ours) {{
          ours = link;
        }} else {{
          link.remove();
        }}
      }}
      if (!ours) {{
        ours = document.createElement('link');
        ours.setAttribute('data-lumen-icon', '1');
        ours.rel = 'icon';
        ours.type = 'image/png';
        ours.sizes = '128x128';
        ours.href = href;
        root.appendChild(ours);
      }} else {{
        if (ours.getAttribute('href') !== href) ours.href = href;
        if (ours.getAttribute('rel') !== 'icon') ours.rel = 'icon';
        if (ours.parentNode !== root) root.appendChild(ours);
      }}
    }} finally {{
      applying = false;
    }}
  }};
  enforce();
  new MutationObserver(enforce).observe(document, {{
    childList: true,
    subtree: true,
    attributes: true,
    attributeFilter: ['href', 'rel'],
  }});
}})()"###
            ))
        })
        .clone()
}

fn ensure_icons() -> Option<&'static CachedIcons> {
    ICONS
        .get_or_init(|| {
            let make = |size: u32| -> isize {
                let Some(png) = render_youtube_music_png(size) else {
                    return 0;
                };
                let hicon: HICON = unsafe {
                    CreateIconFromResourceEx(
                        png.as_ptr(),
                        png.len() as u32,
                        1,
                        0x0003_0000,
                        size as i32,
                        size as i32,
                        0,
                    )
                };
                hicon as isize
            };
            let big = make(128);
            let small = make(32);
            if big == 0 && small == 0 {
                None
            } else {
                Some(CachedIcons { big, small })
            }
        })
        .as_ref()
}

fn get_window_icon(hwnd: HWND, kind: usize) -> Option<isize> {
    let mut result = 0usize;
    let ok = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_GETICON,
            kind,
            0,
            SMTO_ABORTIFHUNG,
            50,
            &mut result,
        )
    };
    if ok == 0 {
        None
    } else {
        Some(result as isize)
    }
}

fn window_icon_matches(hwnd: HWND, icons: &CachedIcons) -> bool {
    let check = |kind: usize, expected: isize| -> bool {
        if expected == 0 {
            return true;
        }
        match get_window_icon(hwnd, kind) {
            Some(actual) => actual == expected,
            None => true,
        }
    };
    check(ICON_BIG as usize, icons.big) && check(ICON_SMALL as usize, icons.small)
}

fn apply_window_icon(hwnd: HWND) {
    let Some(icons) = ensure_icons() else {
        return;
    };
    unsafe {
        if icons.big != 0 {
            let mut result = 0usize;
            SendMessageTimeoutW(
                hwnd,
                WM_SETICON,
                ICON_BIG as usize,
                icons.big,
                SMTO_ABORTIFHUNG,
                150,
                &mut result,
            );
        }
        if icons.small != 0 {
            let mut result = 0usize;
            SendMessageTimeoutW(
                hwnd,
                WM_SETICON,
                ICON_SMALL as usize,
                icons.small,
                SMTO_ABORTIFHUNG,
                150,
                &mut result,
            );
        }
    }
}

pub fn is_valid_bootstrap_title(title: &str) -> bool {
    title
        .strip_prefix("Lumen Music Background ")
        .is_some_and(|hex| {
            hex.len() == 16
                && hex
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        })
}

struct SearchState<'a> {
    pid: u32,
    title: &'a str,
    prefix: String,
    hwnd: isize,
}

unsafe extern "system" fn enum_hide_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let state = &mut *(lparam as *mut SearchState<'_>);
    let mut owner = 0u32;
    GetWindowThreadProcessId(hwnd, &mut owner);
    if owner != state.pid {
        return 1;
    }
    let mut buf = [0u16; 1024];
    let len = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
    if len <= 0 {
        return 1;
    }
    let name = String::from_utf16_lossy(&buf[..len as usize]);
    if name != state.title && !name.starts_with(&state.prefix) {
        return 1;
    }
    ShowWindow(hwnd, SW_HIDE);
    state.hwnd = hwnd as isize;
    0
}

/// Hide only the uniquely titled bootstrap window of the browser we launched.
pub fn hide_music_background_window(pid: u32, title: &str) -> Option<isize> {
    if pid == 0 || !is_valid_bootstrap_title(title) {
        return None;
    }
    let mut state = SearchState {
        pid,
        title,
        prefix: format!("{title} - "),
        hwnd: 0,
    };
    unsafe {
        EnumWindows(Some(enum_hide_proc), &mut state as *mut SearchState<'_> as LPARAM);
    }
    if state.hwnd != 0 {
        Some(state.hwnd)
    } else {
        None
    }
}

fn primary_work_area() -> RECT {
    let mut work: RECT = unsafe { zeroed() };
    let ok = unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut work as *mut RECT as _, 0) };
    if ok != 0 && work.right > work.left && work.bottom > work.top {
        work
    } else {
        RECT {
            left: 0,
            top: 0,
            right: 1280,
            bottom: 800,
        }
    }
}

fn centered_rect_in_work(work: RECT) -> RECT {
    let work_w = (work.right - work.left).max(320);
    let work_h = (work.bottom - work.top).max(240);
    let width = 1100.min(work_w);
    let height = 800.min(work_h);
    let left = work.left + (work_w - width) / 2;
    let top = work.top + (work_h - height) / 2;
    RECT {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}

fn is_valid_normal_rect(rc: RECT) -> bool {
    let w = rc.right - rc.left;
    let h = rc.bottom - rc.top;
    if w < 320 || h < 240 {
        return false;
    }
    unsafe {
        let mut info: MONITORINFO = zeroed();
        info.cbSize = size_of::<MONITORINFO>() as u32;
        let monitor = MonitorFromRect(&rc, MONITOR_DEFAULTTONEAREST);
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return false;
        }
        let work = info.rcWork;
        let overlap_x = rc.right.min(work.right) - rc.left.max(work.left);
        let overlap_y = rc.bottom.min(work.bottom) - rc.top.max(work.top);
        overlap_x >= 160 && overlap_y >= 120 && rc.top >= work.top - 16 && rc.top + 32 <= work.bottom
    }
}

fn ensure_reachable(hwnd: HWND) -> Result<(), String> {
    unsafe {
        let mut bounds: RECT = zeroed();
        let mut info: MONITORINFO = zeroed();
        info.cbSize = size_of::<MONITORINFO>() as u32;
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        if GetWindowRect(hwnd, &mut bounds) == 0 || GetMonitorInfoW(monitor, &mut info) == 0 {
            return Err("Could not place the music window".into());
        }
        let work = info.rcWork;
        let overlap_x = bounds.right.min(work.right) - bounds.left.max(work.left);
        if overlap_x >= 80 && bounds.top >= work.top - 16 && bounds.top + 32 <= work.bottom {
            return Ok(());
        }
        let centered = centered_rect_in_work(work);
        SetWindowPos(
            hwnd,
            null_mut(),
            centered.left,
            centered.top,
            centered.right - centered.left,
            centered.bottom - centered.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        Ok(())
    }
}

pub struct MusicWindowHost {
    hwnd: isize,
    disposed: bool,
}

impl MusicWindowHost {
    pub fn new(hwnd: isize, start_visible: bool) -> Result<Self, String> {
        if hwnd == 0 {
            return Err("Music window is not ready".into());
        }
        let raw = hwnd as HWND;
        apply_window_icon(raw);
        let centered = centered_rect_in_work(primary_work_area());
        unsafe {
            let mut wp: WINDOWPLACEMENT = zeroed();
            wp.length = size_of::<WINDOWPLACEMENT>() as u32;
            let _ = GetWindowPlacement(raw, &mut wp);
            wp.length = size_of::<WINDOWPLACEMENT>() as u32;
            wp.flags = 0;
            wp.rcNormalPosition = centered;
            wp.showCmd = if start_visible {
                SW_SHOWNORMAL as u32
            } else {
                SW_SHOWMINNOACTIVE as u32
            };
            SetWindowPlacement(raw, &wp);
        }
        apply_window_icon(raw);
        Ok(Self {
            hwnd,
            disposed: false,
        })
    }

    pub fn show(&self) {
        if self.disposed {
            return;
        }
        let raw = self.hwnd as HWND;
        let centered = centered_rect_in_work(primary_work_area());
        unsafe {
            let mut wp: WINDOWPLACEMENT = zeroed();
            wp.length = size_of::<WINDOWPLACEMENT>() as u32;
            if GetWindowPlacement(raw, &mut wp) == 0 || !is_valid_normal_rect(wp.rcNormalPosition) {
                wp.rcNormalPosition = centered;
            }
            wp.flags = 0;
            wp.showCmd = SW_SHOWNORMAL as u32;
            SetWindowPlacement(raw, &wp);
            ShowWindow(raw, SW_SHOWNORMAL);
            SetWindowPos(
                raw,
                HWND_TOP,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
            );
            SetForegroundWindow(raw);
        }
        let _ = ensure_reachable(raw);
        apply_window_icon(raw);
    }

    #[allow(dead_code)]
    pub fn hide(&self) {
        if self.disposed {
            return;
        }
        unsafe {
            ShowWindow(self.hwnd as HWND, SW_MINIMIZE);
        }
    }

    pub fn refresh_icon(&self) {
        if !self.disposed {
            apply_window_icon(self.hwnd as HWND);
        }
    }

    pub fn ensure_icon(&self) {
        if self.disposed {
            return;
        }
        let raw = self.hwnd as HWND;
        let Some(icons) = ensure_icons() else {
            return;
        };
        if !window_icon_matches(raw, icons) {
            apply_window_icon(raw);
        }
    }

    pub fn destroy(&mut self) {
        if self.disposed {
            return;
        }
        self.disposed = true;
        unsafe {
            ShowWindow(self.hwnd as HWND, SW_HIDE);
        }
    }
}

impl Drop for MusicWindowHost {
    fn drop(&mut self) {
        self.destroy();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_title_and_icon_rendering() {
        assert!(is_valid_bootstrap_title(
            "Lumen Music Background 0123456789abcdef"
        ));
        assert!(!is_valid_bootstrap_title("Lumen Music Background xyz"));
        assert!(!is_valid_bootstrap_title("Other Window"));

        let png = render_youtube_music_png(32).expect("PNG icon should render");
        assert!(png.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]));
    }

    #[test]
    fn centered_rect_clamps_to_work_area() {
        let big = centered_rect_in_work(RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        });
        assert_eq!(big.right - big.left, 1100);
        assert_eq!(big.bottom - big.top, 800);
        assert_eq!(big.left, 410);
        assert_eq!(big.top, 120);

        let small = centered_rect_in_work(RECT {
            left: 100,
            top: 40,
            right: 900,
            bottom: 640,
        });
        assert_eq!(small.right - small.left, 800);
        assert_eq!(small.bottom - small.top, 600);
        assert_eq!(small.left, 100);
        assert_eq!(small.top, 40);
    }

    #[test]
    fn base64_and_favicon_override_script_render_cleanly() {
        assert_eq!(encode_base64(b""), "");
        assert_eq!(encode_base64(b"M"), "TQ==");
        assert_eq!(encode_base64(b"Ma"), "TWE=");
        assert_eq!(encode_base64(b"Man"), "TWFu");

        let script = youtube_music_favicon_script().expect("favicon script should build");
        assert!(script.contains("data:image/png;base64,"));
        assert!(script.contains("data-lumen-icon"));
    }
}
