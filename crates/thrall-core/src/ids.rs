//! Ids and time. Ids are UUIDv7 with a type prefix, so they sort by creation time and say what
//! they point at.

use std::time::{SystemTime, UNIX_EPOCH};

/// A new id such as `s_0192f3c1…`.
pub fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::now_v7().simple())
}

/// Now, as Unix milliseconds (the time format used in storage and the protocol).
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_carry_prefix_and_differ() {
        let a = new_id("s");
        let b = new_id("s");
        assert!(a.starts_with("s_"));
        assert_ne!(a, b);
    }

    #[test]
    fn ids_sort_by_creation() {
        let a = new_id("t");
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = new_id("t");
        assert!(a < b);
    }

    #[test]
    fn now_is_after_2025() {
        assert!(now_ms() > 1_735_689_600_000);
    }
}
