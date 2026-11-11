"""Windows quick controls, using only ctypes and the standard library.

Read-only unless a JSON command is supplied. Display toggles detach/attach a
desktop display (not DDC power). The primary display can never be detached.
Native layouts follow wingdi.h; COM interfaces follow mmdeviceapi.h and
endpointvolume.h. PolicyConfig is an optional Windows interface: failures are
reported instead of emulating keyboard input.
"""
import ctypes as C
from ctypes import wintypes as W
import json
import struct
import sys
import uuid
from pathlib import Path

DISPLAY_CACHE_PATH = Path(__file__).resolve().parents[1] / "config/display-recovery.json"

P = C.c_void_p
user = C.WinDLL('user32', use_last_error=True)
ole = C.OleDLL('ole32')
user.EnumDisplayDevicesW.argtypes = [W.LPCWSTR, W.DWORD, P, W.DWORD]
user.EnumDisplaySettingsW.argtypes = [W.LPCWSTR, W.DWORD, P]
user.ChangeDisplaySettingsExW.argtypes = [W.LPCWSTR, P, P, W.DWORD, P]
ole.CoCreateInstance.argtypes = [P, P, W.DWORD, P, C.POINTER(P)]
ole.CoTaskMemFree.argtypes = [P]


def guid(value):
    return (C.c_ubyte * 16).from_buffer_copy(uuid.UUID(value).bytes_le)


def call(obj, index, types=(), args=()):
    table = C.cast(obj, C.POINTER(C.POINTER(P))).contents
    method = C.WINFUNCTYPE(C.c_long, P, *types)(table[index])
    hr = method(obj, *args)
    if hr < 0:
        raise RuntimeError('Windows audio API failed (0x%08X)' % (hr & 0xffffffff))


def release(obj):
    if obj:
        call(obj, 2)


def create(clsid, iid):
    obj = P()
    ole.CoCreateInstance(guid(clsid), None, 23, guid(iid), C.byref(obj))
    return obj


def device_id(device):
    value = P()
    call(device, 5, (C.POINTER(P),), (C.byref(value),))
    try:
        return C.wstring_at(value)
    finally:
        ole.CoTaskMemFree(value)


def audio(command, flow=0):
    # eRender=0 (speakers), eCapture=1 (microphones). Never open an audio stream.
    selector, volume_kind, mute_kind = ('microphone', 'mic-volume', 'mic-mute') if flow == 1 else ('audio', 'volume', 'mute')
    endpoint_name = 'microphone' if flow == 1 else 'audio output'
    if command and command['kind'] not in (selector, volume_kind, mute_kind):
        raise ValueError('Control does not match the audio endpoint direction')
    enumerator = create('bcde0395-e52f-467c-8e3d-c4579291692e', 'a95664d2-9614-4f35-a746-de8db63617e6')
    collection, default, volume = P(), P(), P()
    outputs = []
    try:
        try:
            call(enumerator, 4, (C.c_int, C.c_int, C.POINTER(P)), (flow, 1, C.byref(default)))
        except RuntimeError:
            pass  # No default endpoint is a valid state.
        selected = device_id(default) if default else ''
        call(enumerator, 3, (C.c_int, W.DWORD, C.POINTER(P)), (flow, 1, C.byref(collection)))
        count = W.UINT()
        call(collection, 3, (C.POINTER(W.UINT),), (C.byref(count),))
        for i in range(count.value):
            device, store = P(), P()
            try:
                call(collection, 4, (W.UINT, C.POINTER(P)), (i, C.byref(device)))
                identity = device_id(device)
                call(device, 4, (W.DWORD, C.POINTER(P)), (0, C.byref(store)))
                key = C.create_string_buffer(bytes(guid('a45c254e-df1c-4efd-8020-67d146a850e0')) + struct.pack('<I', 14))
                variant = C.create_string_buffer(24)
                call(store, 5, (P, P), (key, variant))
                try:
                    name = C.wstring_at(P.from_buffer(variant, 8).value) if struct.unpack_from('<H', variant)[0] == 31 else identity
                finally:
                    ole.PropVariantClear(variant)
                outputs.append(dict(id=identity, name=name, selected=identity == selected))
            finally:
                release(store)
                release(device)
        if command and command['kind'] == selector:
            if command['target'] not in [d['id'] for d in outputs]:
                raise RuntimeError(endpoint_name.capitalize() + ' is no longer connected')
            policy = create('870af99c-171d-4f9e-af0d-e63df40c2bc9', 'f8679f50-850a-41cf-9c72-430f290290c8')
            try:
                for role in (0, 1, 2):
                    call(policy, 13, (W.LPCWSTR, C.c_int), (command['target'], role))
            finally:
                release(policy)
            return outputs, None, False  # Caller takes a fresh snapshot of the new default.
        if not default:
            if command and command['kind'] in (volume_kind, mute_kind):
                raise RuntimeError('No default ' + endpoint_name + ' is available')
            return outputs, None, False
        call(default, 3, (P, W.DWORD, P, C.POINTER(P)), (guid('5cdf2c82-841e-4546-9722-0cf74078229a'), 23, None, C.byref(volume)))
        if command and command['kind'] == volume_kind:
            call(volume, 7, (C.c_float, P), (command['value'] / 100, None))
        if command and command['kind'] == mute_kind:
            call(volume, 14, (W.BOOL, P), (command['value'], None))
        level, muted = C.c_float(), W.BOOL()
        call(volume, 9, (C.POINTER(C.c_float),), (C.byref(level),))
        call(volume, 15, (C.POINTER(W.BOOL),), (C.byref(muted),))
        return outputs, round(level.value * 100), bool(muted.value)
    finally:
        for obj in (volume, default, collection, enumerator):
            release(obj)


