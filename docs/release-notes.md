Lumen 0.1.15 fixes widget drag pixelation, strengthens Liquid Glass rim chromatic dispersion, and eliminates wallpaper-switch freezes:

- **Smooth Widget Drag & Crisp 3× Liquid Glass Rim Chromatic Dispersion**: Fixed top-down DIB sign handling in `blit_placed_bilinear` to eliminate pixelation when dragging widgets, restored full-resolution BGR24 `SharpImage` rim sampling, and boosted Liquid Glass rim chromatic dispersion 3× across both CPU (`glass.rs`) and D3D11 HLSL shaders (`gpu.rs`).
- **Non-Blocking Async Wallpaper Reloads & Deadlock Fix**: Eliminated UI freezes, ~15–20s black desktop stalls, and missed widget background updates during wallpaper changes by coalescing and timeout-cancelling `IDesktopWallpaper` COM queries (`800 ms`), running single-flight background polling/decoding (`Glass::poll_async`), centralizing debounced wallpaper refreshes in the launcher (`WALLPAPER_DEBOUNCE_TIMER`), removing blocking UI-thread `glass.refresh()` calls from sibling widgets (`sync_from` for local glass), and keeping the previous valid wallpaper when mid-write decoding fails (`try_commit_slots`).

Optional Python entry points remain available:

- `python runtime/launch_lumen.py`
- Double-click `runtime/Launch Lumen.pyw` with Python associated to `.pyw`.

Python 3.10+ must be installed for this optional method. No pip packages or
build tools are needed. Both entry points launch the same native `Lumen.exe`;
keep the full ZIP extracted. This does not change Defender or enterprise policy.
Direct `Lumen.exe` launch remains available without installing Python.

**Portable:** download `Lumen-0.1.15-windows-x86_64.zip`, extract the complete ZIP
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
