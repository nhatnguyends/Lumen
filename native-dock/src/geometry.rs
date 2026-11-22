use windows_sys::Win32::{
    Foundation::{BOOL, HWND, LPARAM, RECT},
    Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, MonitorFromRect, MonitorFromWindow, HDC, HMONITOR,
        MONITORINFO, MONITOR_DEFAULTTONEAREST,
    },
    UI::{
        HiDpi::{
            GetAwarenessFromDpiAwarenessContext, GetDpiForMonitor, GetDpiForWindow,
            GetWindowDpiAwarenessContext, DPI_AWARENESS_UNAWARE, MDT_EFFECTIVE_DPI,
        },
        WindowsAndMessaging::*,
    },
};
pub const DOCK_MIN: [i32; 2] = [180, 120];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DockGridLayout {
    pub cells: Vec<(usize, usize)>,
    pub rows: usize,
    pub group_runs: Vec<(String, Vec<(usize, usize)>)>,
}

pub fn choose_group_width(n: usize, columns: usize) -> usize {
    if columns <= 1 || n <= 1 {
        return 1;
    }
    // Minimize row count first, then choose the narrowest width fitting those rows
    // so icons pack into the tightest rectangular block.
    let min_rows = n.div_ceil(columns);
    n.div_ceil(min_rows).clamp(1, columns)
}

