"""Double-click launcher for Lumen.

All the work lives in ``run_lumen.py`` so the console and windowed launchers
behave identically. This file only exists so the app can be started without a
console window.
"""

from __future__ import annotations

from run_lumen import main


if __name__ == "__main__":
    raise SystemExit(main())