class DisplayDevice(C.Structure):
    _fields_ = [('cb', W.DWORD), ('name', W.WCHAR * 32), ('description', W.WCHAR * 128),
                ('flags', W.DWORD), ('deviceID', W.WCHAR * 128), ('key', W.WCHAR * 128)]


def mode(name, registry=False):
    value = C.create_string_buffer(220)
    struct.pack_into('<H', value, 68, 220)
    if not user.EnumDisplaySettingsW(name, 0xfffffffe if registry else 0xffffffff, value):
        raise RuntimeError('Windows could not read this display mode')
    return value


def display_list():
    result = []
    for i in range(64):
        d = DisplayDevice()
        d.cb = C.sizeof(d)
        if not user.EnumDisplayDevicesW(None, i, C.byref(d), 0):
            break
        if d.flags & 8:  # Skip mirroring pseudo-devices.
            continue
        monitor = DisplayDevice()
        monitor.cb = C.sizeof(monitor)
        has_monitor = user.EnumDisplayDevicesW(d.name, 0, C.byref(monitor), 0)
        if not has_monitor and not d.flags & 1:
            try:
                saved = mode(d.name, registry=True)
                width, height = struct.unpack_from('<II', saved, 172)
                if not width or not height:
                    continue
            except RuntimeError:
                continue
        result.append(dict(id=d.name, name=monitor.description or d.name.replace('\\\\.\\', '') + ' · ' + d.description,
                           enabled=bool(d.flags & 1), primary=bool(d.flags & 4), hdrSupported=False, hdrEnabled=False))
    return result


def packet(kind, size, adapter, target):
    value = C.create_string_buffer(size)
    struct.pack_into('<II8sI', value, 0, kind, size, adapter, target)
    return value