pub fn pack_dock_grid(
    groups: &[Option<&str>],
    has_add_tile: bool,
    columns: usize,
) -> DockGridLayout {
    let cols = columns.max(1);
    let total = groups.len() + usize::from(has_add_tile);
    if total == 0 {
        return DockGridLayout {
            cells: Vec::new(),
            rows: 0,
            group_runs: Vec::new(),
        };
    }
    let mut occupied: Vec<Vec<bool>> = Vec::new();
    let mut cells = vec![(0usize, 0usize); total];
    let mut group_runs = Vec::new();
    let mut max_row = 0usize;
    let mut add_min = (0usize, 0usize);

    let ensure_rows = |occupied: &mut Vec<Vec<bool>>, needed_rows: usize| {
        while occupied.len() < needed_rows {
            occupied.push(vec![false; cols]);
        }
    };

    let mut pos = 0usize;
    while pos < groups.len() {
        let Some(gid) = groups[pos] else {
            let mut r = 0usize;
            'scan_single: loop {
                ensure_rows(&mut occupied, r + 1);
                for c in 0..cols {
                    if !occupied[r][c] {
                        occupied[r][c] = true;
                        cells[pos] = (c, r);
                        max_row = max_row.max(r);
                        if (r, c) > add_min {
                            add_min = (r, c);
                        }
                        break 'scan_single;
                    }
                }
                r += 1;
            }
            pos += 1;
            continue;
        };
        let mut end = pos + 1;
        while end < groups.len() && groups[end] == Some(gid) {
            end += 1;
        }
        let n = end - pos;
        let gw = choose_group_width(n, cols).clamp(1, cols);
        let mut r0 = 0usize;
        let placed_at;
        'scan_block: loop {
            let need_rows = r0 + n.div_ceil(gw);
            ensure_rows(&mut occupied, need_rows);
            for c0 in 0..=(cols - gw) {
                let fits = (0..n).all(|k| {
                    let c = c0 + k % gw;
                    let r = r0 + k / gw;
                    !occupied[r][c]
                });
                if fits {
                    placed_at = (c0, r0);
                    break 'scan_block;
                }
            }
            r0 += 1;
        }
        let mut group_cells = Vec::with_capacity(n);
        for k in 0..n {
            let c = placed_at.0 + k % gw;
            let r = placed_at.1 + k / gw;
            occupied[r][c] = true;
            cells[pos + k] = (c, r);
            group_cells.push((c, r));
            max_row = max_row.max(r);
            if (r, c) > add_min {
                add_min = (r, c);
            }
        }
        group_runs.push((gid.to_owned(), group_cells));
        pos = end;
    }

    if has_add_tile {
        let mut r = add_min.0;
        let mut start_c = if groups.is_empty() { 0 } else { add_min.1 };
        'scan_add: loop {
            ensure_rows(&mut occupied, r + 1);
            for c in start_c..cols {
                if !occupied[r][c] {
                    occupied[r][c] = true;
                    cells[groups.len()] = (c, r);
                    max_row = max_row.max(r);
                    break 'scan_add;
                }
            }
            r += 1;
            start_c = 0;
        }
    }

    DockGridLayout {
        cells,
        rows: max_row + 1,
        group_runs,
    }
}

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
    dock_header_with_toolbar(width, has_query, 5)
}
pub fn dock_header_with_toolbar(width: i32, has_query: bool, toolbar_count: usize) -> DockHeader {
    let count = toolbar_count.max(2);
    let compact = width < 440;
    let step = if compact {
        if width < 224 || count > 6 { 28. } else { 39. }
    } else if (width - 140) as f32 >= count as f32 * 39. {
        39.
    } else {
        30.
    };
    let toolbar_x = if compact {
        (width as f32 - step * (count - 1) as f32) * 0.5
    } else {
        width as f32 - 40. - (count - 1) as f32 * step
    };
    DockHeader {
        compact,
        toolbar_step: step,
        toolbar_x,
        toolbar_y: if compact { 77. } else { 37. },
        clear_x: if compact {
            (width - 38) as f32
        } else {
            toolbar_x - 44.
        },
        edit_width: (if compact {
            width - if has_query { 112 } else { 88 }
        } else {
            toolbar_x as i32 - if has_query { 112 } else { 86 }
        })
        .max(40),
        placeholder_right: if compact {
            (width - 32) as f32
        } else {
            toolbar_x - 36.
        },
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
    } else if x + h - y < grip {
        HTBOTTOMLEFT
    } else if top && left {
        HTTOPLEFT
    } else if top && right {
        HTTOPRIGHT
    } else if bottom && right {
        HTBOTTOMRIGHT
    } else if bottom && left {
        HTBOTTOMLEFT
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

pub fn clamp_rect_to_work(rect: [i32; 4], work: [i32; 4], min_px: [i32; 2]) -> [i32; 4] {
    let work_w = (work[2] - work[0]).max(1);
    let work_h = (work[3] - work[1]).max(1);
    let w = (rect[2] - rect[0]).clamp(min_px[0].min(work_w), work_w);
    let h = (rect[3] - rect[1]).clamp(min_px[1].min(work_h), work_h);
    let left = rect[0].clamp(work[0], (work[2] - w).max(work[0]));
    let top = rect[1].clamp(work[1], (work[3] - h).max(work[1]));
    [left, top, left + w, top + h]
}

pub fn fit_settings_rect(owner_rect: Option<[i32; 4]>, work: [i32; 4], dpi: f32) -> [i32; 4] {
    let dpi = dpi.max(1.0);
    let work_w = (work[2] - work[0]).max(1);
    let work_h = (work[3] - work[1]).max(1);
    let margin = (12. * dpi).round() as i32;
    let avail_w = (work_w - margin * 2).max(work_w.min(280)).min(work_w);
    let avail_h = (work_h - margin * 2).max(work_h.min(240)).min(work_h);
    let w = (568. * dpi).round().min(avail_w as f32).max(280_f32.min(avail_w as f32)) as i32;
    let h = (668. * dpi).round().min(avail_h as f32).max(240_f32.min(avail_h as f32)) as i32;
    let (ideal_x, ideal_y) = match owner_rect {
        Some([ol, ot, or, ob]) if or > ol && ob > ot => (
            ol + (or - ol - w) / 2,
            ot - (80. * dpi).round() as i32,
        ),
        _ => (
            work[0] + (work_w - w) / 2,
            work[1] + (work_h - h) / 2,
        ),
    };
    let x = ideal_x.clamp(work[0], (work[2] - w).max(work[0]));
    let y = ideal_y.clamp(work[1], (work[3] - h).max(work[1]));
    [x, y, x + w, y + h]
}

pub unsafe fn monitor_dpi(hmon: HMONITOR, fallback_hwnd: HWND) -> f32 {
    if !hmon.is_null() {
        let (mut dx, mut dy) = (0u32, 0u32);
        if GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) == 0 && dx > 0 {
            return (dx as f32 / 96.).max(1.0);
        }
    }
    if !fallback_hwnd.is_null() {
        let raw = GetDpiForWindow(fallback_hwnd);
        if raw > 0 {
            return (raw as f32 / 96.).max(1.0);
        }
    }
    1.0
}

pub unsafe fn window_dpi(hwnd: HWND) -> f32 {
    if hwnd.is_null() {
        return 1.0;
    }
    let ctx = GetWindowDpiAwarenessContext(hwnd);
    if !ctx.is_null() && GetAwarenessFromDpiAwarenessContext(ctx) == DPI_AWARENESS_UNAWARE {
        return 1.0;
    }
    monitor_dpi(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), hwnd)
}

