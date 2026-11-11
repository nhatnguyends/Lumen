use windows_sys::Win32::{Foundation::RECT, UI::WindowsAndMessaging::*};
pub const DOCK_MIN: [i32; 2] = [180, 120];
pub struct DockHeader {
    pub compact: bool,
    pub toolbar_x: f32,
    pub toolbar_step: f32,
    pub toolbar_y: f32,
    pub clear_x: f32,
    pub edit_width: i32,
    pub placeholder_right: f32,
    pub grid_top: i32,
}
pub fn dock_header(width: i32, has_query: bool) -> DockHeader {
    let compact = width < 440;
    let step = if width < 224 { 28. } else { 39. };
    DockHeader {
        compact,
        toolbar_step: step,
        toolbar_x: if compact {
            (width as f32 - step * 4.) * 0.5
        } else {
            (width - 196) as f32
        },
        toolbar_y: if compact { 77. } else { 37. },
        clear_x: (width - if compact { 38 } else { 240 }) as f32,
        edit_width: (width
            - if compact {
                if has_query {
                    112
                } else {
                    88
                }
            } else if has_query {
                308
            } else {
                282
            })
        .max(40),
        placeholder_right: (width - if compact { 32 } else { 232 }) as f32,
        grid_top: if compact { 110 } else { 80 },
    }
}
pub fn scrollbar(w: i32, h: i32, top: i32, max: f32, offset: f32) -> Option<[f32; 4]> {
    if max <= 0. {
        return None;
    }
    let start = top as f32 + 12.;
    let track = (h - top - 52).max(18) as f32;
    let thumb = (track * track / (track + max)).max(18.).min(track);
    let y = start + (track - thumb) * (offset / max).clamp(0., 1.);
    Some([w as f32 - 13., y, w as f32 - 10., y + thumb])
}
pub fn scroll_drag(h: i32, top: i32, max: f32, pointer: f32, grab: f32) -> f32 {
    let track = (h - top - 52).max(18) as f32;
    let thumb = (track * track / (track + max)).max(18.).min(track);
    if track <= thumb {
        return 0.;
    }
    ((pointer - grab - (top as f32 + 12.)) / (track - thumb)).clamp(0., 1.) * max
}