def hdr_targets():
    # DISPLAYCONFIG_PATH_INFO=72, DISPLAYCONFIG_MODE_INFO=64, header=20.
    for _ in range(4):
        pc, mc = W.UINT(), W.UINT()
        code = user.GetDisplayConfigBufferSizes(2, C.byref(pc), C.byref(mc))
        if code:
            raise RuntimeError('Windows display query failed (%s)' % code)
        paths, modes = C.create_string_buffer(pc.value * 72), C.create_string_buffer(mc.value * 64)
        code = user.QueryDisplayConfig(2, C.byref(pc), paths, C.byref(mc), modes, None)
        if code == 122:
            continue
        if code:
            raise RuntimeError('Windows display query failed (%s)' % code)
        result = {}
        for i in range(pc.value):
            raw = paths.raw[i * 72:(i + 1) * 72]
            source = packet(1, 84, raw[:8], struct.unpack_from('<I', raw, 8)[0])
            if user.DisplayConfigGetDeviceInfo(source):
                continue
            name = C.wstring_at(C.addressof(source) + 20)
            adapter, target = raw[20:28], struct.unpack_from('<I', raw, 28)[0]
            info = packet(15, 36, adapter, target)
            modern = user.DisplayConfigGetDeviceInfo(info) == 0
            if not modern:
                info = packet(9, 32, adapter, target)
                if user.DisplayConfigGetDeviceInfo(info):
                    continue
            flags = struct.unpack_from('<I', info, 20)[0]
            supported = bool(flags & 16) if modern else bool(flags & 1) and not bool(flags & 12)
            enabled = bool(flags & 32) if modern else bool(flags & 2)
            target_name = packet(2, 420, adapter, target)
            friendly = ''
            if user.DisplayConfigGetDeviceInfo(target_name) == 0:
                friendly = C.wstring_at(C.addressof(target_name) + 36)
            result.setdefault(name, []).append((adapter, target, supported, enabled, modern, friendly))
        return result
    raise RuntimeError('Display layout is changing; try again')



def display_query(flags):
    for _ in range(4):
        pc, mc = W.UINT(), W.UINT()
        code = user.GetDisplayConfigBufferSizes(flags, C.byref(pc), C.byref(mc))
        if code or pc.value > 512 or mc.value > 4096:
            raise RuntimeError('Display query failed (%s)' % code)
        paths, modes = C.create_string_buffer(pc.value * 72), C.create_string_buffer(mc.value * 64)
        code = user.QueryDisplayConfig(flags, C.byref(pc), paths, C.byref(mc), modes, None)
        if code == 122:
            continue
        if code:
            raise RuntimeError('Display query failed (%s)' % code)
        return [paths.raw[i*72:(i+1)*72] for i in range(pc.value)], modes.raw[:mc.value*64]
    raise RuntimeError('Display topology is changing; try again')


def display_cache():
    try:
        saved = json.loads(DISPLAY_CACHE_PATH.read_text(encoding='utf-8'))
        return saved if isinstance(saved, dict) and len(saved) <= 64 else {}
    except (OSError, ValueError):
        return {}


def save_display_cache(saved):
    DISPLAY_CACHE_PATH.parent.mkdir(parents=True, exist_ok=True)
    temp = DISPLAY_CACHE_PATH.with_suffix('.json.tmp')
    temp.write_text(json.dumps(saved, ensure_ascii=True), encoding='utf-8')
    temp.replace(DISPLAY_CACHE_PATH)


def display_connections():
    paths, modes = display_query(1)
    used = {p[:12] for p in paths if struct.unpack_from('<I', p, 68)[0] & 1}
    result = {}
    for raw in paths:
        active = bool(struct.unpack_from('<I', raw, 68)[0] & 1)
        if not struct.unpack_from('<I', raw, 60)[0] or (not active and raw[:12] in used):
            continue
        target = packet(2, 420, raw[20:28], struct.unpack_from('<I', raw, 28)[0])
        source = packet(1, 84, raw[:8], struct.unpack_from('<I', raw, 8)[0])
        if user.DisplayConfigGetDeviceInfo(target) or user.DisplayConfigGetDeviceInfo(source):
            continue
        monitor = C.wstring_at(C.addressof(target) + 164)
        source_name = C.wstring_at(C.addressof(source) + 20)
        name = C.wstring_at(C.addressof(target) + 36)
        index = struct.unpack_from('<I', raw, 12)[0]
        current = None
        if index * 64 + 64 <= len(modes) and struct.unpack_from('<I', modes, index * 64)[0] == 1:
            width, height, _, x, y = struct.unpack_from('<IIIii', modes, index * 64 + 16)
            current = dict(width=width, height=height, x=x, y=y)
        link = dict(monitor_path=monitor, source=source_name, name=name, active=active, raw=raw, mode=current)
        if monitor not in result or active:
            result[monitor] = link
    return list(result.values())


