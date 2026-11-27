//! YouTube Music page JS expressions, input sanitization, and bounded WinHttp artwork cache.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::Path,
    ptr::{null, null_mut},
    time::SystemTime,
};
use windows_sys::Win32::{
    Networking::WinHttp::*,
    Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW},
};

pub const MAX_ART_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ART_FILES: usize = 40;
const ART_TIMEOUT_MS: i32 = 8000;

pub const MUSIC_PLAY_PROBE: &str = r###"(() => {
  if (location.origin !== 'https://music.youtube.com') return { onOrigin: false };
  const pickVideo = () => {
    const main = document.querySelector('#movie_player video.html5-main-video, ytmusic-player #movie_player video, ytmusic-player video.html5-main-video, ytmusic-player video');
    if (main && (main.currentSrc || main.readyState > 0 || !main.paused)) return main;
    const all = [...document.querySelectorAll('video')];
    return all.find(v => !v.paused && !v.ended && (v.currentSrc || v.readyState > 0))
      || all.sort((a, b) => (b.readyState - a.readyState) || ((b.currentSrc ? 1 : 0) - (a.currentSrc ? 1 : 0)))[0]
      || main
      || null;
  };
  const video = pickVideo();
  const btn = document.querySelector('ytmusic-player-bar .play-pause-button');
  const api = document.getElementById?.('movie_player');
  const playerState = typeof api?.getPlayerState === 'function' ? api.getPlayerState() : -1;
  const errNode = document.querySelector('yt-playability-error-supported-renderers #reason, yt-playability-error-supported-renderers .reason, ytmusic-player yt-player-error-message-renderer #reason, ytmusic-player .ytp-error-content-wrap-reason, #movie_player .ytp-error-content-wrap-reason, ytmusic-player #error-screen #reason')
    || document.querySelector('ytmusic-player yt-player-error-message-renderer');
  const errText = errNode && errNode.getBoundingClientRect?.().height > 0 ? (errNode.textContent?.replace(/\s+/g, ' ').trim() || '') : '';
  const err = errText || (video?.error ? `Media error ${video.error.code}` : '');
  return {
    onOrigin: true,
    hasControls: !!(video || btn || (playerState >= 1 && playerState <= 5)),
    readyState: video ? video.readyState : 0,
    error: err
  };
})()"###;