unsafe extern "system" fn collect_topology(
    hmon: HMONITOR,
    _: HDC,
    _: *mut RECT,
    param: LPARAM,
) -> BOOL {
    let out = &mut *(param as *mut Vec<([i32; 4], [i32; 4], u32)>);
    let mut info: MONITORINFO = std::mem::zeroed();
    info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if GetMonitorInfoW(hmon, &mut info) != 0 {
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        let m = info.rcMonitor;
        let w = info.rcWork;
        out.push((
            [m.left, m.top, m.right, m.bottom],
            [w.left, w.top, w.right, w.bottom],
            dx,
        ));
    }
    1
}

pub unsafe fn display_topology_signature() -> Vec<([i32; 4], [i32; 4], u32)> {
    let mut list = Vec::new();
    EnumDisplayMonitors(
        std::ptr::null_mut(),
        std::ptr::null(),
        Some(collect_topology),
        &mut list as *mut _ as isize,
    );
    list.sort_unstable();
    list
}

unsafe fn work_area_for_monitor(hmon: HMONITOR) -> Option<[i32; 4]> {
    if hmon.is_null() {
        return None;
    }
    let mut info: MONITORINFO = std::mem::zeroed();
    info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if GetMonitorInfoW(hmon, &mut info) == 0 {
        return None;
    }
    let w = info.rcWork;
    (w.right > w.left && w.bottom > w.top).then_some([w.left, w.top, w.right, w.bottom])
}

pub unsafe fn clamp_dpi_rect(suggested: RECT, min_dip: [i32; 2], new_dpi: f32) -> RECT {
    let dpi = new_dpi.max(1.0);
    let Some(work) = work_area_for_monitor(MonitorFromRect(&suggested, MONITOR_DEFAULTTONEAREST)) else {
        return suggested;
    };
    let min_px = [
        (min_dip[0] as f32 * dpi).round() as i32,
        (min_dip[1] as f32 * dpi).round() as i32,
    ];
    let c = clamp_rect_to_work(
        [suggested.left, suggested.top, suggested.right, suggested.bottom],
        work,
        min_px,
    );
    RECT {
        left: c[0],
        top: c[1],
        right: c[2],
        bottom: c[3],
    }
}

pub unsafe fn clamp_window_to_work(hwnd: HWND, min_dip: [i32; 2], prev_dpi: f32) -> (f32, bool) {
    if hwnd.is_null() {
        return (prev_dpi.max(1.0), false);
    }
    let hmon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
    let dpi = monitor_dpi(hmon, hwnd);
    let mut r: RECT = std::mem::zeroed();
    if GetWindowRect(hwnd, &mut r) == 0 {
        return (dpi, false);
    }
    let Some(work) = work_area_for_monitor(hmon) else {
        return (dpi, false);
    };
    let old_dpi = prev_dpi.max(1.0);
    let mut w = r.right - r.left;
    let mut h = r.bottom - r.top;
    let dpi_changed = (dpi - old_dpi).abs() > 0.005;
    if dpi_changed {
        let ratio = dpi / old_dpi;
        w = (w as f32 * ratio).round() as i32;
        h = (h as f32 * ratio).round() as i32;
    }
    let min_px = [
        (min_dip[0] as f32 * dpi).round() as i32,
        (min_dip[1] as f32 * dpi).round() as i32,
    ];
    let c = clamp_rect_to_work([r.left, r.top, r.left + w, r.top + h], work, min_px);
    if c != [r.left, r.top, r.right, r.bottom] || dpi_changed {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            c[0],
            c[1],
            c[2] - c[0],
            c[3] - c[1],
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        (dpi, true)
    } else {
        (dpi, false)
    }
}

