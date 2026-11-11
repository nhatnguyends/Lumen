# GitHub distribution and updates — proposed architecture

Status: the native portable **host updater is implemented** in
`native-dock/src/update.rs`; independent binary widget plugins remain a proposal.
User requested
updates for both Lumen and individual widgets on 2026-11-11. The user supplied
`https://github.com/nhatnguyends/Lumen.git` as the official repository.
The user subsequently authorized GitHub package publishing and maximum
portability. Native Rust ZIPs are built under `share/`, with a separate static-CRT
development target. Source launching remains supported; no installer is produced.

Implemented portable update flow: explicit Check, Download and Restart actions;
Windows HTTPS worker, GitHub asset digest validation, strict bounded ZIP/file
index validation, staged app-only replacement, local-edit detection, backups and
rollback on failed/interrupted replacement. A copied native helper waits for
the running host to exit. User config is outside the replaced file index.
The feed is not independently signed. Package APIs/independent widget updates
below describe future work, not a claim of existing plugin support.

## Official repository

- Source: https://github.com/nhatnguyends/Lumen
- Releases: https://github.com/nhatnguyends/Lumen/releases
- Host release metadata endpoint:
  https://api.github.com/repos/nhatnguyends/Lumen/releases/latest
- Official widget artifacts may share this repository with host releases; their
  catalog/versioning must remain independent. The tag/asset convention is still
  to be defined. Do not assume the latest host release is a widget's latest version.
- These are configured design destinations, not confirmation that releases or
  assets have been published or that the repository is publicly accessible.
- The current workspace belongs to the parent Git repository `D:/CustomTools`,
  whose origin is `nhatnguyends/CustomTools`. Preserve that remote. Publishing
  Lumen needs a dedicated checkout/export with a review of included files; do
  not push the parent repository or user config to the supplied destination.

## Current implementation and boundary

- `widgets/*/widget.json` describes six source packages.
- `native-dock/build.rs` compiles their entry modules and lifecycle factories into
  one Rust host. `Registry::discover` only exposes packages whose implementation
  is already compiled in. This is not a downloadable plugin loader.
- The Python source launcher rebuilds changed sources locally. Public users
  should eventually install a compatible prebuilt package without a Rust toolchain.
- Manager previews currently use central illustrative rendering selected by name.
  Distribution must move preview ownership into package metadata/assets.

## Proposed public package contract

Use a versioned `.lumen-widget` archive with a manifest, a static preview, package
assets, a license, and (for native code) an architecture-specific native module.
The extension is proposed; no handler exists today.

Manifest fields should include stable `id`, human-readable name/description,
package `version`, `schemaVersion`, compatible host API range, target OS/CPU,
entry point, preview asset, publisher identity, declared capabilities and update
repository/channel. Folder names, archive paths and IDs must be validated.

Keep source packages for development. Introduce a separate installed-package
manifest instead of silently treating the current `nativeEntry` source path as
a binary entry point. Preserve stable IDs and existing widget settings.

Native modules should expose an explicitly versioned C ABI, implemented in Rust,
with opaque handles and host-owned render/settings/window services. Do not pass
Rust traits, `String`, `Vec`, `Rc`, references or Rust allocator ownership across
the module boundary. Include struct sizes, API negotiation, destruction and
error semantics. Catch Rust panics at exported boundaries.

This can retain one native process and shared glass, rendering, settings, fuse
and wallpaper caches. It is an architectural proposal, not proof of isolation:
native plugin code has the host process's authority and a faulty plugin can
crash it. Capability declarations alone do not sandbox native code. Start with
explicitly trusted publishers; arbitrary third-party code needs a separate
isolation design before being advertised as safe.

Themes/assets may be data-only packages and need no executable entry point.

## Manager UX

- Installed cards use package-provided previews, toggles and separate Settings.
- Add widget supports choosing a local archive or a GitHub repository/release.
- Review publisher, version, compatibility, size and declared capabilities before
  installation. Installing an archive must not execute embedded install scripts.
- Cards show update availability, changelog, Update, and an error/retry state.
- Removing a widget offers to preserve its user data. Updates preserve data,
  geometry, theme, enabled state and stable identity.
- Discover installed metadata at launch and refresh the catalog after install,
  update or removal; filesystem watchers are optional, not a prerequisite.

## Two independent update flows

### Lumen host

Check a configured official GitHub Releases feed for the selected stable channel.
Offer Check for updates, installed/latest versions and release notes in General
Settings. Automatic checking may be enabled; installation should be explicit by
default. Check on a worker, cache results and avoid frame/timer-driven polling.

Stage a verified compatible host update separately from the running executable.
After user confirmation, save state and apply at a controlled restart. Windows
executable replacement needs a designed bootstrap/update mechanism; none is
implemented or authorized for packaging by this document. Preserve config and
retain a recoverable previous version. A failed start must not loop forever.

### Individual widget

Check the package's configured publisher/repository and installed version.
An update must match the widget ID and target architecture, and support the
installed host API. Show incompatible updates with the host version required.

Download to staging, verify, validate and prepare migrations before activation.
Use versioned directories and atomically switch the installed-version pointer.
Retain the previous version and restore it on activation/migration failure.
Keep user config outside versioned code directories, backing it up before any
non-reversible migration.

Initially apply native-code widget updates at the next Lumen restart. Loaded DLLs,
callbacks, HWND procedures, timers and workers make live unloading unsafe unless
the lifecycle contract explicitly drains them. Data-only assets can refresh
without restart where supported. Widget rollback and host rollback are separate.

## Release trust and download correctness

- GitHub Releases hosts artifacts; an app-specific catalog maps compatible
  versions/targets to those artifacts. The official repository is specified above.
- Verify artifact SHA-256 against authenticated release metadata. A hash served
  beside a file detects corruption but does not independently authenticate its
  publisher. Official distribution should add a pinned signing/trust mechanism
  with a planned key rotation and revocation strategy.
- Validate HTTPS origins/redirects, download/archive size limits, duplicate paths,
  traversal, absolute paths and links escaping the staging directory. Never run
  arbitrary archive scripts or interpret metadata as shell commands.
- Handle offline access, cancellation, partial downloads, GitHub rate limits and
  unavailable releases without disturbing installed versions or visibility.
- No GitHub login/token should be necessary for ordinary public downloads.

## Suggested implementation sequence

1. Extract app bootstrap/lifecycle orchestration from App Dock into the core host.
2. Define/test versioned package metadata, compatibility and preview ownership.
3. Design the Rust SDK/C ABI and validate one local plugin using development builds.
4. Implement a local installed-package store and discovery, including rollback.
5. Add GitHub release checks/download staging and separate host/widget update UX.
6. Once release constraints are explicitly revised,
   implement the publishing/signing/bootstrap pipeline. Do not publish yet.

Acceptance tests must cover incompatible API/CPU, wrong ID, tampered archives,
interrupted downloads, offline startup, failed migrations, locked modules,
rollback, config preservation, correct card previews and hidden-widget idle work.

## References

- GitHub release metadata/assets:
  https://docs.github.com/en/rest/releases/releases
- Rust ABI has no stability guarantees; use an explicit interoperable boundary:
  https://doc.rust-lang.org/reference/items/external-blocks.html#abi
