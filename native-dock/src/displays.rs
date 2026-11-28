//! Single display-detection and CCD identity source shared by Control Center and multi-monitor settings.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    mem::{size_of, zeroed},
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    sync::{Mutex, OnceLock},
};
use windows_sys::Win32::{Devices::Display::*, Foundation::*, Graphics::Gdi::*};

#[derive(Clone, Debug, PartialEq)]
pub struct DisplayInfo {
    pub id: String,
    pub source: String,
    pub monitor_path: String,
    pub name: String,
    pub enabled: bool,
    pub primary: bool,
    pub bounds: Option<[i32; 4]>,
    pub work: Option<[i32; 4]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedDisplay {
    pub monitor_path: String,
    pub name: String,
    pub source: String,
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
}

pub type Saved = SavedDisplay;

#[derive(Clone)]
pub struct CcdLink {
    pub path: DISPLAYCONFIG_PATH_INFO,
    pub source: String,
    pub monitor: String,
    pub name: String,
    pub mode: Option<DISPLAYCONFIG_SOURCE_MODE>,
}

#[derive(Default)]
struct Store {
    path: Option<PathBuf>,
    entries: BTreeMap<String, SavedDisplay>,
}

static STORE: OnceLock<Mutex<Store>> = OnceLock::new();

fn store() -> &'static Mutex<Store> {
    STORE.get_or_init(|| Mutex::new(Store::default()))
}

pub fn configure(root: &Path) {
    let path = root.join("config/display-recovery.json");
    let entries = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<BTreeMap<String, SavedDisplay>>(&b).ok())
        .filter(|m| m.len() <= 64)
        .unwrap_or_default();
    if let Ok(mut s) = store().lock() {
        *s = Store {
            path: Some(path),
            entries,
        };
    }
}

fn text(v: &[u16]) -> String {
    String::from_utf16_lossy(&v[..v.iter().position(|x| *x == 0).unwrap_or(v.len())])
}

fn same_luid(a: LUID, b: LUID) -> bool {
    a.HighPart == b.HighPart && a.LowPart == b.LowPart
}

pub fn target_same(a: &DISPLAYCONFIG_PATH_INFO, b: &DISPLAYCONFIG_PATH_INFO) -> bool {
    same_luid(a.targetInfo.adapterId, b.targetInfo.adapterId)
        && a.targetInfo.id == b.targetInfo.id
}

pub fn source_same(a: &DISPLAYCONFIG_PATH_INFO, b: &DISPLAYCONFIG_PATH_INFO) -> bool {
    same_luid(a.sourceInfo.adapterId, b.sourceInfo.adapterId)
        && a.sourceInfo.id == b.sourceInfo.id
}

pub unsafe fn query_ccd(
    flags: u32,
) -> Result<(Vec<DISPLAYCONFIG_PATH_INFO>, Vec<DISPLAYCONFIG_MODE_INFO>), String> {
    for _ in 0..4 {
        let (mut pc, mut mc) = (0, 0);
        let code = GetDisplayConfigBufferSizes(flags, &mut pc, &mut mc);
        if code != 0 {
            return Err(format!("Display query failed ({code})"));
        }
        if pc > 512 || mc > 4096 {
            return Err("Unexpected display topology size".into());
        }
        let mut paths = vec![zeroed(); pc as usize];
        let mut modes = vec![zeroed(); mc as usize];
        let code = QueryDisplayConfig(
            flags,
            &mut pc,
            paths.as_mut_ptr(),
            &mut mc,
            modes.as_mut_ptr(),
            null_mut(),
        );
        if code == 122 {
            continue;
        }
        if code != 0 {
            return Err(format!("Display query failed ({code})"));
        }
        paths.truncate(pc as usize);
        modes.truncate(mc as usize);
        return Ok((paths, modes));
    }
    Err("Display topology is changing; try again".into())
}

pub unsafe fn links() -> Result<Vec<CcdLink>, String> {
    let (paths, modes) = query_ccd(1)?;
    let mut out: Vec<CcdLink> = vec![];
    for p in &paths {
        if p.targetInfo.targetAvailable == 0 {
            continue;
        }
        if p.flags & 1 == 0 && paths.iter().any(|a| a.flags & 1 != 0 && source_same(a, p)) {
            continue;
        }
        let mut target: DISPLAYCONFIG_TARGET_DEVICE_NAME = zeroed();
        target.header = DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: 2,
            size: size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
            adapterId: p.targetInfo.adapterId,
            id: p.targetInfo.id,
        };
        if DisplayConfigGetDeviceInfo(&mut target.header) != 0 {
            continue;
        }
        let mut source: DISPLAYCONFIG_SOURCE_DEVICE_NAME = zeroed();
        source.header = DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: 1,
            size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
            adapterId: p.sourceInfo.adapterId,
            id: p.sourceInfo.id,
        };
        if DisplayConfigGetDeviceInfo(&mut source.header) != 0 {
            continue;
        }
        let mode = modes
            .get(p.sourceInfo.Anonymous.modeInfoIdx as usize)
            .filter(|m| m.infoType == 1)
            .map(|m| m.Anonymous.sourceMode);
        let link = CcdLink {
            path: *p,
            source: text(&source.viewGdiDeviceName),
            monitor: text(&target.monitorDevicePath),
            name: text(&target.monitorFriendlyDeviceName),
            mode,
        };
        if let Some(i) = out.iter().position(|l| target_same(&l.path, p)) {
            if p.flags & 1 != 0 {
                out[i] = link;
            }
        } else {
            out.push(link);
        }
    }
    Ok(out)
}

