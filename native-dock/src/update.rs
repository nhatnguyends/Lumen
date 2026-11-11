//! Native portable updates. Network/ZIP work runs off the UI thread; no shell scripts.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{Cursor, Read},
    os::windows::{fs::MetadataExt, process::CommandExt},
    path::{Path, PathBuf},
    process::Command,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Networking::WinHttp::*,
    System::Threading::*,
    UI::{Shell::ShellExecuteW, WindowsAndMessaging::*},
};
const API: &str = "https://api.github.com/repos/nhatnguyends/Lumen/releases/latest";
pub const RELEASES: &str = "https://github.com/nhatnguyends/Lumen/releases";
const MAX_DOWNLOAD: usize = 256 * 1024 * 1024;
const MAX_EXPANDED: u64 = 768 * 1024 * 1024;
const MARKER: &str = "lumen-portable.json";
const TARGET: &str = "windows-x86_64";
pub type Result<T> = std::result::Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn file_hash(path: &Path) -> Result<String> {
    Ok(hash(&fs::read(path).map_err(err)?))
}
fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    crate::model::wide(s)
}
pub fn portable(root: &Path) -> bool {
    root.join(MARKER).is_file()
}
fn version(s: &str) -> Result<[u64; 3]> {
    let parts: Vec<_> = s.strip_prefix('v').unwrap_or(s).split('.').collect();
    if parts.len() != 3 {
        return Err("Expected a stable major.minor.patch version".into());
    }
    let mut out = [0; 3];
    for (i, p) in parts.iter().enumerate() {
        if p.is_empty()
            || !p.bytes().all(|c| c.is_ascii_digit())
            || (p.len() > 1 && p.starts_with('0'))
        {
            return Err("Invalid version".into());
        }
        out[i] = p.parse().map_err(err)?;
    }
    Ok(out)
}
pub fn installed_version(root: &Path) -> String {
    read_manifest(&root.join(MARKER))
        .map(|m| m.version)
        .unwrap_or_else(|_| env!("CARGO_PKG_VERSION").into())
}
#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub url: String,
    pub digest: String,
}
struct Internet(*mut std::ffi::c_void);
impl Drop for Internet {
    fn drop(&mut self) {
        unsafe {
            WinHttpCloseHandle(self.0);
        }
    }
}
fn handle(p: *mut std::ffi::c_void) -> Result<Internet> {
    if p.is_null() {
        Err(format!(
            "Windows network error: {}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(Internet(p))
    }
}
fn url_parts(url: &str) -> Result<(&str, &str)> {
    let raw = url
        .strip_prefix("https://")
        .ok_or("Only HTTPS downloads are supported")?;
    let (host, path) = raw.split_once('/').ok_or("Missing URL path")?;
    if ![
        "api.github.com",
        "github.com",
        "release-assets.githubusercontent.com",
        "objects.githubusercontent.com",
    ]
    .contains(&host)
        || url.bytes().any(|c| c <= 32 || c == 127)
    {
        return Err("Unexpected update download origin".into());
    }
    Ok((host, path))
}
fn get(url: &str, limit: usize) -> Result<(u32, Vec<u8>)> {
    let mut location = url.to_string();
    for _ in 0..5 {
        let (host, path) = url_parts(&location)?;
        unsafe {
            let session = handle(WinHttpOpen(
                wide("Lumen/0.1 PortableUpdater").as_ptr(),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                null(),
                null(),
                0,
            ))?;
            WinHttpSetTimeouts(session.0, 10000, 10000, 15000, 30000);
            let connection = handle(WinHttpConnect(session.0, wide(host).as_ptr(), 443, 0))?;
            let request = handle(WinHttpOpenRequest(
                connection.0,
                wide("GET").as_ptr(),
                wide(format!("/{path}")).as_ptr(),
                null(),
                null(),
                null(),
                WINHTTP_FLAG_SECURE,
            ))?;
            let never: u32 = WINHTTP_OPTION_REDIRECT_POLICY_NEVER;
            if WinHttpSetOption(
                request.0,
                WINHTTP_OPTION_REDIRECT_POLICY,
                &never as *const _ as _,
                4,
            ) == 0
            {
                return Err("Cannot enforce HTTPS redirect policy".into());
            }
            let headers =
                wide("Accept: application/vnd.github+json\r\nX-GitHub-Api-Version: 2022-11-28\r\n");
            if WinHttpSendRequest(request.0, headers.as_ptr(), u32::MAX, null(), 0, 0, 0) == 0
                || WinHttpReceiveResponse(request.0, null_mut()) == 0
            {
                return Err(format!(
                    "Cannot reach GitHub: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let mut status = 0u32;
            let mut len = 4;
            if WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                null(),
                &mut status as *mut _ as _,
                &mut len,
                null_mut(),
            ) == 0
            {
                return Err("Missing HTTP status".into());
            }
            if [301, 302, 303, 307, 308].contains(&status) {
                let mut buffer = vec![0u16; 8192];
                let mut size = (buffer.len() * 2) as u32;
                if WinHttpQueryHeaders(
                    request.0,
                    WINHTTP_QUERY_LOCATION,
                    null(),
                    buffer.as_mut_ptr() as _,
                    &mut size,
                    null_mut(),
                ) == 0
                {
                    return Err("Missing redirect location".into());
                }
                location = String::from_utf16_lossy(&buffer[..size as usize / 2])
                    .trim_end_matches('\0')
                    .into();
                continue;
            }
            if status != 200 {
                return Ok((status, Vec::new()));
            }
            let mut body = Vec::new();
            let mut chunk = [0u8; 65536];
            loop {
                let mut read = 0;
                if WinHttpReadData(
                    request.0,
                    chunk.as_mut_ptr() as _,
                    chunk.len() as u32,
                    &mut read,
                ) == 0
                {
                    return Err("Download interrupted".into());
                }
                if read == 0 {
                    break;
                }
                if body.len() + read as usize > limit {
                    return Err("Update exceeds download limit".into());
                }
                body.extend_from_slice(&chunk[..read as usize]);
            }
            return Ok((status, body));
        }
    }
    Err("Too many download redirects".into())
}
pub fn check(root: &Path) -> Result<Option<Release>> {
    let (status, body) = get(API, 2 * 1024 * 1024)?;
    if status == 404 {
        return Ok(None);
    }
    if status != 200 {
        return Err(format!("GitHub returned HTTP {status}; try again later"));
    }
    parse_release(&body, &installed_version(root))
}
fn parse_release(bytes: &[u8], current: &str) -> Result<Option<Release>> {
    let v: serde_json::Value = serde_json::from_slice(bytes).map_err(err)?;
    if v["draft"].as_bool() != Some(false) || v["prerelease"].as_bool() != Some(false) {
        return Ok(None);
    }
    let tag = v["tag_name"].as_str().ok_or("Missing release version")?;
    if version(tag)? <= version(current)? {
        return Ok(None);
    }
    let ver = tag.trim_start_matches('v');
    let filename = format!("Lumen-{ver}-{TARGET}.zip");
    let asset = v["assets"]
        .as_array()
        .and_then(|a| a.iter().find(|a| a["name"] == filename))
        .ok_or("A newer release exists but has no compatible Windows x64 portable package")?;
    let url = asset["browser_download_url"]
        .as_str()
        .ok_or("Missing asset URL")?;
    if !url.starts_with(&format!(
        "https://github.com/nhatnguyends/Lumen/releases/download/{tag}/"
    )) {
        return Err("Release asset belongs to an unexpected repository".into());
    }
    let digest = asset["digest"]
        .as_str()
        .and_then(|s| s.strip_prefix("sha256:"))
        .ok_or("Release asset is missing its SHA-256 digest")?;
    if digest.len() != 64
        || !digest.bytes().all(|c| c.is_ascii_hexdigit())
        || asset["size"].as_u64().unwrap_or(u64::MAX) > MAX_DOWNLOAD as u64
    {
        return Err("Invalid release asset metadata".into());
    }
    Ok(Some(Release {
        version: ver.into(),
        url: url.into(),
        digest: digest.to_lowercase(),
    }))
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema_version: u32,
    pub version: String,
    pub target: String,
    pub files: BTreeMap<String, String>,
}
fn read_manifest(path: &Path) -> Result<Manifest> {
    let bytes = fs::read(path).map_err(err)?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("Oversized portable manifest".into());
    }
    let m: Manifest = serde_json::from_slice(&bytes).map_err(err)?;
    validate_manifest(&m)?;
    Ok(m)
}
fn validate_manifest(m: &Manifest) -> Result<()> {
    version(&m.version)?;
    if m.schema_version != 1
        || m.target != TARGET
        || m.files.len() > 10000
        || !m.files.contains_key("Lumen.exe")
    {
        return Err("Incompatible portable package".into());
    }
    let mut seen = HashSet::new();
    for (name, digest) in &m.files {
        safe_path(name)?;
        if name == MARKER
            || !seen.insert(name.to_lowercase())
            || digest.len() != 64
            || !digest.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("Invalid portable file index".into());
        }
    }
    for required in ["resources/tray.png", "widgets/launcher/widget.json"] {
        if !m.files.contains_key(required) {
            return Err("Portable package is incomplete".into());
        }
    }
    Ok(())
}
fn safe_path(name: &str) -> Result<PathBuf> {
    if name.is_empty() || name.contains('\\') || name.len() > 220 {
        return Err("Unsafe archive path".into());
    }
    for part in name.split('/') {
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with(['.', ' '])
            || part.chars().any(|c| c < ' ' || "<>:\"|?*".contains(c))
            || [
                "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
                "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8",
                "LPT9",
            ]
            .contains(&stem.as_str())
        {
            return Err("Unsafe archive path".into());
        }
    }
    let first = name.split('/').next().unwrap();
    if ![
        "Lumen.exe",
        MARKER,
        "README.md",
        "SECURITY.md",
        "LICENSE.txt",
        "resources",
        "widgets",
        "runtime",
        "LICENSES",
    ]
    .contains(&first)
    {
        return Err("Package tries to replace files outside the portable app".into());
    }
    Ok(PathBuf::from(name))
}
fn no_links(path: &Path) -> Result<()> {
    for p in path.ancestors() {
        if let Ok(m) = fs::symlink_metadata(p) {
            if m.file_attributes() & 0x400 != 0 {
                return Err("Update paths must not contain links or junctions".into());
            }
        }
    }
    Ok(())
}
fn json_write(path: &Path, value: &impl Serialize) -> Result<()> {
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(value).map_err(err)?).map_err(err)?;
    unsafe {
        use windows_sys::Win32::Storage::FileSystem::*;
        if MoveFileExW(
            wide(&temporary).as_ptr(),
            wide(path).as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        ) == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
    }
    Ok(())
}
fn unpack(bytes: &[u8], stage: &Path, expected: &str) -> Result<Manifest> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(err)?;
    if zip.len() > 10001 {
        return Err("Too many archive entries".into());
    }
    let mut seen = HashSet::new();
    let mut total = 0;
    // Validate the complete directory before writing anything.
    for i in 0..zip.len() {
        let f = zip.by_index(i).map_err(err)?;
        safe_path(f.name())?;
        if f.is_dir()
            || f.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
            || !seen.insert(f.name().to_lowercase())
        {
            return Err("Duplicate, directory or link entry in archive".into());
        }
        total += f.size();
        if total > MAX_EXPANDED {
            return Err("Archive exceeds expanded size limit".into());
        }
    }
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(err)?;
        let target = stage.join(safe_path(f.name())?);
        no_links(&target)?;
        fs::create_dir_all(target.parent().unwrap()).map_err(err)?;
        let mut out = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)
            .map_err(err)?;
        std::io::copy(&mut f.by_ref().take(MAX_EXPANDED + 1), &mut out).map_err(err)?;
    }
    let m = read_manifest(&stage.join(MARKER))?;
    if m.version != expected || m.files.len() + 1 != seen.len() {
        return Err("Release version/file index mismatch".into());
    }
    for (name, digest) in &m.files {
        if file_hash(&stage.join(name))? != *digest {
            return Err(format!("Checksum mismatch: {name}"));
        }
    }
    Ok(m)
}
#[derive(Serialize, Deserialize)]
struct Pending {
    manifest: Manifest,
    baseline: BTreeMap<String, Option<String>>,
}
fn updates(root: &Path) -> PathBuf {
    root.join("config/updates")
}
pub fn stage(root: &Path, release: &Release) -> Result<()> {
    if !portable(root) {
        return Err("Source builds cannot overwrite themselves with a portable update. Download the portable edition from GitHub Releases.".into());
    }
    let existing = read_manifest(&root.join(MARKER))?;
    let base = updates(root);
    no_links(&base)?;
    fs::create_dir_all(&base).map_err(err)?;
    if base.join("pending.json").exists() || base.join("journal.json").exists() {
        return Err("An update is already pending; restart Lumen first".into());
    }
    let staging = base.join("staged");
    if staging.exists() {
        return Err(
            "Old staging directory exists; close Lumen and inspect config/updates before retrying"
                .into(),
        );
    }
    let (status, body) = get(&release.url, MAX_DOWNLOAD)?;
    if status != 200 {
        return Err(format!("Download returned HTTP {status}"));
    }
    if hash(&body) != release.digest {
        return Err("Downloaded archive SHA-256 does not match GitHub release metadata".into());
    }
    fs::create_dir(&staging).map_err(err)?;
    let result = (|| {
        let m = unpack(&body, &staging, &release.version)?;
        let mut names: HashSet<_> = m
            .files
            .keys()
            .chain(existing.files.keys())
            .cloned()
            .collect();
        names.insert(MARKER.into());
        let mut baseline = BTreeMap::new();
        for name in names {
            let target = root.join(safe_path(&name)?);
            no_links(&target)?;
            let current = if target.exists() {
                Some(file_hash(&target)?)
            } else {
                None
            };
            if name != MARKER {
                if let Some(expected) = existing.files.get(&name) {
                    if current.as_ref() != Some(expected) {
                        return Err(format!("Local app file was changed: {name}; keep this copy and install the new ZIP separately"));
                    }
                } else if current.is_some() {
                    return Err(format!("Update would overwrite an unowned file: {name}"));
                }
            }
            baseline.insert(name, current);
        }
        json_write(
            &base.join("pending.json"),
            &Pending {
                manifest: m,
                baseline,
            },
        )
    })();
    if result.is_err() {
        fs::remove_dir_all(&staging).map_err(err)?;
    }
    result
}
#[derive(Serialize, Deserialize)]
struct Journal {
    files: BTreeMap<String, bool>,
}
fn restore(root: &Path) -> Result<()> {
    let base = updates(root);
    let journal: Journal =
        serde_json::from_slice(&fs::read(base.join("journal.json")).map_err(err)?).map_err(err)?;
    for (name, existed) in journal.files.iter().rev() {
        let target = root.join(safe_path(name)?);
        no_links(&target)?;
        if *existed {
            let backup = base.join("backup").join(safe_path(name)?);
            no_links(&backup)?;
            fs::copy(backup, &target).map_err(err)?;
        } else if target.exists() {
            fs::remove_file(target).map_err(err)?;
        }
    }
    fs::remove_file(base.join("journal.json")).map_err(err)?;
    if base.join("pending.json").exists() {
        fs::remove_file(base.join("pending.json")).map_err(err)?;
    }
    Ok(())
}
pub fn apply(root: &Path) -> Result<()> {
    let base = updates(root);
    no_links(&base)?;
    if base.join("journal.json").exists() {
        return restore(root);
    }
    let p: Pending =
        serde_json::from_slice(&fs::read(base.join("pending.json")).map_err(err)?).map_err(err)?;
    validate_manifest(&p.manifest)?;
    if p.baseline.len() > 10001 {
        return Err("Invalid pending file count".into());
    }
    for (name, prior) in &p.baseline {
        let dest = root.join(safe_path(name)?);
        no_links(&dest)?;
        let current = if dest.exists() {
            Some(file_hash(&dest)?)
        } else {
            None
        };
        if &current != prior {
            return Err(format!("App file changed after download: {name}"));
        }
    }
    for name in p
        .manifest
        .files
        .keys()
        .chain(std::iter::once(&MARKER.to_string()))
    {
        if !p.baseline.contains_key(name) {
            return Err("Incomplete update transaction".into());
        }
    }
    for (name, digest) in &p.manifest.files {
        let path = base.join("staged").join(safe_path(name)?);
        no_links(&path)?;
        if file_hash(&path)? != *digest {
            return Err(format!("Staged file was changed: {name}"));
        }
    }
    let staged_manifest = read_manifest(&base.join("staged").join(MARKER))?;
    if serde_json::to_vec(&staged_manifest).map_err(err)?
        != serde_json::to_vec(&p.manifest).map_err(err)?
    {
        return Err("Staged manifest was changed".into());
    }
    let mut files = BTreeMap::new();
    for (name, prior) in &p.baseline {
        if prior.is_some() {
            let backup = base.join("backup").join(safe_path(name)?);
            no_links(&backup)?;
            fs::create_dir_all(backup.parent().unwrap()).map_err(err)?;
            fs::copy(root.join(name), backup).map_err(err)?;
        }
        files.insert(name.clone(), prior.is_some());
    }
    json_write(&base.join("journal.json"), &Journal { files })?;
    let result: Result<()> = (|| {
        for name in p.baseline.keys() {
            let target = root.join(name);
            if name == MARKER || p.manifest.files.contains_key(name) {
                fs::create_dir_all(target.parent().unwrap()).map_err(err)?;
                fs::copy(base.join("staged").join(name), target).map_err(err)?;
            } else if target.exists() {
                fs::remove_file(target).map_err(err)?;
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        return match restore(root) {
            Ok(()) => Err(format!("Update failed; previous version restored: {error}")),
            Err(rollback) => Err(format!(
                "Update failed: {error}. Recovery required: {rollback}"
            )),
        };
    }
    fs::remove_file(base.join("journal.json")).map_err(err)?;
    fs::remove_file(base.join("pending.json")).map_err(err)?;
    fs::remove_dir_all(base.join("staged")).map_err(err)?;
    Ok(())
}
pub fn pending(root: &Path) -> bool {
    updates(root).join("pending.json").is_file()
}
pub fn restart(root: &Path) -> Result<()> {
    if !portable(root) || (!pending(root) && !updates(root).join("journal.json").exists()) {
        return Err("No portable update is ready".into());
    }
    let helper = updates(root).join("Lumen-updater.exe");
    no_links(&helper)?;
    fs::copy(std::env::current_exe().map_err(err)?, &helper).map_err(err)?;
    Command::new(helper)
        .arg("--apply-update")
        .arg(root)
        .arg(std::process::id().to_string())
        .creation_flags(0x08000000)
        .spawn()
        .map_err(err)?;
    Ok(())
}
pub unsafe fn open_releases() {
    ShellExecuteW(
        null_mut(),
        wide("open").as_ptr(),
        wide(RELEASES).as_ptr(),
        null(),
        null(),
        SW_SHOWNORMAL,
    );
}
/// Executed before renderer/window initialization; helper validates its own location
/// and waits for exactly the parent Lumen image, never kills unrelated processes.
pub fn dispatch() -> bool {
    let args: Vec<_> = std::env::args_os().collect();
    if args.get(1).is_some_and(|s| s == "--apply-update") {
        let result = (|| {
            let root = PathBuf::from(args.get(2).ok_or("Missing update root")?)
                .canonicalize()
                .map_err(err)?;
            let helper = std::env::current_exe()
                .map_err(err)?
                .canonicalize()
                .map_err(err)?;
            if helper
                != updates(&root)
                    .join("Lumen-updater.exe")
                    .canonicalize()
                    .map_err(err)?
            {
                return Err("Unexpected updater location".into());
            }
            no_links(&root)?;
            let pid: u32 = args
                .get(3)
                .and_then(|s| s.to_str())
                .ok_or("Missing parent PID")?
                .parse()
                .map_err(err)?;
            unsafe {
                let parent = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | 0x00100000, 0, pid);
                if !parent.is_null() {
                    let mut name = vec![0u16; 32768];
                    let mut len = name.len() as u32;
                    let valid = QueryFullProcessImageNameW(parent, 0, name.as_mut_ptr(), &mut len)
                        != 0
                        && PathBuf::from(String::from_utf16_lossy(&name[..len as usize]))
                            .canonicalize()
                            .ok()
                            == root.join("Lumen.exe").canonicalize().ok();
                    if !valid {
                        CloseHandle(parent);
                        return Err("Update parent does not match Lumen.exe".into());
                    }
                    let waited = WaitForSingleObject(parent, 60000);
                    CloseHandle(parent);
                    if waited != WAIT_OBJECT_0 {
                        return Err(
                            "Lumen is still running; close it before applying the update".into(),
                        );
                    }
                }
            }
            let applied = apply(&root);
            if let Err(e) = &applied {
                fs::write(updates(&root).join("last-error.txt"), e).map_err(err)?;
                if !updates(&root).join("journal.json").exists() && pending(&root) {
                    fs::rename(
                        updates(&root).join("pending.json"),
                        updates(&root).join("failed-pending.json"),
                    )
                    .map_err(err)?;
                }
            }
            if !updates(&root).join("journal.json").exists() {
                Command::new(root.join("Lumen.exe"))
                    .current_dir(&root)
                    .spawn()
                    .map_err(err)?;
            }
            applied
        })();
        if let Err(error) = result {
            unsafe {
                MessageBoxW(
                    null_mut(),
                    wide(error).as_ptr(),
                    wide("Lumen update").as_ptr(),
                    MB_OK | MB_ICONERROR,
                );
            }
        }
        return true;
    }
    let root = crate::model::root();
    if portable(&root) && (pending(&root) || updates(&root).join("journal.json").exists()) {
        if let Err(e) = restart(&root) {
            unsafe {
                MessageBoxW(
                    null_mut(),
                    wide(e).as_ptr(),
                    wide("Lumen recovery").as_ptr(),
                    MB_OK | MB_ICONERROR,
                );
            }
        }
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "lumen-update-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn fixture(ver: &str) -> (Manifest, BTreeMap<String, Vec<u8>>) {
        let files: BTreeMap<String, Vec<u8>> = BTreeMap::from([
            ("Lumen.exe".into(), format!("binary-{ver}").into_bytes()),
            ("resources/tray.png".into(), b"image".to_vec()),
            ("widgets/launcher/widget.json".into(), b"{}".to_vec()),
        ]);
        (
            Manifest {
                schema_version: 1,
                version: ver.into(),
                target: TARGET.into(),
                files: files.iter().map(|(k, v)| (k.clone(), hash(v))).collect(),
            },
            files,
        )
    }
    fn write_fixture(root: &Path, m: &Manifest, files: &BTreeMap<String, Vec<u8>>) {
        for (name, data) in files {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, data).unwrap();
        }
        json_write(&root.join(MARKER), m).unwrap();
    }
    fn archive(m: &Manifest, files: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
        let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, data) in files {
            out.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            out.write_all(data).unwrap();
        }
        out.start_file(MARKER, zip::write::SimpleFileOptions::default())
            .unwrap();
        out.write_all(&serde_json::to_vec(m).unwrap()).unwrap();
        out.finish().unwrap().into_inner()
    }
    fn prepare(root: &Path) -> Pending {
        let (old, files) = fixture("0.1.0");
        write_fixture(root, &old, &files);
        let (new, files) = fixture("0.2.0");
        let stage = updates(root).join("staged");
        fs::create_dir_all(&stage).unwrap();
        write_fixture(&stage, &new, &files);
        let mut baseline: BTreeMap<_, _> = old
            .files
            .keys()
            .map(|n| (n.clone(), Some(file_hash(&root.join(n)).unwrap())))
            .collect();
        baseline.insert(MARKER.into(), Some(file_hash(&root.join(MARKER)).unwrap()));
        let p = Pending {
            manifest: new,
            baseline,
        };
        json_write(&updates(root).join("pending.json"), &p).unwrap();
        p
    }
    #[test]
    fn versions_origins_and_windows_paths_are_strict() {
        assert!(version("v1.10.0").unwrap() > version("1.9.9").unwrap());
        for bad in ["1.0", "1.0.0-beta", "01.0.0", "../1.2.3"] {
            assert!(version(bad).is_err());
        }
        for bad in [
            "http://github.com/a",
            "https://github.com.evil.com/a",
            "https://user@github.com/a",
            "https://github.com:443/a",
            "https://github.com/a\r\nHeader:x",
        ] {
            assert!(url_parts(bad).is_err());
        }
        for bad in [
            "../escape",
            "config/notes.json",
            "runtime/../../escape",
            "Lumen.exe:stream",
            "runtime/CON.txt",
            "runtime/foo.",
            "runtime\\evil",
            "/Lumen.exe",
        ] {
            assert!(safe_path(bad).is_err(), "{bad}");
        }
    }
    #[test]
    fn release_requires_the_official_compatible_asset_and_digest() {
        let mut json = serde_json::json!({"tag_name":"v0.2.0","draft":false,"prerelease":false,"assets":[{"name":"Lumen-0.2.0-windows-x86_64.zip","browser_download_url":"https://github.com/nhatnguyends/Lumen/releases/download/v0.2.0/Lumen-0.2.0-windows-x86_64.zip","size":100,"digest":format!("sha256:{}",hash(b"zip"))}]});
        assert!(parse_release(&serde_json::to_vec(&json).unwrap(), "0.1.0")
            .unwrap()
            .is_some());
        assert!(parse_release(&serde_json::to_vec(&json).unwrap(), "0.2.0")
            .unwrap()
            .is_none());
        json["assets"][0]["digest"] = serde_json::Value::Null;
        assert!(parse_release(&serde_json::to_vec(&json).unwrap(), "0.1.0").is_err());
    }
    #[test]
    fn archive_validation_prevents_traversal_extra_files_and_tampering() {
        let temp = Temp::new();
        let (m, mut files) = fixture("0.2.0");
        let stage = temp.0.join("valid");
        fs::create_dir(&stage).unwrap();
        assert!(unpack(&archive(&m, &files), &stage, "0.2.0").is_ok());
        files.insert("../escape".into(), b"evil".to_vec());
        let stage = temp.0.join("invalid");
        fs::create_dir(&stage).unwrap();
        assert!(unpack(&archive(&m, &files), &stage, "0.2.0").is_err());
        assert!(fs::read_dir(stage).unwrap().next().is_none());
        files.remove("../escape");
        files.insert("Lumen.exe".into(), b"tampered".to_vec());
        let stage = temp.0.join("tampered");
        fs::create_dir(&stage).unwrap();
        assert!(unpack(&archive(&m, &files), &stage, "0.2.0").is_err());
    }
    #[test]
    fn apply_preserves_user_data_and_backup_and_rejects_local_changes() {
        let temp = Temp::new();
        prepare(&temp.0);
        let user = temp.0.join("config/notes.json");
        fs::write(&user, "private notes").unwrap();
        apply(&temp.0).unwrap();
        assert_eq!(installed_version(&temp.0), "0.2.0");
        assert_eq!(fs::read_to_string(user).unwrap(), "private notes");
        assert_eq!(
            fs::read(updates(&temp.0).join("backup/Lumen.exe")).unwrap(),
            b"binary-0.1.0"
        );
        assert!(!pending(&temp.0));
        let changed = Temp::new();
        prepare(&changed.0);
        fs::write(changed.0.join("Lumen.exe"), "local edits").unwrap();
        assert!(apply(&changed.0).is_err());
        assert_eq!(
            fs::read_to_string(changed.0.join("Lumen.exe")).unwrap(),
            "local edits"
        );
        assert!(!updates(&changed.0).join("journal.json").exists());
    }
    #[test]
    fn incomplete_copy_rolls_back_and_interrupted_journal_recovers() {
        let temp = Temp::new();
        let mut p = prepare(&temp.0);
        // Parent file blocks a later package copy, after Lumen.exe was replaced.
        fs::write(temp.0.join("runtime"), "unowned user file").unwrap();
        p.manifest
            .files
            .insert("runtime/new.txt".into(), hash(b"new"));
        p.baseline.insert("runtime/new.txt".into(), None);
        fs::create_dir_all(updates(&temp.0).join("staged/runtime")).unwrap();
        fs::write(updates(&temp.0).join("staged/runtime/new.txt"), b"new").unwrap();
        json_write(&updates(&temp.0).join("staged").join(MARKER), &p.manifest).unwrap();
        json_write(&updates(&temp.0).join("pending.json"), &p).unwrap();
        assert!(apply(&temp.0)
            .unwrap_err()
            .contains("previous version restored"));
        assert_eq!(installed_version(&temp.0), "0.1.0");
        assert_eq!(
            fs::read_to_string(temp.0.join("runtime")).unwrap(),
            "unowned user file"
        );
        let crash = Temp::new();
        let p = prepare(&crash.0);
        let mut files = BTreeMap::new();
        for (name, prior) in &p.baseline {
            let target = updates(&crash.0).join("backup").join(name);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(crash.0.join(name), target).unwrap();
            files.insert(name.clone(), prior.is_some());
        }
        json_write(&updates(&crash.0).join("journal.json"), &Journal { files }).unwrap();
        fs::write(crash.0.join("Lumen.exe"), "half installed").unwrap();
        apply(&crash.0).unwrap();
        assert_eq!(
            fs::read(crash.0.join("Lumen.exe")).unwrap(),
            b"binary-0.1.0"
        );
        assert!(!pending(&crash.0));
    }
    #[test]
    #[ignore = "Read-only diagnostic of the official public GitHub release feed"]
    fn live_release_feed() {
        let temp = Temp::new();
        let result = check(&temp.0);
        assert!(result.is_ok(), "{result:?}");
    }
    #[test]
    #[ignore = "Downloads the official portable release into a private fixture; never updates the user's app"]
    fn live_portable_stage_and_apply() {
        let temp=Temp::new();let (old,files)=fixture("0.0.0");write_fixture(&temp.0,&old,&files);
        fs::create_dir_all(temp.0.join("config")).unwrap();fs::write(temp.0.join("config/notes.json"),"preserve fixture notes").unwrap();
        let release=check(&temp.0).unwrap().expect("A stable official portable release must exist for this opt-in integration test");
        stage(&temp.0,&release).unwrap();assert!(pending(&temp.0));
        apply(&temp.0).unwrap();assert_eq!(installed_version(&temp.0),release.version);
        assert_eq!(fs::read_to_string(temp.0.join("config/notes.json")).unwrap(),"preserve fixture notes");
        assert!(!pending(&temp.0));assert!(updates(&temp.0).join("backup/Lumen.exe").is_file());
    }
}
