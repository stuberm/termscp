use std::path::PathBuf;

#[derive(Debug, Clone)]
pub(crate) struct DiffView {
    pub left_title: String,
    pub right_title: String,
    pub rows: Vec<DiffRow>,
}

#[derive(Debug, Clone)]
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

impl DiffView {
    pub fn new(left_path: PathBuf, right_path: PathBuf, left: &str, right: &str) -> Self {
        Self {
            left_title: left_path.display().to_string(),
            right_title: right_path.display().to_string(),
            rows: diff_lines(left, right),
        }
    }
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
    use super::{DiffKind, diff_lines};

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
}
