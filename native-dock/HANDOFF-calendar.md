# Native Calendar handoff

**READY — latest 2026-11-10 reference themes + shared surface adapter.**
The user's three screenshots are now Mist (blue weekly/monthly), Agenda
(red/white event split) and Soft (raised white/coral). All nine are selectable in
the existing separate settings host. Full implementation/QA notes are at the top
of `CALENDAR-THEME-PARITY.md`; this supersedes date-only scope below for local
notes/events only. No embedded clock or online event integration.

Changed only Calendar source/docs: `src/calendar/{mod,model,layout,artwork,view}.rs`,
new `reference.rs` and `entry_editor.rs`. Typography unchanged except formatting.
Model/settings use `surface::{normalize,valid,color}`; palettes use shared choices.
Managed gradients/custom skip the previous flat c.round; shared `surface::paint`
is only in cached backdrop construction, before coating/overlay/contrast/bevel.
New theme background faces are also cached before that paint and real alpha.
Legacy flat `ocean` migration is covered; gradient/custom persistence is covered.
Common header/layout/title/fusion hooks retained. No shared files were edited.

Static check compiles tests; 13 pure Calendar tests pass (3 layout, 7 model,
3 artwork); safe-mode check passes. **No HWND tests, app restart, desktop actions,
or native UI QA were run in this pass.** Root owns that serialized slot. Existing
native tests were extended for seven Mist targets/nine themes and gradient/custom
setting application; please execute those with the root integration suite.
Preview test emits 3×3 sheet. Editor IME/tab/save/cancel and all three screenshots
still require the serialized visual/interaction pass.

Root's reported stale `expected grove / actual soft` test failure is fixed:
`compact_themes_keep_all_date_targets_inside_bounds_and_blur_is_private` now
expects `*THEMES.last()` after iterating all nine themes. The final source has
been recompiled with `cargo check --tests`; all 13 pure Calendar tests and
`node scripts/check-safe.mjs` passed after this fix and the final backdrop adapter.

**Latest user-directed theme fidelity pass:** see `CALENDAR-THEME-PARITY.md`.
The six old designs now have individual geometry/artwork/type/states, including
Georgia/vertical type in Almanac, true wide reflow and Prism/Grove layered art.
Added theme-local layout/artwork/typography/view modules; only Calendar source
and its documentation were changed in this pass. Native/HWND tests remain
paused here; pure tests and static compilation only (details in parity doc).

**Latest 2026-11-10 follow-up and native crash/test-freeze report:** see
`HANDOFF-settings-ui.md`. Calendar now has a shared-kit separate settings window,
240×280 compact minimum, private blur and shared chrome header; older inline
settings/360×400 notes below describe the superseded first pass.

Calendar is implemented in Rust using the dock's Win32/GDI+ renderer and shared
wallpaper/compositor, in the same process. No embedded clock, event feed or
Graph/ICS connection was added; the user explicitly confirmed date-only scope.

## Files changed

- `src/calendar/mod.rs`: native Host/window, painting, settings, input and tests.
- `src/calendar/model.rs`: calendar arithmetic, config import/persistence/tests.
- Shared integration: `src/main.rs` module; `src/window.rs` calendar Host field,
  show action, toolbar button 3, startup `--calendar`, teardown; `src/tray.rs`
  Show calendar entry. Calendar tray action is **6** in the current shared tree.
- `../run_native_dock.py`: `--calendar` option and argument forwarding.
- `README.md`: native calendar development and behavior notes.
- `.scratch/calendar-native-themes.png` at repository root: development render
  contact sheet, not a packaged deliverable.

## Behavior

- 42 consecutive calendar-day cells, adjacent-month dates, month navigation,
  selected date, Today, local-date rollover check once per minute, Monday/Sunday.
- Arrow keys select dates; Page Up/Down navigate months; Home selects today;
  Escape leaves settings or cancels movement. Release-inside activation and
  captured sliders; drag blank space and resize all edges/corners.
- Lumen wallpaper glass; Almanac light date panel; Blocks orange/blue cards;
  Pop blue/yellow cards and red selection; Prism pink luminous date/green progress;
  Grove forest/lime/teal cards. Month progress on the four card themes.
- Native glass settings: theme, week start, frost, floating/topmost, opacity, tint.
  Theme changes keep geometry. Background alpha preserves foreground content.
  Wallpaper crop freezes during movement and invalidates at release; shadow sync
  is explicit after move/resize. Idle has a minute timer, not a render loop.
