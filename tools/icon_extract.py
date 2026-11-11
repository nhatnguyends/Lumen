"""Local Windows icon extraction. Standard-library Python; no helper executable.

Prints one {path, png} JSON object per input. Only supplied shell items/resources
are read. PNGs are returned on stdout; no network, persistence or input hooks.
"""
from __future__ import annotations

import argparse
import base64
import ctypes as C
from ctypes import wintypes as W
import json
import os
import struct
import sys
import uuid
import zlib


class GUID(C.Structure):
    _fields_ = [("data", C.c_ubyte * 16)]

    def __init__(self, value):
        super().__init__()
        self.data[:] = uuid.UUID(value).bytes_le


class SIZE(C.Structure):
    _fields_ = [("cx", W.LONG), ("cy", W.LONG)]


class BITMAP(C.Structure):
    _fields_ = [("type", W.LONG), ("width", W.LONG), ("height", W.LONG),
                ("stride", W.LONG), ("planes", W.WORD), ("bits", W.WORD),
                ("pixels", C.c_void_p)]


class INFO(C.Structure):
    _fields_ = [("size", W.DWORD), ("width", W.LONG), ("height", W.LONG),
                ("planes", W.WORD), ("bits", W.WORD), ("compression", W.DWORD),
                ("image_size", W.DWORD), ("xppm", W.LONG), ("yppm", W.LONG),
                ("used", W.DWORD), ("important", W.DWORD), ("color", W.DWORD)]


def bind(library, name, result, *arguments):
    function = getattr(library, name)
    function.restype, function.argtypes = result, arguments
    return function


shell = C.WinDLL("shell32", use_last_error=True)
gdi = C.WinDLL("gdi32", use_last_error=True)
user = C.WinDLL("user32", use_last_error=True)
ole = C.OleDLL("ole32")
create_item = bind(shell, "SHCreateItemFromParsingName", W.LONG,
                   W.LPCWSTR, C.c_void_p, C.POINTER(GUID), C.POINTER(C.c_void_p))
create_app = bind(shell, "SHCreateItemInKnownFolder", W.LONG,
                  C.POINTER(GUID), W.DWORD, W.LPCWSTR, C.POINTER(GUID), C.POINTER(C.c_void_p))
get_object = bind(gdi, "GetObjectW", C.c_int, W.HANDLE, C.c_int, C.POINTER(BITMAP))
create_dc = bind(gdi, "CreateCompatibleDC", W.HDC, W.HDC)
delete_dc = bind(gdi, "DeleteDC", W.BOOL, W.HDC)
delete_object = bind(gdi, "DeleteObject", W.BOOL, W.HANDLE)
get_bits = bind(gdi, "GetDIBits", C.c_int, W.HDC, W.HBITMAP, W.UINT, W.UINT,
                C.c_void_p, C.POINTER(INFO), W.UINT)
create_dib = bind(gdi, "CreateDIBSection", W.HBITMAP, W.HDC, C.POINTER(INFO),
                  W.UINT, C.POINTER(C.c_void_p), W.HANDLE, W.DWORD)
select_object = bind(gdi, "SelectObject", W.HANDLE, W.HDC, W.HANDLE)
flush_gdi = bind(gdi, "GdiFlush", W.BOOL)
draw_icon = bind(user, "DrawIconEx", W.BOOL, W.HDC, C.c_int, C.c_int, W.HICON,
                 C.c_int, C.c_int, W.UINT, W.HBRUSH, W.UINT)
extract_icons = bind(user, "PrivateExtractIconsW", W.UINT, W.LPCWSTR, C.c_int,
                     C.c_int, C.c_int, C.POINTER(W.HICON), C.POINTER(W.UINT), W.UINT, W.UINT)
destroy_icon = bind(user, "DestroyIcon", W.BOOL, W.HICON)
initialize = bind(ole, "CoInitializeEx", W.LONG, C.c_void_p, W.DWORD)
uninitialize = bind(ole, "CoUninitialize", None)
free_task_memory = bind(ole, "CoTaskMemFree", None, C.c_void_p)


