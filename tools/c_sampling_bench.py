#!/usr/bin/env python3
"""Build and run the public C paired benchmark; no photo or payload output."""
import os, pathlib, platform, subprocess
root=pathlib.Path(__file__).resolve().parents[1]
target=pathlib.Path(os.environ.get('CARGO_TARGET_DIR',root/'target'))
if not target.is_absolute(): target=root/target
subprocess.run(['cargo','build','-p','qr-ffi','--release','--locked'],cwd=root,check=True)
libs=['-lpthread','-lm']
if platform.system()=='Darwin': libs+=['-framework','Security','-framework','CoreFoundation','-liconv']
else: libs+=['-ldl']
subprocess.run(['cc','-O2','-std=c11','-Wall','-Wextra','-Werror','-Iinclude','examples/c_sampling_bench.c',str(target/'release/libqr_ffi.a'),*libs,'-o',str(target/'c-sampling-bench')],cwd=root,check=True)
subprocess.run([str(target/'c-sampling-bench')],cwd=root,check=True)
