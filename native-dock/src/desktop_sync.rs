use crate::model::{wide, Dock};
use std::path::PathBuf;
use windows_sys::Win32::{
    Foundation::*,
    Storage::FileSystem::*,
    System::{Com::CoTaskMemFree, Threading::WaitForSingleObject},
    UI::Shell::*,
};

/// Native directory notifications: no directory reads while idle, no worker thread.
pub struct Watch {
    folders: Vec<(PathBuf, HANDLE)>,
}
impl Drop for Watch {
    fn drop(&mut self) {
        for (_, h) in &self.folders {
            unsafe {
                FindCloseChangeNotification(*h);
            }
        }
    }
}
impl Watch {
    pub fn new() -> Self {
        let mut folders = Vec::new();
        for id in [&FOLDERID_Desktop, &FOLDERID_PublicDesktop] {
            unsafe {
                let mut value = std::ptr::null_mut();
                if SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &mut value) >= 0
                    && !value.is_null()
                {
                    let mut len = 0;
                    while *value.add(len) != 0 {
                        len += 1;
                    }
                    let path = PathBuf::from(String::from_utf16_lossy(std::slice::from_raw_parts(
                        value, len,
                    )));
                    CoTaskMemFree(value as _);
                    let h = FindFirstChangeNotificationW(
                        wide(&path).as_ptr(),
                        0,
                        FILE_NOTIFY_CHANGE_FILE_NAME
                            | FILE_NOTIFY_CHANGE_DIR_NAME
                            | FILE_NOTIFY_CHANGE_ATTRIBUTES
                            | FILE_NOTIFY_CHANGE_LAST_WRITE,
                    );
                    if h != INVALID_HANDLE_VALUE {
                        folders.push((path, h));
                    }
                }
            }
        }
        Self { folders }
    }
    pub fn changed(&self) -> bool {
        let mut changed = false;
        for (_, h) in &self.folders {
            unsafe {
                if WaitForSingleObject(*h, 0) == WAIT_OBJECT_0 {
                    FindNextChangeNotification(*h);
                    changed = true;
                }
            }
        }
        changed
    }
    pub fn sync(&self, dock: &mut Dock) -> bool {
        if self.folders.is_empty() {
            return false;
        }
        let mut paths = Vec::new();
        for (folder, _) in &self.folders {
            let Ok(entries) = std::fs::read_dir(folder) else {
                return false;
            };
            for entry in entries {
                let Ok(entry) = entry else {
                    return false;
                };
                let path = entry.path();
                let attrs = unsafe { GetFileAttributesW(wide(&path).as_ptr()) };
                if attrs != INVALID_FILE_ATTRIBUTES
                    && attrs & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM) == 0
                {
                    paths.push(path);
                }
            }
        }
        reconcile(dock, paths)
    }
}
fn reconcile(dock: &mut Dock, mut paths: Vec<PathBuf>) -> bool {
    paths.sort_by_key(|p| p.to_string_lossy().to_lowercase());
    let before = serde_json::to_string(&dock.items).unwrap_or_default();
    let keys: std::collections::HashSet<_> = paths
        .iter()
        .map(|p| p.to_string_lossy().to_lowercase())
        .collect();
    dock.items
        .retain(|i| !i.id.starts_with("desktop-sync:") || keys.contains(&i.target.to_lowercase()));
    for path in paths {
        let target = path.to_string_lossy().into_owned();
        let id = format!("desktop-sync:{}", target.to_lowercase());
        if dock
            .items
            .iter()
            .any(|i| !i.catalog && i.target.eq_ignore_ascii_case(&target))
        {
            continue;
        }
        // Remove a catalog duplicate so the desktop entry is visible without searching.
        dock.items
            .retain(|i| !(i.catalog && i.target.eq_ignore_ascii_case(&target)));
        if dock.add(&target) {
            if let Some(item) = dock.items.last_mut() {
                item.id = id;
            }
        }
    }
    before != serde_json::to_string(&dock.items).unwrap_or_default()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notifications_are_idle_until_directory_changes_and_rearm() {
        let dir = std::env::temp_dir().join(format!(
            "lumen-desktop-watch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let h = unsafe {
            FindFirstChangeNotificationW(
                wide(&dir).as_ptr(),
                0,
                FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME,
            )
        };
        assert_ne!(h, INVALID_HANDLE_VALUE);
        let watch = Watch {
            folders: vec![(dir.clone(), h)],
        };
        assert!(!watch.changed());
        let wait = || {
            for _ in 0..100 {
                if watch.changed() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            panic!("missing directory notification");
        };
        std::fs::write(dir.join("first.txt"), "test").unwrap();
        wait();
        let mut dock = Dock::default();
        assert!(watch.sync(&mut dock));
        assert_eq!(dock.items.len(), 1);
        std::fs::rename(dir.join("first.txt"), dir.join("second.txt")).unwrap();
        wait();
        assert!(watch.sync(&mut dock));
        assert_eq!(dock.items[0].name, "second");
        std::fs::remove_file(dir.join("second.txt")).unwrap();
        wait();
        assert!(watch.sync(&mut dock));
        assert!(dock.items.is_empty());
        drop(watch);
        std::fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn mirror_preserves_manual_items_and_removes_only_desktop_entries() {
        let mut dock = Dock::default();
        dock.add("manual.txt");
        assert!(reconcile(
            &mut dock,
            vec!["desktop.txt".into(), "manual.txt".into()]
        ));
        assert_eq!(dock.items.len(), 2);
        assert!(!reconcile(
            &mut dock,
            vec!["desktop.txt".into(), "manual.txt".into()]
        ));
        assert!(reconcile(&mut dock, vec!["renamed.txt".into()]));
        assert!(dock.items.iter().any(|i| i.target == "manual.txt"));
        assert!(!dock.items.iter().any(|i| i.target == "desktop.txt"));
        reconcile(&mut dock, vec![]);
        assert_eq!(dock.items.len(), 1);
    }
}
