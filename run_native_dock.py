"""Build and run the native Rust app from source. No installer output."""
from __future__ import annotations

import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys
import ctypes

ROOT = Path(__file__).resolve().parent
MANIFEST = ROOT / "native-dock" / "Cargo.toml"
BINARY = ROOT / "native-dock" / "target" / "debug" / "lumen.exe"


def needs_build() -> bool:
    """New/changed native source packages are picked up by the double-click launcher."""
    if not BINARY.is_file() or not (ROOT / "native-dock/target/debug/music-bridge.cjs").is_file():
        return True
    built = BINARY.stat().st_mtime_ns
    sources = [MANIFEST, ROOT / "native-dock" / "build.rs", ROOT / "native-dock" / "Cargo.lock", ROOT / "scripts/build-native-music.mjs", ROOT / "package-lock.json"]
    for folder in (ROOT / "native-dock" / "src", ROOT / "native-dock" / "assets", ROOT / "widgets"):
        sources.extend(p for p in folder.rglob("*") if p.is_file() and (p.suffix in (".rs", ".svg", ".png", ".toml", ".ts") or p.name == "widget.json"))
    return any(p.is_file() and p.stat().st_mtime_ns > built for p in sources)


def already_running() -> bool:
    if os.name != "nt":
        return False
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.OpenMutexW.argtypes = [ctypes.c_uint32, ctypes.c_int, ctypes.c_wchar_p]
    kernel.OpenMutexW.restype = ctypes.c_void_p
    kernel.CloseHandle.argtypes = [ctypes.c_void_p]
    handle = kernel.OpenMutexW(0x00100000, False, "Local\\LumenNativeDockPrototype")
    if not handle:
        return False
    kernel.CloseHandle(handle)
    return True


def toolchain() -> tuple[str, dict[str, str]]:
    environment = os.environ.copy()
    local = ROOT / ".scratch" / "cargo"
    rustup = ROOT / ".scratch" / "rustup"
    if (local / "bin" / "cargo.exe").is_file() and rustup.is_dir():
        environment["CARGO_HOME"] = str(local)
        environment["RUSTUP_HOME"] = str(rustup)
        environment["PATH"] = str(local / "bin") + os.pathsep + environment.get("PATH", "")
        return str(local / "bin" / "cargo.exe"), environment
    cargo = shutil.which("cargo")
    if not cargo:
        raise RuntimeError("Rust is required. Install Rust with the MSVC toolchain and Visual Studio C++ Build Tools, then run this launcher again.")
    return cargo, environment


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build-only", action="store_true", help="Build the native source without opening windows")
    parser.add_argument("--no-build", action="store_true", help="Run the existing local development build")
    parser.add_argument("--check", action="store_true", help="Run safety checks, Rust tests and build without opening a window")
    parser.add_argument("--smoke-test", action="store_true", help="Open for five seconds, report resources and exit without saving position")
    parser.add_argument("--inspect", action="store_true", help="Use a normal foreground window for UI inspection")
    parser.add_argument("--clock", action="store_true", help="Also open the native Rust clock in this process")
    parser.add_argument("--calendar", action="store_true", help="Also open the native Rust calendar in this process")
    parser.add_argument("--music", action="store_true", help="Also open the native Rust YouTube Music widget")
    parser.add_argument("--notes", action="store_true", help="Also open native Rust Sticky Notes in this process")
    parser.add_argument("--widget", action="append", default=[], help="Open a discovered widget package by id")
    parser.add_argument("--manage-widgets", action="store_true", help="Open the widget manager")
    args = parser.parse_args()
    if os.name != "nt":
        parser.error("Lumen requires Windows.")
    if args.check and args.no_build:
        parser.error("--check and --no-build cannot be combined")
    if already_running() and not args.check and not args.build_only:
        args.no_build = True
    environment = os.environ.copy()
    environment["LUMEN_PYTHON"] = sys.executable
    try:
        if not (args.check or args.build_only or args.smoke_test):
            import startup_windows
            startup_windows.ensure(ROOT)
        node = shutil.which("node")
        if not node:
            raise RuntimeError("Node.js is required to run the repository's safety guard.")
        subprocess.run([node, str(ROOT / "scripts" / "check-safe.mjs")], cwd=ROOT, check=True)
        if not args.no_build:
            subprocess.run([node, str(ROOT / "scripts" / "build-native-music.mjs")], cwd=ROOT, check=True)
            cargo, environment = toolchain()
            environment["LUMEN_PYTHON"] = sys.executable
            if args.check:
                subprocess.run([node, "--test", "--test-isolation=none", str(ROOT / "scripts" / "test-check-safe.mjs")], cwd=ROOT, check=True)
                subprocess.run([cargo, "test", "--locked", "--manifest-path", str(MANIFEST)], cwd=ROOT, env=environment, check=True)
            subprocess.run([cargo, "build", "--locked", "--manifest-path", str(MANIFEST)], cwd=ROOT, env=environment, check=True)
        if args.check or args.build_only:
            return 0
        if not BINARY.is_file():
            raise RuntimeError("No native dock build exists. Run without --no-build first.")
        command = [str(BINARY)] + (["--smoke-test"] if args.smoke_test else []) + (["--inspect"] if args.inspect else []) + (["--clock"] if args.clock else []) + (["--music"] if args.music else []) + (["--calendar"] if args.calendar else [])
        if args.notes:
            command.append("--notes")
        for widget in args.widget:
            if not widget or len(widget) > 64 or not all(c in "abcdefghijklmnopqrstuvwxyz0123456789-" for c in widget):
                raise RuntimeError("Invalid widget package id")
            command.append("--" + widget)
        if args.manage_widgets:
            command.append("--manage-widgets")
        # Pipe GUI-subsystem stdout explicitly so startup/resource metrics survive
        # Windows' detached standard handles. Also keep a local diagnostic log.
        logs = ROOT / "config" / "logs"
        logs.mkdir(parents=True, exist_ok=True)
        with (logs / "native-dock.log").open("a", encoding="utf-8") as log:
            with subprocess.Popen(command, cwd=ROOT, env=environment, stdout=subprocess.PIPE,
                                  stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace") as process:
                assert process.stdout is not None
                for line in process.stdout:
                    print(line, end="", flush=True)
                    log.write(line)
                    log.flush()
                return process.wait()
    except (RuntimeError, OSError, subprocess.CalledProcessError) as error:
        print(f"Native dock: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
