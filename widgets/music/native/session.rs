//! Last visible music session, loaded locally before the first widget frame.
//! This is display state only: loading it never starts a browser or playback.
use super::*;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Snapshot {
    version: u32,
    pub playback: Playback,
    pub art_files: BTreeMap<String, String>,
    pub last_cover: Option<String>,
}
pub(super) fn valid_file(file: &str) -> bool {
    file.len() == 68
        && file.ends_with(".img")
        && file.as_bytes()[..64].iter().all(u8::is_ascii_hexdigit)
}
impl Snapshot {
    pub fn load(root: &Path) -> Option<Self> {
        let dir = root.join("config/native-music-player");
        let file = dir.join("last-view.json");
        let mut result: Self = if file.exists() {
            if fs::metadata(&file).ok()?.len() > 256 * 1024 {
                return None;
            }
            serde_json::from_slice(&fs::read(file).ok()?).ok()?
        } else {
            // Older versions cached images but not the URL/metadata manifest.
            // Retain their most recent decodable artwork without inventing a
            // title, artist, queue or playable track association.
            let mut files: Vec<_> = fs::read_dir(dir.join("art"))
                .ok()?
                .filter_map(Result::ok)
                .filter(|entry| valid_file(&entry.file_name().to_string_lossy()))
                .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
                .collect();
            files.sort_by_key(|entry| std::cmp::Reverse(entry.0));
            let path = files.into_iter().take(20).map(|e| e.1).find(|path| {
                image::ImageReader::open(path)
                    .ok()
                    .and_then(|r| r.with_guessed_format().ok())
                    .and_then(|mut r| {
                        let mut limits = image::Limits::default();
                        limits.max_alloc = Some(32 * 1024 * 1024);
                        r.limits(limits);
                        r.decode().ok()
                    })
                    .is_some()
            })?;
            let name = path.file_name()?.to_str()?.to_owned();
            let url = format!("cached:{name}");
            Self {
                version: 1,
                playback: Playback {
                    art: Some(url.clone()),
                    ..Playback::default()
                },
                art_files: BTreeMap::from([(url.clone(), name)]),
                last_cover: Some(url),
            }
        };
        if result.version != 1 {
            return None;
        }
        result.art_files.retain(|url, name| {
            url.len() <= 8192 && valid_file(name) && dir.join("art").join(name).is_file()
        });
        if result.art_files.len() > 128 {
            return None;
        }
        result.last_cover = result
            .last_cover
            .filter(|url| result.art_files.contains_key(url));
        if result.last_cover.is_none() {
            result.last_cover = result
                .playback
                .art
                .clone()
                .filter(|url| result.art_files.contains_key(url));
        }
        result.playback.queue.truncate(60);
        result
            .playback
            .title
            .truncate(result.playback.title.floor_char_boundary(2048));
        result
            .playback
            .artist
            .truncate(result.playback.artist.floor_char_boundary(2048));
        if !result.playback.duration.is_finite() {
            result.playback.duration = 0.;
        }
        result.playback.duration = result.playback.duration.clamp(0., 86400.);
        result.playback.position = 0.;
        result.playback.available = false;
        result.playback.playing = false;
        result.playback.can_next = false;
        result.playback.can_previous = false;
        result.playback.can_seek = false;
        result.playback.error = None;
        Some(result)
    }
    fn save(&self, root: &Path) -> Result<(), String> {
        let dir = root.join("config/native-music-player");
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let file = dir.join("last-view.json");
        let tmp = dir.join("last-view.json.tmp");
        fs::write(&tmp, serde_json::to_vec(self).map_err(|e| e.to_string())?)
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
impl State {
    pub(super) fn accept_playback(&mut self, mut next: Playback) {
        // Loading/disconnection must not erase the previous session's artwork
        // or metadata. Transport capabilities always describe the live player.
        if !next.available && !self.playback.title.is_empty() {
            self.playback.available = false;
            self.playback.playing = false;
            self.playback.can_seek = false;
            self.playback.can_next = false;
            self.playback.can_previous = false;
            return;
        }
        if next.art.as_ref().is_none_or(|a| a.is_empty()) {
            next.art = self.playback.art.clone();
        }
        for track in &mut next.queue {
            if track.thumb.is_empty() {
                if let Some(old) = self
                    .playback
                    .queue
                    .iter()
                    .find(|t| t.video_id == track.video_id)
                {
                    track.thumb = old.thumb.clone();
                }
            }
        }
        if next.queue.is_empty()
            && self
                .playback
                .queue
                .iter()
                .any(|t| t.video_id == next.video_id)
        {
            next.queue = self
                .playback
                .queue
                .iter()
                .cloned()
                .map(|mut t| {
                    t.selected = t.video_id == next.video_id;
                    t
                })
                .collect();
        }
        self.playback = next;
        self.received = Instant::now();
    }
    pub(super) fn cover_image(&self, url: &str) -> Option<&Image> {
        self.images.get(url).or_else(|| {
            let is_current = url.is_empty() || Some(url) == self.playback.art.as_deref();
            if is_current {
                self.last_cover
                    .as_ref()
                    .and_then(|old| self.images.get(old))
            } else {
                None
            }
        })
    }
    pub(super) fn remember_view(&mut self) {
        if self.smoke || !self.playback.available || self.last_cover.is_none() {
            return;
        }
        let files: BTreeMap<_, _> = self
            .art_files
            .iter()
            .filter(|(url, name)| {
                valid_file(name)
                    && (self.last_cover.as_ref() == Some(*url)
                        || self.playback.art.as_ref() == Some(*url)
                        || self.playback.queue.iter().any(|t| t.thumb == url.as_str()))
            })
            .take(128)
            .map(|(u, f)| (u.clone(), f.clone()))
            .collect();
        let mut playback = self.playback.clone();
        playback.position = 0.;
        playback.playing = false;
        playback.available = false;
        playback.can_seek = false;
        playback.can_next = false;
        playback.can_previous = false;
        playback.error = None;
        let snapshot = Snapshot {
            version: 1,
            playback,
            art_files: files,
            last_cover: self.last_cover.clone(),
        };
        if let Ok(signature) = serde_json::to_string(&snapshot) {
            if signature != self.session_signature {
                match snapshot.save(&self.root) {
                    Ok(()) => self.session_signature = signature,
                    Err(e) => eprintln!("native-music last-view: {e}"),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn last_session_restores_covers_but_never_live_transport_flags() {
        let root = std::env::temp_dir().join(format!("lumen-music-session-{}", std::process::id()));
        let dir = root.join("config/native-music-player/art");
        fs::create_dir_all(&dir).unwrap();
        let name = format!("{}.img", "a".repeat(64));
        fs::write(dir.join(&name), b"cached image").unwrap();
        let snapshot = Snapshot {
            version: 1,
            playback: Playback {
                title: "Previous song".into(),
                artist: "Previous artist".into(),
                art: Some("https://i.ytimg.com/art".into()),
                available: true,
                playing: true,
                can_seek: true,
                can_next: true,
                ..Playback::default()
            },
            art_files: BTreeMap::from([("https://i.ytimg.com/art".into(), name)]),
            last_cover: Some("https://i.ytimg.com/art".into()),
        };
        snapshot.save(&root).unwrap();
        let restored = Snapshot::load(&root).unwrap();
        assert_eq!(restored.playback.title, "Previous song");
        assert!(restored.last_cover.is_some());
        assert!(
            !restored.playback.available
                && !restored.playback.playing
                && !restored.playback.can_seek
                && !restored.playback.can_next
        );
        assert!(!valid_file("../../outside.img"));
        fs::remove_dir_all(root).unwrap();
    }
}
