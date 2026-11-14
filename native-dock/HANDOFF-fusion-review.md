# Fusion correctness / performance review

2026-11-09. Read-only source review of `src/fusion/{mod,geometry}.rs`, fusion changes in `src/compositor.rs`, and the relevant widget callback paths. No runtime/desktop actions, builds, or source edits. Only this requested report was written. References below are source locations at review time; use the named function if concurrent edits move lines.

## Concrete findings

### [P1] Shared coverage double-composites translucent chrome in overlapping HWND margins

**Where:** `fusion/mod.rs:32-35` (`read_member`, four-physical-DIP insets), `geometry.rs:48-62` (`snap_move` aligns pane edges), `compositor.rs:488-516` (`present_with_alpha`, whole-group coverage in every member HWND).

Snapping visible glass edges necessarily overlaps the outer HWND rectangles: at 96 DPI, a 400px-wide window at x=0 has a glass right edge at 396; the next window starts at x=392 to put its glass left edge at 396. Their HWNDs overlap for eight physical pixels. `fusion_cover` fills each member's former transparent margin using the entire union, so both surfaces paint the same seam strip. With opaque chrome this can look correct; at 50% chrome alpha, two alpha-128 surfaces combine to alpha approximately 192 (75%), producing a visibly denser strip. The bridge's `in_window` exclusion does not prevent duplication between the member HWNDs. This is a source-level compositing consequence, not a desktop measurement.

**Recommendation:** partition background coverage between members in overlap regions, or put the fused chrome on one common surface with member content separately composited. Preserve the complete union silhouette, but assign each background pixel once. Do not simply remove the overlap by clipping both original rounded shapes; that reopens the concave gaps.

**Regression:** compose the actual two/three member BGRA outputs over a common checkerboard at 0/25/50/100% opacity; inspect alpha across a straight seam and T junction. Equal 50% backgrounds must not produce a 75% stripe. Include unequal opacities to define the intended blend policy explicitly.

### [P2] Failed/capped bridge rendering still suppresses all individual shadows

**Where:** `fusion/mod.rs:60` (`fused` means geometric membership), `mod.rs:133-143,174-176` (`refresh` skips over-budget/allocation failures and ignores upload result); `compositor.rs:651-655` (`WindowShadow::sync` hides whenever `fused`).

A valid geometric group exceeding the 16,000,000-pixel bounding-box cap gets no bridge, yet every member still considers itself fused, hides its individual shadow, and uses fused coverage/bevel. The same failure occurs when CreateWindow/Buffer allocation fails. Upload failure is also treated as success. This can leave missing outer shadows and unfilled junctions. A four-pane chain of roughly 2200px square members already has a bounding box above 21 million pixels including padding; the cap can be reached with large widgets/high DPI, not only corrupt input.

**Recommendation:** distinguish desired adjacency from successfully committed rendered components. Commit the bridge and suppress standalone shadows only after allocation and upload succeed; otherwise fall back atomically to standalone coverage/bevel/shadows, or tile the bridge into bounded pieces. Preserve the old valid bridge until a replacement succeeds.

**Regression:** inject bridge allocation/upload failure, and exercise an over-cap chain; all members must retain usable standalone chrome/shadows and recover when the group becomes renderable.

### [P2] An unrelated widget change rebuilds every bridge and every cached backdrop synchronously

**Where:** `fusion/mod.rs:38,55-61,127-180` (`changed`, one global revision, destroy/recreate all bridges, invalidate all member HWNDs); `compositor.rs:306-358,488-502` (shared material, shape/bevel/mask recomputation).

A geometry/visibility/opacity change to a separate, unfused member increments the global revision, destroys every bridge, redraws each component's entire bounding rectangle, and invalidates all widgets. All widget backdrop keys include that same revision. This discards otherwise valid wallpaper/glass caches for unaffected groups. Refresh performs per-pixel SDF sorting, extra SDF calls for shadows and opacity weights, and recreates native windows on the UI thread. At the accepted 16M-pixel cap, the DIB and snapshot alone coexist at approximately 122 MiB, excluding server/GPU copies and member surfaces. No runtime latency was measured here; the allocation/work scaling follows directly from the loops.

`pending` coalesces posts, so I do **not** see a self-sustaining invalidation loop when geometry/style is stable. It does not limit the cost per refresh or scope it to affected components. Real resize/position churn or a bad widget callback can turn this into repeated global rebuilds.

**Recommendation:** dirty only the old/new affected connected components; use per-component revisions, persistent bridge HWNDs and cached material/masks; update only changed join/shadow bands or bounded tiles. Keep ordinary standalone motion/opacity from touching unrelated groups. Add counters for posted/processed refreshes, member backdrop rebuilds, bridge allocations and pixel area.

