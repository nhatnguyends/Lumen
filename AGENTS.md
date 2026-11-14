# AGENTS.md

## Publishing on hold

The user requested on 2026-11-11 that pending changes stay local. Do not push
commits/tags or publish a GitHub release unless the user explicitly requests
publishing again. Local edits, builds and checks remain authorized.

## Native Rust only

The user retired the Electron version on 2026-11-11. All widget UI runs in the
native Rust host. Do not restore Electron, React renderers, or WebViews.

Run from source with `python run_lumen.py` or double-click `Launch Lumen.pyw`.
`run_lumen.py` delegates to `run_native_dock.py`. Local development builds stay
under `native-dock/target/debug/`. The user subsequently authorized GitHub source
and package publishing and requested maximum portability on 2026-11-11. Native
Rust portable ZIPs under `share/` are now in scope; keep packaging builds under
`native-dock/target/`. Do not restore Electron, create installers, or use
`release/` output. Keep the source launcher for local development.

- Keep `scripts/check-safe.mjs` passing; no PowerShell helper scripts.
- Do not enumerate local users/groups or inject keystrokes.
- Prefer source edits and restarting the native launcher for testing.
- Preserve `config/`, including legacy settings used for first-run migration.
- The optional Node/Chrome playback adapter belongs to `widgets/music/adapter`;
  it does not host widget UI or use Electron.

## 120Hz Every-Frame Scroll & Animation Standard

All scrollable views and animations across the native Rust app and widgets
(App Dock, Sticky Notes, Control Center, Control Center Manager, YouTube Music
including Coverflow transitions, Calendar, Clock, Widget Settings, and General
App Settings) must update on every single frame at 120Hz (~8.33 ms/frame):

- **VSync Frame Pacing (`pace_vsync`)**: Pair a 10 ms fallback `SetTimer` +
  `FrameClock` (`timeBeginPeriod(1)`) with post-`EndPaint` `DwmFlush()` +
  `PostMessageW(..., MOTION_FRAME, 0, 0)` (skipping `DwmFlush` if CPU+present
  exceeds 5.5 ms so a frame never oversleeps across a 120Hz VSync boundary).
- **Immediate First-Frame Wakeup**: Call `InvalidateRect` immediately when
  arming a scroll or animation timer so the first frame renders without waiting
  for a timer tick.
- **Zero Per-Frame GDI+ Context Creation on Cached Paths**: Keep `Batch::new`
  lazy (do not call `GdipCreateFromHDC` unless uncached GDI+ drawing occurs),
  draw scrollbars via software SDF (`Buffer::fill_round`), and use integer
  fixed-point math in `fade_list`, `fade_list_edges`, and
  `fade_horizontal_edges`.
- **Pixel-Aligned Orthogonal Axis & Fast Blitting**: During 1D scrolling, snap
  the orthogonal coordinate to device pixels (`fx == 0` for vertical scroll) so
  cached sprites/cards hit the `chunks_exact(4)` fast path in
  `Target::translate`, and suppress transient hover scale/glow trails on items
  passing under a stationary cursor while actively scrolling.

