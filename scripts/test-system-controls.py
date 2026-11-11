"""Exercise native command guards and ABI payloads without changing hardware."""
import ctypes
import importlib.util
from pathlib import Path
import struct
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('controls', Path(__file__).resolve().parents[1] / 'tools/system_controls.py')
controls = importlib.util.module_from_spec(spec)
spec.loader.exec_module(controls)


class DisplayControlsTest(unittest.TestCase):
    def setUp(self):
        self.devices = [dict(id='primary', name='Primary', enabled=True, primary=True),
                        dict(id='second', name='Second', enabled=True, primary=False)]
        self.list_patch = patch.object(controls, 'display_list', side_effect=lambda: [dict(d) for d in self.devices])
        self.hdr_patch = patch.object(controls, 'hdr_targets', return_value={'second': [(bytes(8), 7, True, False, True, 'Monitor')]})
        for name, replacement in [('physical_displays', lambda d: d), ('display_cache', lambda: {}), ('display_connections', lambda: [dict(source=d['id'], name=d['name'], monitor_path=d['id']) for d in self.devices]), ('remember_display', lambda *a: None)]:
            mocked = patch.object(controls, name, replacement)
            mocked.start()
            self.addCleanup(mocked.stop)
        self.list_patch.start()
        self.hdr_patch.start()
        self.addCleanup(self.list_patch.stop)
        self.addCleanup(self.hdr_patch.stop)

    def command(self, kind='display', target='second', value=False):
        return controls.displays(dict(kind=kind, target=target, value=value))

    def test_primary_and_last_display_cannot_be_detached(self):
        with patch.object(controls.user, 'ChangeDisplaySettingsExW') as change:
            with self.assertRaisesRegex(RuntimeError, 'primary'):
                self.command(target='primary')
            self.devices[0]['enabled'] = False
            with self.assertRaisesRegex(RuntimeError, 'primary'):
                self.command()
            change.assert_not_called()

    def test_missing_device_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, 'no longer connected'):
            self.command(target='missing')

    def test_detach_payload_retains_position_and_does_not_overwrite_registry(self):
        mode = ctypes.create_string_buffer(220)
        struct.pack_into('<ii', mode, 76, -1920, 80)
        struct.pack_into('<II', mode, 172, 1920, 1080)
        with patch.object(controls, 'mode', return_value=mode), patch.object(controls.user, 'ChangeDisplaySettingsExW', return_value=0) as change:
            self.command()
            self.assertEqual(struct.unpack_from('<I', mode, 72)[0], 0x180020)
            self.assertEqual(struct.unpack_from('<II', mode, 172), (0, 0))
            self.assertEqual(struct.unpack_from('<ii', mode, 76), (-1920, 80))
            self.assertEqual(change.call_args.args[3], 0)

    def test_attach_uses_physical_reattachment(self):
        self.devices[1]['enabled'] = False
        with patch.object(controls, 'reattach_display') as restore, patch.object(controls.user, 'ChangeDisplaySettingsExW') as gdi:
            self.command(value=True)
            restore.assert_called_once_with(dict(kind='display', target='second', value=True))
            gdi.assert_not_called()

    def test_restore_plan_preserves_active_paths_and_saved_position(self):
        active = bytes([7]) * 72
        modes = bytes([3]) * 64
        path = bytes(72)
        paths, planned = controls.restore_plan([active], modes, path, dict(width=3840, height=2160, x=-3840, y=0))
        self.assertEqual(paths[:72], active)
        self.assertEqual(planned[:64], modes)
        self.assertEqual(struct.unpack_from('<I', paths, 72+12)[0], 1)
        self.assertEqual(struct.unpack_from('<I', paths, 72+32)[0], 0xffffffff)
        self.assertEqual(struct.unpack_from('<IIIii', planned, 64+16), (3840, 2160, 4, -3840, 0))

    def test_driver_rejection_is_reported(self):
        with patch.object(controls, 'mode', return_value=ctypes.create_string_buffer(220)), patch.object(controls.user, 'ChangeDisplaySettingsExW', return_value=-2):
            with self.assertRaisesRegex(RuntimeError, 'rejected'):
                self.command()

    def test_modern_hdr_uses_hdr_state_not_wide_color(self):
        with patch.object(controls.user, 'DisplayConfigSetDeviceInfo', return_value=0) as change:
            self.command(kind='hdr', value=True)
            payload = change.call_args.args[0]
            self.assertEqual(struct.unpack_from('<II', payload), (16, 24))
            self.assertEqual(struct.unpack_from('<I', payload, 20)[0], 1)

    def test_legacy_hdr_fallback(self):
        with patch.object(controls, 'hdr_targets', return_value={'second': [(bytes(8), 7, True, True, False, '')]}), patch.object(controls.user, 'DisplayConfigSetDeviceInfo', return_value=0) as change:
            self.command(kind='hdr', value=False)
            self.assertEqual(struct.unpack_from('<I', change.call_args.args[0])[0], 10)

    def test_hdr_query_failure_preserves_display_controls(self):
        errors = []
        with patch.object(controls, 'hdr_targets', side_effect=RuntimeError('Unavailable')):
            state = controls.displays(None, errors)
        self.assertEqual(len(state), 2)
        self.assertFalse(state[0]['hdrSupported'])
        self.assertEqual(errors, ['HDR: Unavailable'])


