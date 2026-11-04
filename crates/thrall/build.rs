//! Gives `thrall.exe` its version and name in Windows file properties, so Task Manager and
//! Properties show the same version as the desktop app. Does nothing on other systems.

use std::env;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return Ok(());
    }
    let version = env::var("CARGO_PKG_VERSION")?;
    let numbers = format!(
        "{},{},{},0",
        env::var("CARGO_PKG_VERSION_MAJOR")?,
        env::var("CARGO_PKG_VERSION_MINOR")?,
        env::var("CARGO_PKG_VERSION_PATCH")?
    );
    let rc = format!(
        r#"#include <winver.h>
1 VERSIONINFO
FILEVERSION {numbers}
PRODUCTVERSION {numbers}
FILEOS VOS_NT_WINDOWS32
FILETYPE VFT_APP
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "Thrall"
      VALUE "FileDescription", "Thrall engine"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "thrall"
      VALUE "OriginalFilename", "thrall.exe"
      VALUE "ProductName", "Thrall"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0409, 1200
  END
END
"#
    );
    let out = PathBuf::from(env::var("OUT_DIR")?).join("thrall.rc");
    fs::write(&out, rc)?;
    embed_resource::compile(&out, embed_resource::NONE).manifest_optional()?;
    Ok(())
}