pub const MUSIC_PAGE_STATE: &str = r###"(() => {
  if (location.origin !== 'https://music.youtube.com') return null;
  const text = n => n?.textContent?.trim() || '';
  const imgSrc = el => {
    const raw = el?.getAttribute?.('src')?.trim() || '';
    if (!raw || raw.startsWith('data:')) return '';
    const resolved = el.src || raw;
    if (resolved === location.href || resolved === location.origin + '/' || resolved === location.origin) return '';
    return resolved;
  };
  const pickVideo = () => {
    const main = document.querySelector('#movie_player video.html5-main-video, ytmusic-player #movie_player video, ytmusic-player video.html5-main-video, ytmusic-player video');
    if (main && (main.currentSrc || main.readyState > 0 || !main.paused)) return main;
    const all = [...document.querySelectorAll('video')];
    return all.find(v => !v.paused && !v.ended && (v.currentSrc || v.readyState > 0))
      || all.sort((a, b) => (b.readyState - a.readyState) || ((b.currentSrc ? 1 : 0) - (a.currentSrc ? 1 : 0)))[0]
      || main
      || null;
  };
  const bar = document.querySelector('ytmusic-player-bar');
  const video = pickVideo();
  const meta = navigator.mediaSession?.metadata;
  const ad = !!document.querySelector('.ad-showing');
  const api = document.getElementById?.('movie_player');
  const videoData = api?.getVideoData?.() || {};
  const playerState = typeof api?.getPlayerState === 'function' ? api.getPlayerState() : -1;
  const activeId = videoData.video_id || bar?.data?.videoId || '';
  const panel = document.querySelector('ytmusic-player-queue ytmusic-playlist-panel-renderer, ytmusic-player-queue');
  const rows = [...(panel || document).querySelectorAll('ytmusic-player-queue-item')]
    .filter(row => !row.closest?.('#counterpart-renderer, [slot="counterpart"]'));
  const contents = panel?.data?.contents || panel?.data?.playlistPanelRenderer?.contents;
  const unwrap = item => {
    const w = item?.playlistPanelVideoWrapperRenderer;
    if (!w) return item?.playlistPanelVideoRenderer || item;
    const primary = w.primaryRenderer?.playlistPanelVideoRenderer;
    const alts = (w.counterpart || []).map(c => c?.counterpartRenderer?.playlistPanelVideoRenderer).filter(Boolean);
    return alts.find(c => (activeId && c.videoId === activeId) || (!primary?.selected && c.selected === true)) || primary;
  };
  const items = Array.isArray(contents) && contents.some(x => x?.playlistPanelVideoRenderer || x?.playlistPanelVideoWrapperRenderer)
    ? contents.map(x => ({data: unwrap(x)})) : rows;
  const runs = v => v?.runs?.map(r => r.text || '').join('') || v?.simpleText || '';
  const queue = items.slice(0, 60).flatMap(row => {
    const d = row.data?.playlistPanelVideoRenderer || row.data || {};
    const link = row.querySelector?.('a[href*="watch?v="]');
    const id = d.videoId || (link ? new URL(link.href).searchParams.get('v') : '');
    if (!/^[\w-]{11}$/.test(id || '')) return [];
    const thumb = d.thumbnail?.thumbnails?.at(-1)?.url || imgSrc(row.querySelector?.('img')) || '';
    return [{videoId:id,title:runs(d.title) || text(row.querySelector?.('.song-title, #video-title')) || text(link),
      artist:runs(d.shortBylineText) || text(row.querySelector?.('.byline')),thumb,duration:0,
      url:'https://music.youtube.com/watch?v='+id, selected:row.hasAttribute?.('selected') || d.selected === true}];
  });
  const videoId = activeId || queue.find(t => t.selected)?.videoId || new URL(location.href).searchParams.get('v') || '';
  const current = queue.find(t => t.videoId === videoId);
  for (const item of queue) item.selected = item.videoId === videoId;
  const docTitle = (document.title || '').replace(/\s*[-|]\s*YouTube Music\s*$/i, '').trim();
  const title = current?.title || text(bar?.querySelector('.title')) || videoData.title || meta?.title || (activeId ? docTitle : '') || '';
  const artist = current?.artist || text(bar?.querySelector('.byline a')) || text(bar?.querySelector('.subtitle .byline')) || videoData.author || meta?.artist || '';
  const loggedIn = window.ytcfg?.get?.('LOGGED_IN') ?? window.ytcfg?.data_?.LOGGED_IN;
  const avatar = !!document.querySelector('ytmusic-user-menu-renderer #avatar, ytmusic-user-menu-renderer #avatar-btn, ytmusic-nav-bar #avatar, ytmusic-nav-bar #avatar-btn, ytmusic-nav-bar #avatar-button');
  const signIn = !!document.querySelector('ytmusic-nav-bar a[href*="accounts.google.com"], ytmusic-nav-bar a[href*="ServiceLogin"]');
  const signedIn = typeof loggedIn === 'boolean' ? loggedIn : avatar ? true : signIn ? false : null;
  const playing = (!!video && !video.paused && !video.ended && (video.readyState >= 1 || video.currentTime > 0)) || playerState === 1;
  const duration = (Number.isFinite(video?.duration) && video.duration > 0) ? video.duration : (Number.isFinite(api?.getDuration?.()) && api.getDuration() > 0 ? api.getDuration() : 0);
  const position = (Number.isFinite(video?.currentTime) && (video.currentTime > 0 || !api)) ? video.currentTime : (Number.isFinite(api?.getCurrentTime?.()) ? api.getCurrentTime() : 0);
  const hasMedia = (!!video && (video.readyState >= 1 || !!video.currentSrc || !video.paused)) || playerState === 1 || playerState === 2 || playerState === 3 || playerState === 5;
  const available = !!title && hasMedia;
  const fallbackArt = videoId ? 'https://i.ytimg.com/vi/' + videoId + '/hqdefault.jpg' : null;
  const art = current?.thumb || imgSrc(bar?.querySelector('img')) || meta?.artwork?.at(-1)?.src || fallbackArt;
  const errNode = document.querySelector('yt-playability-error-supported-renderers #reason, yt-playability-error-supported-renderers .reason, ytmusic-player yt-player-error-message-renderer #reason, ytmusic-player .ytp-error-content-wrap-reason, #movie_player .ytp-error-content-wrap-reason, ytmusic-player #error-screen #reason')
    || document.querySelector('ytmusic-player yt-player-error-message-renderer');
  const errText = (errNode && errNode.getBoundingClientRect?.().height > 0 ? text(errNode).replace(/\s+/g, ' ').trim() : '')
    || (video?.error ? `Media error ${video.error.code}` : '');
  return { available, playing,
    title, artist,
    album:meta?.album || '', art,
    position, duration, canSeek:!ad && duration > 0,
    canNext:!ad && !!bar?.querySelector('.next-button:not([disabled])'),
    canPrevious:!ad && !!bar?.querySelector('.previous-button:not([disabled])'),
    videoId,
    playlistId:new URL(location.href).searchParams.get('list') || api?.getPlaylistId?.() || '',
    signedIn,
    queue,error:errText || null};
})()"###;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct QueueItem {
    pub video_id: String,
    pub title: String,
    pub artist: String,
    pub duration: f64,
    pub thumb: String,
    pub url: String,
    pub selected: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PlayerPageState {
    pub available: bool,
    pub playing: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub art: Option<String>,
    pub position: f64,
    pub duration: f64,
    pub can_next: bool,
    pub can_previous: bool,
    pub can_seek: bool,
    pub video_id: String,
    pub signed_in: bool,
    pub auth_known: bool,
    pub queue: Vec<QueueItem>,
    pub error: Option<String>,
    #[serde(skip_serializing)]
    pub playlist_id: String,
}

