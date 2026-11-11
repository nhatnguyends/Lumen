use serde::{Deserialize, Serialize};
use std::{fs, path::Path};
pub const MIN_WIDTH: i32 = 168;
pub const MIN_HEIGHT: i32 = 190;

pub const THEMES: [&str; 9] = [
    "lumen", "almanac", "blocks", "pop", "prism", "grove", "mist", "agenda", "soft",
];
pub const MONTHS: [&str; 12] = [
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Date {
    pub year: i32,
    pub month: i32,
    pub day: i32,
}
impl Date {
    pub fn today() -> Self {
        unsafe {
            let mut time = std::mem::zeroed();
            windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut time);
            Self {
                year: time.wYear as i32,
                month: time.wMonth as i32,
                day: time.wDay as i32,
            }
        }
    }
    pub fn days_in_month(self) -> i32 {
        match self.month {
            4 | 6 | 9 | 11 => 30,
            2 => {
                if self.year % 4 == 0 && (self.year % 100 != 0 || self.year % 400 == 0) {
                    29
                } else {
                    28
                }
            }
            _ => 31,
        }
    }
    pub fn month_offset(self, delta: i32) -> Self {
        let month = self.year * 12 + self.month - 1 + delta;
        let mut date = Self {
            year: month.div_euclid(12).clamp(1601, 9999),
            month: month.rem_euclid(12) + 1,
            day: self.day,
        };
        date.day = date.day.min(date.days_in_month());
        date
    }
    pub fn add_days(mut self, delta: i32) -> Self {
        self.day += delta;
        while self.day < 1 {
            let d = self.day;
            self = self.month_offset(-1);
            self.day = d + self.days_in_month();
        }
        while self.day > self.days_in_month() {
            let d = self.day - self.days_in_month();
            self = self.month_offset(1);
            self.day = d;
        }
        self
    }
    pub fn weekday(self) -> usize {
        let offsets = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
        let y = self.year - if self.month < 3 { 1 } else { 0 };
        ((y + y / 4 - y / 100 + y / 400 + offsets[self.month as usize - 1] + self.day) % 7) as usize
    }
    pub fn matrix(self, monday: bool) -> [Self; 42] {
        let first = Self { day: 1, ..self };
        let offset = (first.weekday() + if monday { 6 } else { 0 }) % 7;
        std::array::from_fn(|i| first.add_days(i as i32 - offset as i32))
    }
    pub fn week(self, monday: bool) -> [Self; 7] {
        let offset = (self.weekday() + if monday { 6 } else { 0 }) % 7;
        std::array::from_fn(|i| self.add_days(i as i32 - offset as i32))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub version: u32,
    pub theme: String,
    pub week_starts_on: u8,
    pub open: bool,
    pub floating: bool,
    pub frosted: bool,
    pub opacity: f32,
    pub tint: f32,
    pub frost_strength: f32,
    pub blur: f32,
    pub palette: String,
    pub lumen_surface: String,
    pub mist_weekly: bool,
    pub position: Option<[i32; 2]>,
    pub size: [i32; 2],
}
impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            theme: "lumen".into(),
            week_starts_on: 1,
            open: false,
            floating: false,
            frosted: true,
            opacity: 100.,
            tint: 18.,
            frost_strength: 100.,
            blur: 13.,
            palette: "wallpaper-auto".into(),
            lumen_surface: "glass".into(),
            mist_weekly: true,
            position: None,
            size: [420, 460],
        }
    }
}
impl Config {
    pub fn load(root: &Path) -> Result<Self, String> {
        let path = root.join("config/native-calendar.json");
        let mut c: Self = if path.exists() {
            serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?).map_err(|e| {
                format!("Cannot read native calendar config: {e}. File left untouched.")
            })?
        } else {
            let mut c = Self::default();
            let old = root.join("config/calendar.json");
            if old.exists() {
                let v: serde_json::Value =
                    serde_json::from_slice(&fs::read(old).map_err(|e| e.to_string())?)
                        .map_err(|e| format!("Cannot import calendar: {e}"))?;
                if let Some(v) = v.get("config") {
                    if let Some(s) = v
                        .get("theme")
                        .and_then(|v| v.as_str())
                        .filter(|s| matches!(*s, "glass" | "paper" | "midnight"))
                    {
                        c.lumen_surface = s.into();
                    }
                    c.week_starts_on = v
                        .get("weekStartsOn")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(1)
                        .min(1) as u8;
                    if let (Some(x), Some(y)) = (
                        v.pointer("/position/x").and_then(|v| v.as_i64()),
                        v.pointer("/position/y").and_then(|v| v.as_i64()),
                    ) {
                        c.position = Some([
                            x.clamp(-100000, 100000) as i32,
                            y.clamp(-100000, 100000) as i32,
                        ]);
                    }
                    if let (Some(w), Some(h)) = (
                        v.pointer("/size/width").and_then(|v| v.as_i64()),
                        v.pointer("/size/height").and_then(|v| v.as_i64()),
                    ) {
                        c.size = [
                            w.clamp(MIN_WIDTH as i64, 1600) as i32,
                            h.clamp(MIN_HEIGHT as i64, 1200) as i32,
                        ];
                    }
                }
            }
            let state = root.join("config/state.json");
            if state.exists() {
                let v: serde_json::Value =
                    serde_json::from_slice(&fs::read(state).map_err(|e| e.to_string())?)
                        .map_err(|e| format!("Cannot import calendar appearance: {e}"))?;
                if let Some(theme) = v
                    .pointer("/settings/widgetDesign/calendar")
                    .and_then(|v| v.as_str())
                {
                    c.theme = theme.into();
                }
                c.opacity = v
                    .pointer("/settings/widgetOpacity/calendar")
                    .or_else(|| v.pointer("/settings/backgroundOpacity"))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(100.) as f32;
                c.tint = v
                    .pointer("/settings/veil")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(18.) as f32;
            }
            c
        };
        c.palette = crate::surface::normalize(&c.palette).unwrap_or(c.palette);
        if c.version != 1
            || !THEMES.contains(&c.theme.as_str())
            || c.week_starts_on > 1
            || !c.opacity.is_finite()
            || !c.tint.is_finite()
            || !c.frost_strength.is_finite()
            || !c.blur.is_finite()
            || !matches!(c.lumen_surface.as_str(), "glass" | "paper" | "midnight")
            || !crate::surface::valid(&c.palette)
        {
            return Err("Invalid native calendar config. File left untouched.".into());
        }
        c.size = [
            c.size[0].clamp(MIN_WIDTH, 1600),
            c.size[1].clamp(MIN_HEIGHT, 1200),
        ];
        c.opacity = c.opacity.clamp(0., 100.);
        c.tint = c.tint.clamp(0., 100.);
        c.frost_strength = c.frost_strength.clamp(0., 100.);
        c.blur = c.blur.clamp(0., 80.);
        Ok(c)
    }
    pub fn save(&self, root: &Path) -> Result<(), String> {
        let folder = root.join("config");
        fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        let dest = folder.join("native-calendar.json");
        let temp = folder.join("native-calendar.json.tmp");
        fs::write(
            &temp,
            serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        unsafe {
            use windows_sys::Win32::Storage::FileSystem::*;
            if MoveFileExW(
                crate::model::wide(temp).as_ptr(),
                crate::model::wide(dest).as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            ) == 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn date_only_calendar_loads_earlier_configs_without_reintroducing_entry_features() {
        let root =
            std::env::temp_dir().join(format!("lumen-calendar-date-only-{}", std::process::id()));
        let path = root.join("config/native-calendar.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let old=br#"{"theme":"agenda","size":[168,190],"mistWeekly":false,"entries":[{"date":{"year":2026,"month":11,"day":10},"note":"Old note","events":["Old event"]}]}"#;
        std::fs::write(&path, old).unwrap();
        let config = Config::load(&root).unwrap();
        assert_eq!(config.theme, "agenda");
        assert_eq!(config.size, [168, 190]);
        assert!(!config.mist_weekly);
        assert_eq!(std::fs::read(&path).unwrap(), old);
        let serialized = serde_json::to_value(config).unwrap();
        assert!(serialized.get("entries").is_none());
        assert!(serialized.get("events").is_none());
        assert!(serialized.get("note").is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn reference_themes_preserve_materials_and_view_across_restart() {
        let root =
            std::env::temp_dir().join(format!("lumen-calendar-references-{}", std::process::id()));
        for (theme, palette) in [
            ("mist", "gradient:afterglow"),
            ("agenda", "gradient:dawn"),
            ("soft", "custom:#12abef"),
        ] {
            let c = Config {
                theme: theme.into(),
                palette: palette.into(),
                mist_weekly: false,
                ..Config::default()
            };
            c.save(&root).unwrap();
            let restored = Config::load(&root).unwrap();
            assert_eq!(restored.theme, theme);
            assert_eq!(
                restored.palette,
                crate::surface::normalize(palette).unwrap()
            );
            assert!(!restored.mist_weekly);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn weekly_view_crosses_year_and_obeys_first_weekday() {
        let date = Date {
            year: 2027,
            month: 1,
            day: 1,
        };
        let monday = date.week(true);
        assert_eq!(
            monday[0],
            Date {
                year: 2026,
                month: 12,
                day: 28
            }
        );
        assert_eq!(
            monday[6],
            Date {
                year: 2027,
                month: 1,
                day: 3
            }
        );
        assert_eq!(date.week(false)[0].weekday(), 0);
        assert_eq!(date.add_days(7).week(true)[0], monday[0].add_days(7));
    }
    #[test]
    fn compact_bounds_and_material_settings_survive_restart() {
        let root =
            std::env::temp_dir().join(format!("lumen-calendar-small-model-{}", std::process::id()));
        let c = Config {
            size: [MIN_WIDTH, MIN_HEIGHT],
            blur: 42.,
            frost_strength: 65.,
            palette: "ocean".into(),
            lumen_surface: "paper".into(),
            ..Config::default()
        };
        c.save(&root).unwrap();
        let loaded = Config::load(&root).unwrap();
        assert_eq!(loaded.size, [MIN_WIDTH, MIN_HEIGHT]);
        assert_eq!(loaded.blur, 42.);
        assert_eq!(loaded.frost_strength, 65.);
        assert_eq!(loaded.palette, crate::surface::normalize("ocean").unwrap());
        assert_eq!(loaded.lumen_surface, "paper");
        for size in [[1, 1], [180, 200], [168, 680], [800, 190]] {
            let c = Config { size, ..c.clone() };
            c.save(&root).unwrap();
            let loaded = Config::load(&root).unwrap();
            assert_eq!(
                loaded.size,
                [size[0].max(MIN_WIDTH), size[1].max(MIN_HEIGHT)]
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn leap_centuries_and_rollover() {
        for (y, n) in [(1900, 28), (2000, 29), (2024, 29), (2100, 28)] {
            assert_eq!(
                Date {
                    year: y,
                    month: 2,
                    day: 1
                }
                .days_in_month(),
                n
            );
        }
        assert_eq!(
            Date {
                year: 2024,
                month: 3,
                day: 1
            }
            .add_days(-1),
            Date {
                year: 2024,
                month: 2,
                day: 29
            }
        );
        assert_eq!(
            Date {
                year: 2026,
                month: 12,
                day: 31
            }
            .add_days(1),
            Date {
                year: 2027,
                month: 1,
                day: 1
            }
        );
        assert_eq!(
            Date {
                year: 2024,
                month: 1,
                day: 31
            }
            .month_offset(1),
            Date {
                year: 2024,
                month: 2,
                day: 29
            }
        );
    }
    #[test]
    fn grid_has_42_consecutive_calendar_days() {
        for year in 1999..2030 {
            for month in 1..=12 {
                for monday in [false, true] {
                    let grid = Date {
                        year,
                        month,
                        day: 1,
                    }
                    .matrix(monday);
                    assert_eq!(grid[0].weekday(), if monday { 1 } else { 0 });
                    assert!(grid
                        .iter()
                        .any(|d| d.year == year && d.month == month && d.day == 1));
                    for i in 1..42 {
                        assert_eq!(grid[i - 1].add_days(1), grid[i]);
                    }
                }
            }
        }
    }
    #[test]
    fn imports_without_modifying_old_data_and_round_trips_native_settings() {
        let root =
            std::env::temp_dir().join(format!("lumen-calendar-import-{}", std::process::id()));
        fs::create_dir_all(root.join("config")).unwrap();
        let source=br#"{"config":{"theme":"paper","weekStartsOn":0,"position":{"x":120,"y":200},"size":{"width":450,"height":500}},"events":[{"subject":"Untouched"}]}"#;
        fs::write(root.join("config/calendar.json"), source).unwrap();
        fs::write(root.join("config/state.json"),br#"{"settings":{"widgetDesign":{"calendar":"grove"},"widgetOpacity":{"calendar":47}}}"#).unwrap();
        let mut c = Config::load(&root).unwrap();
        assert_eq!(c.week_starts_on, 0);
        assert_eq!(c.theme, "grove");
        assert_eq!(c.lumen_surface, "paper");
        assert_eq!(c.opacity, 47.);
        assert_eq!(c.position, Some([120, 200]));
        assert_eq!(c.size, [450, 500]);
        c.floating = true;
        c.save(&root).unwrap();
        assert!(Config::load(&root).unwrap().floating);
        assert_eq!(fs::read(root.join("config/calendar.json")).unwrap(), source);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn corrupt_and_future_native_config_are_left_untouched() {
        let root =
            std::env::temp_dir().join(format!("lumen-calendar-invalid-{}", std::process::id()));
        fs::create_dir_all(root.join("config")).unwrap();
        let file = root.join("config/native-calendar.json");
        for value in [
            "broken",
            "{\"version\":2}",
            "{\"theme\":\"unknown\"}",
            "{\"weekStartsOn\":4}",
        ] {
            fs::write(&file, value).unwrap();
            assert!(Config::load(&root).is_err());
            assert_eq!(fs::read_to_string(&file).unwrap(), value);
        }
        fs::remove_dir_all(root).unwrap();
    }
}
