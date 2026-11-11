"""Build the user-authorized native Windows x64 portable ZIP (no installer)."""
from __future__ import annotations
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tomllib
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
import run_native_dock as native

def archive_portable(destination):
    manifest=json.loads((destination/"lumen-portable.json").read_text(encoding="utf-8"))
    archive=destination.with_name(destination.name+".zip")
    with zipfile.ZipFile(archive,"w",zipfile.ZIP_DEFLATED,strict_timestamps=False) as out:
        for name,digest in sorted(manifest["files"].items()):
            file=destination/name
            if hashlib.sha256(file.read_bytes()).hexdigest()!=digest:raise RuntimeError(f"Changed bundle file: {name}")
            out.write(file,name)
        out.write(destination/"lumen-portable.json","lumen-portable.json")
    archive.with_suffix(".zip.sha256").write_text(hashlib.sha256(archive.read_bytes()).hexdigest()+"  "+archive.name+"\n",encoding="utf-8")
    print(archive)

def main():
    version = tomllib.loads(native.MANIFEST.read_text(encoding="utf-8"))["package"]["version"]
    if "--archive-only" in sys.argv:
        archive_portable(ROOT/"share"/f"Lumen-{version}-windows-x86_64")
        return
    cargo, env = native.toolchain()
    env["RUSTFLAGS"] = "-C target-feature=+crt-static"
    env["CARGO_TARGET_DIR"] = str(ROOT / "native-dock/target/portable-debug")
    subprocess.run(["node", "scripts/check-safe.mjs"], cwd=ROOT, check=True)
    subprocess.run(["node", "scripts/build-native-music.mjs"], cwd=ROOT, check=True)
    subprocess.run([cargo, "build", "--locked", "--manifest-path", str(native.MANIFEST)], env=env, cwd=ROOT, check=True)
    destination = ROOT / "share" / f"Lumen-{version}-windows-x86_64"
    if (destination / "lumen-portable.json").exists() and "--refresh" not in sys.argv:
        raise RuntimeError(f"Output already exists; keep it or move it before rebuilding: {destination}")
    if (destination / "config").exists():
        raise RuntimeError("Build output contains user data; use a fresh output folder instead of repackaging it")
    destination.mkdir(parents=True, exist_ok=True)

    def copy(source, name):
        target = destination / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)

    copy(Path(env["CARGO_TARGET_DIR"]) / "debug/lumen-native-dock.exe", "Lumen.exe")
    copy(ROOT / "resources/tray.png", "resources/tray.png")
    copy(ROOT / "native-dock/lumen.manifest", "resources/lumen.manifest")
    copy(ROOT / "native-dock/target/debug/music-bridge.cjs", "runtime/music-bridge.cjs")
    node = Path(shutil.which("node"))
    copy(node, "runtime/node.exe")
    node_version = subprocess.check_output([str(node), "--version"], text=True).strip()
    # Node's distribution LICENSE includes its bundled third-party notices.
    with urllib.request.urlopen(f"https://raw.githubusercontent.com/nodejs/node/{node_version}/LICENSE", timeout=30) as response:
        license_bytes = response.read(4 * 1024 * 1024)
    (destination / "LICENSES").mkdir(exist_ok=True)
    (destination / "LICENSES/Node-LICENSE.txt").write_bytes(license_bytes)
    for name in ["koffi", "@koromix/koffi-win32-x64"]:
        source = ROOT / "node_modules" / name
        for file in source.rglob("*"):
            if file.is_file() and not set(file.relative_to(source).parts) & {"doc", "vendor", "build"} and file.suffix.lower() in {".js", ".cjs", ".json", ".node", ".txt", ".md"}:
                copy(file, "runtime/node_modules/" + name + "/" + file.relative_to(source).as_posix())
    # Include Rust dependency license texts rather than removing third-party notices.
    metadata = json.loads(subprocess.check_output([cargo, "metadata", "--locked", "--format-version", "1", "--manifest-path", str(native.MANIFEST)], env=env))
    for package in metadata["packages"]:
        folder = Path(package["manifest_path"]).parent
        if package.get("source"):
            for file in folder.iterdir():
                if file.is_file() and file.name.upper().startswith(("LICENSE", "COPYING", "NOTICE")):
                    copy(file, f"LICENSES/{package['name']}-{package['version']}/{file.name}")
    for manifest in (ROOT / "widgets").glob("*/widget.json"):
        copy(manifest, manifest.relative_to(ROOT).as_posix())
    readme = """# Lumen portable\n\nExtract the entire ZIP to a writable folder, then double-click Lumen.exe.\nNo Python, Rust, Visual Studio or Node installation is needed. Node for the\noptional Music adapter is included; browser playback requires installed Chrome\nor Edge and internet access. Other widgets do not require a browser.\n\nTarget: Windows 10/11 x64. ARM64 native and 32-bit builds are not provided.\nThis build has not yet been certified on a clean Windows VM or other PCs.\n\nKeep the full folder together. User data is stored in config/ next to Lumen.exe.\nUse a folder you can write to, such as Downloads or your user tools folder,\nnot Program Files. Settings enable Start with Windows by default; you can turn\nit off. Moving the folder requires launching Lumen once to refresh that entry.\n\nGeneral Settings > Updates checks official GitHub Releases. Download update\nthen Restart and apply update. Settings, notes, browser profile and geometry\nare preserved. All six native widget implementations update with the app.\nIndependent downloadable native plugins are not implemented yet.\n\nThe package is not code-signed. Windows may show an unknown-publisher warning.\nSource and release notes: https://github.com/nhatnguyends/Lumen\n"""
    (destination / "README.md").write_text(readme, encoding="utf-8")
    files = {p.relative_to(destination).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
             for p in sorted(destination.rglob("*")) if p.is_file() and p.name != "lumen-portable.json"}
    manifest = {"schemaVersion": 1, "version": version, "target": "windows-x86_64", "files": files}
    (destination / "lumen-portable.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    archive_portable(destination)


if __name__ == "__main__":
    main()