pub fn validate_video_id(video_id: &str) -> Result<(), String> {
    if video_id.len() == 11
        && video_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        Ok(())
    } else {
        Err("Invalid track".into())
    }
}

pub fn sanitize_playlist_id(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .take(128)
        .collect()
}

pub fn music_page_select(video_id: &str) -> Result<String, String> {
    validate_video_id(video_id)?;
    let id_json = serde_json::to_string(video_id).map_err(|e| e.to_string())?;
    Ok(format!(
        r###"(() => {{
    if (location.origin !== 'https://music.youtube.com') return false;
    const id={id_json};
    const row=[...document.querySelectorAll('ytmusic-player-queue-item')].find(row => {{
      const data=row.data?.playlistPanelVideoRenderer || row.data || {{}};
      const link=row.querySelector('a[href*="watch?v="]');
      return (data.videoId || (link ? new URL(link.href).searchParams.get('v') : '')) === id;
    }});
    if (!row) return false;
    const button=row.querySelector('#play-button, .play-button, ytmusic-play-button-renderer button');
    if (button) button.click();
    else row.dispatchEvent(new MouseEvent('dblclick',{{bubbles:true,cancelable:true,composed:true}}));
    return true;
  }})()"###
    ))
}

pub fn music_page_command(action: &str, value: Option<f64>) -> Result<String, String> {
    if !matches!(
        action,
        "toggle" | "play" | "pause" | "next" | "previous" | "seek"
    ) {
        return Err("Unknown playback control".into());
    }
    let action_json = serde_json::to_string(action).map_err(|e| e.to_string())?;
    let value_json = match value {
        Some(v) if v.is_finite() => serde_json::to_string(&v).map_err(|e| e.to_string())?,
        _ => "null".into(),
    };
    Ok(format!(
        r###"(async () => {{
    if (location.origin !== 'https://music.youtube.com') throw new Error('Music player is still loading');
    const action={action_json}, value={value_json};
    const pickVideo = () => {{
      const main = document.querySelector('#movie_player video.html5-main-video, ytmusic-player #movie_player video, ytmusic-player video.html5-main-video, ytmusic-player video');
      if (main && (main.currentSrc || main.readyState > 0 || !main.paused)) return main;
      const all = [...document.querySelectorAll('video')];
      return all.find(v => !v.paused && !v.ended && (v.currentSrc || v.readyState > 0))
        || all.sort((a, b) => (b.readyState - a.readyState) || ((b.currentSrc ? 1 : 0) - (a.currentSrc ? 1 : 0)))[0]
        || main
        || null;
    }};
    const video=pickVideo();
    const api=document.getElementById?.('movie_player');
    const btn=document.querySelector('ytmusic-player-bar .play-pause-button');
    if (!video && !api && !(btn && (action === 'play' || action === 'toggle'))) {{
      throw new Error('Choose a song first');
    }}
    if (action === 'play' || (action === 'toggle' && (!video || video.paused) && api?.getPlayerState?.() !== 1)) {{
      if (video) {{
        let playErr = null;
        try {{ await video.play(); }} catch (err) {{ playErr = err; }}
        if (video.paused) {{
          api?.playVideo?.();
          if (video.paused && btn) btn.click();
          else if (video.paused && playErr) throw playErr;
        }}
      }} else {{
        api?.playVideo?.();
        if (api?.getPlayerState?.() !== 1 && btn) btn.click();
      }}
    }}
    else if (action === 'pause' || action === 'toggle') {{
      video?.pause?.();
      if (api?.getPlayerState?.() === 1) api?.pauseVideo?.();
      else if (!video && btn) btn.click();
    }}
    else if (action === 'seek' && value !== null && !document.querySelector('.ad-showing')) {{
      if (video && Number.isFinite(video.duration) && video.duration > 0) {{
        video.currentTime=Math.max(0,Math.min(video.duration,value));
      }} else if (api?.seekTo) {{
        api.seekTo(Math.max(0, value), true);
      }}
    }}
    else if (action === 'next' || action === 'previous') {{
      const button=document.querySelector('ytmusic-player-bar .' + action + '-button:not([disabled])');
      if (!button || document.querySelector('.ad-showing')) throw new Error('This control is unavailable for the current track');
      button.click();
    }}
    return true;
  }})()"###
    ))
}

