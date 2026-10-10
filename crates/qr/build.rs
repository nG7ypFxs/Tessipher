use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rustc-check-cfg=cfg(tessipher_repository_fixtures)");

    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let repository_root = manifest_dir.join("../..");
    let expected = [
        "Cargo.toml",
        ".git",
        "fixtures/all-model2.bin",
        "fixtures/v7.bin",
        "fixtures/v2.bin",
        "fixtures/v1.pgm",
    ];
    for path in expected {
        println!(
            "cargo:rerun-if-changed={}",
            repository_root.join(path).display()
        );
    }

    // The fixture corpus is intentionally repository-only. Requiring the
    // repository marker as well as the expected files prevents an unpacked
    // consumer archive from activating tests merely because a similarly
    // named directory exists next to it.
    if expected
        .iter()
        .all(|path| repository_root.join(path).exists())
        && is_repository_root(&repository_root)
    {
        println!("cargo:rustc-cfg=tessipher_repository_fixtures");
    }
}

fn is_repository_root(path: &Path) -> bool {
    path.join(".git").is_dir() || path.join(".git").is_file()
}
