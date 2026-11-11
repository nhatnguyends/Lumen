# Shared gradient surfaces — 2026-11-10 implementation contract

Root owns new `surface.rs`, `color_picker.rs`, shared settings_ui/render APIs,
Dock settings and Dock model/window integration. Widget chats adapt only their
own widget files; no runtime restarts/native test processes during root QA.

API (root publishing now):
- `crate::surface::PRESETS`: `[(&str,&str,u32);12]`,10 gradients + wallpaper-auto + custom picker action.
- `valid(key:&str)->bool`: supported gradients, old legacy color IDs and persisted `custom:#RRGGBB`; NOT bare custom action, and NOT Clock's private theme key.
- `normalize(key:&str)->Option<String>`: gradients/auto/custom hex canonical; old flat IDs -> equivalent custom hex so existing appearance is preserved. Do this on load before validation. Clock must keep its own `theme` key.
- `color(key:&str,wallpaper:u32)->u32`: representative COLORREF for contrast/tint/controls; custom parses exact picked color; legacy IDs preserved.
- `paint(canvas:&crate::render::Canvas,pane:[f32;4],key:&str,opacity:f32)`: paint selected gradient or custom solid on cached widget background; opacity0..1 normally tint/100 (1 if frosted disabled). Auto/theme/legacy no-op: preserve existing theme/auto pipelines. Place AFTER original flat tint backing but BEFORE coating/noise/spotlight/contrast sample/bevel. Called only when background cache rebuilds. Frame compositor supplies native rounded mask.
- Shared `settings_ui::SURFACE_PALETTES` aliases PRESETS. `palette_choices()` returns the new common choices. In widget settings use this rather than duplicating the old solid list.
- Shared settings UI intercepts the bare `custom` choice and opens an HSV wheel; it only emits `Value::Choice("custom:#RRGGBB")` after Apply. Cancel preserves prior choice. All palette change guards must use surface::valid rather than membership-only checks.
- Gradient previews/solid picker swatch are shared-kit/root work, not per-widget.

Retain native config validation/data/theme/bounds, opacity (real desktop alpha),
frost strengths and blur independence. Surface palette is material selection,
not a content-theme switch. Clock `theme` remains optional to keep its existing
9 themes intact. Notes note-color palette is unrelated: do NOT change note colors.
Tests: widget-local model roundtrip for a gradient and custom hex, legacy flat
migration, own setting application and cached rendering. Pure/static checks okay;
root will run one serialized native full test and reload after handoffs ready.
