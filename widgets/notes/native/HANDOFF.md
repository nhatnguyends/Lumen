# Notes native theme review — 2026-11-10

## Sharper Notebook / Honey — 2026-11-11 latest

Reduced Honey cover/paper transition from24 to6 DIP; Notebook binding fold
from9 to3 DIP with deeper leather brown. Notebook cards now have10-DIP corners,
stronger paper backing, rules and red margin. Honey cards have14-DIP corners,
stronger lower separation and a slightly translucent cream fill (242 alpha).
Outer full-bleed material, compositor bevel, fusion revision/cache, background
opacity and viewport fade paths are unchanged, retaining shared fuse blending.
Only theme.rs/mod.rs edited. Rustfmt only; runtime/fused visual QA remains for
root's serialized slot. Recommended notes::theme::tests:: and ignored
notes::tests::capture_refined_themes, plus live fused/unfused comparison.

## Smaller Music glass Add + 160-DIP minimum — latest

Source READY for root's serialized integration. No app build, native test
process, restart or desktop operation was performed; running PID37520 untouched.

- Add is36 DIP instead of44,28 DIP in tiny layouts. New local lenses.rs uses
  Music's exact directional optical shader and cache (frosted translucent body,
  shoulder reflection, antialiased rim, pressed/hover states), plus its soft
  shadow treatment. It replaces the old generic glass_surface Add pass. Cache
  is bounded to24 entries and cleared on hide; no GDI handles in static storage.
- New pure layout.rs is used by paint AND hit-testing. At tiny widths/heights,
  Add/search/settings/close share the top row, search expands only when opened;
  a 160x160 short search view temporarily hides cards. Footer uses count and
  compact Undo/Done/folder targets. Done finishes either note or search input.
  Title remains governed by the shared header hook, hidden in tiny layouts.
- All four designs keep a real14-DIP inline text line at160x160. Short/narrow
  cards omit date/format-bar chrome; native editing/format shortcuts remain,
  and the footer checkmark finishes editing. Larger layouts restore the normal
  controls. Palette wraps all7 colors into two rows when narrow.
- Config clamp and native WM_GETMINMAXINFO both use model::MIN_SIZE=160 DIP.
  No default/restored bounds are changed beyond validation of undersized data;
  existing valid saved sizes stay intact. Header/footer actions win over the
  diagonal corner resize grip; outer edge areas remain available for resizing.
- Edited: mod.rs,model.rs,typography.rs,editor.rs,HANDOFF.md; new layout.rs and
  lenses.rs. Lifecycle package.rs and widget.json were not edited in this pass.

Checks: rustfmt parsed/formatted only these source files. Static audit verified
one outer bevel, no theme inset panel, consistent160 clamp/min-track paths,
exact pixel shader parity with Music, and local-only module changes. No Rust
build/test executed under root's native-process freeze.

Recommended root filters:
- Pure: `notes::layout::tests::`, `notes::lenses::tests::`,
  `notes::model::tests::tiny_bounds_round_trip_without_changing_larger_saved_bounds`.
- Native: `notes::tests::tiny_layout_keeps_native_input_and_corner_button_hits_usable`
  checks min track, all designs' input area, inline root ownership, search/Done
  flow, no card/search overlap, plus button winning over resize corner grip;
  HWND stays hidden and live config is untouched.
- Capture: `notes::tests::capture_refined_themes -- --ignored --exact --test-threads=1`.
  Includes160x160,160x320,320x160,240x240,300x650,880x540 for all4 designs,
  besides default420x650 and scrolling. Same .scratch/notes-themes-refined path.
- Compile all tests first, then run serialized `notes::` to catch integration.

Latest user change: reduced all card preview bodies from16 to14 DIP, with
20-DIP leading (Notebook24/Honey22). Inline RichEdit now uses7/8 view zoom,
combined with DPI zoom as a reduced fraction; original RTF sizes/formatting
remain unchanged. Search/captions/card bounds unchanged. Edited typography.rs
and editor.rs; rustfmt check only, no build/native process/restart.


Source ready for root-owned integration/desktop QA. No app build, native test
process, desktop operation or runtime restart performed in this pass.

## Fixed

- Theme material now fills [0,0,width,height] for Pinboard/Notebook/Index/Honey.
  The compositor is the sole outer rounded mask and bevel. Removed the inset
  sheet/detail pass entirely, including Honey's second coating and Notebook's
  smaller paper panel/rounded binding strip.
- Honey transitions from amber cover to cream paper across the FULL width.
  Notebook's paper texture/binding fold is baked into the full-height material;
  Index retains its full green backdrop. Intentional individual note cards,
  note-color beads and inline editor are preserved.
- Compact New note moves from y6 to y12 and x14 to x18. Search has a 12-DIP
  gap from Add, compact card edges are 16 DIP (Notebook24), footer keeps its
  existing 20/24-DIP inset. Shared header hooks remain unchanged.
- All changes are within this source package. package.rs and widget.json were
  preserved. No shared host/render/settings/Control Center files edited.

## Checks and recommended root filters

- Rustfmt parsed/formatted the package. Static material audit passed: one
  bevel_for call, no inset theme detail pass or 4-DIP outer material rectangle.
  No native test processes launched.
- Pure texture regression: `notes::theme::tests::theme_materials_fill_edges_without_an_inset_sheet_boundary`
  checks full opaque coverage and continuity across the former inner-panel
  edges at 240/420/880-DIP widths and 100%/150% DPI. No HWNDs in that test.
- Hidden render regression/capture: `notes::tests::capture_refined_themes`
  with `--ignored --exact --test-threads=1`. Extended all four designs to assert
  Add/search/card/footer insets and render 240x240,300x650,880x540, in addition
  to 420x650 default/scroll frames. Synthetic notes and isolated config only;
  asserts the HWND stays invisible. Resize calls drop the State borrow before
  SetWindowPos so synchronous native resize callbacks can reenter safely.
  Outputs .scratch/notes-themes-refined/.
- Existing inline/DPI tests remain recommended: filter `notes::` serialized.
- New tests are authored, not executed here under root's native-process freeze.

Messaging limitation: this chat has no explicit human authorization to send
messages to another chat. The tool specifically says an orchestrator's request
to reply is not sufficient authorization. Therefore no send_message_to_thread
call was made; this handoff and the user-facing reply report readiness.