def remember_display(saved, identity, link):
    current = link['mode']
    if current is None:
        try:
            raw = mode(link['source'], registry=True)
            width, height = struct.unpack_from('<II', raw, 172)
            x, y = struct.unpack_from('<ii', raw, 76)
            if width and height:
                current = dict(width=width, height=height, x=x, y=y)
        except RuntimeError:
            pass
    value = dict(saved.get(identity, {}))
    value.update(monitor_path=link['monitor_path'], source=link['source'], name=link['name'])
    if current:
        value.update(current)
    if value != saved.get(identity):
        saved[identity] = value
        save_display_cache(saved)


def physical_displays(fallback):
    saved = display_cache()
    links = display_connections()
    if not links:
        return fallback
    primary = {d['id'] for d in fallback if d['primary']}
    result = []
    for link in links:
        identity = next((k for k, v in saved.items() if v.get('monitor_path') == link['monitor_path']), None)
        identity = identity or (link['source'] if link['source'] not in saved else 'ccd:' + link['monitor_path'])
        try:
            remember_display(saved, identity, link)
        except OSError:
            pass  # Reading devices remains available on a read-only config.
        result.append(dict(id=identity, name=link['name'], enabled=link['active'], primary=link['active'] and link['source'] in primary, hdrSupported=False, hdrEnabled=False))
    return result


def resolve_display(command, links, saved):
    binding = saved.get(command['target'], {})
    if binding.get('monitor_path'):
        return next((l for l in links if l['monitor_path'] == binding['monitor_path']), None)
    label = command.get('label', '')
    return next((l for l in links if l['name'] and l['name'] in label), None) or next((l for l in links if l['source'] == command['target']), None)


