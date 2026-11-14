//! Windows control APIs run on a private COM thread; no subprocess/browser/audio stream.
use super::model::{Control, Kind};
use std::{
    ffi::c_void,
    mem::{size_of, transmute_copy, zeroed},
    ptr::{null, null_mut},
    sync::mpsc,
};
use windows_sys::{
    core::GUID,
    Win32::{Devices::Display::*, Foundation::*, Graphics::Gdi::*, System::Com::*},
};
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub selected: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Display {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub primary: bool,
    pub hdr_supported: bool,
    pub hdr_enabled: bool,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub sources: std::collections::BTreeMap<String, Vec<Device>>,
    pub displays: Vec<Display>,
    pub outputs: Vec<Device>,
    pub inputs: Vec<Device>,
    pub volume: Option<f32>,
    pub muted: bool,
    pub mic_volume: Option<f32>,
    pub mic_muted: bool,
    pub errors: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct Command {
    pub control: Control,
    pub value: f32,
}
pub struct Worker {
    tx: mpsc::Sender<Option<Command>>,
    pub rx: mpsc::Receiver<Snapshot>,
}
impl Worker {
    pub fn new(root: &std::path::Path) -> Result<Self, String> {
        let root = root.to_path_buf();
        let (tx, requests) = mpsc::channel::<Option<Command>>();
        let (results, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("lumen-controls".into())
            .spawn(move || unsafe {
                super::display_restore::configure(&root);
                super::input_source::configure(&root);
                let initialized = CoInitializeEx(null(), COINIT_MULTITHREADED as u32) >= 0;
                while let Ok(command) = requests.recv() {
                    let error = command.as_ref().and_then(|c| execute(c).err());
                    let mut state = snapshot();
                    if let Some(error) = error {
                        state.errors.insert(0, error);
                    }
                    if results.send(state).is_err() {
                        break;
                    }
                }
                if initialized {
                    CoUninitialize();
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self { tx, rx })
    }
    pub fn read(&self) {
        let _ = self.tx.send(None);
    }
    pub fn command(&self, command: Command) {
        let _ = self.tx.send(Some(command));
    }
}
fn hr(value: i32) -> Result<(), String> {
    if value < 0 {
        Err(format!("Windows audio API failed (0x{:08X})", value as u32))
    } else {
        Ok(())
    }
}
struct Com(*mut c_void);
impl Drop for Com {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() {
                let f: unsafe extern "system" fn(*mut c_void) -> u32 = method(self.0, 2);
                f(self.0);
            }
        }
    }
}
unsafe fn method<T: Copy>(object: *mut c_void, index: usize) -> T {
    let vtable = *(object as *const *const *const c_void);
    transmute_copy(&*vtable.add(index))
}
unsafe fn create(clsid: u128, iid: u128) -> Result<Com, String> {
    let mut p = null_mut();
    hr(CoCreateInstance(
        &GUID::from_u128(clsid),
        null_mut(),
        CLSCTX_ALL,
        &GUID::from_u128(iid),
        &mut p,
    ))?;
    if p.is_null() {
        Err("Windows returned an empty audio interface".into())
    } else {
        Ok(Com(p))
    }
}
fn string(value: &[u16]) -> String {
    String::from_utf16_lossy(&value[..value.iter().position(|v| *v == 0).unwrap_or(value.len())])
}
unsafe fn allocated_string(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut n = 0;
    while n < 32768 && *ptr.add(n) != 0 {
        n += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(ptr, n))
}
unsafe fn device_id(device: &Com) -> Result<String, String> {
    let mut p = null_mut();
    let f: unsafe extern "system" fn(*mut c_void, *mut *mut u16) -> i32 = method(device.0, 5);
    hr(f(device.0, &mut p))?;
    let s = allocated_string(p);
    CoTaskMemFree(p as _);
    Ok(s)
}
#[repr(C)]
struct PropertyKey {
    fmtid: GUID,
    pid: u32,
}
#[link(name = "ole32")]
extern "system" {
    fn PropVariantClear(value: *mut c_void) -> i32;
}
unsafe fn audio(
    command: Option<&Command>,
    flow: i32,
) -> Result<(Vec<Device>, Option<f32>, bool), String> {
    let enumerator = create(
        0xbcde0395_e52f_467c_8e3d_c4579291692e,
        0xa95664d2_9614_4f35_a746_de8db63617e6,
    )?;
    let mut default = Com(null_mut());
    let get_default: unsafe extern "system" fn(*mut c_void, i32, i32, *mut *mut c_void) -> i32 =
        method(enumerator.0, 4);
    let _ = get_default(enumerator.0, flow, 1, &mut default.0);
    let selected = if default.0.is_null() {
        String::new()
    } else {
        device_id(&default)?
    };
    let mut collection = Com(null_mut());
    let enumerate: unsafe extern "system" fn(*mut c_void, i32, u32, *mut *mut c_void) -> i32 =
        method(enumerator.0, 3);
    hr(enumerate(enumerator.0, flow, 1, &mut collection.0))?;
    let mut count = 0;
    let get_count: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32 =
        method(collection.0, 3);
    hr(get_count(collection.0, &mut count))?;
    let mut devices = vec![];
    for i in 0..count.min(256) {
        let mut device = Com(null_mut());
        let item: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32 =
            method(collection.0, 4);
        hr(item(collection.0, i, &mut device.0))?;
        let id = device_id(&device)?;
        let mut store = Com(null_mut());
        let open: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32 =
            method(device.0, 4);
        hr(open(device.0, 0, &mut store.0))?;
        let key = PropertyKey {
            fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
            pid: 14,
        };
        let mut variant = [0usize; 3];
        let get: unsafe extern "system" fn(*mut c_void, *const PropertyKey, *mut c_void) -> i32 =
            method(store.0, 5);
        let ok = get(store.0, &key, variant.as_mut_ptr() as _);
        let name = if ok >= 0 && variant[0] & 0xffff == 31 {
            allocated_string(variant[1] as *const u16)
        } else {
            id.clone()
        };
        let _ = PropVariantClear(variant.as_mut_ptr() as _);
        devices.push(Device {
            selected: id == selected,
            id,
            name,
        });
    }
    if let Some(c) = command.filter(|c| matches!(c.control.kind, Kind::Audio | Kind::Microphone)) {
        if !devices.iter().any(|d| d.id == c.control.target) {
            return Err("Audio device is no longer connected".into());
        }
        // Optional PolicyConfig interface; failure is surfaced, never emulated with input.
        let policy = create(
            0x870af99c_171d_4f9e_af0d_e63df40c2bc9,
            0xf8679f50_850a_41cf_9c72_430f290290c8,
        )?;
        let set: unsafe extern "system" fn(*mut c_void, *const u16, i32) -> i32 =
            method(policy.0, 13);
        let target = crate::model::wide(&c.control.target);
        for role in 0..3 {
            hr(set(policy.0, target.as_ptr(), role))?;
        }
        return Ok((devices, None, false));
    }
    if default.0.is_null() {
        if command.is_some() {
            return Err("No default audio device is available".into());
        }
        return Ok((devices, None, false));
    }
    let mut volume = Com(null_mut());
    let activate: unsafe extern "system" fn(
        *mut c_void,
        *const GUID,
        u32,
        *mut c_void,
        *mut *mut c_void,
    ) -> i32 = method(default.0, 3);
    hr(activate(
        default.0,
        &GUID::from_u128(0x5cdf2c82_841e_4546_9722_0cf74078229a),
        CLSCTX_ALL,
        null_mut(),
        &mut volume.0,
    ))?;
    if let Some(c) = command {
        if c.control.kind.is_volume() {
            let set: unsafe extern "system" fn(*mut c_void, f32, *const GUID) -> i32 =
                method(volume.0, 7);
            hr(set(volume.0, c.value / 100., null()))?;
        } else {
            let set: unsafe extern "system" fn(*mut c_void, i32, *const GUID) -> i32 =
                method(volume.0, 14);
            hr(set(volume.0, if c.value > 0. { 1 } else { 0 }, null()))?;
        }
    }
    let mut level = 0.;
    let get: unsafe extern "system" fn(*mut c_void, *mut f32) -> i32 = method(volume.0, 9);
    hr(get(volume.0, &mut level))?;
    let mut muted = 0;
    let get: unsafe extern "system" fn(*mut c_void, *mut i32) -> i32 = method(volume.0, 15);
    hr(get(volume.0, &mut muted))?;
    Ok((devices, Some((level * 100.).round()), muted != 0))
}
#[repr(C)]
struct Advanced {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    flags: u32,
    encoding: i32,
    bits: u32,
    mode: i32,
}
#[repr(C)]
struct SetColor {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    enabled: u32,
}
#[derive(Clone)]
struct HdrTarget {
    source: String,
    adapter: LUID,
    id: u32,
    supported: bool,
    enabled: bool,
    modern: bool,
    name: String,
}
fn header(kind: i32, size: usize, adapter: LUID, id: u32) -> DISPLAYCONFIG_DEVICE_INFO_HEADER {
    DISPLAYCONFIG_DEVICE_INFO_HEADER {
        r#type: kind,
        size: size as u32,
        adapterId: adapter,
        id,
    }
}
unsafe fn hdr_targets() -> Result<Vec<HdrTarget>, String> {
    for _ in 0..4 {
        let (mut pc, mut mc) = (0, 0);
        let code = GetDisplayConfigBufferSizes(2, &mut pc, &mut mc);
        if code != 0 {
            return Err(format!("Display query failed ({code})"));
        }
        if pc > 256 || mc > 2048 {
            return Err("Unexpected display topology size".into());
        }
        let mut paths = vec![zeroed::<DISPLAYCONFIG_PATH_INFO>(); pc as usize];
        let mut modes = vec![zeroed::<DISPLAYCONFIG_MODE_INFO>(); mc as usize];
        let code = QueryDisplayConfig(
            2,
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
        let mut result = vec![];
        for p in paths.iter().take(pc as usize) {
            let mut source: DISPLAYCONFIG_SOURCE_DEVICE_NAME = zeroed();
            source.header = header(
                1,
                size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>(),
                p.sourceInfo.adapterId,
                p.sourceInfo.id,
            );
            if DisplayConfigGetDeviceInfo(&mut source.header) != 0 {
                continue;
            }
            let mut info = Advanced {
                header: header(
                    15,
                    size_of::<Advanced>(),
                    p.targetInfo.adapterId,
                    p.targetInfo.id,
                ),
                flags: 0,
                encoding: 0,
                bits: 0,
                mode: 0,
            };
            let modern = DisplayConfigGetDeviceInfo(&mut info.header) == 0;
            if !modern {
                info.header = header(9, 32, p.targetInfo.adapterId, p.targetInfo.id);
                if DisplayConfigGetDeviceInfo(&mut info.header) != 0 {
                    continue;
                }
            }
            let mut name: DISPLAYCONFIG_TARGET_DEVICE_NAME = zeroed();
            name.header = header(
                2,
                size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>(),
                p.targetInfo.adapterId,
                p.targetInfo.id,
            );
            let _ = DisplayConfigGetDeviceInfo(&mut name.header);
            result.push(HdrTarget {
                source: string(&source.viewGdiDeviceName),
                adapter: p.targetInfo.adapterId,
                id: p.targetInfo.id,
                supported: if modern {
                    info.flags & 16 != 0
                } else {
                    info.flags & 1 != 0 && info.flags & 12 == 0
                },
                enabled: info.flags & if modern { 32 } else { 2 } != 0,
                modern,
                name: string(&name.monitorFriendlyDeviceName),
            });
        }
        return Ok(result);
    }
    Err("Display layout is changing; try again".into())
}
unsafe fn mode(name: &str, saved: bool) -> Result<DEVMODEW, String> {
    let mut m: DEVMODEW = zeroed();
    m.dmSize = size_of::<DEVMODEW>() as u16;
    if EnumDisplaySettingsW(
        crate::model::wide(name).as_ptr(),
        if saved {
            ENUM_REGISTRY_SETTINGS
        } else {
            ENUM_CURRENT_SETTINGS
        },
        &mut m,
    ) == 0
    {
        Err("Windows could not read this display mode".into())
    } else {
        Ok(m)
    }
}
unsafe fn displays() -> Vec<Display> {
    let mut result = vec![];
    for i in 0..64 {
        let mut d: DISPLAY_DEVICEW = zeroed();
        d.cb = size_of::<DISPLAY_DEVICEW>() as u32;
        if EnumDisplayDevicesW(null(), i, &mut d, 0) == 0 {
            break;
        }
        if d.StateFlags & 8 != 0 {
            continue;
        }
        let id = string(&d.DeviceName);
        let mut monitor: DISPLAY_DEVICEW = zeroed();
        monitor.cb = size_of::<DISPLAY_DEVICEW>() as u32;
        let has = EnumDisplayDevicesW(d.DeviceName.as_ptr(), 0, &mut monitor, 0) != 0;
        if !has
            && d.StateFlags & 1 == 0
            && !mode(&id, true).is_ok_and(|m| m.dmPelsWidth > 0 && m.dmPelsHeight > 0)
        {
            continue;
        }
        result.push(Display {
            id,
            name: if has {
                string(&monitor.DeviceString)
            } else {
                string(&d.DeviceString)
            },
            enabled: d.StateFlags & 1 != 0,
            primary: d.StateFlags & 4 != 0,
            hdr_supported: false,
            hdr_enabled: false,
        });
    }
    super::display_restore::decorate(&mut result);
    result
}
pub fn can_detach(displays: &[Display], id: &str) -> bool {
    displays.iter().any(|d| d.id == id && !d.primary)
        && displays.iter().filter(|d| d.enabled).count() > 1
}
pub unsafe fn execute(command: &Command) -> Result<(), String> {
    if !command.value.is_finite()
        || command.value < 0.
        || command.value
            > if command.control.kind == Kind::Source {
                255.
            } else {
                100.
            }
        || command.control.target.len() > 1024
    {
        return Err("Invalid control command".into());
    }
    if command.control.kind == Kind::Source {
        if command.value.fract() != 0. {
            return Err("Invalid input code".into());
        }
        return super::input_source::set(&command.control.target, command.value as u32);
    }
    if !command.control.kind.is_volume() && command.value != 0. && command.value != 1. {
        return Err("Invalid toggle command".into());
    }
    match command.control.kind {
        Kind::Display => {
            let list = displays();
            let d = list
                .iter()
                .find(|d| d.id == command.control.target)
                .ok_or("Display is no longer connected")?;
            let enabled = command.value > 0.;
            if enabled == d.enabled {
                return Ok(());
            }
            if !enabled && !can_detach(&list, &d.id) {
                return Err("Keep the primary display enabled".into());
            }
            if enabled {
                return super::display_restore::reattach(&command.control);
            }
            let actual_source = super::display_restore::remember_before_detach(&command.control)?;
            let mut m = mode(&actual_source, false)?;
            if !enabled {
                m.dmFields = DM_POSITION | DM_PELSWIDTH | DM_PELSHEIGHT;
                m.dmPelsWidth = 0;
                m.dmPelsHeight = 0;
            }
            let code = ChangeDisplaySettingsExW(
                crate::model::wide(&actual_source).as_ptr(),
                &m,
                null_mut(),
                0,
                null(),
            );
            if code != 0 {
                return Err(format!("Windows rejected display change ({code})"));
            }
            Ok(())
        }
        Kind::Hdr => {
            let source = super::display_restore::source(&command.control.target);
            let targets = hdr_targets()?;
            let targets: Vec<_> = targets.iter().filter(|t| t.source == source).collect();
            if targets.is_empty() || targets.iter().any(|t| !t.supported) {
                return Err("HDR is unavailable for this display".into());
            }
            for t in targets {
                let p = SetColor {
                    header: header(
                        if t.modern { 16 } else { 10 },
                        size_of::<SetColor>(),
                        t.adapter,
                        t.id,
                    ),
                    enabled: if command.value > 0. { 1 } else { 0 },
                };
                let code = DisplayConfigSetDeviceInfo(&p.header);
                if code != 0 {
                    return Err(format!("Windows rejected HDR change ({code})"));
                }
            }
            Ok(())
        }
        _ => {
            audio(
                Some(command),
                if command.control.kind.is_input() {
                    1
                } else {
                    0
                },
            )?;
            Ok(())
        }
    }
}
pub unsafe fn snapshot() -> Snapshot {
    let mut s = Snapshot::default();
    s.displays = displays();
    s.sources = super::input_source::snapshot(&s.displays);
    match hdr_targets() {
        Ok(targets) => {
            for d in &mut s.displays {
                let source = super::display_restore::source(&d.id);
                let ts: Vec<_> = targets.iter().filter(|t| t.source == source).collect();
                d.hdr_supported = !ts.is_empty() && ts.iter().all(|t| t.supported);
                d.hdr_enabled = !ts.is_empty() && ts.iter().all(|t| t.enabled);
                if let Some(t) = ts.first().filter(|t| !t.name.is_empty()) {
                    d.name = t.name.clone();
                }
            }
        }
        Err(e) => s.errors.push(e),
    }
    match audio(None, 0) {
        Ok((d, v, m)) => {
            s.outputs = d;
            s.volume = v;
            s.muted = m;
        }
        Err(e) => s.errors.push(e),
    }
    match audio(None, 1) {
        Ok((d, v, m)) => {
            s.inputs = d;
            s.mic_volume = v;
            s.mic_muted = m;
        }
        Err(e) => s.errors.push(e),
    }
    s
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ffi_layouts_match_windows_headers() {
        assert_eq!(size_of::<Advanced>(), 36);
        assert_eq!(size_of::<SetColor>(), 24);
        assert_eq!(size_of::<DISPLAYCONFIG_PATH_INFO>(), 72);
        assert_eq!(size_of::<DEVMODEW>(), 220);
        assert_eq!(size_of::<PropertyKey>(), 20);
    }
    #[test]
    fn display_guard_protects_primary_and_last_active_screen() {
        let d = |id: &str, primary| Display {
            id: id.into(),
            name: id.into(),
            enabled: true,
            primary,
            hdr_supported: false,
            hdr_enabled: false,
        };
        assert!(!can_detach(&[d("one", false)], "one"));
        assert!(!can_detach(&[d("one", true), d("two", false)], "one"));
        assert!(can_detach(&[d("one", true), d("two", false)], "two"));
    }
    #[test]
    fn malformed_commands_never_reach_windows() {
        let c = Control {
            id: "x".into(),
            kind: Kind::Volume,
            target: String::new(),
            label: "Volume".into(),
        };
        unsafe {
            assert!(execute(&Command {
                control: c.clone(),
                value: f32::NAN
            })
            .is_err());
            assert!(execute(&Command {
                control: c,
                value: 101.
            })
            .is_err());
        }
    }
    #[test]
    #[ignore = "Read-only diagnostic of connected Windows devices"]
    fn native_snapshot_reads_without_opening_audio_streams() {
        unsafe {
            let hr = CoInitializeEx(null(), 0);
            let s = snapshot();
            eprintln!(
                "native-controls-snapshot displays={} outputs={} inputs={} errors={}",
                s.displays.len(),
                s.outputs.len(),
                s.inputs.len(),
                s.errors.len()
            );
            assert!(!s.displays.is_empty());
            assert!(s.volume.is_none_or(|v| (0.0..=100.0).contains(&v)));
            if hr >= 0 {
                CoUninitialize();
            }
        }
    }
}