- `config/native-calendar.json` only. Import old calendar geometry/week start and
  app design/opacity/tint; no writes to Electron files, tokens or events.
  Bad/unsupported native config errors leave the file intact. Smoke skips saves.

## Checks actually run

- `python run_native_dock.py --check`: passed during integration (guard, 6 guard
  regressions, then 75 Rust tests passed / 2 ignored, development build passed).
- Calendar-specific tests after theme work: 5 passed / preview ignored, including
  native HWND show/render all themes/settings/zero alpha/hide/drop and no-save.
- Final `cargo check --locked --manifest-path native-dock/Cargo.toml`: passed.
- Final calendar model tests: 4 passed (leap/century/year boundaries, 42-cell grid,
  migration without old-state writes, corrupt/future config preservation).
- Final `node scripts/check-safe.mjs`: passed.
- Hidden-buffer preview test ran after latest artwork changes; contact sheet
  visually inspected. It renders hidden windows, never captures the desktop.

Commands use the workspace-local toolchain via `run_native_dock.toolchain()` when
Cargo is not on PATH. Full source launcher remains `python run_native_dock.py
--calendar`; `python run_lumen.py` stays the primary Electron entry point.

## Remaining integration QA / limits

- Live dock Calendar button was clicked and emitted `native-calendar-ready` in
  the shared log. CUA discovery was unreliable for the secondary native window;
  end-to-end click/drag/resize/theme/persistence verification is **not complete**.
  Coordinate sequential desktop QA and restart only through the coordinating chat.
- A source-launch inspection process (PID 17436 when launched) was left alone
  after the coordinator requested no further restarts/closing shared app windows.
  It may run older artwork code; check current PID/state before any restart.
- Six theme renditions preserve colors/cards and behavior, but are not exact
  browser layouts. Almanac still uses the shared Segoe font rather than Georgia;
  wide layouts do not switch to the old CSS side-by-side composition. Review
  responsive layout and the top chrome at multiple sizes/DPI in final visual QA.
- Main widgets currently use the dock's existing native input pattern; this
  custom-painted grid has arrow navigation but no per-cell UI Automation tree.
- Independent calendar frost strength/blur controls are not exposed; wallpaper
  source/blur is shared with dock. Frost applies to Lumen; artwork themes have
  their designed colored backgrounds. Settings write failures are logged.
- No idle/resource claim for all widgets; profile once integrated and settled.

No SECURITY.md was read, per user instruction. No installers/release output,
PowerShell helpers, account enumeration or runtime synthetic input were added.

## Follow-up audit / fusion contract

- Calendar backing now calls `bevel_for(self.hwnd)` and includes
  `fusion::revision(self.hwnd)` in the backdrop key. No shared compositor edits.
- New root APIs: `hwnd`, `is_visible`, `hide`, `show_settings`,
  `apply_common(opacity: Option<f32>, floating: Option<bool>)`.
- Opening Settings no longer enlarges/saves a new widget size. Hide kills both
  timers and drops viewport/shadow buffers; Show refreshes date/recreates them.
- `hidden_calendar_settings_preserve_geometry_and_hide_releases_surfaces` added
  and passed. Final non-desktop calendar run: 5 passed, preview ignored, visible
  Host test skipped deliberately per coordinator's serialized desktop QA request.
- Shared `cargo check` and source guard passed after fusion API changes.
- Calendar uses captured manual `SetWindowPos` movement/resize, not the Windows
  modal move loop. The shared fusion subclass must support that path; relying
  only on WM_MOVING would not snap Calendar. Do final fuse/tear-off QA sequentially.
- General Settings implementation and root wiring contract are recorded in
  HANDOFF-general.md. That module has not been wired into shared main/window/tray
  by this chat, per coordinator instruction.

Window discovery follow-up: Calendar now explicitly calls SetWindowTextW after
CreateWindowExW with `Lumen · Calendar`; WM_NCCREATE had bypassed default title
initialization. The hidden lifecycle regression checks GetWindowTextW returns
that title. Root can select this exact title during the next serialized UI run.

Reentrancy follow-up: Calendar releases the RefCell guard before every default
WndProc dispatch, before SetWindowPos in drag/DPI/cancel paths, and before
apply_window_mode from Host APIs. The floating settings action posts APPLY_MODE
(WM_APP + 82) so native mode changes run without a borrowed state. Its hidden
regression now performs a real app-owned SetWindowPos and asserts the nested
WM_SIZE handler ran; passed after the change. General received the same audit.
