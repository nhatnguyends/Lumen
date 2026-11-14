"""Double-click to open the current native Lumen widgets without a console."""
from __future__ import annotations

import ctypes
from pathlib import Path
import subprocess
import sys
from run_native_dock import needs_build, already_running

ROOT = Path(__file__).resolve().parent


def main() -> int:
    logs = ROOT / "config" / "logs"
    try:
        logs.mkdir(parents=True, exist_ok=True)
        python = Path(sys.executable).with_name("python.exe")
        command = [str(python), str(ROOT / "run_native_dock.py")]
        # Reuse the local development build for a fast everyday launch.
        # The existing source launcher can build it on the first run.
        if already_running() or not needs_build():
            command.append("--no-build")
        with (logs / "double-click-launcher.log").open("a", encoding="utf-8") as log:
            result = subprocess.run(
                command, cwd=ROOT, stdin=subprocess.DEVNULL,
                stdout=log, stderr=subprocess.STDOUT,
                creationflags=subprocess.CREATE_NO_WINDOW,
            )
        if result.returncode:
            raise RuntimeError(
                "Lumen could not start. See config\\logs\\double-click-launcher.log "
                "in the Lumen folder for details."
            )
        return 0
    except (OSError, RuntimeError) as error:
        ctypes.windll.user32.MessageBoxW(None, str(error), "Lumen launcher", 0x10)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
