Native Rust Lumen for Windows x64: App Dock, Clock, Calendar, YouTube Music,
Sticky Notes and Control Center, with shared glass surfaces and snap/fuse.

**Portable:** download `Lumen-0.1.0-windows-x86_64.zip`, extract the complete ZIP
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
