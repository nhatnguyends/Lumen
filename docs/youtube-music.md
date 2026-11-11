# YouTube Music

## Native Rust widget

Run `python run_native_dock.py --music`, or choose **Show music** from the native
dock tray. The Music toolbar button opens that same native widget. Its UI uses
Rust/Win32 and the dock's wallpaper glass/compositor, without an Electron renderer
or WebView. Playback currently reuses the Node/Chrome adapter over inherited JSON
pipes; this is a Rust UI port, not an all-Rust playback backend.

Appearance offers Coverflow, Compact, Sleeve, Turntable, floating mode, opacity
and tint. Playlist rows/cover cards select real tracks; previous/next and seek
follow the page's capabilities. The playlist button toggles its loaded queue;
the wheel scrolls it. Space toggles playback; Escape dismisses settings. Hiding
Music retains playback; closing the browser frame minimizes that same frame.

Preferences use `config/native-music.json`. The isolated browser profile, resume
state and cover cache use `config/native-music-player/`. Existing Electron state
is only read to import design/opacity on first use; sign in once through the
native player's **Browse music / account** window. Both the adapter bundle and
Rust dev executable are built only under `native-dock/target/debug/`.

Typechecking, 18 Music tests, seven Rust Music tests (including hidden native resize and offscreen layout),
the adapter build and safety guard pass. Live playback, browser parenting, DPI
and visual QA are pending the coordinated desktop pass. Coverflow transitions,
rotating vinyl and the related-radio fallback have not been ported yet.

