# Calendar outer surface review — 2026-11-10

READY for root integration. Changes are limited to this Calendar native package.
`package.rs`, `widgets/calendar/widget.json`, lifecycle registration and shared
files were not changed.

## Fixed

- Removed the second rounded Mist/Agenda/Soft face painter. In particular Soft
  no longer has a second large inset border/shadow/highlight around its background.
- All nine designs now paint their outer material on a zero-radius, full-frame
  rectangle. The compositor owns the rounded silhouette and the one outer bevel.
  Shared surface palettes also paint this full-frame backing; coating and corner
  reflection still align with the compositor's existing 4-DIP outer contour.
- Agenda's red region reaches the top, left and bottom of the outer frame, with
  a square join to the month background. It is cached before coating/contrast/
  bevel/background alpha rather than drawn as an opaque inset pill every frame.
- Soft/Agenda use the contrast result of the actual final backing. Compact
  designs keep at least 16-DIP side/bottom content insets (12 DIP inside visible
  chrome); Mist/Agenda footer controls were moved inward by 4 DIP. Existing
  content cards in Blocks/Pop/Prism/Grove remain intentional. Shared compact
  header clearance and separate settings behavior are retained.

## Edited files

- `mod.rs`: full-frame unrounded background material/tint/palette, single outer
  coating/bevel, cached Agenda split.
- `reference.rs`: removed inset background surfaces, full-height Agenda detail,
  foreground contrast follows the visible backing.
- `layout.rs`: full-frame background geometry, content insets and full-height
  Agenda region; all-nine-design bounds regression.
- `artwork.rs`: pure pixel regression for full coverage and continuous edge
  colors across all outer materials at 100/125/150/200% DPI.

## Validation / root-owned next steps

Rustfmt parsing/check passes for the four edited Rust files. No app build,
native test process, HWND creation, reload, restart or desktop action was run.
Recommended filters for root's next serialized test slot:

```
calendar::layout::tests::
calendar::artwork::tests::
calendar::tests::compact_themes_keep_all_date_targets_inside_bounds_and_blur_is_private
calendar::tests::native_calendar_renders_themes_settings_and_transparency_without_writing_old_state
```

New regressions are `all_designs_keep_content_insets_separate_from_full_bleed_background`
and `outer_materials_fill_backing_without_an_inner_rounded_alpha_edge`.
Visual QA should compare all nine at 240×280, 420×460 and 800×280, including
gradient/custom material, opacity 0/100, and Agenda's red/white join. The shared
compositor remains responsible for the final per-pixel rounded mask.

Cross-thread messaging was not used: that tool requires direct human
authorization, and an incoming delegated request does not grant it. Results are
delivered through the response and this handoff without waiting for root polling.
