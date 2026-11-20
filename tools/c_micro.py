#!/usr/bin/env python3
# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Build and test default, matrix-only, and detector-enabled Micro QR C ABIs."""

import pathlib
import platform
import re
import subprocess
import tempfile

from host_ffi import rust_host


ROOT = pathlib.Path(__file__).resolve().parents[1]
VARIANTS = {
    "default": ([], 0, 0),
    "matrix": (["--features", "micro-qr"], 1, 0),
    "detect": (["--features", "micro-qr-detect"], 1, 1),
}


def export(directory, cargo_args):
    subprocess.run(
        ["uv", "run", str(ROOT / "tools/export_ffi.py"),
         "--output-dir", directory.name, "--target", rust_host(), *cargo_args],
        cwd=directory.parent,
        check=True,
    )
    return directory / "libqr_ffi.a", directory / "tessipher.h"


def normalized_header(path):
    text = path.read_text()
    return re.sub(r"(#define QR_HAS_MICRO_QR(?:_DETECT)?) [01]", r"\1 FEATURE", text)


def symbols(archive):
    sysroot = pathlib.Path(subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip())
    matches = list(sysroot.glob("lib/rustlib/*/bin/llvm-nm"))
    nm = matches[0] if matches else pathlib.Path("nm")
    output = subprocess.check_output(
        [str(nm), "-g", str(archive)], text=True, stderr=subprocess.DEVNULL
    )
    return {match.group(1) for match in re.finditer(r"\b_?(qr_[A-Za-z0-9_]+)$", output, re.M)}


def main(target):
    libs = ["-lpthread", "-lm"]
    if platform.system() == "Darwin":
        libs += ["-framework", "Security", "-framework", "CoreFoundation", "-liconv"]
    else:
        libs += ["-ldl"]
    builds = {}
    reference = None
    for name, (cargo_args, matrix, detect) in VARIANTS.items():
        archive, header = export(target / name, cargo_args)
        current = normalized_header(header)
        if reference is None:
            reference = current
        elif current != reference:
            raise RuntimeError(f"generated {name} header differs beyond feature macro values")
        executable = target / name / "c-micro"
        subprocess.run(
            [
                "cc",
                "-std=c11",
                "-Wall",
                "-Wextra",
                "-Werror",
                f"-DEXPECT_MATRIX={matrix}",
                f"-DEXPECT_DETECT={detect}",
                f"-I{header.parent}",
                str(ROOT / "examples/c_micro.c"),
                str(archive),
                *libs,
                "-o",
                str(executable),
            ],
            cwd=ROOT,
            check=True,
        )
        subprocess.run([str(executable)], cwd=ROOT, check=True)
        builds[name] = (archive, symbols(archive))

    matrix_symbols = {"qr_micro_memory", "qr_decode_micro_matrix"}
    detect_symbols = {
        "qr_decode_micro_gray",
        "qr_decode_micro_gray_with_budget",
        "qr_decode_micro_gray_with_sampling",
    }
    assert not (builds["default"][1] & (matrix_symbols | detect_symbols))
    assert matrix_symbols <= builds["matrix"][1] and not (builds["matrix"][1] & detect_symbols)
    assert matrix_symbols | detect_symbols <= builds["detect"][1]

    distributed = target / "detect" / "distributed-header-micro"
    subprocess.run(
        [
            "cc",
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-DEXPECT_MATRIX=1",
            "-DEXPECT_DETECT=1",
            "-DQR_HAS_MICRO_QR=1",
            "-DQR_HAS_MICRO_QR_DETECT=1",
            f"-I{ROOT / 'include'}",
            str(ROOT / "examples/c_micro.c"),
            str(builds["detect"][0]),
            *libs,
            "-o",
            str(distributed),
        ],
        cwd=ROOT,
        check=True,
    )
    subprocess.run([str(distributed)], cwd=ROOT, check=True)

    old_header = (ROOT / "tests/ffi/abi1/tessipher.h").read_bytes()
    with tempfile.TemporaryDirectory() as directory:
        include = pathlib.Path(directory)
        (include / "tessipher.h").write_bytes(old_header)
        for name, (archive, _) in builds.items():
            executable = target / name / "old-header-smoke"
            subprocess.run(
                [
                    "cc",
                    "-std=c11",
                    "-Wall",
                    "-Wextra",
                    "-Werror",
                    "-DQR_TEST_LEGACY_HEADER",
                    f"-I{include}",
                    str(ROOT / "examples/c_smoke.c"),
                    str(archive),
                    *libs,
                    "-o",
                    str(executable),
                ],
                cwd=ROOT,
                check=True,
            )
            subprocess.run([str(executable)], cwd=ROOT, check=True)
    print("Micro QR C headers, symbols, calls, and old ABI-1 callers passed")


if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="tessipher-c-micro-") as directory:
        main(pathlib.Path(directory))
