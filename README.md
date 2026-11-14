# Lumen

Native Rust desktop widgets for Windows: App Dock, Clock, Calendar, YouTube
Music, Notes and Control Center. Widget UI shares one Rust/Win32 process and
wallpaper compositor. The Electron app and its build/runtime dependencies have
been removed.

## Portable download

Download `Lumen-<version>-windows-x86_64.zip` from
[GitHub Releases](https://github.com/nhatnguyends/Lumen/releases), extract the
whole archive to a writable folder, and open **Lumen.exe**. The native portable
edition bundles its Music runtime: Python, Node, Rust and Visual Studio do not
need to be installed. Music playback uses installed Chrome or Edge and internet;
the other widgets do not require a browser. No Electron or WebView runtime is used.

The initial target is Windows 10/11 **x64**. Native ARM64 and 32-bit builds are
not supplied, and cross-machine compatibility still needs broader testing.
The binary is currently unsigned. Keep all extracted files together and use
a writable user folder rather than Program Files. Data stays in `config/` beside
the executable. Start with Windows defaults on and can be disabled in General
Settings. Launch once after moving the folder to refresh the startup path.

General Settings > Updates checks official stable releases, downloads and
verifies the portable ZIP, then restarts to apply it. Network work runs away from
the UI thread. Updates preserve `config/` and keep file backups in
`config/updates/`. Modified app files cause an update to stop rather than overwrite
local edits. GitHub HTTPS release metadata/digests provide integrity; this is not
an independently signed update feed. All six native widget implementations
currently update together with Lumen. Independent binary plugins remain planned.

## Optional Python launch (portable)

The portable ZIP also includes optional Python entry files: run
`python runtime/launch_lumen.py` from its extracted folder, or double-click
`runtime/Launch Lumen.pyw` if Python is associated with `.pyw` files. This option
requires installed Python 3.10+ but no pip packages, Rust or build tools. It still
launches the same Rust `Lumen.exe` and does not change Windows security checks.
Keep the executable and all runtime files together. Launcher logs are written
to `config/logs/python-launcher.log`.

## Run from source

Double-click **Launch Lumen.pyw**, or run:

```text
python run_lumen.py
python run_lumen.py --no-build
python run_lumen.py --widget controls
```

Build prerequisites: Python, Node.js, Rust MSVC and Visual Studio C++ Build
Tools. Run `npm ci` once for the optional music playback adapter and build tools.
`python run_lumen.py --build-only` builds without opening a window. Local builds
stay in `native-dock/target/debug/`. Portable packaging uses a separate static-CRT
development build under `native-dock/target/portable-debug/`, writing ZIPs under
`share/`; no installer is generated.
The optional music adapter controls an owned Chrome player over JSON pipes;
widget windows remain Rust. See [Music](docs/youtube-music.md).

## Widgets

Open the native tray menu to show the widget manager. Every widget has an On/Off
switch and independent settings. Appearance, size and position are remembered.
Packages live in `widgets/<id>/native/`; see [package development](widgets/README.md).

Control Center groups displays, audio outputs and microphones into one control
per family. Device names lead each row; newly connected devices appear in the
corresponding group. Main Display is identified with a disabled gray toggle.
Volume and microphone volume include mute on the icon. Edit supports drag/drop,
rename and remove. Add opens a separate searchable visual control manager and
keeps the widget geometry. Small layouts retain labels and scroll horizontally.
Visible draggable scrollbars indicate overflow; edge fades appear only where
more content remains. Soft Violet uses the raised pale glass material.

## Checks

```text
node scripts/check-safe.mjs
node --test scripts/test-check-safe.mjs scripts/test-native-music.mjs
python -I scripts/test-system-controls.py
python run_lumen.py --check
```

Settings live in `config/`. Legacy settings are retained for import; removing
Electron does not delete widget data or browser sign-in profiles.

## Credits and license

Created by **Nhat Ng**. Copyright © 2026 Nhat Ng.
Lumen's original source is licensed under [MIT](LICENSE.txt).

Thank you to the developers and contributors behind Rust, windows-rs, resvg,
tiny-skia, image, Serde, RustCrypto, zip-rs, Node.js, Koffi, Lucide, Feather,
Vite, Rolldown, TypeScript, Python and all transitive dependencies.
See [Credits](CREDITS.md) and the [third-party inventory](LICENSES/THIRD-PARTY-NOTICES.md).
Full license texts are included under `LICENSES/` in source and portable packages.
General Settings → Credits & licenses opens the acknowledgements.
Third-party components keep their own licenses; Lumen's MIT license does not replace them.