pub fn resized(r: RECT, edge: u32, dx: i32, dy: i32, min: [i32; 2]) -> RECT {
    let mut out = r;
    if matches!(edge, HTLEFT | HTTOPLEFT | HTBOTTOMLEFT) {
        out.left = (r.left + dx).min(r.right - min[0]);
    }
    if matches!(edge, HTRIGHT | HTTOPRIGHT | HTBOTTOMRIGHT) {
        out.right = (r.right + dx).max(r.left + min[0]);
    }
    if matches!(edge, HTTOP | HTTOPLEFT | HTTOPRIGHT) {
        out.top = (r.top + dy).min(r.bottom - min[1]);
    }
    if matches!(edge, HTBOTTOM | HTBOTTOMLEFT | HTBOTTOMRIGHT) {
        out.bottom = (r.bottom + dy).max(r.top + min[1]);
    }
    out
}
pub fn edge(x: i32, y: i32, w: i32, h: i32, inset: i32, grip: i32) -> u32 {
    let left = x < inset;
    let right = x >= w - inset;
    let top = y < inset;
    let bottom = y >= h - inset;
    if x + y < grip {
        HTTOPLEFT
    } else if w - x + y < grip {
        HTTOPRIGHT
    } else if w - x + h - y < grip {
        HTBOTTOMRIGHT
    } else if x < grip && y >= h - grip {
        HTBOTTOMLEFT
    } else if top && left {
        HTTOPLEFT
    } else if top && right {
        HTTOPRIGHT
    } else if bottom && right {
        HTBOTTOMRIGHT
    } else if left {
        HTLEFT
    } else if right {
        HTRIGHT
    } else if top {
        HTTOP
    } else if bottom {
        HTBOTTOM
    } else {
        HTCLIENT
    }
}
/// Match the Dock's diagonal corner grip, including the visible rounded arc.
/// A narrow rectangular edge alone misses the middle of a 34-DIP corner.
pub fn widget_edge(x: f32, y: f32, w: f32, h: f32) -> u32 {
    edge(
        x.floor() as i32,
        y.floor() as i32,
        w.round() as i32,
        h.round() as i32,
        10,
        40,
    )
}
#[cfg(test)]
pub unsafe fn assert_widget_corner_hits(hwnd: windows_sys::Win32::Foundation::HWND, dpi: f32) {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    let mut r: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut r);
    let n = (14. * dpi).round() as i32;
    for (x, y, wanted) in [
        (n, n, HTTOPLEFT),
        (r.right - n, n, HTTOPRIGHT),
        (n, r.bottom - n, HTBOTTOMLEFT),
        (r.right - n, r.bottom - n, HTBOTTOMRIGHT),
    ] {
        let mut p = POINT { x, y };
        ClientToScreen(hwnd, &mut p);
        let packed = ((p.x as u16 as usize) | ((p.y as u16 as usize) << 16)) as isize;
        assert_eq!(
            SendMessageW(hwnd, WM_NCHITTEST, 0, packed),
            wanted as isize,
            "visible rounded arc {x},{y} must resize diagonally"
        );
    }
}
pub fn columns(width: i32, requested: usize, icon: i32) -> usize {
    requested.min(((width - 24) / (icon + 42)).max(1) as usize)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rounded_widget_arcs_resize_diagonally_without_claiming_content() {
        for dpi in [1.0_f32, 1.25, 1.5, 1.75, 2.] {
            for (w, h) in [(120., 120.), (168., 190.), (420., 560.), (880., 540.)] {
                for (x, y, wanted) in [
                    (14., 14., HTTOPLEFT),
                    (w - 14., 14., HTTOPRIGHT),
                    (14., h - 14., HTBOTTOMLEFT),
                    (w - 14., h - 14., HTBOTTOMRIGHT),
                ] {
                    let x = (x * dpi).round() / dpi;
                    let y = (y * dpi).round() / dpi;
                    assert_eq!(widget_edge(x, y, w, h), wanted);
                }
                assert_eq!(widget_edge(w / 2., h / 2., w, h), HTCLIENT);
                assert_eq!(widget_edge(w - 32., 28., w, h), HTCLIENT);
                assert_eq!(widget_edge(6., h / 2., w, h), HTLEFT);
            }
        }
    }
    #[test]
    fn compact_header_keeps_search_and_all_five_buttons_separate() {
        for width in [180, 200, 240, 300, 439, 440, 744] {
            let h = dock_header(width, true);
            assert!(h.toolbar_x - 17. >= 16.);
            assert!(h.toolbar_x + h.toolbar_step * 4. + 17. <= width as f32 - 16.);
            assert!(58. + h.edit_width as f32 <= h.clear_x - 10.);
            if h.compact {
                assert!(h.toolbar_y - 17. >= 60.);
            }
        }
    }
    #[test]
    fn grabbing_scrollbar_preserves_offset_and_clamps_endpoints() {
        let bar = scrollbar(580, 370, 80, 120., 47.25).unwrap();
        assert!((scroll_drag(370, 80, 120., bar[1] + 20., 20.) - 47.25).abs() < 0.001);
        assert_eq!(scroll_drag(370, 80, 120., -200., 20.), 0.);
        assert_eq!(scroll_drag(370, 80, 120., 900., 20.), 120.);
    }
    #[test]
    fn scrollbar_stays_inside_the_panel_at_both_ends() {
        for offset in [0., 40., 1000.] {
            let r = scrollbar(580, 370, 80, 120., offset).unwrap();
            assert_eq!(r[0], 567.);
            assert!(r[1] >= 92. && r[3] <= 330.01);
        }
        assert!(scrollbar(580, 370, 80, 0., 0.).is_none());
    }
    #[test]
    fn fractional_dpi_rounding_keeps_the_eighth_column() {
        assert_eq!(columns(744, 8, 48), 8);
        assert_eq!(columns(745, 8, 48), 8);
        assert_eq!(columns(360, 8, 48), 3);
        assert_eq!(columns(1800, 4, 48), 4);
    }
    #[test]
    fn every_edge_preserves_the_opposite_anchor() {
        let r = RECT {
            left: 100,
            top: 100,
            right: 800,
            bottom: 500,
        };
        for e in [
            HTLEFT,
            HTRIGHT,
            HTTOP,
            HTBOTTOM,
            HTTOPLEFT,
            HTTOPRIGHT,
            HTBOTTOMLEFT,
            HTBOTTOMRIGHT,
        ] {
            let a = resized(r, e, 30, 20, [360, 240]);
            assert_eq!(
                a.left,
                if matches!(e, HTLEFT | HTTOPLEFT | HTBOTTOMLEFT) {
                    130
                } else {
                    100
                }
            );
            assert_eq!(
                a.right,
                if matches!(e, HTRIGHT | HTTOPRIGHT | HTBOTTOMRIGHT) {
                    830
                } else {
                    800
                }
            );
            assert_eq!(
                a.top,
                if matches!(e, HTTOP | HTTOPLEFT | HTTOPRIGHT) {
                    120
                } else {
                    100
                }
            );
            assert_eq!(
                a.bottom,
                if matches!(e, HTBOTTOM | HTBOTTOMLEFT | HTBOTTOMRIGHT) {
                    520
                } else {
                    500
                }
            );
        }
    }
    #[test]
    fn minimum_size_does_not_move_the_opposite_corner() {
        let a = resized(
            RECT {
                left: 100,
                top: 100,
                right: 800,
                bottom: 500,
            },
            HTBOTTOMLEFT,
            1000,
            -1000,
            [360, 240],
        );
        assert_eq!((a.left, a.top, a.right, a.bottom), (440, 100, 800, 340));
    }
    #[test]
    fn edges_and_corners_have_resize_hit_targets() {
        assert_eq!(edge(18, 12, 746, 370, 6, 36), HTTOPLEFT);
        assert_eq!(edge(728, 12, 746, 370, 6, 36), HTTOPRIGHT);
        assert_eq!(edge(728, 358, 746, 370, 6, 36), HTBOTTOMRIGHT);
        assert_eq!(edge(32, 30, 746, 370, 6, 36), HTCLIENT);
        assert_eq!(edge(20, 350, 746, 370, 6, 36), HTBOTTOMLEFT);
        assert_eq!(edge(744, 200, 746, 370, 6, 36), HTRIGHT);
        assert_eq!(edge(200, 100, 746, 370, 6, 36), HTCLIENT);
    }
}
