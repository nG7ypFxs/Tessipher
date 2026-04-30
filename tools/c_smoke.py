#!/usr/bin/env python3
import pathlib, subprocess, platform
root=pathlib.Path(__file__).resolve().parents[1]
subprocess.run(['python3','tools/check_header.py'],cwd=root,check=True)
libs=['-lpthread','-lm']
if platform.system()=='Darwin': libs+=['-framework','Security','-framework','CoreFoundation','-liconv']
else: libs+=['-ldl']
subprocess.run(['cc','-std=c11','-Wall','-Wextra','-Werror','-Iinclude','examples/c_smoke.c','target/release/libqr_ffi.a',*libs,'-o','target/c-smoke'],cwd=root,check=True)
subprocess.run(['target/c-smoke'],cwd=root,check=True)
