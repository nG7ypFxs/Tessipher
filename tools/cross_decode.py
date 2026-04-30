#!/usr/bin/env python3
import pathlib,subprocess
root=pathlib.Path(__file__).resolve().parents[1]
(root/'target').mkdir(exist_ok=True)
subprocess.run(['cc','-std=c99','-O2','-Wall','-Wextra','-Itools/vendor/quirc','tools/cross_decode.c','tools/vendor/quirc/decode.c','tools/vendor/quirc/version_db.c','-o','target/cross-decode'],cwd=root,check=True)
subprocess.run(['target/cross-decode'],cwd=root,check=True)
