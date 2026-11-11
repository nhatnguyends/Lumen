# AGENTS.md

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
