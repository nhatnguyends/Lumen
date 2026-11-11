//! Native clock model. Separate persistence; the Electron state is import-only.
use crate::model::wide;
use serde::{Deserialize, Serialize};
use std::{fs, mem::zeroed, path::Path};
use windows_sys::Win32::{
    Foundation::SYSTEMTIME,
    System::{
        SystemInformation::{GetLocalTime, GetSystemTime},
        Time::*,
    },
};

pub const MIN_SIZE: i32 = 120;
pub const MAX_SIZE: i32 = 1100;

pub const THEMES: [(&str, &str); 9] = [
    ("lumen", "Lumen"),
    ("sunset", "Afterglow"),
    ("paper", "Paper"),
    ("neon", "Nightfall"),
    ("modular", "Colour block"),
    ("ocean", "Split flap"),
    ("rose", "Daybook"),
    ("aurora", "Timeline"),
    ("terminal", "Terminal"),
];
pub const ZONES: [(&str, &str); 10] = [
    ("local", ""),
    ("UTC", "UTC"),
    ("Asia/Ho_Chi_Minh", "SE Asia Standard Time"),
    ("Asia/Tokyo", "Tokyo Standard Time"),
    ("Asia/Singapore", "Singapore Standard Time"),
    ("Europe/London", "GMT Standard Time"),
    ("Europe/Paris", "Romance Standard Time"),
    ("America/New_York", "Eastern Standard Time"),
    ("America/Los_Angeles", "Pacific Standard Time"),
    ("Australia/Sydney", "AUS Eastern Standard Time"),
];
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub version: u32,
    pub theme: String,
    pub time_zone: String,
    pub hour12: bool,
    pub show_seconds: bool,
    pub frosted_glass: bool,
    pub open: bool,
    pub floating: bool,
    pub position: Option<[i32; 2]>,
    pub size: [i32; 2],
    pub opacity: f32,
    pub tint: f32,
    pub frost_strength: f32,
    pub blur: f32,
    pub palette: String,
    pub dynamic_animation: bool,
    pub animation_mode: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            theme: "lumen".into(),
            time_zone: "local".into(),
            hour12: false,
            show_seconds: true,
            frosted_glass: true,
            open: false,
            floating: false,
            position: None,
            size: [380, 380],
            opacity: 100.,
            tint: 18.,
            frost_strength: 100.,
            blur: 13.,
            palette: "theme".into(),
            dynamic_animation: true,
            animation_mode: "clock".into(),
        }
    }
}
impl Config {
    pub fn load(root: &Path) -> Result<Self, String> {
        let own = root.join("config/native-clock.json");
        let mut result: Self = if own.exists() {
            serde_json::from_slice(&fs::read(&own).map_err(|e| e.to_string())?).map_err(|e| {
                format!("Cannot read native clock config: {e}. File left untouched.")
            })?
        } else {
            let source = root.join("config/state.json");
            let mut c = Self::default();
            if source.exists() {
                let v: serde_json::Value =
                    serde_json::from_slice(&fs::read(source).map_err(|e| e.to_string())?)
                        .map_err(|e| format!("Cannot import clock: {e}"))?;
                if let Some(old) = v.pointer("/settings/clock") {
                    // Bounds in Electron are objects; native bounds use arrays.
                    let mut old = old.clone();
                    if let Some(o) = old.as_object_mut() {
                        o.remove("position");
                        o.remove("size");
                    }
                    c = serde_json::from_value(old)
                        .map_err(|e| format!("Cannot import clock: {e}"))?;
                    let old = v.pointer("/settings/clock").unwrap();
                    if let (Some(x), Some(y)) = (
                        old.pointer("/position/x").and_then(|v| v.as_i64()),
                        old.pointer("/position/y").and_then(|v| v.as_i64()),
                    ) {
                        c.position = Some([x as i32, y as i32]);
                    }
                    if let (Some(w), Some(h)) = (
                        old.pointer("/size/width").and_then(|v| v.as_i64()),
                        old.pointer("/size/height").and_then(|v| v.as_i64()),
                    ) {
                        c.size = [w as i32, h as i32];
                    }
                }
                c.opacity = v
                    .pointer("/settings/widgetOpacity/clock")
                    .or_else(|| v.pointer("/settings/backgroundOpacity"))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(100.) as f32;
            }
            c
        };
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&mut self) -> Result<(), String> {
        if self.version != 1 {
            return Err("Unsupported native clock config version; file left untouched.".into());
        }
        if !THEMES.iter().any(|t| t.0 == self.theme) {
            return Err(format!(
                "Unknown clock theme: {}. Config left untouched.",
                self.theme
            ));
        }
        zone(&self.time_zone)?;
        if !["clock", "loop"].contains(&self.animation_mode.as_str()) {
            return Err("Unknown Clock animation mode. Config left untouched.".into());
        }
        self.size = self.size.map(|v| v.clamp(MIN_SIZE, MAX_SIZE));
        self.opacity = self.opacity.clamp(0., 100.);
        self.tint = self.tint.clamp(0., 100.);
        self.frost_strength = self.frost_strength.clamp(0., 100.);
        self.blur = self.blur.clamp(0., 80.);
        if self.palette != "theme" {
            self.palette = crate::surface::normalize(&self.palette)
                .ok_or("Unknown clock surface palette. Config left untouched.")?;
            if !crate::surface::valid(&self.palette) {
                return Err("Unknown clock surface palette. Config left untouched.".into());
            }
        }
        Ok(())
    }
    pub fn save(&self, root: &Path) -> Result<(), String> {
        let folder = root.join("config");
        fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        let dest = folder.join("native-clock.json");
        let tmp = folder.join("native-clock.json.tmp");
        fs::write(
            &tmp,
            serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        unsafe {
            use windows_sys::Win32::Storage::FileSystem::*;
            if MoveFileExW(
                wide(&tmp).as_ptr(),
                wide(&dest).as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            ) == 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        Ok(())
    }
}
// Windows owns the timezone rules, including DST and historical transitions.
// ICU maps arbitrary IANA identifiers from imported Electron settings.
fn windows_zone(id: &str) -> Result<String, String> {
    if let Some((_, key)) = ZONES.iter().find(|z| z.0 == id) {
        return Ok((*key).into());
    }
    unsafe {
        use windows_sys::Win32::System::LibraryLoader::*;
        let lib = LoadLibraryExW(
            wide("icu.dll").as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        );
        if !lib.is_null() {
            type Map = unsafe extern "C" fn(*const u16, i32, *mut u16, i32, *mut i32) -> i32;
            let proc = GetProcAddress(lib, c"ucal_getWindowsTimeZoneID".as_ptr() as _);
            let answer = if let Some(proc) = proc {
                let map: Map = std::mem::transmute(proc);
                let input = wide(id);
                let mut out = [0u16; 128];
                let mut status = 0;
                let len = map(
                    input.as_ptr(),
                    input.len() as i32 - 1,
                    out.as_mut_ptr(),
                    128,
                    &mut status,
                );
                if status <= 0 && len > 0 && len < 128 {
                    Some(String::from_utf16_lossy(&out[..len as usize]))
                } else {
                    None
                }
            } else {
                None
            };
            windows_sys::Win32::Foundation::FreeLibrary(lib);
            if let Some(key) = answer {
                return Ok(key);
            }
        }
    }
    Err(format!(
        "Time zone {id} is unavailable on this Windows installation. Config left untouched."
    ))
}
pub fn zone(id: &str) -> Result<Option<DYNAMIC_TIME_ZONE_INFORMATION>, String> {
    if id == "local" {
        return Ok(None);
    }
    let key = windows_zone(id)?;
    unsafe {
        let mut i = 0;
        loop {
            let mut z: DYNAMIC_TIME_ZONE_INFORMATION = zeroed();
            let status = EnumDynamicTimeZoneInformation(i, &mut z);
            if status != 0 {
                break;
            }
            let len = z
                .TimeZoneKeyName
                .iter()
                .position(|v| *v == 0)
                .unwrap_or(128);
            if String::from_utf16_lossy(&z.TimeZoneKeyName[..len]) == key {
                return Ok(Some(z));
            }
            i += 1;
        }
    }
    Err(format!(
        "Windows time zone {key} is unavailable. Config left untouched."
    ))
}
/// Display the selected clock-zone city, never a device geolocation lookup.
pub fn location_label(id: &str) -> String {
    let selected = if id == "local" {
        unsafe {
            let mut info: DYNAMIC_TIME_ZONE_INFORMATION = zeroed();
            let status = GetDynamicTimeZoneInformation(&mut info);
            let len = info
                .TimeZoneKeyName
                .iter()
                .position(|v| *v == 0)
                .unwrap_or(128);
            let key = String::from_utf16_lossy(&info.TimeZoneKeyName[..len]);
            if status != u32::MAX {
                ZONES
                    .iter()
                    .find(|(_, win)| *win == key)
                    .map(|(zone, _)| (*zone).to_owned())
                    .unwrap_or_else(|| "Local time".into())
            } else {
                "Local time".into()
            }
        }
    } else {
        id.to_owned()
    };
    selected
        .split('/')
        .next_back()
        .unwrap_or("Local time")
        .replace('_', " ")
}
pub fn now(zone: &Option<DYNAMIC_TIME_ZONE_INFORMATION>) -> SYSTEMTIME {
    unsafe {
        let mut t = zeroed();
        if let Some(zone) = zone {
            let mut utc = zeroed();
            GetSystemTime(&mut utc);
            SystemTimeToTzSpecificLocalTimeEx(zone, &utc, &mut t);
        } else {
            GetLocalTime(&mut t);
        }
        t
    }
}
pub fn hour(h: u16, twelve: bool) -> u16 {
    if twelve {
        if h % 12 == 0 {
            12
        } else {
            h % 12
        }
    } else {
        h
    }
}
pub fn digital(t: &SYSTEMTIME, twelve: bool) -> String {
    format!("{:02}:{:02}", hour(t.wHour, twelve), t.wMinute)
}
pub fn date(t: &SYSTEMTIME) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    format!(
        "{}, {} {}, {}",
        DAYS[t.wDayOfWeek as usize % 7],
        MONTHS[t.wMonth.saturating_sub(1) as usize % 12],
        t.wDay,
        t.wYear
    )
}
pub fn progress(t: &SYSTEMTIME) -> f32 {
    (t.wHour as f32 * 3600. + t.wMinute as f32 * 60. + t.wSecond as f32) / 86400.
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn named_clock_zones_display_city_without_underscores() {
        assert_eq!(location_label("Asia/Ho_Chi_Minh"), "Ho Chi Minh");
        assert_eq!(location_label("America/New_York"), "New York");
        assert_eq!(location_label("UTC"), "UTC");
    }
    #[test]
    fn midnight_noon_and_day_progress() {
        let mut t: SYSTEMTIME = unsafe { zeroed() };
        assert_eq!(digital(&t, true), "12:00");
        t.wHour = 12;
        assert_eq!(digital(&t, true), "12:00");
        assert_eq!(progress(&t), 0.5);
        t.wHour = 23;
        t.wMinute = 59;
        t.wSecond = 59;
        assert!(progress(&t) < 1.);
    }
    #[test]
    fn dst_rules_change_new_york_at_transition() {
        unsafe {
            let z = zone("America/New_York").unwrap().unwrap();
            let mut utc: SYSTEMTIME = zeroed();
            utc.wYear = 2026;
            utc.wMonth = 3;
            utc.wDay = 8;
            utc.wHour = 6;
            utc.wMinute = 59;
            let mut out = zeroed();
            assert_ne!(SystemTimeToTzSpecificLocalTimeEx(&z, &utc, &mut out), 0);
            assert_eq!(out.wHour, 1);
            utc.wHour = 7;
            utc.wMinute = 0;
            SystemTimeToTzSpecificLocalTimeEx(&z, &utc, &mut out);
            assert_eq!(out.wHour, 3);
        }
    }
    #[test]
    fn imports_and_round_trips_without_writing_electron_state() {
        let root = std::env::temp_dir().join(format!("lumen-clock-model-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("config")).unwrap();
        let source=br#"{"settings":{"clock":{"theme":"aurora","timeZone":"Asia/Tokyo","hour12":true,"showSeconds":false,"position":{"x":-400,"y":80},"size":{"width":500,"height":320}},"widgetOpacity":{"clock":40}}}"#;
        fs::write(root.join("config/state.json"), source).unwrap();
        let c = Config::load(&root).unwrap();
        assert_eq!(c.theme, "aurora");
        assert_eq!(c.position, Some([-400, 80]));
        assert_eq!(c.size, [500, 320]);
        assert_eq!(c.opacity, 40.);
        assert!(c.hour12);
        assert!(!c.show_seconds);
        c.save(&root).unwrap();
        assert_eq!(fs::read(root.join("config/state.json")).unwrap(), source);
        let saved = Config::load(&root).unwrap();
        assert_eq!(saved.size, c.size);
        assert_eq!(saved.time_zone, c.time_zone);
        fs::write(root.join("config/native-clock.json"), b"{broken").unwrap();
        assert!(Config::load(&root).is_err());
        assert_eq!(
            fs::read(root.join("config/native-clock.json")).unwrap(),
            b"{broken"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn compact_bounds_and_clock_local_appearance_survive_restart() {
        let root =
            std::env::temp_dir().join(format!("lumen-clock-compact-model-{}", std::process::id()));
        let mut c = Config::default();
        c.size = [160, 176];
        c.position = Some([-40, 22]);
        c.theme = "paper".into();
        c.blur = 27.;
        c.frost_strength = 42.;
        c.palette = "ocean".into();
        c.validate().unwrap();
        c.save(&root).unwrap();
        let loaded = Config::load(&root).unwrap();
        assert_eq!(loaded.size, [160, 176]);
        assert_eq!(loaded.position, c.position);
        assert_eq!(loaded.theme, "paper");
        assert_eq!(loaded.blur, 27.);
        assert_eq!(loaded.frost_strength, 42.);
        assert_eq!(loaded.palette, crate::surface::normalize("ocean").unwrap());
        let mut old: Config =
            serde_json::from_str(r#"{"theme":"lumen","size":[160,160]}"#).unwrap();
        old.validate().unwrap();
        assert_eq!(old.size, [160, 160]);
        assert_eq!(old.blur, 13.);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn tiny_bounds_survive_restart_and_old_minimum_is_preserved() {
        let root =
            std::env::temp_dir().join(format!("lumen-clock-tiny-model-{}", std::process::id()));
        for size in [[120, 120], [128, 128], [120, 320], [320, 120], [160, 176]] {
            let mut c = Config::default();
            c.size = size;
            c.validate().unwrap();
            assert_eq!(c.size, size);
            c.save(&root).unwrap();
            assert_eq!(Config::load(&root).unwrap().size, size);
        }
        let mut c = Config::default();
        c.size = [1, 119];
        c.validate().unwrap();
        assert_eq!(c.size, [120, 120]);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn bad_config_is_not_reset() {
        let mut c = Config::default();
        c.theme = "missing".into();
        assert!(c.validate().is_err());
        assert!(zone("Nowhere/Invalid").is_err());
        assert!(zone("Asia/Bangkok").is_ok());
    }
    #[test]
    fn surface_gradient_custom_and_legacy_flat_survive_restart() {
        let root =
            std::env::temp_dir().join(format!("lumen-clock-surfaces-{}", std::process::id()));
        for palette in ["gradient:dawn", "custom:#345678", "ocean", "theme"] {
            let mut c = Config::default();
            c.palette = palette.into();
            c.validate().unwrap();
            let expected = if palette == "theme" {
                "theme".to_owned()
            } else {
                crate::surface::normalize(palette).unwrap()
            };
            assert_eq!(c.palette, expected);
            if palette == "ocean" {
                assert!(expected.starts_with("custom:#"));
            }
            c.save(&root).unwrap();
            let loaded = Config::load(&root).unwrap();
            assert_eq!(loaded.palette, expected);
            assert_eq!(loaded.theme, c.theme);
        }
        for palette in ["custom", "custom:#xyzxyz", "missing-surface"] {
            let mut c = Config::default();
            c.palette = palette.into();
            assert!(c.validate().is_err());
        }
        fs::remove_dir_all(root).unwrap();
    }
}
