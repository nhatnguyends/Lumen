//! Physical display identity and CCD reattachment via the shared `crate::displays` source.
use super::model::Control;
use crate::displays::{
    self, links, query_ccd, remember, resolve_link, saved_entry, source_same, target_same, Saved,
};
use std::path::Path;
use windows_sys::Win32::{Devices::Display::*, Foundation::POINTL};

pub fn configure(root: &Path) {
    displays::configure(root);
}

pub unsafe fn source(id: &str) -> String {
    displays::resolve_source(id)
}

pub unsafe fn remember_before_detach(c: &Control) -> Result<String, String> {
    displays::remember_before_detach(&c.target, &c.label)
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
    let link = resolve_link(&all, &c.target, &c.label).ok_or("Display is no longer connected")?;
    if link.path.flags & 1 != 0 {
        return Ok(());
    }
    remember(&c.target, link)?;
    let saved = saved_entry(&c.target);
    let (active, modes) = query_ccd(2)?;
    let (paths, _) = query_ccd(1)?;
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
    use std::mem::zeroed;
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
