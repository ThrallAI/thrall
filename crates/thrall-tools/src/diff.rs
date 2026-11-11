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

/// The new side of a unified diff as numbered lines, like `read_file` shows them: each changed
/// part with its context, parts separated by `…`, at most `max_lines` lines.
pub fn new_lines(unified: &str, max_lines: usize) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut line = 0usize;
    let mut shown = 0usize;
    let mut left = 0usize;
    for row in unified.lines() {
        if let Some(header) = row.strip_prefix("@@") {
            // "@@ -a,b +c,d @@": the new side starts at line c.
            line = header
                .split_whitespace()
                .find_map(|part| part.strip_prefix('+'))
                .and_then(|range| range.split(',').next())
                .and_then(|start| start.parse().ok())
                .unwrap_or(1);
            if !out.is_empty() && shown < max_lines {
                out.push("     …".to_owned());
            }
            continue;
        }
        if row.starts_with("+++") || row.starts_with("---") || row.starts_with('\\') {
            continue;
        }
        let text = match row.chars().next() {
            Some(' ' | '+') => &row[1..],
            Some('-') => continue,
            _ => row,
        };
        if shown < max_lines {
            out.push(format!("{line:>6}\t{text}"));
            shown += 1;
        } else {
            left += 1;
        }
        line += 1;
    }
    if left > 0 {
        out.push(format!(
            "({left} more lines not shown; read the file if you need them)"
        ));
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_lines_are_numbered_as_the_file_now_reads() {
        let old = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\n";
        let new = "a\nB\nc\nd\ne\nf\ng\nh\ni\nj\nK\nl\n";
        let d = make("x.txt", Some(old), new);
        let shown = new_lines(&d.unified, 80);
        assert!(shown.contains("     2\tB"), "{shown}");
        assert!(shown.contains("    11\tK"), "{shown}");
        assert!(shown.contains('…'), "{shown}");
        assert!(!shown.lines().any(|l| l.ends_with("\tb")), "{shown}");
        let short = new_lines(&d.unified, 2);
        assert!(short.contains("more lines not shown"), "{short}");
    }

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
