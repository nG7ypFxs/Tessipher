#!/usr/bin/env python3
"""Build the C library and compare/copy its schema-generated bundled header."""
import json, pathlib, subprocess, sys
root=pathlib.Path(__file__).resolve().parents[1]
p=subprocess.run(['cargo','build','-p','qr-ffi','--release','--message-format=json'],cwd=root,stdout=subprocess.PIPE,text=True,check=True)
paths=[pathlib.Path(r['out_dir'])/'tessipher.h' for line in p.stdout.splitlines() if (r:=json.loads(line))['reason']=='build-script-executed' and 'qr-ffi' in r['package_id']]
assert len(paths)==1, paths
generated=paths[0].read_bytes();bundled=root/'include/tessipher.h'
if '--write' in sys.argv:bundled.write_bytes(generated)
assert bundled.read_bytes()==generated, 'Bundled C header differs; run tools/check_header.py --write'
print('Generated and bundled C headers match')
