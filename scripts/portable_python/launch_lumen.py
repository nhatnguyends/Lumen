"""Optional Python 3.10+ entry point for the native portable app.

Installed as runtime/launch_lumen.py. It starts Lumen.exe normally; it does not
replace the Rust runtime or change Windows security checks.
"""
from __future__ import annotations
from pathlib import Path
import subprocess
import sys


def main() -> int:
    root = Path(__file__).resolve().parent.parent
    try:
        executable = root / "Lumen.exe"
        if not executable.is_file() or not (root / "lumen-portable.json").is_file():
            raise RuntimeError("Extract the complete Lumen portable ZIP first. Keep the runtime folder next to Lumen.exe.")
        logs = root / "config" / "logs"
        logs.mkdir(parents=True, exist_ok=True)
        with (logs / "python-launcher.log").open("a", encoding="utf-8") as log:
            # Explicit arguments and cwd support spaces without a command shell.
            process = subprocess.Popen(
                [str(executable), *sys.argv[1:]], cwd=root,
                stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT,
                creationflags=subprocess.CREATE_NO_WINDOW,
            )
            code = process.wait()
        if code:
            raise RuntimeError(f"Lumen exited with code {code}. See config/logs/python-launcher.log.")
        return 0
    except (OSError, RuntimeError) as error:
        if sys.stderr is not None:
            print(f"Lumen: {error}", file=sys.stderr)
        else:
            import ctypes
            ctypes.windll.user32.MessageBoxW(None, str(error), "Lumen Python launcher", 0x10)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