**Regression:** keep A+B fused and stationary, then move/change opacity on distant C. A+B's bridge HWND, revision and render counts should remain unchanged. Record counts and UI-thread cost while resizing a middle member of a four-widget chain, and verify timers/repaints settle after release.

### [P2] Snap acceptance and fused adjacency disagree at mixed DPI

**Where:** `geometry.rs:19-22` (`adjacent` uses max pair DPI); `geometry.rs:49,55-59` (`snap_move` uses moving member DPI); `compositor.rs:318-325` (seam orientation uses another tolerance based only on the rendering member DPI).

Example with valid-sized panes: stationary DPI=2 pane `[0,0,600,800]`, moving DPI=1 pane `[609,740,909,1040]`. Their overlap is 60 physical pixels. `snap_move` accepts the overlap (>=48), moves x by -9, and sticks the edges. `adjacent` rejects the result (needs >=96), so the magnetic placement never forms a fused group. The reverse drag direction can behave differently. Separately, adjacency accepts gaps up to `1.5 * max_dpi`, while a low-DPI member's material blend infers orientation with `<2 * own_dpi`; an accepted vertical join can fall into the horizontal-join branch.

**Recommendation:** define one pairwise seam/eligibility function returning orientation, overlap, common DPI tolerance and candidate displacement. Use it for snapping, membership and seam-material blending. Validate the snapped candidate with that predicate before accepting it.

**Regression:** low-DPI→high-DPI and high-DPI→low-DPI partial overlaps around 48/96px; two-/three-pixel accepted gaps; both seam orientations; negative monitor coordinates. Require every accepted snap to satisfy the intended adjacency predicate.

## Callback / detach / chain assessment

- `components` does traverse connected chains/T junctions transitively, and the existing middle-member geometry test covers a useful basic split. `snap_move` changes only the dragged rectangle. Fusion's WINDOWPOS handler changes x/y only; it does not directly reduce member cx/cy.
- The manager's `try_borrow`/`try_borrow_mut` guards and taking the Manager out of TLS during `refresh` avoid an obvious direct recursive RefCell panic in that path. Posted REFRESH messages also avoid synchronously refreshing while the manager is borrowed. Callbacks while the manager is absent return default/unfused state or skip updates; this deserves a targeted reentrancy test, but I did not establish a concrete lost-update failure from the current call graph.
- **Fusion can amplify the Music/Notes shrinking bug:** each refresh invalidates every widget, which causes additional paints/UpdateLayeredWindow calls. Those synchronous native callbacks can hit a widget's RefCell-busy fallback. Before the peer fix, delegating `WM_NCCALCSIZE` to DefWindowProc with framed style could repeatedly change the client/outer size; fusion then observed each geometry change, incremented the revision and posted another global refresh. This is a feedback amplifier, not a direct width/height write by fusion. At review time both Music and Notes already answer `WM_NCCALCSIZE` before borrowing State, addressing that specific trigger. Keep this invariant for every native main widget and child/editor surface.
- The moving flag is set before native resize processing and cleared on release/cancel/capture change; removal/rejoin of a middle member should break/rebuild the neighboring components. While moving, widget frost caches intentionally remain frozen; validate the bevel separately so the detached moving widget does not keep a missing old internal edge until release.
- No snap target latch or distinct entry/exit threshold is present. This is a design gap for hysteresis, rather than a proven crash/correctness failure. Test pointer jitter around the 12-DIP threshold and define release behavior before tuning it.
- Bridge z-order uses only HWND_TOPMOST/HWND_NOTOPMOST plus one member as owner, not an explicit insertion relative to the entire group/desktop band. Sequential live QA should cover a regular application over a non-floating fused group, task switching and topmost toggles; I did not claim a demonstrated z-order bug without running that scenario.

## Alpha / shadow checks that looked sound

- Main-window alpha masking scales premultiplied RGB and coverage together; the hit-test alpha floor remains inside nonzero coverage. Bridge shadow pixels use RGB=0, so their alpha is premultiplied correctly.
- The initial whole-component minimum-opacity shadow policy changed during this review to local distance-weighted member opacity (`refresh:147-161`). The earlier concern that one transparent member erases the entire opaque neighbor's shadow is therefore **not** a finding against the current snapshot. Still test local continuity and the overlapping-member alpha issue above; premultiplied validity alone cannot detect double compositing.

Suggested fix order: translucent seam ownership, bridge-failure fallback, scoped/cached refresh, unified mixed-DPI seam predicate; then chain detach/rejoin and native callback regression scenarios. Root owns all implementation changes.
