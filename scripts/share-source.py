"""Create a native source archive without binaries, runtimes or user settings."""
from pathlib import Path
import zipfile
ROOT=Path(__file__).resolve().parents[1]
SKIP={'.git','target','node_modules','__pycache__','bin','obj','config','.scratch'}
def main():
    output=ROOT/'share/Lumen-native-source.zip'
    output.parent.mkdir(exist_ok=True)
    files=[ROOT/n for n in ['AGENTS.md','README.md','SECURITY.md','package.json','package-lock.json','run_lumen.py','run_lumen.pyw','run_native_dock.py','run_native_clock.py','Launch Lumen.pyw','run.cmd']]
    for folder in ['native-dock','widgets','scripts','tools','resources','docs']:
        files.extend(p for p in (ROOT/folder).rglob('*') if p.is_file() and not set(p.relative_to(ROOT).parts)&SKIP and p.suffix.lower() not in {'.exe','.dll','.pdb','.lnk','.log','.ps1'})
    with zipfile.ZipFile(output,'w',zipfile.ZIP_DEFLATED) as archive:
        for p in files: archive.write(p,'Lumen/'+p.relative_to(ROOT).as_posix())
    print(output)
if __name__=='__main__': main()
