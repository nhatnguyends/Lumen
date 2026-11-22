//! Manages independent per-monitor duplicate widget instances, native-DPI
//! placement/clamping, and per-display layout persistence.
use crate::displays::{self, DisplayInfo};
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    collections::HashMap,
    mem::{size_of, zeroed},
    path::{Path, PathBuf},
    ptr::null_mut,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH},
    UI::WindowsAndMessaging::*,
};

pub const SYNC_MESSAGE: u32 = WM_APP + 23;
pub const WIDGET_SYNC_MESSAGE: u32 = WM_APP + 230;
pub const HIDE_WIDGET_MESSAGE: u32 = WM_APP + 231;
const LAYOUT_FILE: &str = "multi-monitor-layout.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedRect {
    pub x: i32,
    pub y: i32,
    pub w_dip: i32,
    pub h_dip: i32,
}

type LayoutStore = HashMap<String, HashMap<String, SavedRect>>;

#[derive(Clone)]
struct DuplicateMeta {
    widget_id: String,
    display_id: String,
}

#[derive(Default)]
struct State {
    enabled: bool,
    selected_ids: Vec<String>,
    displays: Vec<DisplayInfo>,
    root: Option<PathBuf>,
    smoke: bool,
    host_hwnd: usize,
    layout: LayoutStore,
    duplicates: HashMap<usize, DuplicateMeta>,
    instances: HashMap<usize, String>,
    pending_hides: Vec<String>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn load_layout(root: &Path) -> LayoutStore {
    let path = root.join("config").join(LAYOUT_FILE);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            crate::logger::warn(
                "multi-monitor",
                format!("ignoring corrupt {LAYOUT_FILE}: {e}"),
            );
            HashMap::new()
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
        Err(e) => {
            crate::logger::warn("multi-monitor", format!("failed reading {LAYOUT_FILE}: {e}"));
            HashMap::new()
        }
    }
}

fn save_layout(root: &Path, layout: &LayoutStore) -> Result<(), String> {
    let dir = root.join("config");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let temp = dir.join(format!("{LAYOUT_FILE}.tmp"));
    let dest = dir.join(LAYOUT_FILE);
    std::fs::write(
        &temp,
        serde_json::to_vec_pretty(layout).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    unsafe {
        if MoveFileExW(
            crate::model::wide(&temp).as_ptr(),
            crate::model::wide(&dest).as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        ) == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
    }
    Ok(())
}

pub fn active_targets<'a>(
    displays: &'a [DisplayInfo],
    selected_ids: &[String],
    src_bounds: [i32; 4],
) -> Vec<&'a DisplayInfo> {
    displays
        .iter()
        .filter(|d| {
            d.enabled
                && d.bounds.is_some()
                && d.work.is_some()
                && d.bounds != Some(src_bounds)
                && selected_ids.iter().any(|id| id == &d.id)
        })
        .collect()
}

pub fn map_rect_native_dpi(
    src_rect: [i32; 4],
    src_work: [i32; 4],
    src_dpi: f32,
    dst_work: [i32; 4],
    dst_dpi: f32,
    min_dip: [i32; 2],
) -> [i32; 4] {
    let src_dpi = src_dpi.max(1.0);
    let dst_dpi = dst_dpi.max(1.0);
    let min_w_dip = min_dip[0].max(1) as f32;
    let min_h_dip = min_dip[1].max(1) as f32;
    let w_dip = ((src_rect[2] - src_rect[0]).max(1) as f32 / src_dpi).max(min_w_dip);
    let h_dip = ((src_rect[3] - src_rect[1]).max(1) as f32 / src_dpi).max(min_h_dip);
    let dst_w_px = (w_dip * dst_dpi).round() as i32;
    let dst_h_px = (h_dip * dst_dpi).round() as i32;
    let src_work_w = (src_work[2] - src_work[0]).max(1) as f32;
    let src_work_h = (src_work[3] - src_work[1]).max(1) as f32;
    let dst_work_w = (dst_work[2] - dst_work[0]).max(1) as f32;
    let dst_work_h = (dst_work[3] - dst_work[1]).max(1) as f32;
    let rel_x = (src_rect[0] - src_work[0]) as f32 / src_work_w;
    let rel_y = (src_rect[1] - src_work[1]) as f32 / src_work_h;
    let x = dst_work[0] + (rel_x * dst_work_w).round() as i32;
    let y = dst_work[1] + (rel_y * dst_work_h).round() as i32;
    let min_px = [
        (min_w_dip * dst_dpi).round() as i32,
        (min_h_dip * dst_dpi).round() as i32,
    ];
    crate::geometry::clamp_rect_to_work([x, y, x + dst_w_px, y + dst_h_px], dst_work, min_px)
}