fn persist(s: &Store) -> Result<(), String> {
    let Some(path) = &s.path else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(&s.entries).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    unsafe {
        use windows_sys::Win32::Storage::FileSystem::*;
        if MoveFileExW(
            crate::model::wide(&tmp).as_ptr(),
            crate::model::wide(path).as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        ) == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
    }
    Ok(())
}

fn logical_id(l: &CcdLink) -> String {
    let Ok(s) = store().lock() else {
        return l.source.clone();
    };
    if !l.monitor.is_empty() {
        if let Some((k, _)) = s.entries.iter().find(|(_, v)| v.monitor_path == l.monitor) {
            return k.clone();
        }
    }
    if !l.monitor.is_empty() && s.entries.contains_key(&l.source) {
        format!("ccd:{}", l.monitor)
    } else {
        l.source.clone()
    }
}

pub unsafe fn remember(id: &str, l: &CcdLink) -> Result<(), String> {
    let old = store().lock().ok().and_then(|s| s.entries.get(id).cloned());
    let mode = l.mode.or_else(|| {
        let mut m: DEVMODEW = zeroed();
        m.dmSize = size_of::<DEVMODEW>() as u16;
        if EnumDisplaySettingsW(
            crate::model::wide(&l.source).as_ptr(),
            ENUM_REGISTRY_SETTINGS,
            &mut m,
        ) != 0
            && m.dmPelsWidth > 0
            && m.dmPelsHeight > 0
        {
            Some(DISPLAYCONFIG_SOURCE_MODE {
                width: m.dmPelsWidth,
                height: m.dmPelsHeight,
                pixelFormat: 4,
                position: m.Anonymous1.Anonymous2.dmPosition,
            })
        } else {
            None
        }
    });
    let value = SavedDisplay {
        monitor_path: l.monitor.clone(),
        name: l.name.clone(),
        source: l.source.clone(),
        width: mode
            .map(|m| m.width)
            .unwrap_or_else(|| old.as_ref().map_or(0, |v| v.width)),
        height: mode
            .map(|m| m.height)
            .unwrap_or_else(|| old.as_ref().map_or(0, |v| v.height)),
        x: mode
            .map(|m| m.position.x)
            .unwrap_or_else(|| old.as_ref().map_or(0, |v| v.x)),
        y: mode
            .map(|m| m.position.y)
            .unwrap_or_else(|| old.as_ref().map_or(0, |v| v.y)),
    };
    if let Ok(mut s) = store().lock() {
        if s.entries.get(id) != Some(&value) && s.entries.len() < 64 {
            s.entries.insert(id.into(), value);
            persist(&s)?;
        }
    }
    Ok(())
}

pub fn resolve_link<'a>(all: &'a [CcdLink], id: &str, label: &str) -> Option<&'a CcdLink> {
    let binding = store().lock().ok().and_then(|s| s.entries.get(id).cloned());
    if let Some(saved) = binding {
        return all.iter().find(|l| l.monitor == saved.monitor_path);
    }
    all.iter()
        .find(|l| !l.name.is_empty() && label.contains(&l.name))
        .or_else(|| all.iter().find(|l| l.source == id))
}

pub fn saved_entry(id: &str) -> Option<SavedDisplay> {
    store().lock().ok().and_then(|s| s.entries.get(id).cloned())
}

pub unsafe fn resolve_source(id: &str) -> String {
    links()
        .ok()
        .and_then(|v| resolve_link(&v, id, "").map(|l| l.source.clone()))
        .unwrap_or_else(|| id.into())
}

pub unsafe fn remember_before_detach(target: &str, label: &str) -> Result<String, String> {
    let all = links()?;
    let l = resolve_link(&all, target, label).ok_or("Display is no longer connected")?;
    remember(target, l)?;
    Ok(l.source.clone())
}

unsafe fn gdi_mode_valid(name: &str) -> bool {
    let mut m: DEVMODEW = zeroed();
    m.dmSize = size_of::<DEVMODEW>() as u16;
    EnumDisplaySettingsW(
        crate::model::wide(name).as_ptr(),
        ENUM_REGISTRY_SETTINGS,
        &mut m,
    ) != 0
        && m.dmPelsWidth > 0
        && m.dmPelsHeight > 0
}

