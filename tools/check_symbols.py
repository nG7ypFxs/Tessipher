#!/usr/bin/env python3
"""Inspect the real Cortex no-host static archive for heap/unwind runtime imports."""
import os,pathlib,subprocess,re
root=pathlib.Path(__file__).resolve().parents[1]
target=pathlib.Path(os.environ.get('CARGO_TARGET_DIR',root/'target'))
if not target.is_absolute(): target=root/target
archive=target/'thumbv7em-none-eabi/release/libqr_ffi.a'
text=subprocess.check_output(['nm','-u',str(archive)],text=True)
forbidden=re.compile(r'(?:\b(?:malloc|calloc|realloc|free|_Unwind_\w+|rust_eh_personality)\b|__rust_(?:alloc|dealloc|realloc)|alloc_error_handler)')
bad=[line for line in text.splitlines() if forbidden.search(line)]
assert not bad,'\n'.join(bad)
# The symbol rust_begin_unwind names Rust's panic entry even with panic=abort.
# In this build it resolves to the crate's explicit non-unwinding infinite-loop handler.
print('Cortex no-host static archive: no malloc/calloc/realloc/free/Rust allocation/_Unwind/personality imports')