def restore_plan(active, modes, path, saved):
    new = bytearray(path)
    struct.pack_into('<I', new, 68, 1)
    struct.pack_into('<I', new, 12, 0xffffffff)
    struct.pack_into('<I', new, 32, 0xffffffff)
    if saved and 0 < saved.get('width', 0) <= 32768 and 0 < saved.get('height', 0) <= 32768:
        source = bytearray(64)
        struct.pack_into('<II8sIIIii', source, 0, 1, struct.unpack_from('<I', path, 8)[0], path[:8], saved['width'], saved['height'], 4, saved['x'], saved['y'])
        struct.pack_into('<I', new, 12, len(modes) // 64)
        modes += source
    return b''.join(active) + new, modes


def reattach_display(command):
    saved = display_cache()
    link = resolve_display(command, display_connections(), saved)
    if not link:
        raise RuntimeError('Display is no longer connected')
    if link['active']:
        return
    remember_display(saved, command['target'], link)
    active, modes = display_query(2)
    all_paths, _ = display_query(1)
    used = {p[:12] for p in active}
    choices = [p for p in all_paths if p[20:32] == link['raw'][20:32] and struct.unpack_from('<I', p, 60)[0] and p[:12] not in used]
    choices.sort(key=lambda p: p[:12] != link['raw'][:12])
    last = 87
    for path in choices:
        paths, new_modes = restore_plan(active, modes, path, saved.get(command['target']))
        pbuf, mbuf = C.create_string_buffer(paths), C.create_string_buffer(new_modes)
        code = user.SetDisplayConfig(len(active)+1, pbuf, len(new_modes)//64, mbuf, 0x460)
        if code:
            last = code
            continue
        code = user.SetDisplayConfig(len(active)+1, pbuf, len(new_modes)//64, mbuf, 0x4a0)
        if code:
            last = code
            continue
        if any(l['monitor_path'] == link['monitor_path'] and l['active'] for l in display_connections()):
            return
        raise RuntimeError('Windows did not activate the requested monitor')
    raise RuntimeError('Windows could not reattach the monitor (%s)' % last)


def displays(command, errors=None):
    result = display_list()
    try:
        result = physical_displays(result)
    except RuntimeError:
        pass
    try:
        targets = hdr_targets()
    except Exception as error:
        if command and command['kind'] == 'hdr':
            raise
        targets = {}
        if errors is not None:
            errors.append('HDR: ' + str(error))
    hdr_entries = {}
    for d in result:
        binding = display_cache().get(d['id'], {})
        source = next((l['source'] for l in display_connections() if l['monitor_path'] == binding.get('monitor_path')), d['id']) if binding else d['id']
        entries = targets.get(source, [])
        hdr_entries[d['id']] = entries
        d['hdrSupported'] = bool(entries) and all(e[2] for e in entries)
        d['hdrEnabled'] = bool(entries) and all(e[3] for e in entries)
        if entries and entries[0][5]:
            d['name'] = entries[0][5] + ' · ' + d['id'].replace('\\\\.\\', '')
    if not command or command['kind'] not in ('display', 'hdr'):
        return result
    d = next((d for d in result if d['id'] == command['target']), None)
    if not d:
        raise RuntimeError('Display is no longer connected')
    if command['kind'] == 'display':
        enabled = command['value']
        if enabled == d['enabled']:
            return result
        if not enabled and (d['primary'] or sum(x['enabled'] for x in result) <= 1):
            raise RuntimeError('Keep the primary display enabled')
        if enabled:
            reattach_display(command)
            return result
        saved = display_cache()
        link = resolve_display(command, display_connections(), saved)
        if not link:
            raise RuntimeError('Display is no longer connected')
        remember_display(saved, command['target'], link)
        source = link['source']
        value = mode(source)
        if not enabled:
            struct.pack_into('<I', value, 72, 0x180020)  # POSITION | PELSWIDTH | PELSHEIGHT
            struct.pack_into('<II', value, 172, 0, 0)
        # Do not overwrite the registry: it retains the mode used to reattach.
        code = user.ChangeDisplaySettingsExW(source, value, None, 0, None)
        if code != 0:
            raise RuntimeError('Windows rejected the display change (%s)' % code)
    else:
        if not d['enabled'] or not d['hdrSupported']:
            raise RuntimeError('HDR is unavailable for this display')
        for adapter, target, _, _, modern, _ in hdr_entries[d['id']]:
            value = packet(16 if modern else 10, 24, adapter, target)
            struct.pack_into('<I', value, 20, int(command['value']))
            code = user.DisplayConfigSetDeviceInfo(value)
            if code:
                raise RuntimeError('Windows rejected the HDR change (%s)' % code)
    return result


def snapshot():
    result = dict(displays=[], outputs=[], inputs=[], volume=None, muted=False, micVolume=None, micMuted=False, errors=[])
    try:
        result['displays'] = displays(None, result['errors'])
    except Exception as error:
        result['errors'].append('Displays: ' + str(error))
    try:
        result['outputs'], result['volume'], result['muted'] = audio(None)
    except Exception as error:
        result['errors'].append('Audio: ' + str(error))
    try:
        result['inputs'], result['micVolume'], result['micMuted'] = audio(None, flow=1)
    except Exception as error:
        result['errors'].append('Microphone: ' + str(error))
    return result


def main():
    ole.CoInitializeEx(None, 2)
    try:
        command = json.loads(sys.argv[1]) if len(sys.argv) > 1 else None
        if command:
            kind, value = command.get('kind'), command.get('value')
            if kind not in ('display', 'hdr', 'audio', 'volume', 'mute', 'microphone', 'mic-volume', 'mic-mute') or not isinstance(command.get('target'), str):
                raise ValueError('Invalid control command')
            is_volume = kind in ('volume', 'mic-volume')
            if (is_volume and (type(value) not in (int, float) or not 0 <= value <= 100)) or (not is_volume and type(value) is not bool):
                raise ValueError('Invalid control value')
            if kind in ('display', 'hdr'):
                displays(command)
            else:
                audio(command, flow=1 if kind in ('microphone', 'mic-volume', 'mic-mute') else 0)
        print(json.dumps(snapshot(), ensure_ascii=True))
    except Exception as error:
        print(json.dumps(dict(error=str(error)), ensure_ascii=True))
    finally:
        ole.CoUninitialize()


if __name__ == '__main__':
    main()