unsafe fn monitor_bounds(hwnd: HWND) -> Option<([i32; 4], [i32; 4])> {
    if hwnd.is_null() {
        return None;
    }
    let hmon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
    if hmon.is_null() {
        return None;
    }
    let mut info: MONITORINFO = zeroed();
    info.cbSize = size_of::<MONITORINFO>() as u32;
    if GetMonitorInfoW(hmon, &mut info) == 0 {
        return None;
    }
    let m = info.rcMonitor;
    let w = info.rcWork;
    Some((
        [m.left, m.top, m.right, m.bottom],
        [w.left, w.top, w.right, w.bottom],
    ))
}

pub fn configure_root(root: &Path, smoke: bool) {
    let layout = if smoke {
        HashMap::new()
    } else {
        load_layout(root)
    };
    STATE.with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            s.root = Some(root.to_path_buf());
            s.smoke = smoke;
            s.layout = layout;
        }
    });
}

pub unsafe fn set_host(hwnd: HWND) {
    STATE.with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            s.host_hwnd = hwnd as usize;
        }
    });
}

pub unsafe fn request_sync() {
    let host = STATE.with(|s| {
        s.try_borrow()
            .ok()
            .map(|s| s.host_hwnd)
            .filter(|&h| h != 0)
    });
    if let Some(hwnd) = host {
        let hwnd = hwnd as HWND;
        if IsWindow(hwnd) != 0 {
            PostMessageW(hwnd, SYNC_MESSAGE, 0, 0);
        }
    }
}

pub unsafe fn configure(enabled: bool, selected_ids: Vec<String>) {
    let enumerated = if enabled {
        displays::enumerate()
    } else {
        Vec::new()
    };
    STATE.with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            s.enabled = enabled;
            s.selected_ids = selected_ids;
            if !enumerated.is_empty() || !enabled {
                s.displays = enumerated;
            }
        }
    });
    request_sync();
}

pub unsafe fn refresh_displays() {
    let enumerated = displays::enumerate();
    STATE.with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            s.displays = enumerated;
        }
    });
    request_sync();
}

pub unsafe fn sync_visibility() {
    request_sync();
}

pub fn is_enabled() -> bool {
    STATE.with(|s| s.try_borrow().map_or(false, |s| s.enabled))
}

pub unsafe fn target_displays_for(src_hwnd: HWND) -> Vec<DisplayInfo> {
    let Some((src_mon, _)) = monitor_bounds(src_hwnd) else {
        return Vec::new();
    };
    let need_enum = STATE.with(|s| {
        s.try_borrow()
            .map_or(false, |s| s.enabled && !s.selected_ids.is_empty() && s.displays.is_empty())
    });
    let enumerated = if need_enum {
        Some(displays::enumerate())
    } else {
        None
    };
    STATE.with(|s| {
        let Ok(mut s) = s.try_borrow_mut() else {
            return Vec::new();
        };
        if !s.enabled || s.selected_ids.is_empty() {
            return Vec::new();
        }
        if let Some(list) = enumerated {
            if s.displays.is_empty() {
                s.displays = list;
            }
        }
        active_targets(&s.displays, &s.selected_ids, src_mon)
            .into_iter()
            .cloned()
            .collect()
    })
}

