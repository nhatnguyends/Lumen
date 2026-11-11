"""Windows integration checks for the Python icon helper; no third-party packages."""
import base64
import json
from pathlib import Path
import struct
import subprocess
import sys
import zlib
import runpy

ROOT = Path(__file__).resolve().parent.parent
HELPER = ROOT / "tools" / "icon_extract.py"


def invoke(size, paths):
    return subprocess.run([sys.executable, "-I", str(HELPER), "--size", str(size),
                           "--", *paths], capture_output=True, check=True, timeout=20)


def decode_png(data):
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    width, height = struct.unpack(">II", data[16:24])
    assert data[24:26] == bytes((8, 6)), "Expected 8-bit RGBA"
    offset, compressed = 8, bytearray()
    while offset < len(data):
        length = struct.unpack(">I", data[offset:offset+4])[0]
        kind, payload = data[offset+4:offset+8], data[offset+8:offset+8+length]
        crc = struct.unpack(">I", data[offset+8+length:offset+12+length])[0]
        assert zlib.crc32(kind + payload) == crc
        if kind == b"IDAT":
            compressed.extend(payload)
        offset += length + 12
    rows = zlib.decompress(compressed)
    stride = 1 + width * 4
    assert len(rows) == stride * height
    assert all(rows[y*stride] == 0 for y in range(height))
    alpha = [rows[y*stride+4+x*4] for y in range(height) for x in range(width)]
    # Windows' scaled legacy HICON compositing can round full opacity to 254.
    assert min(alpha) == 0 and max(alpha) >= 250, "Transparency or visible pixels missing"
    return width, height, rows


def main():
    helper = runpy.run_path(str(HELPER))
    # The shell returns straight-alpha colors; bright edge pixels must not be
    # divided by alpha again. Legacy HICON compositing still needs conversion.
    source = bytes((50, 100, 200, 128, 0, 0, 0, 0, 25, 50, 100, 255))
    _, _, rows = decode_png(helper['encode_png'](3, 1, source))
    assert rows[14:18] == bytes((200, 100, 50, 128)), 'Shell edge colors changed'
    _, _, rows = decode_png(helper['encode_png'](3, 1, source, premultiplied=True))
    assert rows[14:18] == bytes((255, 199, 99, 128)), 'Legacy alpha conversion lost'
    windows = Path(__import__("os").environ["SystemRoot"])
    paths = [str(windows / "explorer.exe"), str(ROOT / "build" / "icon.ico"),
             "shell:AppsFolder\\windows.immersivecontrolpanel_cw5n1h2txyewy!microsoft.windows.immersivecontrolpanel",
             "shell:AppsFolder\\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App",
             str(windows / "System32" / "shell32.dll") + "|0",
             str(windows / "System32" / "shell32.dll") + "|-4"]
    count = 0
    for size in (48, 96, 256):
        rows = [json.loads(line) for line in invoke(size, paths).stdout.decode().splitlines()]
        assert [row["path"] for row in rows] == paths
        artwork = []
        for row in rows:
            width, height, pixels = decode_png(base64.b64decode(row["png"]))
            assert width == height and width >= size
            artwork.append(pixels)
            count += 1
        assert artwork[2] != artwork[3], "Store app artwork must differ"
        assert artwork[4] != artwork[5], "Resource indexes must be preserved"
    missing = str(ROOT / ".scratch" / "missing-icon-file.exe")
    assert not Path(missing).exists()
    assert json.loads(invoke(96, [missing]).stdout)["png"] == ""
    programs = subprocess.run([sys.executable, '-I', str(HELPER), '--list-programs'],
                              capture_output=True, check=True, timeout=20)
    programs = [json.loads(line) for line in programs.stdout.decode().splitlines()]
    assert programs and all(row.get('name') and row.get('target') for row in programs)
    for size in (0, 1025):
        try:
            invoke(size, paths[:1])
        except subprocess.CalledProcessError as error:
            assert error.returncode == 2
        else:
            raise AssertionError("Invalid size accepted")
    print(f"PASS: {count} icons; edge colors, PNG CRC, alpha, sizes, Store apps, resource indexes; {len(programs)} catalog entries")


if __name__ == "__main__":
    main()