pub unsafe fn refit_settings_window(hwnd: HWND, owner: HWND) -> f32 {
    let mon_hwnd = if !owner.is_null() { owner } else { hwnd };
    let ref_hwnd = if !hwnd.is_null() { hwnd } else { owner };
    let hmon = MonitorFromWindow(mon_hwnd, MONITOR_DEFAULTTONEAREST);
    let dpi = monitor_dpi(hmon, ref_hwnd);
    let Some(work) = work_area_for_monitor(hmon) else {
        return dpi;
    };
    let owner_rect = if !owner.is_null() {
        let mut or: RECT = std::mem::zeroed();
        (GetWindowRect(owner, &mut or) != 0).then_some([or.left, or.top, or.right, or.bottom])
    } else {
        None
    };
    let c = fit_settings_rect(owner_rect, work, dpi);
    if !hwnd.is_null() {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            c[0],
            c[1],
            c[2] - c[0],
            c[3] - c[1],
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
    dpi
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
        assert_eq!(edge(18, 358, 746, 370, 6, 36), HTBOTTOMLEFT);
        assert_eq!(edge(744, 200, 746, 370, 6, 36), HTRIGHT);
        assert_eq!(edge(200, 100, 746, 370, 6, 36), HTCLIENT);
    }
    #[test]
    fn geometry_bottom_left_symmetry_unit_test() {
        let h = 200;
        let w = 200;
        let inset = 10;
        let grip = 40;
        assert_eq!(edge(38, h - 38, w, h, inset, grip), HTCLIENT);
        assert_eq!(edge(14, h - 14, w, h, inset, grip), HTBOTTOMLEFT);
    }
    #[test]
    fn clamp_rect_to_work_keeps_oversized_and_offscreen_windows_inside() {
        let low_res_work = [0, 0, 1280, 720];
        let clamped = clamp_rect_to_work([2200, 950, 2200 + 1500, 950 + 900], low_res_work, [180, 120]);
        assert_eq!(clamped, [0, 0, 1280, 720]);

        let huge_min = clamp_rect_to_work([100, 100, 300, 300], [0, 0, 640, 480], [900, 700]);
        assert_eq!(huge_min, [0, 0, 640, 480]);
    }
    #[test]
    fn fit_settings_rect_scales_and_fits_on_low_res_high_dpi_screens() {
        for (work, dpi) in [([0, 0, 1280, 720], 1.5_f32), ([0, 0, 1366, 768], 1.25_f32)] {
            let r = fit_settings_rect(Some([100, 400, 800, 700]), work, dpi);
            assert!(r[0] >= work[0] && r[2] <= work[2], "width overflowed {r:?} on {work:?}");
            assert!(r[1] >= work[1] && r[3] <= work[3], "height overflowed {r:?} on {work:?}");
            assert!(r[2] - r[0] >= 280 && r[3] - r[1] >= 240);
        }
    }
    #[test]
    fn choose_group_width_forms_compact_rectangles_for_wide_groups() {
        let expected_8 = [
            (2, 2),
            (3, 3),
            (5, 5),
            (6, 6),
            (7, 7),
            (8, 8),
            (9, 5),
            (10, 5),
            (11, 6),
            (12, 6),
            (13, 7),
            (14, 7),
            (15, 8),
            (16, 8),
            (17, 6),
        ];
        for (n, gw) in expected_8 {
            assert_eq!(choose_group_width(n, 8), gw, "n={n} columns=8");
        }
        assert_eq!(choose_group_width(5, 4), 3);
        assert_eq!(choose_group_width(6, 4), 3);
        assert_eq!(choose_group_width(8, 4), 4);
    }
    #[test]
    fn pack_dock_grid_packs_10_icon_group_as_5x2_with_adjacent_items_beside_it() {
        let mut groups: Vec<Option<&str>> = Vec::new();
        groups.extend(std::iter::repeat_n(Some("g1"), 3));
        groups.extend(std::iter::repeat_n(Some("g2"), 2));
        groups.extend(std::iter::repeat_n(Some("g3"), 3));
        groups.extend(std::iter::repeat_n(Some("wide"), 10));
        groups.extend(std::iter::repeat_n(None, 6));

        let layout = pack_dock_grid(&groups, true, 8);
        assert_eq!(layout.rows, 4);

        let wide_run = layout
            .group_runs
            .iter()
            .find(|(id, _)| id == "wide")
            .unwrap();
        let expected_wide = vec![
            (0, 1), (1, 1), (2, 1), (3, 1), (4, 1),
            (0, 2), (1, 2), (2, 2), (3, 2), (4, 2),
        ];
        assert_eq!(wide_run.1, expected_wide);

        let expected_ungrouped = [(5, 1), (6, 1), (7, 1), (5, 2), (6, 2), (7, 2)];
        assert_eq!(&layout.cells[18..24], &expected_ungrouped);
        assert_eq!(layout.cells[24], (0, 3));

        let mut seen = std::collections::HashSet::new();
        for cell in &layout.cells {
            assert!(seen.insert(*cell), "duplicate cell {cell:?}");
        }
    }
}
