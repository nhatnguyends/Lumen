"""Export only reviewed native source roots, without user state or local builds."""
from pathlib import Path
import argparse
import shutil
import zipfile
ROOT=Path(__file__).resolve().parents[1]
SKIP={'.git','target','node_modules','__pycache__','bin','obj','config','.scratch'}
TOP=['.gitignore','AGENTS.md','README.md','SECURITY.md','PACKAGE-DISTRIBUTION.md','design.md',
     'package.json','package-lock.json','run_lumen.py','run_lumen.pyw','run_native_dock.py',
     'run_native_clock.py','Launch Lumen.pyw','run.cmd','startup_windows.py','test_startup_windows.py','CREDITS.md','LICENSE.txt']

def source_files():
    files=[ROOT/n for n in TOP]
    for folder in ['.github','native-dock','widgets','scripts','tools','resources','docs','LICENSES']:
        for p in (ROOT/folder).rglob('*'):
            if (p.is_file() and not set(p.relative_to(ROOT).parts)&SKIP
                and p.suffix.lower() not in {'.exe','.dll','.node','.pdb','.lnk','.log','.ps1','.pyc'}
                and 'HANDOFF' not in p.name and p.name!='INTEGRATION-QA.md'):
                files.append(p)
    return sorted(set(files))

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--destination',type=Path,required=True)
    args=parser.parse_args()
    destination=args.destination.resolve()
    if destination.exists(): raise RuntimeError('Use a fresh destination to avoid overwriting a checkout')
    for p in source_files():
        out=destination/p.relative_to(ROOT)
        out.parent.mkdir(parents=True,exist_ok=True)
        shutil.copy2(p,out)
    archive=ROOT/'share/Lumen-native-source.zip'
    archive.parent.mkdir(exist_ok=True)
    with zipfile.ZipFile(archive,'w',zipfile.ZIP_DEFLATED,strict_timestamps=False) as out:
        for p in source_files():out.write(p,'Lumen/'+p.relative_to(ROOT).as_posix())
    print(f'Exported {len(source_files())} files to {destination}')
    print(archive)

if __name__=='__main__':main()
