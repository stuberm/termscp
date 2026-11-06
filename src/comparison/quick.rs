//! Metadata-only classification. These results never populate the content cache.
use std::time::{SystemTime, UNIX_EPOCH};

use super::settings::QuickCheck;
use super::{Row, Status};

// Floor to UTC seconds, including timestamps before the Unix epoch. Remote
// listings commonly omit subsecond precision; do not compare local nanoseconds
// with the synthetic zero nanoseconds of a remote timestamp.
pub(super) fn seconds(time: SystemTime) -> i128 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => i128::from(duration.as_secs()),
        Err(error) => {
            let duration = error.duration();
            -i128::from(duration.as_secs()) - i128::from(duration.subsec_nanos() != 0)
        }
    }
}

/// Default-policy shorthand for existing metadata regression fixtures.
#[cfg(test)]
pub(super) fn compare(row: &mut Row) {
    compare_with(row, QuickCheck::default());
}

/// Called only after shared classification accepts a pair of regular files.
pub(super) fn compare_with(row: &mut Row, checks: QuickCheck) {
    // Pools route None to Full; never let an empty predicate imply a match.
    if checks == QuickCheck::None {
        row.status = Status::Unverified;
        row.detail = "Quick checks disabled; full content verification required".into();
        return;
    }
    let (Some(left), Some(right)) = (&row.left, &row.right) else {
        return;
    };
    let sizes = left.size.zip(right.size);
    let times = left.modified.zip(right.modified);
    let sizes_differ = checks.uses_size() && sizes.is_some_and(|(a, b)| a != b);
    let times_differ =
        checks.uses_timestamp() && times.is_some_and(|(a, b)| seconds(a) != seconds(b));
    let complete =
        (!checks.uses_size() || sizes.is_some()) && (!checks.uses_timestamp() || times.is_some());
    (row.status, row.detail) = if sizes_differ || times_differ {
        (
            Status::Different,
            format!(
                "Quick comparison ({}): metadata differs; contents not read (F5 for full comparison)",
                checks.label()
            ),
        )
    } else if complete {
        (
            Status::QuickMatch,
            format!(
                "Quick comparison ({}): selected metadata matches{}; contents not verified (F5 for full comparison)",
                checks.label(),
                if checks.uses_timestamp() {
                    " (whole-second precision)"
                } else {
                    ""
                }
            ),
        )
    } else {
        (
            Status::Unverified,
            format!(
                "Quick comparison ({}): selected metadata unavailable; contents not read (F5 for full comparison)",
                checks.label()
            ),
        )
    };
}

#[cfg(test)]
#[path = "quick_tests.rs"]
mod tests;
