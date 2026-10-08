#!/usr/bin/env python3
# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
import os,pathlib, subprocess, platform
root=pathlib.Path(__file__).resolve().parents[1]
target=pathlib.Path(os.environ.get('CARGO_TARGET_DIR',root/'target'))
if not target.is_absolute(): target=root/target
subprocess.run(['uv','run','tools/check_header.py'],cwd=root,check=True)
libs=['-lpthread','-lm']
if platform.system()=='Darwin': libs+=['-framework','Security','-framework','CoreFoundation','-liconv']
else: libs+=['-ldl']
subprocess.run(['cc','-std=c11','-Wall','-Wextra','-Werror','-Iinclude','examples/c_smoke.c',str(target/'release/libqr_ffi.a'),*libs,'-o',str(target/'c-smoke')],cwd=root,check=True)
subprocess.run([str(target/'c-smoke')],cwd=root,check=True)

subprocess.run(['cc','-std=c11','-Wall','-Wextra','-Werror','-Iinclude','examples/c_tracking.c',str(target/'release/libqr_ffi.a'),*libs,'-o',str(target/'c-tracking')],cwd=root,check=True)
subprocess.run([str(target/'c-tracking')],cwd=root,check=True)
