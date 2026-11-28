//! Manages independent per-monitor duplicate widget instances, native-DPI
//! placement/clamping, and per-display layout persistence.
use crate::displays::{self, DisplayInfo};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
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
const PRIMARY_LAYOUT_FILE: &str = "primary-monitor-layout.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedRect {
    pub x: i32,
    pub y: i32,
    pub w_dip: i32,
    pub h_dip: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rel_x: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rel_y: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dpi_x100: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floating: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrimaryHome {
    pub display_id: String,
    pub rect: SavedRect,
}

type LayoutStore = HashMap<String, HashMap<String, SavedRect>>;
type PrimaryLayoutStore = HashMap<String, PrimaryHome>;

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
    creating_duplicate: u32,
    layout: LayoutStore,
    primary_layout: PrimaryLayoutStore,
    duplicates: HashMap<usize, DuplicateMeta>,
    instances: HashMap<usize, String>,
    pending_hides: Vec<String>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn load_json_store<T: DeserializeOwned + Default>(root: &Path, file: &str) -> T {
    let path = root.join("config").join(file);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            crate::logger::warn("multi-monitor", format!("ignoring corrupt {file}: {e}"));
            T::default()
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => T::default(),
        Err(e) => {
            crate::logger::warn("multi-monitor", format!("failed reading {file}: {e}"));
            T::default()
        }
    }
}

fn save_json_store<T: Serialize>(root: &Path, file: &str, data: &T) -> Result<(), String> {
    let dir = root.join("config");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let temp = dir.join(format!("{file}.tmp"));
    let dest = dir.join(file);
    std::fs::write(
        &temp,
        serde_json::to_vec_pretty(data).map_err(|e| e.to_string())?,
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

pub fn resolve_saved_rect(
    saved: SavedRect,
    dst_work: [i32; 4],
    dst_dpi: f32,
    min_dip: [i32; 2],
    legacy_origin: Option<[i32; 2]>,
) -> [i32; 4] {
    let dst_dpi = dst_dpi.max(1.0);
    let min_w_dip = min_dip[0].max(1);
    let min_h_dip = min_dip[1].max(1);
    let w = (saved.w_dip.max(min_w_dip) as f32 * dst_dpi).round() as i32;
    let h = (saved.h_dip.max(min_h_dip) as f32 * dst_dpi).round() as i32;
    let min_px = [
        (min_w_dip as f32 * dst_dpi).round() as i32,
        (min_h_dip as f32 * dst_dpi).round() as i32,
    ];
    let (x, y) = if let (Some(rx), Some(ry)) = (saved.rel_x, saved.rel_y) {
        let (sx, sy) = match saved.dpi_x100 {
            Some(d) if d > 0 => {
                let saved_dpi = d as f32 / 100.0;
                if (dst_dpi - saved_dpi).abs() > 0.005 {
                    let scale = dst_dpi / saved_dpi;
                    (
                        (rx as f32 * scale).round() as i32,
                        (ry as f32 * scale).round() as i32,
                    )
                } else {
                    (rx, ry)
                }
            }
            _ => (rx, ry),
        };
        (dst_work[0] + sx, dst_work[1] + sy)
    } else if saved.x >= dst_work[0]
        && saved.x < dst_work[2]
        && saved.y >= dst_work[1]
        && saved.y < dst_work[3]
    {
        (saved.x, saved.y)
    } else if let Some([ox, oy]) = legacy_origin {
        (dst_work[0] + (saved.x - ox), dst_work[1] + (saved.y - oy))
    } else {
        (saved.x, saved.y)
    };
    crate::geometry::clamp_rect_to_work([x, y, x + w, y + h], dst_work, min_px)
}

fn make_saved_rect(r: RECT, work: [i32; 4], dpi: f32) -> SavedRect {
    let dpi = dpi.max(1.0);
    SavedRect {
        x: r.left,
        y: r.top,
        w_dip: ((r.right - r.left).max(1) as f32 / dpi).round() as i32,
        h_dip: ((r.bottom - r.top).max(1) as f32 / dpi).round() as i32,
        rel_x: Some(r.left - work[0]),
        rel_y: Some(r.top - work[1]),
        dpi_x100: Some((dpi * 100.0).round() as u32),
        floating: None,
    }
}

fn find_display_for_bounds<'a>(
    displays: &'a [DisplayInfo],
    mon_bounds: [i32; 4],
) -> Option<&'a DisplayInfo> {
    displays
        .iter()
        .find(|d| d.enabled && d.bounds == Some(mon_bounds) && d.work.is_some())
}

