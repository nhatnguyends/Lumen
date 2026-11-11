"""Relocate the ZIP and smoke-test with only system tools on PATH."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import zipfile

ROOT=Path(__file__).resolve().parents[1]
def main():
    archive=Path(sys.argv[1]).resolve()
    (ROOT/'.scratch').mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='Lumen portable ',dir=ROOT/'.scratch') as directory:
        root=Path(directory)
        with zipfile.ZipFile(archive) as z:z.extractall(root)
        env=os.environ.copy();env['PATH']=str(Path(os.environ['SystemRoot'])/'System32')+';'+os.environ['SystemRoot']
        (root/'config').mkdir()
        (root/'config/native-general.json').write_text(json.dumps({'version':1,'startWithWindows':False}),encoding='utf-8')
        with (root/'smoke-test.log').open('w',encoding='utf-8') as log:
            process=subprocess.Popen([str(root/'Lumen.exe'),'--smoke-test','--clock','--calendar','--music','--notes','--controls'],cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT)
            try:code=process.wait(timeout=30)
            except subprocess.TimeoutExpired:
                process.kill();process.wait();raise
        output=(root/'smoke-test.log').read_text(encoding='utf-8',errors='replace')
        print(output);assert code==0
        for item in ['native-dock-ready','native-clock-created','native-calendar-ready','native-music-ready','native-notes-ready','native-controls-ready']:
            assert item in output,item
        node=subprocess.run([str(root/'runtime/node.exe'),'-e',"const k=require('./runtime/node_modules/koffi'); k.load('user32.dll'); console.log('Bundled Node and Koffi OK')"],cwd=root,env=env,capture_output=True,text=True,timeout=10)
        print(node.stdout,node.stderr);assert node.returncode==0
        bridge=subprocess.run([str(root/'runtime/node.exe'),str(root/'runtime/music-bridge.cjs')],cwd=root,env=env,input='{"action":"info"}\n{"action":"quit"}\n',capture_output=True,text=True,timeout=15)
        print(bridge.stdout,bridge.stderr);assert bridge.returncode==0
        assert '"type":"ready"' in bridge.stdout
        assert '"type":"error"' not in bridge.stdout
        print('Relocated portable smoke test passed without developer tools on PATH')

if __name__=='__main__':main()