pub unsafe fn display_dpi(work: [i32; 4]) -> f32 {
    let rect = RECT {
        left: work[0],
        top: work[1],
        right: work[2],
        bottom: work[3],
    };
    let hmon = MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST);
    crate::geometry::monitor_dpi(hmon, null_mut())
}

pub fn register_duplicate(hwnd: HWND, widget_id: &str, display_id: &str) {
    if hwnd.is_null() {
        return;
    }
    STATE.with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            let key = hwnd as usize;
            s.duplicates.insert(
                key,
                DuplicateMeta {
                    widget_id: widget_id.to_owned(),
                    display_id: display_id.to_owned(),
                },
            );
            s.instances.insert(key, widget_id.to_owned());
        }
    });
}

pub fn unregister_duplicate(hwnd: HWND) {
    if hwnd.is_null() {
        return;
    }
    STATE.with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            let key = hwnd as usize;
            s.duplicates.remove(&key);
            s.instances.remove(&key);
        }
    });
}

pub fn unregister_widget(hwnd: HWND) {
    if hwnd.is_null() {
        return;
    }
    STATE.with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            let key = hwnd as usize;
            s.duplicates.remove(&key);
            s.instances.remove(&key);
        }
    });
}

pub fn peer_hwnds_for(
    instances: &HashMap<usize, String>,
    widget_id: &str,
    source_hwnd: usize,
) -> Vec<usize> {
    instances
        .iter()
        .filter_map(|(&hwnd, id)| (id == widget_id && hwnd != source_hwnd).then_some(hwnd))
        .collect()
}

pub unsafe fn notify_peers(widget_id: &str, source_hwnd: HWND) {
    let peers = STATE.with(|s| {
        s.try_borrow()
            .ok()
            .map(|s| peer_hwnds_for(&s.instances, widget_id, source_hwnd as usize))
            .unwrap_or_default()
    });
    for peer in peers {
        let hwnd = peer as HWND;
        if !hwnd.is_null() && IsWindow(hwnd) != 0 {
            PostMessageW(hwnd, WIDGET_SYNC_MESSAGE, 0, 0);
        }
    }
}

pub fn is_duplicate(hwnd: HWND) -> bool {
    if hwnd.is_null() {
        return false;
    }
    STATE.with(|s| {
        s.try_borrow()
            .map_or(false, |s| s.duplicates.contains_key(&(hwnd as usize)))
    })
}

pub unsafe fn record_duplicate_bounds(hwnd: HWND, dpi: f32) -> bool {
    if hwnd.is_null() {
        return false;
    }
    let meta = STATE.with(|s| {
        s.try_borrow()
            .ok()
            .and_then(|s| s.duplicates.get(&(hwnd as usize)).cloned())
    });
    let Some(meta) = meta else {
        return false;
    };
    let mut r: RECT = zeroed();
    if GetWindowRect(hwnd, &mut r) == 0 {
        return true;
    }
    let dpi = dpi.max(1.0);
    let saved = SavedRect {
        x: r.left,
        y: r.top,
        w_dip: ((r.right - r.left).max(1) as f32 / dpi).round() as i32,
        h_dip: ((r.bottom - r.top).max(1) as f32 / dpi).round() as i32,
    };
    STATE.with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            s.layout
                .entry(meta.display_id)
                .or_default()
                .insert(meta.widget_id, saved);
            if !s.smoke {
                if let Some(root) = &s.root {
                    if let Err(e) = save_layout(root, &s.layout) {
                        crate::logger::warn("multi-monitor", format!("failed saving layout: {e}"));
                    }
                }
            }
        }
    });
    true
}

