//! Windows Applications catalog and Start Menu entries. No account enumeration.
use crate::model::Item;
use std::{
    collections::HashSet,
    ffi::c_void,
    hash::{Hash, Hasher},
    ptr::{null, null_mut},
    sync::mpsc::{self, Receiver},
};
use windows_sys::{
    core::GUID,
    Win32::{System::Com::*, UI::Shell::*},
};
struct Com(*mut c_void);
pub struct Apartment(bool);
impl Apartment {
    pub fn new() -> Self {
        Self(unsafe { CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32) >= 0 })
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                CoUninitialize();
            }
        }
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() {
                let release: unsafe extern "system" fn(*mut c_void) -> u32 =
                    std::mem::transmute(slot(self.0, 2));
                release(self.0);
            }
        }
    }
}
unsafe fn slot(pointer: *mut c_void, index: usize) -> *const c_void {
    *(*(pointer as *const *const *const c_void)).add(index)
}
unsafe fn name(item: *mut c_void, flag: u32) -> Option<String> {
    let get: unsafe extern "system" fn(*mut c_void, u32, *mut *mut u16) -> i32 =
        std::mem::transmute(slot(item, 5));
    let mut text = null_mut();
    if get(item, flag, &mut text) < 0 || text.is_null() {
        return None;
    }
    let mut len = 0;
    while len < 32768 && *text.add(len) != 0 {
        len += 1;
    }
    let value = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
    CoTaskMemFree(text as _);
    (!value.is_empty()).then_some(value)
}
fn entry(name: String, target: String) -> Item {
    let absolute = std::path::Path::new(&target).is_absolute();
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    target.to_lowercase().hash(&mut hash);
    Item {
        id: format!("catalog-{:016x}", hash.finish()),
        name,
        target: target.clone(),
        kind: if absolute { "file" } else { "aumid" }.into(),
        args: None,
        cwd: None,
        icon_source: Some(if absolute {
            target
        } else {
            format!("shell:AppsFolder\\{target}")
        }),
        icon: None,
        group: None,
        catalog: true,
    }
}
unsafe fn windows_apps() -> Vec<Item> {
    let folder_id = GUID::from_u128(0x1e87508d_89c2_42f0_8a7e_645a0f50ca58);
    let iid = GUID::from_u128(0x43826d1e_e718_42ee_bc55_a1e261c37bfe);
    let handler = GUID::from_u128(0x94f60519_2850_4924_aa5a_d15e84868039);
    let enum_iid = GUID::from_u128(0x70629033_e363_4a28_a567_0db78006e6d7);
    let mut folder = Com(null_mut());
    let mut enumerator = Com(null_mut());
    if SHCreateItemInKnownFolder(&folder_id, 0, null(), &iid, &mut folder.0) < 0 {
        return vec![];
    }
    let bind: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *const GUID,
        *const GUID,
        *mut *mut c_void,
    ) -> i32 = std::mem::transmute(slot(folder.0, 3));
    if bind(folder.0, null_mut(), &handler, &enum_iid, &mut enumerator.0) < 0 {
        return vec![];
    }
    let next: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void, *mut u32) -> i32 =
        std::mem::transmute(slot(enumerator.0, 3));
    let mut items = vec![];
    for _ in 0..10000 {
        let mut item = Com(null_mut());
        let mut count = 0;
        if next(enumerator.0, 1, &mut item.0, &mut count) != 0 || count == 0 {
            break;
        }
        if let (Some(name), Some(target)) = (name(item.0, 0), name(item.0, 0x80018001)) {
            items.push(entry(name, target));
        }
    }
    items
}
fn start_menu(path: &std::path::Path, depth: usize, items: &mut Vec<Item>) {
    if depth > 12 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for file in entries.flatten() {
        let path = file.path();
        let Ok(meta) = file.file_type() else {
            continue;
        };
        if meta.is_symlink() {
            continue;
        }
        if meta.is_dir() {
            start_menu(&path, depth + 1, items);
        } else if path.extension().is_some_and(|ext| {
            matches!(
                ext.to_string_lossy().to_ascii_lowercase().as_str(),
                "lnk" | "url" | "exe" | "appref-ms"
            )
        }) {
            if let Some(name) = path.file_stem() {
                items.push(entry(
                    name.to_string_lossy().into_owned(),
                    path.to_string_lossy().into_owned(),
                ));
            }
        }
    }
}
fn app_paths(items: &mut Vec<Item>) {
    use windows_sys::Win32::System::Registry::*;
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            unsafe {
                let mut key = null_mut();
                if RegOpenKeyExW(
                    hive,
                    crate::model::wide("Software\\Microsoft\\Windows\\CurrentVersion\\App Paths")
                        .as_ptr(),
                    0,
                    KEY_READ | view,
                    &mut key,
                ) != 0
                {
                    continue;
                }
                for index in 0..10000 {
                    let mut name = [0u16; 512];
                    let mut len = name.len() as u32;
                    if RegEnumKeyExW(
                        key,
                        index,
                        name.as_mut_ptr(),
                        &mut len,
                        null(),
                        null_mut(),
                        null_mut(),
                        null_mut(),
                    ) != 0
                    {
                        break;
                    }
                    let mut value = [0u16; 32768];
                    let mut bytes = (value.len() * 2) as u32;
                    if RegGetValueW(
                        key,
                        name.as_ptr(),
                        null(),
                        RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
                        null_mut(),
                        value.as_mut_ptr() as _,
                        &mut bytes,
                    ) == 0
                    {
                        let end = value.iter().position(|c| *c == 0).unwrap_or(value.len());
                        let target = String::from_utf16_lossy(&value[..end])
                            .trim_matches('"')
                            .to_owned();
                        if std::path::Path::new(&target).is_file() {
                            let name = String::from_utf16_lossy(&name[..len as usize]);
                            items.push(entry(name.trim_end_matches(".exe").into(), target));
                        }
                    }
                }
                RegCloseKey(key);
            }
        }
    }
}
pub fn scan() -> Vec<Item> {
    let mut items = unsafe {
        let initialized = CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32) >= 0;
        let items = windows_apps();
        if initialized {
            CoUninitialize();
        }
        items
    };
    for (variable, tail) in [
        ("APPDATA", "Microsoft/Windows/Start Menu/Programs"),
        ("PROGRAMDATA", "Microsoft/Windows/Start Menu/Programs"),
    ] {
        if let Some(base) = std::env::var_os(variable) {
            start_menu(&std::path::PathBuf::from(base).join(tail), 0, &mut items);
        }
    }
    app_paths(&mut items);
    let mut seen = HashSet::new();
    items.retain(|i| seen.insert(i.name.to_lowercase()));
    items.sort_by_key(|i| i.name.to_lowercase());
    items
}
pub fn start() -> Receiver<Vec<Item>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(scan());
    });
    rx
}
pub fn fold(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .filter_map(|c| {
            if ('\u{0300}'..='\u{036f}').contains(&c) {
                return None;
            }
            for (letters, base) in [
                ("áàảãạâấầẩẫậăắằẳẵặä", 'a'),
                ("éèẻẽẹêếềểễệë", 'e'),
                ("íìỉĩịîï", 'i'),
                ("óòỏõọôốồổỗộơớờởỡợö", 'o'),
                ("úùủũụưứừửữựü", 'u'),
                ("ýỳỷỹỵ", 'y'),
                ("đ", 'd'),
            ] {
                if letters.contains(c) {
                    return Some(base);
                }
            }
            Some(c)
        })
        .collect()
}
pub fn score(name: &str, query: &str) -> i32 {
    let name = fold(name);
    let query = fold(query.trim());
    if query.is_empty() {
        return 0;
    }
    if name == query {
        return 1000;
    }
    if name.starts_with(&query) {
        return 800;
    }
    if name.split_whitespace().any(|s| s.starts_with(&query)) {
        return 600;
    }
    if name.contains(&query) {
        return 400;
    }
    if query.split_whitespace().all(|q| name.contains(q)) {
        return 200;
    }
    0
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Reads the current Windows installation catalog"]
    fn installed_catalog_smoke() {
        let apps = scan();
        assert!(!apps.is_empty());
        println!("Installed applications: {}", apps.len());
        for query in ["paint", "onenote", "calculator"] {
            println!(
                "Matches for {query}: {}",
                apps.iter().filter(|i| score(&i.name, query) > 0).count()
            );
        }
    }
    #[test]
    fn search_prefers_exact_then_prefix_then_contains() {
        assert!(score("Paint", "paint") > score("Paint 3D", "paint"));
        assert!(score("Paint 3D", "paint") > score("Microsoft Paint", "paint"));
        assert!(score("Visual Studio Code", "studio code") > 0);
        assert_eq!(score("Calculator", "paint"), 0);
    }
}