def com_method(pointer, slot, result, *arguments):
    table = C.cast(pointer, C.POINTER(C.POINTER(C.c_void_p))).contents
    return C.WINFUNCTYPE(result, C.c_void_p, *arguments)(table[slot])


def bitmap_info(width, height):
    info = INFO()
    info.size = 40
    info.width, info.height, info.planes, info.bits = width, -height, 1, 32
    return info


def shell_pixels(source, size):
    factory, bitmap = C.c_void_p(), W.HBITMAP()
    iid = GUID("bcc18b79-ba16-442f-80c4-8a59c30c463b")
    try:
        prefix = "shell:AppsFolder\\"
        if source.lower().startswith(prefix.lower()):
            folder = GUID("1e87508d-89c2-42f0-8a7e-645a0f50ca58")
            hr = create_app(C.byref(folder), 0, source[len(prefix):], C.byref(iid), C.byref(factory))
        else:
            hr = create_item(source, None, C.byref(iid), C.byref(factory))
        if hr < 0 or not factory:
            return None
        get_image = com_method(factory, 3, W.LONG, SIZE, C.c_int, C.POINTER(W.HBITMAP))
        if get_image(factory, SIZE(size, size), 0x5, C.byref(bitmap)) < 0 or not bitmap:
            return None
        native = BITMAP()
        if not get_object(bitmap, C.sizeof(native), C.byref(native)):
            return None
        width, height = native.width, abs(native.height)
        if not (0 < width <= 4096 and 0 < height <= 4096):
            return None
        info = bitmap_info(width, height)
        pixels = (C.c_ubyte * (width * height * 4))()
        dc = create_dc(None)
        if not dc:
            return None
        try:
            if get_bits(dc, bitmap, 0, height, pixels, C.byref(info), 0) != height:
                return None
        finally:
            delete_dc(dc)
        return width, height, bytes(pixels)
    finally:
        if bitmap:
            delete_object(bitmap)
        if factory:
            com_method(factory, 2, W.ULONG)(factory)


def resource_pixels(source, index, size):
    """Render the requested resource on black/white to preserve legacy masks."""
    icon, resource_id = W.HICON(), W.UINT()
    count = extract_icons(source, index, size, size, C.byref(icon), C.byref(resource_id), 1, 0)
    if count in (0, 0xFFFFFFFF) or not icon:
        return None
    try:
        backgrounds = []
        for value in (0, 255):
            dc = create_dc(None)
            if not dc:
                return None
            bitmap, previous = None, None
            try:
                pointer = C.c_void_p()
                info = bitmap_info(size, size)
                bitmap = create_dib(dc, C.byref(info), 0, C.byref(pointer), None, 0)
                if not bitmap or not pointer:
                    return None
                previous = select_object(dc, bitmap)
                C.memset(pointer, value, size * size * 4)
                if not draw_icon(dc, 0, 0, icon, size, size, 0, None, 3):
                    return None
                flush_gdi()
                backgrounds.append(C.string_at(pointer, size * size * 4))
            finally:
                if previous:
                    select_object(dc, previous)
                if bitmap:
                    delete_object(bitmap)
                delete_dc(dc)
        black, white = backgrounds
        pixels = bytearray(black)
        for offset in range(0, len(pixels), 4):
            pixels[offset + 3] = 255 - max(0, *(white[offset+c] - black[offset+c] for c in range(3)))
        return size, size, pixels
    finally:
        destroy_icon(icon)


def png_chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))


