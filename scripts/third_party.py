"""Collect upstream license texts and a versioned inventory, without local paths."""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
import run_native_dock as native


def notice_file(path):
    name=path.name.upper()
    return path.is_file() and ("LICENSE" in name or name.startswith(("COPYING","NOTICE","COPYRIGHT")))


def slug(text):
    return re.sub(r"[^A-Za-z0-9._-]", "-", text)


def repository(value):
    if isinstance(value,dict):value=value.get("url","")
    if not isinstance(value,str):return ""
    return value.removeprefix("git+").removesuffix(".git")


def fetch_notice(url,destination):
    if destination.is_file():return
    destination.parent.mkdir(parents=True,exist_ok=True)
    with urllib.request.urlopen(url,timeout=30) as response:
        data=response.read(4*1024*1024+1)
    if len(data)>4*1024*1024:raise RuntimeError("Oversized upstream notice")
    destination.write_bytes(data)


def collect(output,metadata=None,node_license=None):
    output=Path(output)
    output.mkdir(parents=True,exist_ok=True)
    cargo,env=native.toolchain()
    if metadata is None:
        metadata=json.loads(subprocess.check_output([cargo,"metadata","--locked","--format-version","1","--manifest-path",str(native.MANIFEST)],env=env))
    entries=[]

    def add(ecosystem,name,version,license_name,repo,files,scope):
        location=output/ecosystem/slug(name+"-"+version)
        location.mkdir(parents=True,exist_ok=True)
        copied=[]
        for file in sorted(set(files)):
            target=location/file.name
            shutil.copyfile(file,target)
            copied.append(target.relative_to(output).as_posix())
        entries.append(dict(ecosystem=ecosystem,name=name,version=version,license=license_name or "Not declared",repository=repository(repo),scope=scope,notices=copied))

    for package in sorted(metadata["packages"],key=lambda p:(p["name"],p["version"])):
        if not package.get("source"):continue
        folder=Path(package["manifest_path"]).parent
        files=[p for p in folder.iterdir() if notice_file(p)]
        if package.get("license_file"):
            declared=folder/package["license_file"]
            if not declared.resolve().is_relative_to(folder.resolve()):raise RuntimeError("License file escapes its crate")
            if declared.is_file():files.append(declared)
        if not files:raise RuntimeError(f"No upstream license text found for Rust crate {package['name']}")
        add("rust",package["name"],package["version"],package.get("license"),package.get("repository"),files,"Cargo resolved graph (runtime/build/target-specific)")

    lock=json.loads((ROOT/"package-lock.json").read_text(encoding="utf-8"))
    for path,locked in sorted(lock["packages"].items()):
        if not path:continue
        folder=ROOT/path
        if not (folder/"package.json").is_file():continue # Optional packages for other platforms are not installed.
        package=json.loads((folder/"package.json").read_text(encoding="utf-8"))
        if package.get("version")!=locked.get("version"):
            raise RuntimeError(f"Installed npm package does not match lockfile: {path}; run npm ci")
        files=[p for p in folder.iterdir() if notice_file(p)]
        name=package.get("name",path.removeprefix("node_modules/"))
        if name.startswith("@koromix/koffi-") and not files:
            files=[ROOT/"node_modules/koffi/LICENSE.txt"]
        if name.startswith("@rolldown/binding-") and not files:
            files=[ROOT/"node_modules/rolldown/LICENSE",ROOT/"node_modules/rolldown/THIRD-PARTY-LICENSE"]
        if not files:raise RuntimeError(f"No upstream license text found for installed npm package {name}")
        add("npm",name,package["version"],package.get("license",locked.get("license")),package.get("repository"),files,"installed locked npm package; includes build tooling")

    node_version=subprocess.check_output([shutil.which("node") or "node","--version"],text=True).strip().removeprefix("v")
    node_notice=output/"node"/f"node-{node_version}"/"LICENSE"
    node_notice.parent.mkdir(parents=True,exist_ok=True)
    if node_license is not None:shutil.copyfile(node_license,node_notice)
    else:fetch_notice(f"https://raw.githubusercontent.com/nodejs/node/v{node_version}/LICENSE",node_notice)
    entries.append(dict(ecosystem="runtime",name="Node.js",version=node_version,license="MIT and bundled third-party licenses",repository="https://github.com/nodejs/node",scope="bundled Music runtime",notices=[node_notice.relative_to(output).as_posix()]))

    rustc=Path(cargo).with_name("rustc.exe")
    rust_version=subprocess.check_output([str(rustc),"--version"],env=env,text=True).split()[1]
    rust_notices=[]
    for name in ["LICENSE-MIT","LICENSE-APACHE"]:
        destination=output/"runtime"/f"rust-{rust_version}"/name
        fetch_notice(f"https://raw.githubusercontent.com/rust-lang/rust/{rust_version}/{name}",destination)
        rust_notices.append(destination.relative_to(output).as_posix())
    entries.append(dict(ecosystem="runtime",name="Rust standard library",version=rust_version,license="MIT OR Apache-2.0; component notices apply",repository="https://github.com/rust-lang/rust",scope="compiled standard library",notices=rust_notices))

    for label,file in [("lucide-music",ROOT/"native-dock/assets/music/LICENSE-lucide.txt"),("lucide-controls",ROOT/"widgets/controls/assets/LICENSE"),("lucide-toolbar",ROOT/"native-dock/assets/toolbar/LICENSE-lucide.txt")]:
        add("assets",label,"vendored","ISC; Feather-derived icons MIT","https://github.com/lucide-icons/lucide",[file],"embedded SVG icon notices")
    credits=(ROOT/"CREDITS.md").read_text(encoding="utf-8")
    (output/"CREDITS.md").write_text(credits,encoding="utf-8")
    (output/"CREDITS.txt").write_text(credits+"\nFull dependency inventory: THIRD-PARTY-NOTICES.txt in this folder.\n",encoding="utf-8")
    shutil.copyfile(ROOT/"LICENSE.txt",output/"LICENSE.txt")
    (output/"inventory.json").write_text(json.dumps({"schemaVersion":1,"scope":"Cargo resolved graph and installed npm packages; not a runtime-only SBOM","entries":entries},indent=2,ensure_ascii=False)+"\n",encoding="utf-8")
    lines=["# Third-party notices", "", "Thank you to all upstream developers and contributors.", "", "Lumen is created by Nhat Ng and licensed under MIT. These components keep", "their own licenses. Exact upstream notice files are linked below.", "", "This inventory includes build tools and target-specific Cargo packages;", "it does not assert that every listed package is part of the running app.", "", "| Component | Version | Declared license | Full notices |", "| --- | --- | --- | --- |"]
    for entry in entries:
        title=entry["name"]
        if entry["repository"].startswith("https://"):title=f"[{title}]({entry['repository']})"
        notices=", ".join(f"[{Path(p).name}]({p})" for p in entry["notices"])
        lines.append(f"| {title} | {entry['version']} | {str(entry['license']).replace('|','/')} | {notices} |")
    (output/"THIRD-PARTY-NOTICES.md").write_text("\n".join(lines)+"\n",encoding="utf-8")
    plain=["Lumen by Nhat Ng — third-party notices", "", "Thank you to all developers and contributors.", "Lumen's original code is MIT; dependencies retain their own terms.", ""]
    for entry in entries:
        plain.extend([f"{entry['name']} {entry['version']}",f"License: {entry['license']}",entry['repository'],*entry['notices'],""])
    (output/"THIRD-PARTY-NOTICES.txt").write_text("\n".join(plain)+"\n",encoding="utf-8")
    print(f"Collected notices for {len(entries)} components in {output}")
    return entries


if __name__=="__main__":
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output",type=Path,default=ROOT/"LICENSES")
    collect(parser.parse_args().output)
