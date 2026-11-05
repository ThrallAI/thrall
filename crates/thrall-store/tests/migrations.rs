//! Database migrations are checksummed where they're applied: a file whose bytes differ between
//! builds (Windows line endings from one editor, not another) stops that database opening. So
//! migrations are plain `\n` text, except one already applied as it is.
#![allow(clippy::unwrap_used, clippy::expect_used)]

/// Applied to databases with a `\r\n` at its end before this was checked; it must stay as it is.
const KEPT_AS_APPLIED: &[&str] = &["0016_people_rules.sql"];

#[test]
fn migrations_are_plain_text_so_every_build_checksums_them_alike() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations/sqlite");
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if KEPT_AS_APPLIED.contains(&name.as_str()) {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        assert!(
            !bytes.contains(&b'\r'),
            "{name} has Windows line endings; save it with plain \\n line endings"
        );
    }
}
