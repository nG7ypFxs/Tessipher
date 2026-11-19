"""Build the native Tessipher FFI library and locate its matching generated header."""

from __future__ import annotations

import json
from pathlib import Path
import subprocess
from typing import NamedTuple

ROOT = Path(__file__).resolve().parents[1]


class HostFfi(NamedTuple):
    archive: Path
    header: Path


def rust_host() -> str:
    version = subprocess.check_output(["rustc", "-vV"], cwd=ROOT, text=True)
    for line in version.splitlines():
        if line.startswith("host: "):
            return line.removeprefix("host: ")
    raise RuntimeError("rustc did not report its host target")


def build_host_ffi(*cargo_args: str) -> HostFfi:
    command = ["cargo", "build", "-p", "tessipher-ffi", "--lib", "--release", "--locked",
               "--target", rust_host(), "--message-format=json-render-diagnostics", *cargo_args]
    messages = subprocess.check_output(command, cwd=ROOT, text=True)
    archives = []
    headers = {}
    for line in messages.splitlines():
        record = json.loads(line)
        if record["reason"] == "build-script-executed":
            headers[record["package_id"]] = Path(record["out_dir"]) / "tessipher.h"
        elif record["reason"] == "compiler-artifact" and record["target"]["name"] == "qr_ffi":
            for filename in record["filenames"]:
                path = Path(filename)
                if path.name == "libqr_ffi.a":
                    archives.append((record["package_id"], path))
    if len(archives) != 1:
        raise RuntimeError("Cargo did not report one native Tessipher static library")
    package, archive = archives[0]
    header = headers.get(package)
    if not archive.is_file() or header is None or not header.is_file():
        raise RuntimeError("The reported Tessipher static library or generated header is missing")
    return HostFfi(archive, header)
