import pathlib
import tempfile
import unittest
from unittest.mock import patch
import startup_windows as startup


class StartupTests(unittest.TestCase):
    def test_first_run_defaults_on_and_saved_off_is_respected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            with patch.object(startup, "set_enabled") as register:
                startup.ensure(root)
                register.assert_called_once_with(True, root)
                (root / "config").mkdir()
                (root / "config/native-general.json").write_text(
                    '{"startWithWindows": false}', encoding="utf-8")
                register.reset_mock()
                startup.ensure(root)
                register.assert_called_once_with(False, root)

    def test_command_quotes_launcher_and_python_paths(self):
        command = startup.command(pathlib.Path("D:/My Widgets"), "C:/Python/python.exe")
        self.assertTrue(command.startswith('"C:'))
        self.assertIn('" "D:', command)
        self.assertTrue(command.endswith('Launch Lumen.pyw"'))

    def test_corrupt_preferences_do_not_register_or_overwrite(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / "config").mkdir()
            file = root / "config/native-general.json"
            file.write_text("{broken", encoding="utf-8")
            with patch.object(startup, "set_enabled") as register:
                with self.assertRaises(ValueError):
                    startup.ensure(root)
                register.assert_not_called()
            self.assertEqual(file.read_text(encoding="utf-8"), "{broken")


if __name__ == "__main__":
    unittest.main()
