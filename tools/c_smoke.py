#!/usr/bin/env python3
import os,pathlib, subprocess, platform
root=pathlib.Path(__file__).resolve().parents[1]
target=pathlib.Path(os.environ.get('CARGO_TARGET_DIR',root/'target'))
if not target.is_absolute(): target=root/target
subprocess.run(['python3','tools/check_header.py'],cwd=root,check=True)
libs=['-lpthread','-lm']
if platform.system()=='Darwin': libs+=['-framework','Security','-framework','CoreFoundation','-liconv']
else: libs+=['-ldl']
subprocess.run(['cc','-std=c11','-Wall','-Wextra','-Werror','-Iinclude','examples/c_smoke.c',str(target/'release/libqr_ffi.a'),*libs,'-o',str(target/'c-smoke')],cwd=root,check=True)
subprocess.run([str(target/'c-smoke')],cwd=root,check=True)
