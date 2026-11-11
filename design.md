
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
