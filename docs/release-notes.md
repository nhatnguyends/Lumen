Lumen 0.1.9 refines App Dock interactions and animations, unifies fused-widget separation across moves and border resizes, and improves tray, startup, and YouTube Music browser reliability:

- **App Dock Context Menu & Selection Polish**: Kept a visible accent selection highlight on the right-clicked icon while its menu is open, fixed outside-click dismissal across caption bars, resize edges, search box, desktop, and inactive windows, and added a **Copy** command (`CF_HDROP` + `CF_UNICODETEXT`) for real files, folders, and shortcuts while hiding it for virtual `shell:` locations (Recycle Bin, This PC) and Store apps.
- **Curved 120Hz Launch Animation**: Clicking an icon on App Dock triggers a smooth cubic-bezier burst scaling the launched icon up to `200%` while fading out cleanly at 120Hz.
- **Live Scroll Hover in App Dock**: Hover glow and surface state remain active and track cursor position accurately while scrolling the App Dock grid.
- **Unified Fusion Drag-to-Unfuse Threshold**: Consolidated a shared `16` DIP drag-to-unfuse threshold across all widgets for both moving and border resizing, so brief clicks/holds never detach fused panes prematurely.
- **Startup, Tray, YouTube Music CDP & Tray Updates**: Added resilient tray registration (`!NIF_GUID` fallback and UIPI message filtering), medium-integrity Windows startup task de-elevation, Chrome/Edge CDP `--do-not-de-elevate` launch reliability across admin/standard sessions, and automatic scroll to the Updates section when clicking Update in the tray menu.

Optional Python entry points remain available:

- `python runtime/launch_lumen.py`
- Double-click `runtime/Launch Lumen.pyw` with Python associated to `.pyw`.

Python 3.10+ must be installed for this optional method. No pip packages or
build tools are needed. Both entry points launch the same native `Lumen.exe`;
keep the full ZIP extracted. This does not change Defender or enterprise policy.
Direct `Lumen.exe` launch remains available without installing Python.

**Portable:** download `Lumen-0.1.9-windows-x86_64.zip`, extract the complete ZIP
to a writable user folder and run `Lumen.exe`. Python, Node, Rust and Visual
Studio do not need to be installed. The Music runtime is included; playback
requires installed Chrome or Edge and an internet connection.

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