fn clean_text(v: &Value) -> String {
    v.as_str()
        .map(|s| s.chars().take(1000).collect())
        .unwrap_or_default()
}

fn clean_seconds(v: &Value) -> f64 {
    v.as_f64()
        .filter(|n| n.is_finite())
        .map(|n| n.max(0.0))
        .unwrap_or(0.0)
}

pub fn has_auth_session_cookies(cookies_result: &Value) -> bool {
    let Some(cookies) = cookies_result["cookies"].as_array() else {
        return false;
    };
    cookies.iter().any(|c| {
        let name = c["name"].as_str().unwrap_or("");
        let value = c["value"].as_str().unwrap_or("");
        !value.is_empty() && matches!(name, "SAPISID" | "__Secure-3PAPISID" | "LOGIN_INFO")
    })
}

pub fn normalize_art_url(url: &str) -> String {
    let Some(rest) = url.strip_prefix("https://") else {
        return url.to_owned();
    };
    let split_idx = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host = rest[..split_idx]
        .strip_suffix(":443")
        .unwrap_or(&rest[..split_idx])
        .to_ascii_lowercase();
    let is_google_art = host == "googleusercontent.com"
        || host.ends_with(".googleusercontent.com")
        || host == "ggpht.com"
        || host.ends_with(".ggpht.com");
    if !is_google_art {
        return url.to_owned();
    }
    let suffix = &rest[split_idx..];
    let q_idx = suffix.find(['?', '#']).unwrap_or(suffix.len());
    let path_part = &suffix[..q_idx];
    let query_frag = &suffix[q_idx..];
    let segment_start = path_part.rfind('/').map_or(0, |i| i + 1);
    let Some(rel_eq) = path_part[segment_start..].rfind('=') else {
        return url.to_owned();
    };
    let eq_idx = segment_start + rel_eq;
    let params = &path_part[eq_idx + 1..];
    if params.is_empty() {
        return url.to_owned();
    }
    let mut changed = false;
    let mut out_params = String::with_capacity(params.len());
    for (i, tok) in params.split('-').enumerate() {
        if i > 0 {
            out_params.push('-');
        }
        if tok == "rw" {
            out_params.push_str("rj");
            changed = true;
        } else {
            out_params.push_str(tok);
        }
    }
    if !changed {
        return url.to_owned();
    }
    format!(
        "https://{}{}={}{}",
        &rest[..split_idx],
        &path_part[..eq_idx],
        out_params,
        query_frag
    )
}

fn clean_https_url(v: &Value) -> Option<String> {
    let s = v.as_str()?;
    if s.len() > 4096 || s.bytes().any(|b| b <= 32 || b == 127) {
        return None;
    }
    let rest = s.strip_prefix("https://")?;
    let host_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host = &rest[..host_end];
    if host.is_empty() || host.contains('@') {
        return None;
    }
    Some(normalize_art_url(s))
}

