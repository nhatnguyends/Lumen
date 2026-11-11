"""User-authorized, per-user startup for the source launcher (no packaged app)."""
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent
VALUE = "Lumen"
KEY = r"Software\Microsoft\Windows\CurrentVersion\Run"

def command(root=ROOT, executable=None):
    python = pathlib.Path(executable or sys.executable)
    hidden = python.with_name("pythonw.exe")
    if hidden.exists():
        python = hidden
    return f'"{python}" "{root / "Launch Lumen.pyw"}"'

def set_enabled(enabled, root=ROOT):
    import winreg
    with winreg.CreateKeyEx(winreg.HKEY_CURRENT_USER, KEY, 0, winreg.KEY_READ | winreg.KEY_SET_VALUE) as key:
        if enabled:
            expected = command(root)
            try:
                current, _ = winreg.QueryValueEx(key, VALUE)
            except FileNotFoundError:
                current = None
            if current != expected:
                winreg.SetValueEx(key, VALUE, 0, winreg.REG_SZ, expected)
        else:
            try:
                winreg.DeleteValue(key, VALUE)
            except FileNotFoundError:
                pass

def ensure(root=ROOT):
    path = root / "config" / "native-general.json"
    # Malformed preferences must not be replaced or silently re-enabled.
    prefs = json.loads(path.read_text(encoding="utf-8")) if path.exists() else {}
    set_enabled(prefs.get("startWithWindows", True), root)

if __name__ == "__main__":
    set_enabled("--disable" not in sys.argv)
