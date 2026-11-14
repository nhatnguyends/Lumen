
- Music khi fuse dùng material phủ kín frame, không bake góc bo riêng vào tint/coating; compositor chung quyết định contour của cụm.


### Native manager and startup — 2026-11-11
- Tray right-click offers only General Settings / Widget Manager and Quit Lumen.
- General Settings lists discovered widget packages as visual preview cards, each with a visibility toggle and separate Settings action. Previews are lightweight illustrations, not live widget instances.
- Start with Windows defaults on, including first launch through the source launcher. The per-user Lumen Run value invokes Pythonw and Launch Lumen.pyw; switching off removes only that value. Preserve an explicit off preference on subsequent launches.
- Floating-window switches are removed from every widget and General Settings. Old saved floating flags must not make normal widgets topmost. Detached settings remain accessible above desktop widgets.
<!-- Package distribution direction: see PACKAGE-DISTRIBUTION.md. User requested
GitHub downloads and independent updates for both Lumen and widgets. Official
repository: https://github.com/nhatnguyends/Lumen. Current source discovery is
not a downloadable binary plugin loader. -->


## Lumen manager and desktop stacking
- Public application/process name is Lumen (`lumen.exe` for source builds). Internal window classes and config filenames remain stable for compatibility.
- Widget manager uses two columns of compact preview cards at normal widths, falling back to one column below 520 DIP. Keep settings buttons and visibility toggles separate.
- General Settings has no Show/Hide All or global background-opacity control. Individual widget appearance remains in each widget settings window.
- Widget and fuse-decoration HWNDs stay in the desktop band. Intercept z-order changes synchronously instead of relying only on periodic correction. Keep keyboard input available for Dock search. Detached settings remain separate regular configuration windows.

- Widget shadows must remain behind their widget and other applications during movement; use unowned decoration windows in the desktop stacking guard.
- App Dock context menu: show Reset icon when an icon override exists; reset clears the override and rendered caches and restores the original target icon.

- App Dock: optional Sync Desktop items toggle (off by default). Mirror visible immediate children of Windows Desktop and Public Desktop, including redirected Desktop locations. Watch native non-recursive directory change notifications; check handles without blocking on the existing timer and reconcile only after notification. Never alter Desktop files. Keep manually pinned entries; toggling off removes only automatically managed entries and closes watcher handles. Reuse existing icon caches; defer updates while dragging.

- Fuse performance: skip bridge raster work behind fully covered pane interiors; reuse the computed contour distance for antialias coverage. Member coverage uses conservative interior strips with exact contour/antialiasing at edges. Keep matching fractional-DPI pixels and avoid changing join curvature or material opacity for speed.

## Liquid glass
- Optional **Liquid glass** toggle in General Settings (`Config::liquid_glass`, off by default) stored in global atomic state (`glass::set_liquid_glass` / `glass::liquid_glass`), bumping `LIQUID_GLASS_REVISION` inside `Glass::revision()` and triggering `Action::RefreshGlass` (`notify_wallpaper_changed`).
- **Curved normal-aligned bevel (`glass.rs`)**:
  - Perimeter band `rim = (44.0 * dpi).clamp(12.0, min_dim * 0.45)`, using a rounded-rect SDF (`rounded_rect_sdf`) with C¹-continuous interior corner normals (`t = (qy / (qx + qy)).clamp(0, 1)`) when `rim > radius` so corners have no 45° diagonal crease; fused widget groups use `fusion::geometry::with_outline` outer contour distance and skip `fusion::is_surface` bridges.
  - Smoothstep refraction curve `curve = u²(3 - 2u)`, `edge_align = 1 - curve`, inward displacement `disp = -bend * edge_align` (`bend = 22.0 * dpi`) along `(nx, ny)`, plus `draw_layer` shift `6.0 * dpi` so the background stays inside the sampled region without clamping straight streaks.
  - **Slider-proportional rim blur falloff**: sample a lightly blurred `rim_source` (`0.25 * blur`, padded by `44 * dpi`) alongside `frosted` (`1.0 * blur`), blending with a steeper blur-only falloff `blur_t = (u * 2.2).clamp(0, 1)`, `clarity = 0.75 * (1 - smoothstep(blur_t))`.
  - **2D-blurred chromatic aberration & specular rim**: `ca_u = (u * 2.2).clamp(0, 1)`, `ca_s = smoothstep(ca_u)`, `ca = 4 * ca_s * (1 - ca_s) * ca_strength` (`ca_strength = 5.5 * dpi`), evaluated with a 5-tap radial+tangential spread (`sample_disp_2d`) on `frosted` (`1.0x`) and `rim_source` (`0.35x`); subtle directional specular highlight for `u < 0.32`.
- **Widget & sub-surface integration**:
  - Clock (`widgets/clock/native/mod.rs`) calls `glass::apply_liquid_glass_rgba` in `draw_frost` using a padded `0.25 * blur` `rim_source`.
  - Music Coverflow album cards (`widgets/music/native/coverflow.rs`) apply `glass::apply_liquid_glass_premul_bgra` before drawing cover art (preserving premultiplied alpha and keying `card_cache` on `liquid_glass()`).
  - Sub-cards and bottom transport/control pills (`compositor::Buffer::restore_glass_card`) apply `glass::refract_bgra_card_pixel` against the owned background buffer when `liquid_glass()` is enabled.