fn clean_artwork_url(v: &Value) -> Option<String> {
    clean_https_url(v).filter(|u| parse_artwork_url(u).is_ok())
}

pub fn sanitize_page_state(raw: &Value, prev_signed_in: Option<bool>) -> Option<PlayerPageState> {
    if raw.is_null() || !raw.is_object() {
        return None;
    }
    let mut queue = Vec::new();
    if let Some(items) = raw["queue"].as_array() {
        for item in items.iter().take(60) {
            let Some(vid) = item["videoId"].as_str() else {
                continue;
            };
            if validate_video_id(vid).is_err() {
                continue;
            }
            queue.push(QueueItem {
                video_id: vid.to_owned(),
                title: clean_text(&item["title"]),
                artist: clean_text(&item["artist"]),
                duration: clean_seconds(&item["duration"]),
                thumb: clean_artwork_url(&item["thumb"]).unwrap_or_default(),
                url: format!("https://music.youtube.com/watch?v={vid}"),
                selected: item["selected"].as_bool() == Some(true),
            });
        }
    }
    let video_id = raw["videoId"]
        .as_str()
        .filter(|id| validate_video_id(id).is_ok())
        .unwrap_or("")
        .to_owned();
    let mut art = clean_artwork_url(&raw["art"]);
    if art.is_none() && validate_video_id(&video_id).is_ok() {
        art = Some(format!("https://i.ytimg.com/vi/{video_id}/hqdefault.jpg"));
    }
    let auth_known = raw["signedIn"].is_boolean();
    let signed_in = raw["signedIn"]
        .as_bool()
        .unwrap_or_else(|| prev_signed_in.unwrap_or(false));
    let err_str = clean_text(&raw["error"]);
    Some(PlayerPageState {
        available: raw["available"].as_bool() == Some(true),
        playing: raw["playing"].as_bool() == Some(true),
        title: clean_text(&raw["title"]),
        artist: clean_text(&raw["artist"]),
        album: clean_text(&raw["album"]),
        art,
        position: clean_seconds(&raw["position"]),
        duration: clean_seconds(&raw["duration"]),
        can_next: raw["canNext"].as_bool() == Some(true),
        can_previous: raw["canPrevious"].as_bool() == Some(true),
        can_seek: raw["canSeek"].as_bool() == Some(true),
        video_id,
        signed_in,
        auth_known,
        queue,
        error: if err_str.is_empty() {
            None
        } else {
            Some(err_str)
        },
        playlist_id: sanitize_playlist_id(raw["playlistId"].as_str().unwrap_or("")),
    })
}

pub fn parse_artwork_url(url: &str) -> Result<(String, String), String> {
    if url.is_empty() || url.len() > 8192 || url.bytes().any(|b| b <= 32 || b == 127) {
        return Err("Invalid artwork URL".into());
    }
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| "Artwork URL must use HTTPS".to_string())?;
    let split_idx = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..split_idx];
    if authority.is_empty() || authority.contains('@') {
        return Err("Invalid artwork authority".into());
    }
    let host_raw = authority.strip_suffix(":443").unwrap_or(authority);
    if host_raw.is_empty()
        || host_raw.contains(':')
        || host_raw.starts_with(['.', '-'])
        || host_raw.ends_with(['.', '-'])
        || !host_raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    {
        return Err("Invalid artwork host".into());
    }
    let host = host_raw.to_ascii_lowercase();
    let allowed = ["ytimg.com", "googleusercontent.com", "ggpht.com"]
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")));
    if !allowed {
        return Err("Artwork host is not allowlisted".into());
    }
    let suffix = &rest[split_idx..];
    let without_fragment = suffix.split('#').next().unwrap_or("");
    let path = if without_fragment.is_empty() {
        "/".to_owned()
    } else if without_fragment.starts_with('/') {
        without_fragment.to_owned()
    } else {
        format!("/{without_fragment}")
    };
    Ok((host, path))
}

pub fn art_filename(url: &str) -> String {
    format!("{:x}.img", Sha256::digest(url.as_bytes()))
}

fn is_valid_art_filename(name: &str) -> bool {
    name.len() == 68
        && name.ends_with(".img")
        && name.as_bytes()[..64].iter().all(u8::is_ascii_hexdigit)
}

