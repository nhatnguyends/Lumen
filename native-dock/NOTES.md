# Native Sticky Notes

Local Rust development prototype, hosted by the existing dock process and UI
thread. Electron remains the primary app entry point.

```text
python run_native_dock.py --notes
```

Open it again with the Notes toolbar button or **Show notes** in the tray. Close
the board with × or Ctrl+W; the dock stays running. An open board restores on the
next launch. Editing finishes with the small checkmark or Escape.

- New note: toolbar, Ctrl+N, or double-click empty board space.
- Search: search pill or Ctrl+F opens native text input; matching is case
  insensitive, including Vietnamese Unicode. × clears the query.
- Click a card and type directly into it. The native RichEdit control is a child
  of the board; no separate editor window opens. Bold, italic, underline,
  strike, bullet and numbered lists are available at the card foot; Ctrl+B/I/U/S
  formats the selection. Selection, IME, clipboard and editing undo are native.
  Paste inserts Unicode text, matching the old editor's plain-paste behavior.
- Top-left glass bead opens the seven-color picker, including Slate. The outline
  pin pins/unpins; the trash icon deletes.
  ↶ in the board toolbar restores the most recently deleted note this session.
- The common outline gear opens one separate settings window using the shared
  Dock settings shell, sliders, toggle switches and choice cards: Pinboard,
  Notebook, Index and Honey designs; frost/strength, blur, palette, opacity, tint and
  floating mode. Opening settings retains the content and inline editor.
  Custom Notes blur uses a local cache and never changes Dock/other widgets.
- The board can resize down to 240×240 DIP and keeps working search, creation,
  inline editing, formatting, scrolling and footer controls. Small layouts keep
  one compact card visible rather than overlapping toolbar/text controls.
- Drag the header; resize all edges/corners. Wheel uses the dock's smooth-scroll
  model and the scrollbar supports dragging. Wallpaper freezes during movement
  and refreshes at release. No continuous redraw timer runs when settled.
- Overflowing cards fade softly at the top and bottom as they scroll; complete
  first/last rows remain clear at the scroll stops. The brighter round New note
  control is 44 DIP, larger than the 32-DIP search pill. Footer text/actions sit
  inside the rounded corners with matching hit targets and a reserved safe area.

First launch imports existing `config/notes.json` (with legacy root/AppData
fallbacks) plus design/opacity preferences in `config/state.json`. Those files
are read-only inputs. Native notes, rich formatting, appearance, bounds and open
state are saved separately in `config/native-notes.json` using a synced temporary
file and replace. Corrupt or unsupported data produces an error; it is never
silently reset. Saving coalesces typing at 260 ms with a 1.2 s maximum delay,
retries failures and flushes on hiding/shutdown. Opening an unchanged note does
not rewrite its body or timestamp. The 400-note cap rejects creation without
discarding older notes.

The board uses the dock's shared wallpaper source, compositor, glass coating,
reflection, rounded alpha edges and shadow. Native child input handles stay in
the card/search bounds for real pointer selection, focus and IME positioning.
Their text and selection are composited onto glass with native text snapshots;
this avoids an opaque input rectangle over the card. A caret timer exists only
while native input has focus. Card order stays stable during typing and changes
only after finishing the edit.

The four content designs use a common readable type scale: 16-DIP body,
11-DIP dates/captions and 14-DIP search. Notebook has a more spacious line rhythm;
Index uses deep-green archive backing with dividers/numbering. Notebook has
a warm paper backdrop, a binding/fold, rules and a red margin. Honey combines
an amber cover/header with cream sheets and warm-white cards. Pinboard retains
the pastel glass sheen. Shared gradient surfaces or a precise custom color
change the cover/material independently of note colors; legacy flat choices
migrate to the equivalent custom hex. Paper content stays readable above it. Preview wrapping measures native glyph
widths and marks hidden lines with an ellipsis, retaining long Unicode tokens.
The font/line-layout cache is bounded and released on hiding. Typography follows
the legibility/hierarchy principles in Apple's UI guidance while retaining
Windows-native Segoe fonts and the common widget header.
Board previews are plain text; rich formatting is retained in the editor and
native file. Imported HTML lists retain bullet/number text; newly formatted
lists use RichEdit paragraph numbering. Emoji may render as missing glyphs in
the board's existing GDI+ font; the RichEdit round-trip preserves their Unicode.
Hover/scrollbar visibility currently switches immediately; smooth hover/fades
and full DPI/IME/desktop interaction QA remain follow-up work.

Model tests use temporary config folders. Native host/editor tests briefly open
isolated test windows and never write live notes; coordinate those tests when
other widget chats are doing desktop QA:

```text
cargo test --locked --manifest-path native-dock/Cargo.toml notes::model::
cargo test --locked --manifest-path native-dock/Cargo.toml notes:: -- --test-threads=1
node scripts/check-safe.mjs
```
