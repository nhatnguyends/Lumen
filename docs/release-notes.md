Lumen 0.1.17 fixes YouTube Music playback reliability, keeps the taskbar icon crisp, and synchronizes Coverflow across multi-monitor duplicate widgets:

- **Reliable YouTube Music Playback Across Browsers**: Removed `Emulation.setFocusEmulationEnabled` and unnecessary `awaitPromise` evaluation on synchronous 1s state polls so YouTube Music plays continuously across Brave, Chrome, and Edge without triggering `"Something went wrong. Refresh or try again later."` interruptions, fixed dead-session reattach loops when a player window closes, and expanded player/media error capture.
- **Crisp YouTube Music Taskbar Icon Without Blur Flash**: Applied Lumen's 128×128 icon before taskbar placement (`MusicWindowHost::new`), verified icon handles via `WM_GETICON` (`ensure_icon`), and installed a pre-navigation CDP script (`Page.addScriptToEvaluateOnNewDocument`) that keeps the high-resolution 128×128 PNG icon active instead of flashing the low-res web favicon.
- **Multi-Monitor Coverflow & Playback Sync**: Shared live `playback`, full `queue`, `art_files`, and sign-in state across duplicate Music widgets (`SHARED_PLAYBACK` + peer messages) and forwarded control commands to the active backend owner so every monitor displays the complete Coverflow album art fan without falling back to single-track SMTC.
- **Missing-Browser Notice in Music Settings**: Displays a highlighted warning in **Account → Player browser** when a selected browser (such as Brave) is not installed on the PC, showing which fallback browser is used or prompting to install one.

Optional Python entry points remain available:

- `python runtime/launch_lumen.py`
- Double-click `runtime/Launch Lumen.pyw` with Python associated to `.pyw`.

Python 3.10+ must be installed for this optional method. No pip packages or
build tools are needed. Both entry points launch the same native `Lumen.exe`;
keep the full ZIP extracted. This does not change Defender or enterprise policy.
Direct `Lumen.exe` launch remains available without installing Python.

**Portable:** download `Lumen-0.1.17-windows-x86_64.zip`, extract the complete ZIP
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