pub fn prune_art_cache(cache_dir: &Path, max_files: usize) -> Vec<String> {
    let Ok(entries) = fs::read_dir(cache_dir) else {
        return Vec::new();
    };
    let mut files: Vec<_> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !is_valid_art_filename(&name) {
                return None;
            }
            let meta = entry.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            let modified = meta.modified().ok()?;
            Some((modified, name, entry.path()))
        })
        .collect();
    if files.len() <= max_files {
        return Vec::new();
    }
    files.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    let mut removed = Vec::new();
    for (_, name, path) in files.into_iter().skip(max_files) {
        if fs::remove_file(path).is_ok() {
            removed.push(name);
        }
    }
    removed
}

struct Internet(*mut std::ffi::c_void);
impl Drop for Internet {
    fn drop(&mut self) {
        unsafe {
            WinHttpCloseHandle(self.0);
        }
    }
}

fn winhttp_handle(p: *mut std::ffi::c_void) -> Result<Internet, String> {
    if p.is_null() {
        Err(format!("WinHttp error: {}", std::io::Error::last_os_error()))
    } else {
        Ok(Internet(p))
    }
}

fn fetch_artwork_bytes(host: &str, path: &str) -> Result<Vec<u8>, String> {
    unsafe {
        let session = winhttp_handle(WinHttpOpen(
            crate::model::wide("Lumen/0.1 MusicArt").as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            null(),
            null(),
            0,
        ))?;
        WinHttpSetTimeouts(
            session.0,
            ART_TIMEOUT_MS,
            ART_TIMEOUT_MS,
            ART_TIMEOUT_MS,
            ART_TIMEOUT_MS,
        );
        let connection = winhttp_handle(WinHttpConnect(
            session.0,
            crate::model::wide(host).as_ptr(),
            443,
            0,
        ))?;
        let request = winhttp_handle(WinHttpOpenRequest(
            connection.0,
            crate::model::wide("GET").as_ptr(),
            crate::model::wide(path).as_ptr(),
            null(),
            null(),
            null(),
            WINHTTP_FLAG_SECURE,
        ))?;
        let policy: u32 = WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP;
        if WinHttpSetOption(
            request.0,
            WINHTTP_OPTION_REDIRECT_POLICY,
            &policy as *const _ as _,
            4,
        ) == 0
        {
            return Err("Cannot enforce artwork redirect policy".into());
        }
        let headers = crate::model::wide("Accept: image/jpeg,image/png\r\n");
        if WinHttpSendRequest(request.0, headers.as_ptr(), u32::MAX, null(), 0, 0, 0) == 0
            || WinHttpReceiveResponse(request.0, null_mut()) == 0
        {
            return Err("Artwork request failed".into());
        }
        let mut final_url_buf = [0u16; 2048];
        let mut final_url_bytes = (final_url_buf.len() * 2) as u32;
        if WinHttpQueryOption(
            request.0,
            WINHTTP_OPTION_URL,
            final_url_buf.as_mut_ptr() as _,
            &mut final_url_bytes,
        ) != 0
        {
            let final_url = String::from_utf16_lossy(&final_url_buf[..(final_url_bytes as usize / 2)])
                .trim_end_matches('\0')
                .to_owned();
            parse_artwork_url(&final_url)?;
        }
        let mut status = 0u32;
        let mut status_len = 4u32;
        if WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            &mut status as *mut _ as _,
            &mut status_len,
            null_mut(),
        ) == 0
            || status != 200
        {
            return Err(format!("Artwork HTTP status {status} rejected"));
        }
        let mut ctype_buf = [0u16; 256];
        let mut ctype_bytes = (ctype_buf.len() * 2) as u32;
        if WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_CONTENT_TYPE,
            null(),
            ctype_buf.as_mut_ptr() as _,
            &mut ctype_bytes,
            null_mut(),
        ) == 0
        {
            return Err("Missing artwork content type".into());
        }
        let ctype = String::from_utf16_lossy(&ctype_buf[..(ctype_bytes as usize / 2)])
            .trim_end_matches('\0')
            .to_ascii_lowercase();
        if !ctype.starts_with("image/") {
            return Err("Artwork response is not an image".into());
        }
        let mut body = Vec::new();
        let mut chunk = [0u8; 32768];
        loop {
            let mut read = 0u32;
            if WinHttpReadData(
                request.0,
                chunk.as_mut_ptr() as _,
                chunk.len() as u32,
                &mut read,
            ) == 0
            {
                return Err("Artwork read failed".into());
            }
            if read == 0 {
                break;
            }
            if body.len() + read as usize > MAX_ART_BYTES {
                return Err("Artwork exceeds 4MB limit".into());
            }
            body.extend_from_slice(&chunk[..read as usize]);
        }
        Ok(body)
    }
}

