//! Physical display identity and CCD reattachment. GDI display names can move
//! when a screen is detached; never restore by a stale DISPLAY slot alone.
use super::{backend::Display, model::Control};
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    mem::{size_of, zeroed},
    path::{Path, PathBuf},
    ptr::null_mut,
};
use windows_sys::Win32::{Devices::Display::*, Foundation::*, Graphics::Gdi::*};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    pub monitor_path: String,
    pub name: String,
    pub source: String,
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
}
#[derive(Default)]
struct Store {
    path: Option<PathBuf>,
    entries: BTreeMap<String, Saved>,
}
thread_local! {static STORE:RefCell<Store>=RefCell::new(Store::default());}
pub fn configure(root: &Path) {
    let path = root.join("config/display-recovery.json");
    let entries = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<BTreeMap<String, Saved>>(&b).ok())
        .filter(|m| m.len() <= 64)
        .unwrap_or_default();
    STORE.with(|s| {
        *s.borrow_mut() = Store {
            path: Some(path),
            entries,
        }
    });
}
fn text(v: &[u16]) -> String {
    String::from_utf16_lossy(&v[..v.iter().position(|x| *x == 0).unwrap_or(v.len())])
}
fn same(a: LUID, b: LUID) -> bool {
    a.HighPart == b.HighPart && a.LowPart == b.LowPart
}
fn target_same(a: &DISPLAYCONFIG_PATH_INFO, b: &DISPLAYCONFIG_PATH_INFO) -> bool {
    same(a.targetInfo.adapterId, b.targetInfo.adapterId) && a.targetInfo.id == b.targetInfo.id
}
fn source_same(a: &DISPLAYCONFIG_PATH_INFO, b: &DISPLAYCONFIG_PATH_INFO) -> bool {
    same(a.sourceInfo.adapterId, b.sourceInfo.adapterId) && a.sourceInfo.id == b.sourceInfo.id
}
unsafe fn query(
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
#[derive(Clone)]
struct Link {
    path: DISPLAYCONFIG_PATH_INFO,
    source: String,
    monitor: String,
    name: String,
    mode: Option<DISPLAYCONFIG_SOURCE_MODE>,
}
unsafe fn links() -> Result<Vec<Link>, String> {
    let (paths, modes) = query(1)?;
    let mut out: Vec<Link> = vec![];
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
        let link = Link {
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
fn persist(store: &Store) -> Result<(), String> {
    let Some(path) = &store.path else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(&store.entries).map_err(|e| e.to_string())?,
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
fn logical(l: &Link) -> String {
    STORE
        .with(|s| {
            s.borrow()
                .entries
                .iter()
                .find(|(_, v)| v.monitor_path == l.monitor)
                .map(|(k, _)| k.clone())
        })
        .unwrap_or_else(|| {
            STORE.with(|s| {
                if s.borrow().entries.contains_key(&l.source) {
                    format!("ccd:{}", l.monitor)
                } else {
                    l.source.clone()
                }
            })
        })
}
unsafe fn remember(id: &str, l: &Link) -> Result<(), String> {
    let old = STORE.with(|s| s.borrow().entries.get(id).cloned());
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
    let value = Saved {
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
    STORE.with(|s| {
        let mut s = s.borrow_mut();
        if s.entries.get(id) != Some(&value) && s.entries.len() < 64 {
            s.entries.insert(id.into(), value);
            persist(&s)?;
        }
        Ok(())
    })
}
fn resolve<'a>(all: &'a [Link], id: &str, label: &str) -> Option<&'a Link> {
    let binding = STORE.with(|s| s.borrow().entries.get(id).cloned());
    if let Some(saved) = binding {
        return all.iter().find(|l| l.monitor == saved.monitor_path);
    }
    all.iter()
        .find(|l| !l.name.is_empty() && label.contains(&l.name))
        .or_else(|| all.iter().find(|l| l.source == id))
}
/// Connected monitors, including detached ones, with stable legacy control ids.
pub unsafe fn decorate(displays: &mut Vec<Display>) {
    let Ok(all) = links() else {
        return;
    };
    if all.is_empty() {
        return;
    }
    let previous = displays.clone();
    let mut out = vec![];
    for l in &all {
        let id = logical(l);
        let _ = remember(&id, l);
        out.push(Display {
            id,
            name: l.name.clone(),
            enabled: l.path.flags & 1 != 0,
            primary: l.path.flags & 1 != 0
                && previous.iter().any(|d| d.id == l.source && d.primary),
            hdr_supported: false,
            hdr_enabled: false,
        });
    }
    *displays = out;
}
pub unsafe fn source(id: &str) -> String {
    links()
        .ok()
        .and_then(|v| resolve(&v, id, "").map(|l| l.source.clone()))
        .unwrap_or_else(|| id.into())
}
pub unsafe fn remember_before_detach(c: &Control) -> Result<String, String> {
    let all = links()?;
    let l = resolve(&all, &c.target, &c.label).ok_or("Display is no longer connected")?;
    remember(&c.target, l)?;
    Ok(l.source.clone())
}
fn prepare_restore(
    active: &[DISPLAYCONFIG_PATH_INFO],
    mut modes: Vec<DISPLAYCONFIG_MODE_INFO>,
    mut path: DISPLAYCONFIG_PATH_INFO,
    saved: Option<&Saved>,
) -> (Vec<DISPLAYCONFIG_PATH_INFO>, Vec<DISPLAYCONFIG_MODE_INFO>) {
    path.flags = 1;
    path.sourceInfo.Anonymous.modeInfoIdx = u32::MAX;
    path.targetInfo.Anonymous.modeInfoIdx = u32::MAX;
    if let Some(s) =
        saved.filter(|s| s.width > 0 && s.height > 0 && s.width <= 32768 && s.height <= 32768)
    {
        path.sourceInfo.Anonymous.modeInfoIdx = modes.len() as u32;
        modes.push(DISPLAYCONFIG_MODE_INFO {
            infoType: 1,
            id: path.sourceInfo.id,
            adapterId: path.sourceInfo.adapterId,
            Anonymous: DISPLAYCONFIG_MODE_INFO_0 {
                sourceMode: DISPLAYCONFIG_SOURCE_MODE {
                    width: s.width,
                    height: s.height,
                    pixelFormat: 4,
                    position: POINTL { x: s.x, y: s.y },
                },
            },
        });
    }
    let mut paths = active.to_vec();
    paths.push(path);
    (paths, modes)
}
pub unsafe fn reattach(c: &Control) -> Result<(), String> {
    let all = links()?;
    let link = resolve(&all, &c.target, &c.label).ok_or("Display is no longer connected")?;
    if link.path.flags & 1 != 0 {
        return Ok(());
    }
    remember(&c.target, link)?;
    let saved = STORE.with(|s| s.borrow().entries.get(&c.target).cloned());
    let (active, modes) = query(2)?;
    let (paths, _) = query(1)?;
    let mut choices: Vec<_> = paths
        .into_iter()
        .filter(|p| {
            p.targetInfo.targetAvailable != 0
                && target_same(p, &link.path)
                && !active.iter().any(|a| source_same(a, p))
        })
        .collect();
    choices.sort_by_key(|p| if source_same(p, &link.path) { 0 } else { 1 });
    let mut last = 87;
    for path in choices {
        let (mut paths, mut modes) = prepare_restore(&active, modes.clone(), path, saved.as_ref());
        let code = SetDisplayConfig(
            paths.len() as u32,
            paths.as_mut_ptr(),
            modes.len() as u32,
            modes.as_mut_ptr(),
            0x20 | 0x40 | 0x400,
        );
        if code != 0 {
            last = code;
            continue;
        }
        let code = SetDisplayConfig(
            paths.len() as u32,
            paths.as_mut_ptr(),
            modes.len() as u32,
            modes.as_mut_ptr(),
            0x20 | 0x80 | 0x400,
        );
        if code != 0 {
            last = code;
            continue;
        }
        if links()?
            .iter()
            .any(|l| l.monitor == link.monitor && l.path.flags & 1 != 0)
        {
            return Ok(());
        }
        return Err("Windows did not activate the requested monitor".into());
    }
    Err(format!("Windows could not reattach the monitor ({last})"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restoring_a_path_keeps_other_screens_and_remembers_resolution_position() {
        unsafe {
            let mut primary: DISPLAYCONFIG_PATH_INFO = zeroed();
            primary.flags = 1;
            primary.sourceInfo.id = 0;
            let mut secondary: DISPLAYCONFIG_PATH_INFO = zeroed();
            secondary.sourceInfo.id = 1;
            secondary.targetInfo.id = 9;
            let saved = Saved {
                monitor_path: "monitor".into(),
                name: "Secondary".into(),
                source: "display1".into(),
                width: 3840,
                height: 2160,
                x: -3840,
                y: 0,
            };
            let (paths, modes) = prepare_restore(&[primary], vec![], secondary, Some(&saved));
            assert_eq!(paths.len(), 2);
            assert_eq!(paths[0].sourceInfo.id, 0);
            assert_eq!(paths[0].flags, 1);
            assert_eq!(paths[1].targetInfo.Anonymous.modeInfoIdx, u32::MAX);
            let mode = modes[paths[1].sourceInfo.Anonymous.modeInfoIdx as usize]
                .Anonymous
                .sourceMode;
            assert_eq!(
                (mode.width, mode.height, mode.position.x, mode.position.y),
                (3840, 2160, -3840, 0)
            );
        }
    }
}
