Lumen 0.1.18 adds Pop-Out floating mode for YouTube Music and Sticky Notes, fixes Start Menu browser launches for YouTube Music, and improves multi-monitor layout persistence:

- **YouTube Music Floating (Pop-Out) Mode & Per-Monitor Isolation**: Added a header Pop-Out button and **Layout → Floating mode** toggle to float YouTube Music always-on-top while keeping its exact theme and layout in-place and detaching cleanly from Fusion mode. Pop-out state is isolated and persisted per monitor so duplicated Music widgets only float on the monitor where clicked.
- **Reliable YouTube Music Launch from Start Menu**: Sanitized the child browser environment block (`CREATE_UNICODE_ENVIRONMENT` + `env_remove("__COMPAT_LAYER")`) to strip Windows compatibility shim variables such as `__COMPAT_LAYER=DetectorsAppHealth`, fixing immediate Edge startup exits when Lumen is launched from Start Menu shortcuts, and added exit-code logging on browser startup failures.
- **Sticky Notes Floating Pop-Out Windows**: Added always-on-top pop-out windows for individual Sticky Notes with atomic 120Hz shadow dragging (`WindowShadow::move_with_owner`), smooth unified scrolling, clipboard image paste support, and live two-way sync.
- **Multi-Monitor Layout Persistence & Liquid Glass Blur Polish**: Saved monitor-relative primary and duplicate widget layouts across display origin shifts, reconnects, and DPI changes, refined display ID matching, and softened Liquid Glass rim chromatic dispersion at higher blur settings.

Optional Python entry points remain available:

- `python runtime/launch_lumen.py`
- Double-click `runtime/Launch Lumen.pyw` with Python associated to `.pyw`.

Python 3.10+ must be installed for this optional method. No pip packages or
build tools are needed. Both entry points launch the same native `Lumen.exe`;
keep the full ZIP extracted. This does not change Defender or enterprise policy.
Direct `Lumen.exe` launch remains available without installing Python.

**Portable:** download `Lumen-0.1.18-windows-x86_64.zip`, extract the complete ZIP
to a writable user folder and run `Lumen.exe`. Python, Node, Rust and Visual
Studio do not need to be installed. The Music runtime is included; playback
requires installed Brave, Chrome or Edge and an internet connection.

**Updates:** General Settings → Updates checks this repository's stable releases,
verifies the portable archive and applies it at restart. Config, notes, browser
profile and layout are preserved. Startup with Windows is enabled by default
and can be disabled in General Settings.

All six widget packages are included in source under `widgets/`. They currently
compile and update together; individual binary plugin installation is not yet
supported. The source ZIP requires the documented development toolchain.

Initial target: Windows 10/11 x64; no native ARM64 or 32-bit build. This is an
early, unsigned portable build. Broader testing on other PCs is still needed.
No Electron, WebView, installer or personal config is included.