pub fn download_artwork(url: &str, cache_dir: &Path) -> Result<(String, Vec<String>), String> {
    let normalized = normalize_art_url(url);
    let (host, path) = parse_artwork_url(&normalized)?;
    let name = art_filename(&normalized);
    let dest = cache_dir.join(&name);
    if dest.is_file() {
        if let Ok(f) = fs::OpenOptions::new().write(true).open(&dest) {
            let _ = f.set_modified(SystemTime::now());
        }
        return Ok((name, Vec::new()));
    }
    let bytes = fetch_artwork_bytes(&host, &path)?;
    fs::create_dir_all(cache_dir).map_err(|e| e.to_string())?;
    let tmp = cache_dir.join(format!("{name}.tmp"));
    fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    let moved = unsafe {
        MoveFileExW(
            crate::model::wide(&tmp).as_ptr(),
            crate::model::wide(&dest).as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        let err = std::io::Error::last_os_error().to_string();
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    let pruned = prune_art_cache(cache_dir, MAX_ART_FILES);
    Ok((name, pruned))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn artwork_url_allowlist_enforces_https_and_trusted_domains() {
        assert_eq!(
            parse_artwork_url("https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg").unwrap(),
            (
                "i.ytimg.com".into(),
                "/vi/dQw4w9WgXcQ/hqdefault.jpg".into()
            )
        );
        assert!(parse_artwork_url("https://lh3.googleusercontent.com/abc=w226-h226-l90-rj").is_ok());
        assert!(parse_artwork_url("https://yt3.ggpht.com/abc").is_ok());
        assert!(parse_artwork_url("https://ytimg.com/img.png").is_ok());

        for bad in [
            "",
            "http://i.ytimg.com/a.jpg",
            "https://evil.com/a.jpg",
            "https://ytimg.com.evil.com/a.jpg",
            "https://evil.com?i.ytimg.com/a.jpg",
            "https://user@i.ytimg.com/a.jpg",
            "https://i.ytimg.com:8443/a.jpg",
            "https://i.ytimg.com/a b.jpg",
            "https://i.ytimg.com/a\n.jpg",
        ] {
            assert!(parse_artwork_url(bad).is_err(), "should reject {bad}");
        }
    }

    #[test]
    fn input_sanitization_validates_video_ids_actions_and_state() {
        assert!(validate_video_id("dQw4w9WgXcQ").is_ok());
        assert!(validate_video_id("a_b-C123456").is_ok());
        for bad in ["", "short", "toolong12345", "bad;alert(1)", "../12345678"] {
            assert!(validate_video_id(bad).is_err());
            assert!(music_page_select(bad).is_err());
        }
        let select_js = music_page_select("dQw4w9WgXcQ").unwrap();
        assert!(select_js.contains("\"dQw4w9WgXcQ\""));

        assert!(music_page_command("eval", None).is_err());
        assert!(music_page_command("play", None).is_ok());
        let seek_nan = music_page_command("seek", Some(f64::NAN)).unwrap();
        assert!(seek_nan.contains("value=null"));

        let long_title = "x".repeat(1500);
        let raw = json!({
            "available": true,
            "playing": true,
            "title": long_title,
            "artist": "Artist",
            "art": "http://insecure.example/a.jpg",
            "position": -5.0,
            "duration": f64::NAN,
            "videoId": "bad-id",
            "playlistId": "PL123;drop",
            "signedIn": null,
            "queue": [
                {"videoId": "dQw4w9WgXcQ", "title": "Valid", "artist": "A", "thumb": "https://i.ytimg.com/vi/dQw4w9WgXcQ/0.jpg", "selected": true},
                {"videoId": "invalid", "title": "Bad"}
            ]
        });
        let state = sanitize_page_state(&raw, Some(true)).unwrap();
        assert_eq!(state.title.chars().count(), 1000);
        assert_eq!(state.art, None);
        assert_eq!(state.position, 0.0);
        assert_eq!(state.duration, 0.0);
        assert_eq!(state.video_id, "");
        assert_eq!(state.playlist_id, "PL123drop");
        assert!(state.signed_in);
        assert!(!state.auth_known);
        assert_eq!(state.queue.len(), 1);
        assert_eq!(state.queue[0].video_id, "dQw4w9WgXcQ");
    }

    #[test]
    fn normalize_art_url_rewrites_rw_tokens_to_rj_on_google_hosts() {
        assert_eq!(
            normalize_art_url("https://lh3.googleusercontent.com/abc=w544-h544-l90-rw"),
            "https://lh3.googleusercontent.com/abc=w544-h544-l90-rj"
        );
        assert_eq!(
            normalize_art_url("https://lh3.googleusercontent.com/abc=w226-h226-rw-l90?sqp=1"),
            "https://lh3.googleusercontent.com/abc=w226-h226-rj-l90?sqp=1"
        );
        assert_eq!(
            normalize_art_url("https://lh3.googleusercontent.com/id-rw-1=rw"),
            "https://lh3.googleusercontent.com/id-rw-1=rj"
        );
        assert_eq!(
            normalize_art_url("https://yt3.ggpht.com/avatar=s240-c-k-c0x00ffffff-no-rw"),
            "https://yt3.ggpht.com/avatar=s240-c-k-c0x00ffffff-no-rj"
        );
        assert_eq!(
            normalize_art_url("https://googleusercontent.com/img=s544-rw#frag"),
            "https://googleusercontent.com/img=s544-rj#frag"
        );
        assert_eq!(
            normalize_art_url("https://lh3.googleusercontent.com/abc=w226-h226-l90-rj"),
            "https://lh3.googleusercontent.com/abc=w226-h226-l90-rj"
        );
        assert_eq!(
            normalize_art_url("https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg?x=w100-rw"),
            "https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg?x=w100-rw"
        );
    }

    #[test]
    fn auth_session_cookies_detection() {
        assert!(!has_auth_session_cookies(&json!({"cookies": []})));
        assert!(!has_auth_session_cookies(&json!({
            "cookies": [{"name": "SAPISID", "value": ""}, {"name": "VISITOR_INFO1_LIVE", "value": "abc"}]
        })));
        assert!(has_auth_session_cookies(&json!({
            "cookies": [{"name": "SAPISID", "value": "token123"}]
        })));
        assert!(has_auth_session_cookies(&json!({
            "cookies": [{"name": "__Secure-3PAPISID", "value": "token456"}]
        })));
        assert!(has_auth_session_cookies(&json!({
            "cookies": [{"name": "LOGIN_INFO", "value": "AFmmF2swRQIh"}]
        })));
    }

    #[test]
    fn sanitize_page_state_rejects_origin_url_and_falls_back_to_ytimg() {
        let raw = json!({
            "available": true,
            "playing": true,
            "title": "Video Mode Track",
            "artist": "Artist",
            "art": "https://music.youtube.com/",
            "videoId": "dQw4w9WgXcQ",
            "queue": [
                {
                    "videoId": "dQw4w9WgXcQ",
                    "title": "Video Mode Track",
                    "artist": "Artist",
                    "thumb": "https://music.youtube.com/",
                    "selected": true
                }
            ]
        });
        let state = sanitize_page_state(&raw, Some(true)).unwrap();
        assert_eq!(
            state.art.as_deref(),
            Some("https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg")
        );
        assert_eq!(state.queue[0].thumb, "");
    }

    #[test]
    fn page_scripts_include_movie_player_and_image_src_guards() {
        assert!(MUSIC_PAGE_STATE.contains("#movie_player video.html5-main-video"));
        assert!(MUSIC_PAGE_STATE.contains("imgSrc"));
        assert!(MUSIC_PAGE_STATE.contains("getPlayerState"));
        assert!(MUSIC_PAGE_STATE.contains("getVideoData"));
        let cmd = music_page_command("toggle", None).unwrap();
        assert!(cmd.contains("#movie_player video.html5-main-video"));
        assert!(cmd.contains("getPlayerState"));
    }
}
