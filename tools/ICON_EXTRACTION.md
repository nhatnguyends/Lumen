# Python icon extraction

Lumen calls `icon_extract.py` through the Python interpreter passed down by
`run_lumen.py`. It uses only the Python standard library and Windows Shell/GDI
APIs, with no custom executable, .NET SDK, Pillow, or pip packages required.
The destination PC needs Python, just as the launcher does.

```text
python -I tools/icon_extract.py --size 96 "C:\Windows\explorer.exe"
python -I tools/icon_extract.py --size 96 "shell:AppsFolder\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"
python -I tools/icon_extract.py --size 256 "C:\Windows\System32\shell32.dll|-4"
```

Each argument produces one JSON line: `path` is the original argument and `png`
is a base64 RGBA PNG (empty if unavailable). Diagnostics use stderr. `--size`
accepts 16–1024 physical pixels, default 256. A `|index` suffix selects a resource;
negative indexes are resource IDs. Output preserves alpha and pads non-square
artwork. Requests cannot create detail missing from the original artwork.

The app selects a size bucket above icon DIP size × highest connected display
scale. Size is part of the cache key, and changing icon size or display scale
refreshes the selected cache entries. Store apps resolve relative to the
Applications known folder, avoiding generic file icons from Explorer aliases.

`python -I tools/icon_extract.py --list-programs` reads the current Windows
Applications catalog and emits one `{name, target}` JSON object per entry.
This supports installed program search; it does not enumerate user accounts.

The script reads requested icons or the Applications catalog and writes JSON to stdout. It has no
network client, registry startup changes, account discovery, keyboard hooks, or
synthetic input. Python runs in isolated mode (`-I`) without a command shell.
This follows `SECURITY.md` by avoiding a separately shipped unsigned icon helper;
it does not guarantee that antivirus heuristics will never flag the script.
