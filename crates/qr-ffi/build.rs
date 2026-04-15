use std::{env, fs, path::PathBuf};
include!("abi_schema.rs");
fn main() {
    println!("cargo:rerun-if-changed=abi_schema.rs");
    println!("cargo:rerun-if-changed=prototypes.h");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let mut dimensions = Vec::new();
    for name in ["QR_MAX_WIDTH", "QR_MAX_HEIGHT"] {
        println!("cargo:rerun-if-env-changed={name}");
        let n = env::var(name)
            .ok()
            .map(|s| {
                s.parse::<usize>()
                    .expect("image maximum must be an integer")
            })
            .unwrap_or(640);
        assert!((1..=32768).contains(&n));
        dimensions.push(n);
    }
    fs::write(
        out.join("dimensions.rs"),
        format!(
            "pub const MAX_WIDTH: usize = {};\npub const MAX_HEIGHT: usize = {};\n",
            dimensions[0], dimensions[1]
        ),
    )
    .unwrap();
    let mut header=String::from("/* Generated from qr-ffi/abi_schema.rs and prototypes.h. Do not edit. */\n#ifndef TESSIPHER_H\n#define TESSIPHER_H\n#include <stddef.h>\n#include <stdint.h>\n#ifdef __cplusplus\nextern \"C\" {\n#endif\n");
    macro_rules! c_type {
        (u32) => {
            "uint32_t"
        };
        (usize) => {
            "size_t"
        };
        (f32) => {
            "float"
        };
    }
    macro_rules! abi_struct {($rust:ident => $c:literal {$($field:ident:$ty:ident),*})=>{{header.push_str(concat!("typedef struct ",$c," {\n"));$(header.push_str(concat!("    ",c_type!($ty)," ",stringify!($field),";\n"));)*header.push_str(concat!("} ",$c,";\n"));}};}
    abi_schema!();
    header.push_str(include_str!("prototypes.h"));
    header.push_str("\n#ifdef __cplusplus\n}\n#endif\n#endif\n");
    fs::write(out.join("tessipher.h"), header).unwrap();
}
