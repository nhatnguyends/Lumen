use crate::model::wide;
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

pub const DESIGNS: [&str; 5] = [
    "coverflow",
    "compact",
    "turntable",
    "orbit",
    "disc",
];
pub const MIN_SIZE: [i32; 2] = [168, 104];
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub version: u32,
    pub design: String,
    pub position: Option<[i32; 2]>,
    pub size: [i32; 2],
    pub floating: bool,
    pub open: bool,
    pub queue: bool,
    pub opacity: f32,
    pub tint: f32,
    pub frosted_glass: bool,
    pub frost_strength: f32,
    pub blur: f32,
    pub palette: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            design: "coverflow".into(),
            position: None,
            size: [491, 479],
            floating: false,
            open: true,
            queue: false,
            opacity: 100.,
            tint: crate::model::Dock::default().tint,
            frosted_glass: true,
            frost_strength: 100.,
            blur: 4.,
            palette: crate::accent::THEME.into(),
        }
    }
}
impl Config {
    pub fn load(root: &Path) -> Result<Self, String> {
        let file = root.join("config/native-music.json");
        let mut config: Self = if file.exists() {
            serde_json::from_slice(&fs::read(&file).map_err(|e| e.to_string())?).map_err(|e| {
                format!("Cannot read native Music config: {e}. File left untouched.")
            })?
        } else {
            let mut config = Self::default();
            let old = root.join("config/state.json");
            if old.exists() {
                let state: serde_json::Value =
                    serde_json::from_slice(&fs::read(old).map_err(|e| e.to_string())?)
                        .map_err(|e| format!("Cannot import Music settings: {e}"))?;
                if let Some(design) = state
                    .pointer("/settings/widgetDesign/music")
                    .and_then(|v| v.as_str())
                {
                    config.design = design.into();
                }
                config.opacity = state
                    .pointer("/settings/widgetOpacity/music")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(100.) as f32;
            }
            config
        };
        if matches!(config.design.as_str(), "sleeve" | "lumen" | "sky") {
            config.design = "compact".into();
        }
        if let Some(palette) = crate::surface::normalize(&config.palette) {
            config.palette = palette;
        }
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&mut self) -> Result<(), String> {
        if self.version != 1
            || !DESIGNS.contains(&self.design.as_str())
            || !self.opacity.is_finite()
            || !self.tint.is_finite()
            || !self.frost_strength.is_finite()
            || !self.blur.is_finite()
            || !crate::surface::valid(&self.palette)
        {
            return Err("Invalid native Music config. File left untouched.".into());
        }
        self.size = [
            self.size[0].clamp(MIN_SIZE[0], 1100),
            self.size[1].clamp(MIN_SIZE[1], 1000),
        ];
        self.opacity = self.opacity.clamp(0., 100.);
        self.tint = self.tint.clamp(0., 100.);
        self.frost_strength = self.frost_strength.clamp(0., 100.);
        self.blur = self.blur.clamp(0., 40.);
        Ok(())
    }
    pub fn save(&self, root: &Path) -> Result<(), String> {
        let dir = root.join("config");
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let file = dir.join("native-music.json");
        let tmp = dir.join("native-music.json.tmp");
        fs::write(
            &tmp,
            serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        unsafe {
            use windows_sys::Win32::Storage::FileSystem::*;
            if MoveFileExW(
                wide(&tmp).as_ptr(),
                wide(&file).as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            ) == 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        Ok(())
    }
}
#[derive(Clone, Default, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Track {
    pub video_id: String,
    pub title: String,
    pub artist: String,
    pub thumb: String,
    pub selected: bool,
}
#[derive(Clone, Default, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Playback {
    pub signed_in: bool,
    pub auth_known: bool,
    pub album: String,
    pub video_id: String,
    pub available: bool,
    pub playing: bool,
    pub title: String,
    pub artist: String,
    pub art: Option<String>,
    pub position: f64,
    pub duration: f64,
    pub can_next: bool,
    pub can_previous: bool,
    pub can_seek: bool,
    pub queue: Vec<Track>,
    pub error: Option<String>,
}
pub fn time(seconds: f64) -> String {
    let seconds = if seconds.is_finite() {
        seconds.max(0.) as u64
    } else {
        0
    };
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_config_is_rejected_and_geometry_is_bounded() {
        let mut c = Config::default();
        c.design = "missing".into();
        assert!(c.validate().is_err());
        c.design = "compact".into();
        c.size = [1, 99999];
        c.validate().unwrap();
        assert_eq!(c.size, [168, 1000]);
        c.version = 2;
        assert!(c.validate().is_err());
    }
    #[test]
    fn format_media_time() {
        assert_eq!(time(f64::NAN), "0:00");
        assert_eq!(time(-10.), "0:00");
        assert_eq!(time(125.9), "2:05");
    }
    #[test]
    fn account_status_reads_live_signed_in_state_and_defaults_for_old_sessions() {
        let old: Playback = serde_json::from_str(r#"{"title":"Previous track"}"#).unwrap();
        assert!(!old.signed_in);
        let live: Playback =
            serde_json::from_str(r#"{"signedIn":true,"available":false}"#).unwrap();
        assert!(live.signed_in);
        assert!(
            !live.available,
            "account status is independent from playback availability"
        );
    }
    #[test]
    fn surface_palettes_round_trip_and_legacy_flat_migrates_without_changing_color() {
        let root =
            std::env::temp_dir().join(format!("lumen-music-surfaces-{}", std::process::id()));
        let mut c = Config::default();
        c.design = "orbit".into();
        c.position = Some([-600, 180]);
        c.size = [480, 620];
        for palette in [
            "gradient:afterglow",
            "gradient:dawn",
            "gradient:dusk",
            "gradient:dust",
            "custom:#12ABEF",
        ] {
            c.palette = palette.into();
            c.validate().unwrap();
            c.save(&root).unwrap();
            let restored = Config::load(&root).unwrap();
            assert_eq!(
                restored.palette,
                crate::surface::normalize(palette).unwrap()
            );
            assert_eq!(restored.design, c.design);
            assert_eq!(restored.position, c.position);
            assert_eq!(restored.size, c.size);
        }
        for legacy in ["ocean", "porcelain", "obsidian"] {
            c.palette = legacy.into();
            c.save(&root).unwrap();
            let restored = Config::load(&root).unwrap();
            assert!(restored.palette.starts_with("custom:#"));
            assert_eq!(
                crate::surface::color(&restored.palette, 0),
                crate::surface::color(legacy, 0)
            );
        }
        for invalid in ["custom", "custom:#123", "custom:#GG0000", "missing"] {
            c.palette = invalid.into();
            assert!(c.validate().is_err());
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn native_settings_round_trip_without_overwriting_electron_and_bad_data_stays() {
        let root = std::env::temp_dir().join(format!("lumen-music-model-{}", std::process::id()));
        fs::create_dir_all(root.join("config")).unwrap();
        let old =
            br#"{"settings":{"widgetDesign":{"music":"sleeve"},"widgetOpacity":{"music":65}}}"#;
        let source = root.join("config/state.json");
        fs::write(&source, old).unwrap();
        let mut c = Config::load(&root).unwrap();
        assert_eq!(c.design, "compact");
        assert_eq!(c.opacity, 65.);
        c.size = MIN_SIZE;
        c.position = Some([-800, 120]);
        c.queue = true;
        c.floating = true;
        c.save(&root).unwrap();
        let restored = Config::load(&root).unwrap();
        assert_eq!(restored.size, MIN_SIZE);
        assert_eq!(restored.position, Some([-800, 120]));
        assert!(restored.queue && restored.floating);
        assert_eq!(fs::read(&source).unwrap(), old);
        let own = root.join("config/native-music.json");
        fs::write(&own, "{broken").unwrap();
        assert!(Config::load(&root).is_err());
        assert_eq!(fs::read_to_string(&own).unwrap(), "{broken");
        fs::remove_dir_all(root).unwrap();
    }
}