unsafe extern "system" fn collect_monitor_rect(
    h: HMONITOR,
    _: HDC,
    _: *mut RECT,
    param: LPARAM,
) -> BOOL {
    let map = &mut *(param as *mut HashMap<String, ([i32; 4], [i32; 4])>);
    let mut info: MONITORINFOEXW = zeroed();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(h, &mut info as *mut _ as _) != 0 {
        let name = String::from_utf16_lossy(
            &info.szDevice[..info.szDevice.iter().position(|v| *v == 0).unwrap_or(32)],
        );
        let m = info.monitorInfo.rcMonitor;
        let w = info.monitorInfo.rcWork;
        map.insert(
            name,
            (
                [m.left, m.top, m.right, m.bottom],
                [w.left, w.top, w.right, w.bottom],
            ),
        );
    }
    1
}

unsafe fn active_monitor_rects() -> HashMap<String, ([i32; 4], [i32; 4])> {
    let mut map = HashMap::new();
    EnumDisplayMonitors(
        null_mut(),
        null(),
        Some(collect_monitor_rect),
        &mut map as *mut _ as isize,
    );
    map
}

/// Enumerates connected displays from the single GDI + CCD source shared across Lumen.
pub unsafe fn enumerate() -> Vec<DisplayInfo> {
    let rects = active_monitor_rects();
    let mut gdi = vec![];
    for i in 0..64 {
        let mut d: DISPLAY_DEVICEW = zeroed();
        d.cb = size_of::<DISPLAY_DEVICEW>() as u32;
        if EnumDisplayDevicesW(null(), i, &mut d, 0) == 0 {
            break;
        }
        if d.StateFlags & 8 != 0 {
            continue;
        }
        let id = text(&d.DeviceName);
        let mut monitor: DISPLAY_DEVICEW = zeroed();
        monitor.cb = size_of::<DISPLAY_DEVICEW>() as u32;
        let has = EnumDisplayDevicesW(d.DeviceName.as_ptr(), 0, &mut monitor, 0) != 0;
        let enabled = d.StateFlags & 1 != 0;
        if !has && !enabled && !gdi_mode_valid(&id) {
            continue;
        }
        let geom = enabled.then(|| rects.get(&id).copied()).flatten();
        gdi.push(DisplayInfo {
            id: id.clone(),
            source: id,
            monitor_path: String::new(),
            name: if has {
                text(&monitor.DeviceString)
            } else {
                text(&d.DeviceString)
            },
            enabled,
            primary: d.StateFlags & 4 != 0,
            bounds: geom.map(|g| g.0),
            work: geom.map(|g| g.1),
        });
    }
    if let Ok(all) = links() {
        if !all.is_empty() {
            let mut out = Vec::with_capacity(all.len());
            for l in &all {
                let id = logical_id(l);
                let _ = remember(&id, l);
                let enabled = l.path.flags & 1 != 0;
                let primary = enabled && gdi.iter().any(|g| g.source == l.source && g.primary);
                let geom = enabled.then(|| rects.get(&l.source).copied()).flatten();
                out.push(DisplayInfo {
                    id,
                    source: l.source.clone(),
                    monitor_path: l.monitor.clone(),
                    name: l.name.clone(),
                    enabled,
                    primary,
                    bounds: geom.map(|g| g.0),
                    work: geom.map(|g| g.1),
                });
            }
            return out;
        }
    }
    gdi
}

pub fn label(d: &DisplayInfo) -> String {
    let base = if d.name.trim().is_empty() {
        d.id.trim_start_matches(r"\\.\")
    } else {
        d.name.trim()
    };
    if let Some([l, t, r, b]) = d.bounds {
        format!("{base} ({}×{})", r - l, b - t)
    } else {
        base.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_includes_resolution_when_active() {
        let active = DisplayInfo {
            id: r"\\.\DISPLAY2".into(),
            source: r"\\.\DISPLAY2".into(),
            monitor_path: "mon-2".into(),
            name: "Dell U2723QE".into(),
            enabled: true,
            primary: false,
            bounds: Some([1920, 0, 5760, 2160]),
            work: Some([1920, 0, 5760, 2112]),
        };
        assert_eq!(label(&active), "Dell U2723QE (3840×2160)");

        let detached = DisplayInfo {
            id: r"\\.\DISPLAY3".into(),
            source: r"\\.\DISPLAY3".into(),
            monitor_path: "mon-3".into(),
            name: String::new(),
            enabled: false,
            primary: false,
            bounds: None,
            work: None,
        };
        assert_eq!(label(&detached), "DISPLAY3");
    }

    #[test]
    fn enumerate_discovers_connected_monitors() {
        unsafe {
            let list = enumerate();
            assert!(!list.is_empty(), "system must detect at least one display");
            assert!(
                list.iter().any(|d| d.enabled && d.bounds.is_some()),
                "at least one display must be active with bounds"
            );
        }
    }
}
