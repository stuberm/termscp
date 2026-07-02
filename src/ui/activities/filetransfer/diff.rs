use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DiffView {
    pub left_title: String,
    pub right_title: String,
    pub rows: Vec<DiffRow>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DiffRow {
    pub kind: DiffKind,
    pub left_no: Option<usize>,
    pub left: String,
    pub right_no: Option<usize>,
    pub right: String,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum DiffKind {
    Equal,
    Added,
    Removed,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum DiffApplyDirection {
    LeftToRight,
    RightToLeft,
}

impl DiffView {
    pub fn new(left_path: PathBuf, right_path: PathBuf, left: &str, right: &str) -> Self {
        Self {
            left_title: left_path.display().to_string(),
            right_title: right_path.display().to_string(),
            rows: diff_lines(left, right),
        }
    }
}

pub(crate) fn apply_change(
    left: &str,
    right: &str,
    row_index: usize,
    direction: DiffApplyDirection,
) -> Result<String, String> {
    let rows = diff_lines(left, right);
    if rows
        .get(row_index)
        .is_none_or(|row| row.kind == DiffKind::Equal)
    {
        return Err("Select a changed line to copy".to_string());
    }

    let mut start = row_index;
    while start > 0 && rows[start - 1].kind != DiffKind::Equal {
        start -= 1;
    }
    let mut end = row_index + 1;
    while end < rows.len() && rows[end].kind != DiffKind::Equal {
        end += 1;
    }

    let hunk = &rows[start..end];
    let left_lines = left.lines().map(ToString::to_string).collect::<Vec<_>>();
    let right_lines = right.lines().map(ToString::to_string).collect::<Vec<_>>();
    let left_start = hunk_start(&rows, start, true);
    let right_start = hunk_start(&rows, start, false);
    let left_count = hunk.iter().filter(|row| row.left_no.is_some()).count();
    let right_count = hunk.iter().filter(|row| row.right_no.is_some()).count();
    let left_block = hunk
        .iter()
        .filter_map(|row| row.left_no.map(|_| row.left.clone()))
        .collect::<Vec<_>>();
    let right_block = hunk
        .iter()
        .filter_map(|row| row.right_no.map(|_| row.right.clone()))
        .collect::<Vec<_>>();

    let mut target = match direction {
        DiffApplyDirection::LeftToRight => right_lines,
        DiffApplyDirection::RightToLeft => left_lines,
    };
    let (start, count, replacement) = match direction {
        DiffApplyDirection::LeftToRight => (right_start, right_count, left_block),
        DiffApplyDirection::RightToLeft => (left_start, left_count, right_block),
    };
    target.splice(start..start + count, replacement);

    let original_target = match direction {
        DiffApplyDirection::LeftToRight => right,
        DiffApplyDirection::RightToLeft => left,
    };
    Ok(join_lines(target, original_target.ends_with('\n')))
}

fn hunk_start(rows: &[DiffRow], start: usize, left: bool) -> usize {
    rows[start..]
        .iter()
        .take_while(|row| row.kind != DiffKind::Equal)
        .find_map(|row| if left { row.left_no } else { row.right_no })
        .map(|no| no - 1)
        .unwrap_or_else(|| {
            rows[..start]
                .iter()
                .rev()
                .find_map(|row| if left { row.left_no } else { row.right_no })
                .unwrap_or(0)
        })
}

fn join_lines(lines: Vec<String>, trailing_newline: bool) -> String {
    let mut content = lines.join("\n");
    if trailing_newline && !content.is_empty() {
        content.push('\n');
    }
    content
}

pub(crate) fn diff_lines(left: &str, right: &str) -> Vec<DiffRow> {
    let left_lines = left.lines().collect::<Vec<_>>();
    let right_lines = right.lines().collect::<Vec<_>>();
    let m = left_lines.len();
    let n = right_lines.len();
    let mut lcs = vec![vec![0usize; n + 1]; m + 1];

    for i in (0..m).rev() {
        for j in (0..n).rev() {
            lcs[i][j] = if left_lines[i] == right_lines[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }

    let mut rows = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < m || j < n {
        if i < m && j < n && left_lines[i] == right_lines[j] {
            rows.push(DiffRow {
                kind: DiffKind::Equal,
                left_no: Some(i + 1),
                left: left_lines[i].to_string(),
                right_no: Some(j + 1),
                right: right_lines[j].to_string(),
            });
            i += 1;
            j += 1;
        } else if j < n && (i == m || lcs[i][j + 1] > lcs[i + 1][j]) {
            rows.push(DiffRow {
                kind: DiffKind::Added,
                left_no: None,
                left: String::new(),
                right_no: Some(j + 1),
                right: right_lines[j].to_string(),
            });
            j += 1;
        } else if i < m {
            rows.push(DiffRow {
                kind: DiffKind::Removed,
                left_no: Some(i + 1),
                left: left_lines[i].to_string(),
                right_no: None,
                right: String::new(),
            });
            i += 1;
        }
    }

    rows
}

#[cfg(test)]
mod tests {
    use super::{DiffApplyDirection, DiffKind, apply_change, diff_lines};

    #[test]
    fn should_diff_lines() {
        let rows = diff_lines("a\nb\nc", "a\nx\nc\nd");
        let kinds = rows.iter().map(|row| row.kind).collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                DiffKind::Equal,
                DiffKind::Removed,
                DiffKind::Added,
                DiffKind::Equal,
                DiffKind::Added,
            ]
        );
    }

    #[test]
    fn should_apply_replacement_left_to_right() {
        let applied = apply_change("a\nb\nc\n", "a\nx\nc\n", 1, DiffApplyDirection::LeftToRight)
            .expect("apply change");
        assert_eq!(applied, "a\nb\nc\n");
    }

    #[test]
    fn should_apply_insertion_right_to_left() {
        let applied = apply_change("a\nc", "a\nb\nc", 1, DiffApplyDirection::RightToLeft)
            .expect("apply change");
        assert_eq!(applied, "a\nb\nc");
    }

    #[test]
    fn should_apply_deletion_left_to_right() {
        let applied = apply_change("a\nc", "a\nb\nc", 1, DiffApplyDirection::LeftToRight)
            .expect("apply change");
        assert_eq!(applied, "a\nc");
    }
}
