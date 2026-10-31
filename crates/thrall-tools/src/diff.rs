//! Unified diffs for file edits (protocol §8).

use similar::{ChangeTag, TextDiff};
use thrall_protocol::{Diff, DiffKind};

/// The diff between `old` and `new` for `path`. `old` is `None` for a new file.
pub fn make(path: &str, old: Option<&str>, new: &str) -> Diff {
    let before = old.unwrap_or_default();
    let text = TextDiff::from_lines(before, new);
    let mut added = 0u32;
    let mut removed = 0u32;
    for change in text.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => added += 1,
            ChangeTag::Delete => removed += 1,
            ChangeTag::Equal => {}
        }
    }
    let unified = text
        .unified_diff()
        .context_radius(3)
        .header(
            &if old.is_some() {
                format!("a/{path}")
            } else {
                "/dev/null".to_owned()
            },
            &format!("b/{path}"),
        )
        .to_string();
    Diff {
        path: path.to_owned(),
        kind: if old.is_some() {
            DiffKind::Modify
        } else {
            DiffKind::Create
        },
        unified,
        added,
        removed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_lines() {
        let d = make("a.txt", Some("one\ntwo\n"), "one\nTWO\nthree\n");
        assert_eq!((d.added, d.removed), (2, 1));
        assert_eq!(d.kind, DiffKind::Modify);
        assert!(d.unified.contains("+TWO"));
    }

    #[test]
    fn new_files_are_creates() {
        let d = make("n.txt", None, "hi\n");
        assert_eq!(d.kind, DiffKind::Create);
        assert_eq!(d.added, 1);
    }
}
