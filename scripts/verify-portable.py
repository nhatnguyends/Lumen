"""Inspect direct Windows PE dependencies and integrity without executing a bundle."""
import hashlib
import json
from pathlib import Path
import struct
import sys

def imports(file):
    data=Path(file).read_bytes()
    pe=struct.unpack_from('<I',data,0x3c)[0]
    if data[pe:pe+4]!=b'PE\0\0':raise ValueError('Not a PE image')
    machine,sections=struct.unpack_from('<HH',data,pe+4)
    if machine!=0x8664:raise ValueError(f'Expected x64 PE: {file}')
    optional=pe+24
    size=struct.unpack_from('<H',data,pe+20)[0]
    directory=optional+(112 if struct.unpack_from('<H',data,optional)[0]==0x20b else 96)
    headers=[]
    for i in range(sections):
        vsize,va,rawsize,offset=struct.unpack_from('<IIII',data,optional+size+i*40+8)
        headers.append((va,max(vsize,rawsize),offset))
    def at(rva):
        for va,length,offset in headers:
            if va<=rva<va+length:return offset+rva-va
        raise ValueError('Invalid image RVA')
    rva=struct.unpack_from('<I',data,directory+8)[0]
    names=[]
    if rva:
        offset=at(rva)
        while any(data[offset:offset+20]):
            name=at(struct.unpack_from('<I',data,offset+12)[0])
            names.append(data[name:data.index(0,name)].decode('ascii'))
            offset+=20
    return names

def main():
    root=Path(sys.argv[1]).resolve()
    manifest=json.loads((root/'lumen-portable.json').read_text(encoding='utf-8'))
    assert manifest['target']=='windows-x86_64'
    for name,digest in manifest['files'].items():
        file=root/name
        assert hashlib.sha256(file.read_bytes()).hexdigest()==digest,name
        if file.suffix.lower() in {'.exe','.dll','.node'}:
            names=imports(file)
            print(name+': '+', '.join(names))
            for dependency in names:
                # VC++ redistributable must be app-local if any included image needs it.
                if dependency.lower().startswith(('vcruntime','msvcp','concrt')):
                    assert (file.parent/dependency).exists(),f'Missing app-local runtime: {dependency}'
    assert all(not name.startswith(('config/','target/','.scratch/')) for name in manifest['files'])
    print(f"Verified {len(manifest['files'])} portable files; user state is excluded")

if __name__=='__main__':main()