unsafe fn ensure_displays_loaded() {
    let need = STATE.with(|s| s.try_borrow().map_or(false, |s| s.displays.is_empty()));
    if need {
        let enumerated = displays::enumerate();
        STATE.with(|s| {
            if let Ok(mut s) = s.try_borrow_mut() {
                if s.displays.is_empty() {
                    s.displays = enumerated;
                }
            }
        });
    }
}

pub fn configure_root(root: &Path, smoke: bool) {
    let (layout, primary_layout) = if smoke {
        (HashMap::new(), HashMap::new())
    } else {
        (
            load_json_store(root, LAYOUT_FILE),
            load_json_store(root, PRIMARY_LAYOUT_FILE),
        )
    };
    STATE.with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            s.root = Some(root.to_path_buf());
            s.smoke = smoke;
            s.layout = layout;
            s.primary_layout = primary_layout;
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

pub unsafe fn refresh_displays_now() {
    let enumerated = displays::enumerate();
    STATE.with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            s.displays = enumerated;
        }
    });
}

pub unsafe fn refresh_displays() {
    refresh_displays_now();
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

pub unsafe fn notify_peers_with_message(widget_id: &str, source_hwnd: HWND, message: u32) {
    let peers = STATE.with(|s| {
        s.try_borrow()
            .ok()
            .map(|s| peer_hwnds_for(&s.instances, widget_id, source_hwnd as usize))
            .unwrap_or_default()
    });
    for peer in peers {
        let hwnd = peer as HWND;
        if !hwnd.is_null() && IsWindow(hwnd) != 0 {
            PostMessageW(hwnd, message, 0, 0);
        }
    }
}

pub unsafe fn notify_peers(widget_id: &str, source_hwnd: HWND) {
    notify_peers_with_message(widget_id, source_hwnd, WIDGET_SYNC_MESSAGE);
}

pub struct DuplicateCreationGuard;

impl DuplicateCreationGuard {
    pub fn enter() -> Self {
        STATE.with(|s| {
            if let Ok(mut s) = s.try_borrow_mut() {
                s.creating_duplicate = s.creating_duplicate.saturating_add(1);
            }
        });
        Self
    }
}

impl Drop for DuplicateCreationGuard {
    fn drop(&mut self) {
        STATE.with(|s| {
            if let Ok(mut s) = s.try_borrow_mut() {
                s.creating_duplicate = s.creating_duplicate.saturating_sub(1);
            }
        });
    }
}

pub fn is_duplicate(hwnd: HWND) -> bool {
    if hwnd.is_null() {
        return false;
    }
    STATE.with(|s| {
        s.try_borrow().map_or(false, |s| {
            s.creating_duplicate > 0 || s.duplicates.contains_key(&(hwnd as usize))
        })
    })
}

pub unsafe fn record_duplicate_bounds(hwnd: HWND, dpi: f32) -> bool {
    if hwnd.is_null() {
        return false;
    }
    let (creating, meta) = STATE.with(|s| {
        s.try_borrow()
            .ok()
            .map(|s| {
                (
                    s.creating_duplicate > 0,
                    s.duplicates.get(&(hwnd as usize)).cloned(),
                )
            })
            .unwrap_or((false, None))
    });
    if creating {
        return true;
    }
    let Some(meta) = meta else {
        return false;
    };
    let mut r: RECT = zeroed();
    if GetWindowRect(hwnd, &mut r) == 0 {
        return true;
    }
    let Some((cur_mon, _)) = monitor_bounds(hwnd) else {
        return true;
    };
    ensure_displays_loaded();
    STATE.with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            let assigned_work = s
                .displays
                .iter()
                .find(|d| d.id == meta.display_id && d.enabled && d.bounds == Some(cur_mon))
                .and_then(|d| d.work);
            let Some(work) = assigned_work else {
                return;
            };
            let mut saved = make_saved_rect(r, work, dpi);
            saved.floating = s
                .layout
                .get(&meta.display_id)
                .and_then(|m| m.get(&meta.widget_id))
                .and_then(|prev| prev.floating);
            s.layout
                .entry(meta.display_id)
                .or_default()
                .insert(meta.widget_id, saved);
            if !s.smoke {
                if let Some(root) = &s.root {
                    if let Err(e) = save_json_store(root, LAYOUT_FILE, &s.layout) {
                        crate::logger::warn("multi-monitor", format!("failed saving layout: {e}"));
                    }
                }
            }
        }
    });
    true
}

