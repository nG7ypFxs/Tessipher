#!/usr/bin/env python3
import os,pathlib,subprocess
root=pathlib.Path(__file__).resolve().parents[1]
target=pathlib.Path(os.environ.get('CARGO_TARGET_DIR',root/'target'))
if not target.is_absolute(): target=root/target
target.mkdir(parents=True,exist_ok=True)
subprocess.run(['cc','-std=c99','-O2','-Wall','-Wextra','-Itools/vendor/quirc','tools/cross_decode.c','tools/vendor/quirc/decode.c','tools/vendor/quirc/version_db.c','-o',str(target/'cross-decode')],cwd=root,check=True)
subprocess.run([str(target/'cross-decode')],cwd=root,check=True)