class AudioControlsTest(unittest.TestCase):
    def setUp(self):
        self.calls = []
        self.has_default = True
        self.count = 1

        def create(clsid, _iid):
            return ctypes.c_void_p(7 if clsid.startswith('870af99c') else 1)

        def call(obj, index, types=(), args=()):
            identity = obj.value
            self.calls.append((identity, index, args))
            if identity == 1 and index == 4:
                if not self.has_default:
                    raise RuntimeError('No default')
                args[-1]._obj.value = 2
            elif identity == 1 and index == 3:
                args[-1]._obj.value = 3
            elif identity == 3 and index == 3:
                args[0]._obj.value = self.count
            elif identity == 3 and index == 4:
                args[-1]._obj.value = 5
            elif identity == 5 and index == 4:
                args[-1]._obj.value = 6
            elif identity == 2 and index == 3:
                args[-1]._obj.value = 4
            elif identity == 4 and index == 9:
                args[0]._obj.value = 0.73
            elif identity == 4 and index == 15:
                args[0]._obj.value = 1

        for name, replacement in [('create', create), ('call', call), ('release', lambda obj: None), ('device_id', lambda obj: 'connected-device')]:
            p = patch.object(controls, name, replacement)
            p.start()
            self.addCleanup(p.stop)

    def test_microphone_reads_capture_endpoints_and_its_own_level(self):
        devices, level, muted = controls.audio(None, flow=1)
        self.assertEqual(devices, [dict(id='connected-device', name='connected-device', selected=True)])
        self.assertEqual(level, 73)
        self.assertTrue(muted)
        self.assertTrue(all(args[0] == 1 for obj, index, args in self.calls if obj == 1 and index in (3, 4)))

    def test_microphone_volume_and_mute_write_only_capture_default(self):
        for kind, value, method in [('mic-volume', 38, 7), ('mic-mute', True, 14)]:
            self.calls.clear()
            controls.audio(dict(kind=kind, target='', value=value), flow=1)
            writes = [(obj, index, args) for obj, index, args in self.calls if obj == 4 and index in (7, 14)]
            self.assertEqual(len(writes), 1)
            self.assertEqual(writes[0][1], method)
            self.assertEqual(writes[0][2][0], value / 100 if kind == 'mic-volume' else value)
            self.assertTrue(all(args[0] == 1 for obj, index, args in self.calls if obj == 1 and index in (3, 4)))

    def test_speaker_volume_keeps_render_direction(self):
        controls.audio(dict(kind='volume', target='', value=40))
        self.assertTrue(all(args[0] == 0 for obj, index, args in self.calls if obj == 1 and index in (3, 4)))
        self.assertTrue(any(obj == 4 and index == 7 and args[0] == .4 for obj, index, args in self.calls))

    def test_microphone_selection_updates_all_default_roles(self):
        controls.audio(dict(kind='microphone', target='connected-device', value=True), flow=1)
        self.assertEqual([args for obj, index, args in self.calls if obj == 7 and index == 13], [('connected-device', 0), ('connected-device', 1), ('connected-device', 2)])

    def test_disconnected_microphone_cannot_be_selected(self):
        with self.assertRaisesRegex(RuntimeError, 'no longer connected'):
            controls.audio(dict(kind='microphone', target='disconnected', value=True), flow=1)
        self.assertFalse(any(obj == 7 for obj, _, _ in self.calls))

    def test_no_microphone_is_a_readable_state_but_blocks_muting(self):
        self.has_default = False
        self.count = 0
        self.assertEqual(controls.audio(None, flow=1), ([], None, False))
        with self.assertRaisesRegex(RuntimeError, 'No default microphone'):
            controls.audio(dict(kind='mic-mute', target='', value=True), flow=1)

    def test_microphone_command_rejects_playback_direction(self):
        with self.assertRaisesRegex(ValueError, 'direction'):
            controls.audio(dict(kind='mic-mute', target='', value=True), flow=0)
        self.assertEqual(self.calls, [])

    def test_microphone_failure_does_not_hide_audio_outputs(self):
        def read(command, flow=0):
            if flow == 1:
                raise RuntimeError('Unavailable')
            return [dict(id='speaker', name='Speakers', selected=True)], 47, False
        with patch.object(controls, 'displays', return_value=[]), patch.object(controls, 'audio', side_effect=read):
            state = controls.snapshot()
        self.assertEqual(state['volume'], 47)
        self.assertEqual(state['outputs'][0]['id'], 'speaker')
        self.assertIsNone(state['micVolume'])
        self.assertEqual(state['errors'], ['Microphone: Unavailable'])


if __name__ == '__main__':
    unittest.main()