pub fn duplicate_floating(hwnd: HWND) -> bool {
    if hwnd.is_null() {
        return false;
    }
    STATE.with(|s| {
        s.try_borrow()
            .ok()
            .and_then(|s| {
                let meta = s.duplicates.get(&(hwnd as usize))?;
                s.layout
                    .get(&meta.display_id)?
                    .get(&meta.widget_id)?
                    .floating
            })
            .unwrap_or(false)
    })
}

pub unsafe fn record_duplicate_floating(hwnd: HWND, dpi: f32, floating: bool) -> bool {
    if hwnd.is_null() {
        return false;
    }
    let (creating, meta) = STATE.with(|s| {
        s.try_borrow()
            .ok()
            .map(|s| {
                (
                    s.creating_duplicate > 0,
                    s.duplicates.get(&(hwnd as usize)).cloned(),
                )
            })
            .unwrap_or((false, None))
    });
    if creating {
        return true;
    }
    let Some(meta) = meta else {
        return false;
    };
    let has_entry = STATE.with(|s| {
        s.try_borrow()
            .ok()
            .and_then(|s| s.layout.get(&meta.display_id)?.get(&meta.widget_id).copied())
            .is_some()
    });
    let mut rect: RECT = zeroed();
    let mon_bounds = if !has_entry {
        if GetWindowRect(hwnd, &mut rect) == 0 {
            None
        } else {
            ensure_displays_loaded();
            monitor_bounds(hwnd)
        }
    } else {
        None
    };
    STATE.with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            if let Some(entry) = s
                .layout
                .get_mut(&meta.display_id)
                .and_then(|m| m.get_mut(&meta.widget_id))
            {
                entry.floating = Some(floating);
            } else if let Some((cur_mon, mon_work)) = mon_bounds {
                let work = s
                    .displays
                    .iter()
                    .find(|d| d.id == meta.display_id && d.enabled && d.bounds == Some(cur_mon))
                    .and_then(|d| d.work)
                    .unwrap_or(mon_work);
                let mut saved = make_saved_rect(rect, work, dpi);
                saved.floating = Some(floating);
                s.layout
                    .entry(meta.display_id)
                    .or_default()
                    .insert(meta.widget_id, saved);
            } else {
                return;
            }
            if !s.smoke {
                if let Some(root) = &s.root {
                    if let Err(e) = save_json_store(root, LAYOUT_FILE, &s.layout) {
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
        return resolve_saved_rect(
            saved,
            dst_work,
            dst_dpi,
            min_dip,
            displays::saved_entry(display_id).map(|e| [e.x, e.y]),
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

unsafe fn set_window_rect(hwnd: HWND, r: [i32; 4]) -> bool {
    let mut cur: RECT = zeroed();
    if GetWindowRect(hwnd, &mut cur) != 0 && [cur.left, cur.top, cur.right, cur.bottom] == r {
        return false;
    }
    let flags = SWP_NOZORDER | SWP_NOACTIVATE;
    SetWindowPos(hwnd, null_mut(), r[0], r[1], (r[2] - r[0]).max(1), (r[3] - r[1]).max(1), flags);
    // A cross-DPI move triggers WM_DPICHANGED synchronously, which may shift the
    // window to Windows's suggested rect; re-apply once on the target monitor.
    if GetWindowRect(hwnd, &mut cur) != 0 && [cur.left, cur.top, cur.right, cur.bottom] != r {
        SetWindowPos(hwnd, null_mut(), r[0], r[1], (r[2] - r[0]).max(1), (r[3] - r[1]).max(1), flags);
    }
    true
}

pub unsafe fn place_duplicate_window(
    hwnd: HWND,
    widget_id: &str,
    display_id: &str,
    src_hwnd: HWND,
    dst_work: [i32; 4],
    dst_dpi: f32,
    min_dip: [i32; 2],
    force_notify: bool,
) {
    if hwnd.is_null() || IsWindow(hwnd) == 0 {
        return;
    }
    let r = compute_duplicate_rect(widget_id, display_id, src_hwnd, dst_work, dst_dpi, min_dip);
    let moved = set_window_rect(hwnd, r);
    if moved || force_notify {
        SendMessageW(hwnd, WM_DISPLAYCHANGE, 0, 0);
    }
}

pub unsafe fn record_primary_bounds(hwnd: HWND, widget_id: &str, dpi: f32) {
    if hwnd.is_null() || is_duplicate(hwnd) {
        return;
    }
    let mut r: RECT = zeroed();
    if GetWindowRect(hwnd, &mut r) == 0 || r.right <= r.left || r.bottom <= r.top {
        return;
    }
    let Some((cur_mon, _)) = monitor_bounds(hwnd) else {
        return;
    };
    ensure_displays_loaded();
    STATE.with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            if let Some(existing) = s.primary_layout.get(widget_id) {
                if !s.displays.is_empty()
                    && !s
                        .displays
                        .iter()
                        .any(|d| d.id == existing.display_id && d.enabled && d.work.is_some())
                {
                    return;
                }
            }
            let Some(disp) = find_display_for_bounds(&s.displays, cur_mon) else {
                return;
            };
            let Some(work) = disp.work else {
                return;
            };
            let home = PrimaryHome {
                display_id: disp.id.clone(),
                rect: make_saved_rect(r, work, dpi),
            };
            s.primary_layout.insert(widget_id.to_owned(), home);
            if !s.smoke {
                if let Some(root) = &s.root {
                    if let Err(e) = save_json_store(root, PRIMARY_LAYOUT_FILE, &s.primary_layout) {
                        crate::logger::warn(
                            "multi-monitor",
                            format!("failed saving primary layout: {e}"),
                        );
                    }
                }
            }
        }
    });
}

pub fn is_primary_displaced(widget_id: &str) -> bool {
    STATE.with(|s| {
        s.try_borrow().map_or(false, |s| {
            if s.displays.is_empty() {
                return false;
            }
            let Some(home) = s.primary_layout.get(widget_id) else {
                return false;
            };
            !s.displays
                .iter()
                .any(|d| d.id == home.display_id && d.enabled && d.work.is_some())
        })
    })
}

pub unsafe fn restore_primary_window(hwnd: HWND, widget_id: &str, min_dip: [i32; 2]) -> bool {
    if hwnd.is_null() || IsWindow(hwnd) == 0 || is_duplicate(hwnd) {
        return false;
    }
    ensure_displays_loaded();
    let cur_mon = monitor_bounds(hwnd).map(|(m, _)| m);
    let target = STATE.with(|s| {
        s.try_borrow().ok().and_then(|s| {
            let home = s.primary_layout.get(widget_id)?;
            if let Some(home_disp) = s
                .displays
                .iter()
                .find(|d| d.id == home.display_id && d.enabled && d.work.is_some())
            {
                let work = home_disp.work?;
                return Some((home_disp.id.clone(), home.rect, work));
            }
            let cur_mon = cur_mon?;
            let cur_disp = find_display_for_bounds(&s.displays, cur_mon)?;
            let work = cur_disp.work?;
            let saved = s.layout.get(&cur_disp.id)?.get(widget_id).copied()?;
            Some((cur_disp.id.clone(), saved, work))
        })
    });
    let Some((target_id, saved, work)) = target else {
        return false;
    };
    let r = resolve_saved_rect(
        saved,
        work,
        display_dpi(work),
        min_dip,
        displays::saved_entry(&target_id).map(|e| [e.x, e.y]),
    );
    set_window_rect(hwnd, r)
}

pub unsafe fn register_widget(hwnd: HWND, widget_id: &str) {
    if hwnd.is_null() {
        return;
    }
    let need_seed = STATE.with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            s.instances.insert(hwnd as usize, widget_id.to_owned());
            !s.primary_layout.contains_key(widget_id)
        } else {
            false
        }
    });
    if need_seed {
        let mut r: RECT = zeroed();
        if GetWindowRect(hwnd, &mut r) != 0 && r.right > r.left && r.bottom > r.top {
            record_primary_bounds(hwnd, widget_id, crate::geometry::window_dpi(hwnd));
        }
    }
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
        assert!(load_json_store::<LayoutStore>(&root, LAYOUT_FILE).is_empty());

        let mut store: LayoutStore = HashMap::new();
        store.entry("disp-2".into()).or_default().insert(
            "clock".into(),
            SavedRect {
                x: 2040,
                y: 120,
                w_dip: 260,
                h_dip: 140,
                rel_x: Some(120),
                rel_y: Some(120),
                dpi_x100: Some(150),
                floating: Some(true),
            },
        );
        save_json_store(&root, LAYOUT_FILE, &store).unwrap();
        let loaded: LayoutStore = load_json_store(&root, LAYOUT_FILE);
        assert_eq!(loaded, store);

        let corrupt_primary = root.join("config").join(PRIMARY_LAYOUT_FILE);
        std::fs::write(&corrupt_primary, "{bad-json").unwrap();
        assert!(load_json_store::<PrimaryLayoutStore>(&root, PRIMARY_LAYOUT_FILE).is_empty());

        let mut primary: PrimaryLayoutStore = HashMap::new();
        primary.insert(
            "notes".into(),
            PrimaryHome {
                display_id: "disp-1".into(),
                rect: SavedRect {
                    x: 100,
                    y: 80,
                    w_dip: 440,
                    h_dip: 420,
                    rel_x: Some(100),
                    rel_y: Some(80),
                    dpi_x100: Some(150),
                    floating: None,
                },
            },
        );
        save_json_store(&root, PRIMARY_LAYOUT_FILE, &primary).unwrap();
        let loaded_primary: PrimaryLayoutStore = load_json_store(&root, PRIMARY_LAYOUT_FILE);
        assert_eq!(loaded_primary, primary);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_saved_rect_preserves_monitor_relative_position_on_origin_shift_and_dpi_change() {
        let saved = SavedRect {
            x: -3041,
            y: 1099,
            w_dip: 312,
            h_dip: 161,
            rel_x: Some(799),
            rel_y: Some(1099),
            dpi_x100: Some(150),
            floating: None,
        };
        // Origin shifted from [-3840, 0] to [0, 0] at same 150% DPI.
        let shifted = resolve_saved_rect(saved, [0, 0, 3840, 2112], 1.5, [110, 80], None);
        assert_eq!(shifted, [799, 1099, 799 + 468, 1099 + 242]);

        // DPI changed from 150% to 100%.
        let rescaled = resolve_saved_rect(saved, [0, 0, 1920, 1040], 1.0, [110, 80], None);
        assert_eq!(rescaled[2] - rescaled[0], 312);
        assert_eq!(rescaled[3] - rescaled[1], 161);
        assert_eq!(rescaled[0], (799.0_f32 / 1.5).round() as i32);
        assert_eq!(rescaled[1], (1099.0_f32 / 1.5).round() as i32);
    }

    #[test]
    fn resolve_saved_rect_recovers_legacy_coords_using_origin_when_offscreen() {
        let legacy = SavedRect {
            x: -3041,
            y: 1099,
            w_dip: 312,
            h_dip: 161,
            rel_x: None,
            rel_y: None,
            dpi_x100: None,
            floating: None,
        };
        // Legacy origin was [-3840, 0], monitor moved to [0, 0].
        let recovered =
            resolve_saved_rect(legacy, [0, 0, 3840, 2112], 1.5, [110, 80], Some([-3840, 0]));
        assert_eq!(recovered, [799, 1099, 799 + 468, 1099 + 242]);

        // Legacy coords still inside dst_work keep exact coordinates.
        let in_place = SavedRect {
            x: 2040,
            y: 120,
            w_dip: 260,
            h_dip: 140,
            rel_x: None,
            rel_y: None,
            dpi_x100: None,
            floating: None,
        };
        let kept =
            resolve_saved_rect(in_place, [1920, 0, 3840, 1040], 1.0, [110, 80], Some([0, 0]));
        assert_eq!(kept, [2040, 120, 2040 + 260, 120 + 140]);
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
