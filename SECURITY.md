# Native Rust development and portable distribution

Lumen runs its widget UI in a single Rust/Win32 host, built locally under
`native-dock/target/debug/`. There is no Electron runtime, renderer, installer or
Electron packaging. Use the Python source launcher for development. The user
authorized native portable ZIP distribution; it includes the Rust executable,
assets and optional Music Node runtime, without an installer.

The safety guard rejects PowerShell helpers, synthetic input, capture/input-hook
APIs and local account enumeration in native packages. Microphone controls only
read endpoint settings and change defaults/volume/mute; they do not record audio.
The primary display cannot be detached. Monitor recovery retains physical device
identity and saved geometry without changing the registry's saved display mode.

Music optionally opens an owned Chrome player using a local Node adapter over
inherited JSON pipes. Its profile is kept in `config/native-music-player/`.

Portable updates use the official `nhatnguyends/Lumen` GitHub release feed over
Windows HTTPS, with bounded downloads and SHA-256 verification against GitHub's
asset metadata. Archive paths, duplicate names, links, file counts, total size,
architecture, version and file hashes are validated. App files are backed up
before replacement; the updater waits for the native host to exit and supports
rollback after failed file replacement or an interrupted transaction. It preserves
user config, notes and browser profiles. Native startup registration writes only
Lumen's HKCU Run value, as requested.

The first portable release is unsigned. GitHub artifact checksums are not
independent publisher signatures. Do not advertise support for arbitrary native
third-party plugins or a sandbox: source packages are still compiled into Lumen.
