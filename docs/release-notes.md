Lumen 0.1.10 introduces multi-monitor duplicate widgets with two-way shared-source synchronization, automatic live DPI/resolution scaling, unified multi-row App Dock group contours, and reorganized General Settings:

- **Multi-Monitor Independent Duplicates & Two-Way Shared Sync**: Duplicate widgets run independently per selected display at native DPI with isolated hover/click states, per-monitor fusion, and separate saved position/size per monitor, while syncing Sticky Notes content, App Dock ordering/groups, Control Center controls, and individual widget settings two-way across all displays.
- **Automatic Live DPI Scaling & Work-Area Clamping**: Widgets stay bounded within the active monitor work area and rescale automatically without manual interaction whenever display resolution or Windows scale changes.
- **Non-Destructive Fusion Overlap**: Dragging another widget over part of a fused group no longer breaks the existing fused widgets apart.
- **App Dock Multi-Row Group Contours, Fewest-Rows Packing & Crisp Icons**: Multi-row groups render inside one unified rounded background polygon, pack horizontally with the fewest rows before forming compact rectangular blocks, and trim/extract Windows icons consistently across machines.
- **General Settings Reorganization**: Moved visual material controls (blur size, frosted glass, liquid glass) into a top **Background** section and grouped remaining options under **Misc**.

Optional Python entry points remain available:

- `python runtime/launch_lumen.py`
- Double-click `runtime/Launch Lumen.pyw` with Python associated to `.pyw`.

Python 3.10+ must be installed for this optional method. No pip packages or
build tools are needed. Both entry points launch the same native `Lumen.exe`;
keep the full ZIP extracted. This does not change Defender or enterprise policy.
Direct `Lumen.exe` launch remains available without installing Python.

**Portable:** download `Lumen-0.1.10-windows-x86_64.zip`, extract the complete ZIP
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
