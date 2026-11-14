Lumen 0.1.2 adds credits and open-source license acknowledgements:

- Created by Nhat Ng. Lumen's original code is now MIT licensed.
- General Settings → Credits & licenses opens the acknowledgements.
- Portable and source packages include an exact dependency inventory and upstream
  license texts, including Lucide/Feather icon notices and bundled runtime notices.
- Thank you to every library author, maintainer and contributor supporting Lumen.

Optional Python entry points remain available:

- `python runtime/launch_lumen.py`
- Double-click `runtime/Launch Lumen.pyw` with Python associated to `.pyw`.

Python 3.10+ must be installed for this optional method. No pip packages or
build tools are needed. Both entry points launch the same native `Lumen.exe`;
keep the full ZIP extracted. This does not change Defender or enterprise policy.
Direct `Lumen.exe` launch remains available without installing Python.

**Portable:** download `Lumen-0.1.2-windows-x86_64.zip`, extract the complete ZIP
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
