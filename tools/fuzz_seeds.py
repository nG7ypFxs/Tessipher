#!/usr/bin/env python3
import pathlib,shutil
root=pathlib.Path(__file__).resolve().parents[1]
shutil.copytree(root/'fuzz/seeds',root/'fuzz/corpus',dirs_exist_ok=True)
print('Copied immutable fuzz seeds to working corpora')