pub unsafe fn compute_duplicate_rect(
    widget_id: &str,
    display_id: &str,
    src_hwnd: HWND,
    dst_work: [i32; 4],
    dst_dpi: f32,
    min_dip: [i32; 2],
) -> [i32; 4] {
    let saved = STATE.with(|s| {
        s.try_borrow().ok().and_then(|s| {
            s.layout
                .get(display_id)
                .and_then(|m| m.get(widget_id))
                .copied()
        })
    });
    let dst_dpi = dst_dpi.max(1.0);
    let min_px = [
        (min_dip[0].max(1) as f32 * dst_dpi).round() as i32,
        (min_dip[1].max(1) as f32 * dst_dpi).round() as i32,
    ];
    if let Some(saved) = saved {
        let w = (saved.w_dip.max(min_dip[0]) as f32 * dst_dpi).round() as i32;
        let h = (saved.h_dip.max(min_dip[1]) as f32 * dst_dpi).round() as i32;
        return crate::geometry::clamp_rect_to_work(
            [saved.x, saved.y, saved.x + w, saved.y + h],
            dst_work,
            min_px,
        );
    }
    let mut r: RECT = zeroed();
    if GetWindowRect(src_hwnd, &mut r) == 0 {
        let w = (min_dip[0].max(200) as f32 * dst_dpi).round() as i32;
        let h = (min_dip[1].max(160) as f32 * dst_dpi).round() as i32;
        return crate::geometry::clamp_rect_to_work(
            [dst_work[0] + 40, dst_work[1] + 40, dst_work[0] + 40 + w, dst_work[1] + 40 + h],
            dst_work,
            min_px,
        );
    }
    let src_rect = [r.left, r.top, r.right, r.bottom];
    let src_work = monitor_bounds(src_hwnd)
        .map(|(_, w)| w)
        .unwrap_or(dst_work);
    let src_dpi = crate::geometry::window_dpi(src_hwnd);
    map_rect_native_dpi(src_rect, src_work, src_dpi, dst_work, dst_dpi, min_dip)
}

pub unsafe fn place_duplicate_window(
    hwnd: HWND,
    widget_id: &str,
    display_id: &str,
    src_hwnd: HWND,
    dst_work: [i32; 4],
    dst_dpi: f32,
    min_dip: [i32; 2],
) {
    if hwnd.is_null() || IsWindow(hwnd) == 0 {
        return;
    }
    let r = compute_duplicate_rect(widget_id, display_id, src_hwnd, dst_work, dst_dpi, min_dip);
    SetWindowPos(
        hwnd,
        null_mut(),
        r[0],
        r[1],
        (r[2] - r[0]).max(1),
        (r[3] - r[1]).max(1),
        SWP_NOZORDER | SWP_NOACTIVATE,
    );
    SendMessageW(hwnd, WM_DISPLAYCHANGE, 0, 0);
}

pub unsafe fn register_widget(hwnd: HWND, widget_id: &str) {
    if hwnd.is_null() {
        return;
    }
    STATE.with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            s.instances.insert(hwnd as usize, widget_id.to_owned());
        }
    });
}

pub unsafe fn request_hide_widget(widget_id: &str) {
    let host = STATE.with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            if !s.pending_hides.iter().any(|id| id == widget_id) {
                s.pending_hides.push(widget_id.to_owned());
            }
            (s.host_hwnd != 0).then_some(s.host_hwnd)
        } else {
            None
        }
    });
    if let Some(hwnd) = host {
        let hwnd = hwnd as HWND;
        if IsWindow(hwnd) != 0 {
            PostMessageW(hwnd, HIDE_WIDGET_MESSAGE, 0, 0);
        }
    }
}

pub fn take_pending_hides() -> Vec<String> {
    STATE.with(|s| {
        s.try_borrow_mut()
            .map(|mut s| std::mem::take(&mut s.pending_hides))
            .unwrap_or_default()
    })
}

