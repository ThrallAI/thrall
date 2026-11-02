//! Copies the built web app (`web/dist`) into this crate's build folder so it's embedded in the
//! binary. Without a build there, a one-page notice saying how to build it is embedded instead,
//! so the engine always compiles.

use std::path::{Path, PathBuf};

const PLACEHOLDER: &str = "<!doctype html><meta charset=\"utf-8\"><title>Thrall</title>\
<body style=\"font-family:system-ui;background:#111018;color:#ecebf2;padding:32px\">\
<h1>The web app isn't built into this copy of Thrall.</h1>\
<p>Run <code>npm ci</code> and <code>npm run build</code> in <code>web/</code>, then build Thrall again.</p>";

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn main() -> std::io::Result<()> {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let dist = manifest.join("../../web/dist");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default()).join("web");
    println!("cargo:rerun-if-changed={}", dist.display());
    if out.exists() {
        std::fs::remove_dir_all(&out)?;
    }
    if dist.join("index.html").exists() {
        copy_dir(&dist, &out)?;
    } else {
        std::fs::create_dir_all(&out)?;
        std::fs::write(out.join("index.html"), PLACEHOLDER)?;
        println!("cargo:warning=web/dist is missing: embedding a placeholder page");
    }
    Ok(())
}