def encode_png(width, height, pixels, premultiplied=False):
    """Pad to a square, preserving the shell's straight-alpha BGRA pixels.

    Only the HICON black/white compositor returns premultiplied channels.
    Unpremultiplying shell DIB pixels a second time creates bright white edges.
    """
    side = max(width, height)
    rows = bytearray(side * (1 + side * 4))
    left, top = (side - width) // 2, (side - height) // 2
    for y in range(height):
        for x in range(width):
            source = (y * width + x) * 4
            dest = (y + top) * (1 + side * 4) + 1 + (x + left) * 4
            b, g, r, alpha = pixels[source:source+4]
            if alpha:
                if premultiplied:
                    r, g, b = (min(255, channel * 255 // alpha) for channel in (r, g, b))
                rows[dest:dest+4] = bytes((r, g, b, alpha))
    header = struct.pack(">IIBBBBB", side, side, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + png_chunk(b"IHDR", header) + png_chunk(b"IDAT", zlib.compress(rows)) + png_chunk(b"IEND", b"")


def extract(source, size):
    path, separator, resource = source.rpartition("|")
    if separator:
        try:
            pixels = resource_pixels(os.path.expandvars(path), int(resource), size)
        except ValueError:
            pixels = None
    else:
        pixels = shell_pixels(os.path.expandvars(source), size)
    return encode_png(*pixels, premultiplied=bool(separator)) if pixels else b""


def installed_programs():
    """Enumerate the current desktop's Applications folder, not user accounts."""
    folder, enumerator = C.c_void_p(), C.c_void_p()
    folder_id = GUID("1e87508d-89c2-42f0-8a7e-645a0f50ca58")
    item_iid = GUID("43826d1e-e718-42ee-bc55-a1e261c37bfe")
    handler = GUID("94f60519-2850-4924-aa5a-d15e84868039")
    enum_iid = GUID("70629033-e363-4a28-a567-0db78006e6d7")
    try:
        create_app(C.byref(folder_id), 0, None, C.byref(item_iid), C.byref(folder))
        if not folder:
            raise RuntimeError("Applications folder unavailable")
        bind_handler = com_method(folder, 3, W.LONG, C.c_void_p, C.POINTER(GUID),
                                  C.POINTER(GUID), C.POINTER(C.c_void_p))
        hr = bind_handler(folder, None, C.byref(handler), C.byref(enum_iid), C.byref(enumerator))
        if hr < 0 or not enumerator:
            raise RuntimeError("Applications catalog unavailable")
        next_item = com_method(enumerator, 3, W.LONG, W.ULONG, C.POINTER(C.c_void_p), C.POINTER(W.ULONG))
        while True:
            item, fetched = C.c_void_p(), W.ULONG()
            hr = next_item(enumerator, 1, C.byref(item), C.byref(fetched))
            if hr != 0 or not fetched.value:
                break
            try:
                values = []
                get_name = com_method(item, 5, W.LONG, W.UINT, C.POINTER(C.c_void_p))
                for flag in (0, 0x80018001):  # display name; parent-relative parsing name
                    text = C.c_void_p()
                    try:
                        if get_name(item, flag, C.byref(text)) < 0 or not text:
                            break
                        values.append(C.wstring_at(text))
                    finally:
                        if text:
                            free_task_memory(text)
                if len(values) == 2 and all(values):
                    yield {"name": values[0], "target": values[1]}
            finally:
                com_method(item, 2, W.ULONG)(item)
    finally:
        for pointer in (enumerator, folder):
            if pointer:
                com_method(pointer, 2, W.ULONG)(pointer)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--size", type=int, default=256)
    parser.add_argument("--list-programs", action="store_true", help="list Windows Applications catalog")
    parser.add_argument("paths", nargs="*")
    args = parser.parse_args()
    if not 16 <= args.size <= 1024:
        parser.error("size must be between 16 and 1024 physical pixels")
    initialize(None, 2)
    try:
        if args.list_programs:
            for program in installed_programs():
                print(json.dumps(program), flush=True)
            return
        for source in args.paths:
            try:
                png = extract(source, args.size)
            except Exception as error:
                print(f"Icon extraction failed: {error}", file=sys.stderr)
                png = b""
            print(json.dumps({"path": source, "png": base64.b64encode(png).decode("ascii")}), flush=True)
    finally:
        uninitialize()


if __name__ == "__main__":
    main()