pub unsafe fn shutdown() {
    STATE.with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            s.enabled = false;
            s.host_hwnd = 0;
            s.duplicates.clear();
            s.instances.clear();
            s.pending_hides.clear();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_rect_native_dpi_scales_across_mixed_dpi_and_resolutions() {
        // 4K @ 150% DPI -> 1080p @ 100% DPI preserves logical DIP size cleanly.
        let r = map_rect_native_dpi(
            [384, 210, 384 + 600, 210 + 300],
            [0, 0, 3840, 2100],
            1.5,
            [3840, 0, 5760, 1040],
            1.0,
            [110, 80],
        );
        assert_eq!(r[2] - r[0], 400);
        assert_eq!(r[3] - r[1], 200);
        assert_eq!(r[0], 3840 + 192);
        assert_eq!(r[1], 104);

        // Same DPI and same resolution preserves exact DIP size.
        let same = map_rect_native_dpi(
            [100, 80, 500, 380],
            [0, 0, 1920, 1080],
            1.0,
            [1920, 0, 3840, 1080],
            1.0,
            [110, 80],
        );
        assert_eq!((same[0], same[1], same[2] - same[0], same[3] - same[1]), (2020, 80, 400, 300));
    }

    #[test]
    fn map_rect_native_dpi_clamps_to_portrait_work_area() {
        let r = map_rect_native_dpi(
            [1600, 1600, 2300, 2000],
            [0, 0, 1920, 1080],
            1.0,
            [1920, 0, 3000, 1920],
            1.25,
            [160, 120],
        );
        assert!(r[0] >= 1920);
        assert!(r[2] <= 3000);
        assert!(r[1] >= 0);
        assert!(r[3] <= 1920);
    }

    #[test]
    fn active_targets_excludes_disabled_and_source_displays() {
        let displays = vec![
            DisplayInfo {
                id: "d1".into(),
                source: r"\\.\DISPLAY1".into(),
                monitor_path: "m1".into(),
                name: "Primary".into(),
                enabled: true,
                primary: true,
                bounds: Some([0, 0, 2560, 1440]),
                work: Some([0, 0, 2560, 1400]),
            },
            DisplayInfo {
                id: "d2".into(),
                source: r"\\.\DISPLAY2".into(),
                monitor_path: "m2".into(),
                name: "Secondary".into(),
                enabled: true,
                primary: false,
                bounds: Some([2560, 0, 4480, 1080]),
                work: Some([2560, 0, 4480, 1040]),
            },
            DisplayInfo {
                id: "d3".into(),
                source: r"\\.\DISPLAY3".into(),
                monitor_path: "m3".into(),
                name: "Turned Off".into(),
                enabled: false,
                primary: false,
                bounds: None,
                work: None,
            },
        ];
        let selected = vec!["d1".into(), "d2".into(), "d3".into()];
        let targets = active_targets(&displays, &selected, [0, 0, 2560, 1440]);
        let ids: Vec<&str> = targets.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, vec!["d2"]);
    }

    #[test]
    fn layout_store_round_trips_and_ignores_corrupt_files() {
        let root = std::env::temp_dir().join(format!("lumen-mm-layout-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("config")).unwrap();

        let corrupt = root.join("config").join(LAYOUT_FILE);
        std::fs::write(&corrupt, "{not-valid-json").unwrap();
        assert!(load_layout(&root).is_empty());

        let mut store: LayoutStore = HashMap::new();
        store.entry("disp-2".into()).or_default().insert(
            "clock".into(),
            SavedRect {
                x: 2040,
                y: 120,
                w_dip: 260,
                h_dip: 140,
            },
        );
        save_layout(&root, &store).unwrap();
        let loaded = load_layout(&root);
        assert_eq!(loaded, store);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn peer_hwnds_for_returns_only_other_instances_of_same_widget() {
        let mut instances = HashMap::new();
        instances.insert(101, "notes".into());
        instances.insert(202, "notes".into());
        instances.insert(303, "notes".into());
        instances.insert(404, "clock".into());

        let mut peers = peer_hwnds_for(&instances, "notes", 202);
        peers.sort_unstable();
        assert_eq!(peers, vec![101, 303]);
        assert!(peer_hwnds_for(&instances, "calendar", 101).is_empty());
    }
}
