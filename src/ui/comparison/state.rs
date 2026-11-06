use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::comparison::{Row, Status, aggregate, is_ignored};

#[derive(Default)]
pub(super) struct Browser {
    pub rows: BTreeMap<PathBuf, Row>,
    pub visible: Vec<PathBuf>,
    pub directory: PathBuf,
    pub selected: usize,
    pub equal_count: usize,
    pub marked: BTreeSet<PathBuf>,
    pub(super) anchor: Option<PathBuf>,
    pub differences_only: bool,
    pub recursive: bool,
}

impl Browser {
    /// Update streamed data/counts. Return whether visible membership/order can
    /// change; ordinary status/detail updates need repainting, not a tree rebuild.
    pub fn update(&mut self, row: Row) -> bool {
        let equal = usize::from(row.status.is_match());
        match self.rows.entry(row.path.clone()) {
            Entry::Occupied(mut current) => {
                let previous = current.get();
                let rebuild = previous.is_directory() != row.is_directory()
                    || (self.differences_only && previous.status.is_match() != (equal != 0));
                self.equal_count -= usize::from(previous.status.is_match());
                self.equal_count += equal;
                current.insert(row);
                rebuild
            }
            Entry::Vacant(current) => {
                self.equal_count += equal;
                current.insert(row);
                true
            }
        }
    }

    /// Merge a scoped copy observation without discarding the folder snapshot.
    pub fn apply_copy_updates(&mut self, roots: &[PathBuf], updates: Result<Vec<Row>, String>) {
        let affected = |path: &Path| roots.iter().any(|root| path.starts_with(root));
        let complete = updates.as_ref().is_ok_and(|rows| {
            !rows.is_empty()
                && rows
                    .iter()
                    .all(|row| !matches!(row.status, Status::Pending | Status::Error))
        });
        if complete {
            self.rows.retain(|path, _| !affected(path));
        } else {
            // Failed/cancelled copies may have left partial writes. Never keep
            // old equal/quick-match claims for a scope we could not recheck.
            for row in self.rows.values_mut().filter(|row| affected(&row.path)) {
                row.status = Status::Unverified;
                row.detail = "Copy may have changed this entry; recheck incomplete (r/F5)".into();
                row.aggregate_base = None;
            }
        }
        for mut row in updates.unwrap_or_default() {
            let own_status = row
                .aggregate_base
                .as_ref()
                .map_or(row.status, |(status, _)| *status);
            if !affected(&row.path)
                && row.is_directory()
                && own_status.is_match()
                && let Some(previous) = self.rows.get(&row.path)
            {
                let (status, detail) = previous
                    .aggregate_base
                    .as_ref()
                    .map(|(status, detail)| (*status, detail.clone()))
                    .unwrap_or((previous.status, previous.detail.clone()));
                // Parent metadata alone cannot clear an incomplete inventory or
                // upgrade a quick-match parent to full equality. Missing parents
                // created by the copy, however, now have a fresh paired base.
                if !matches!(
                    status,
                    Status::LeftOnly | Status::RightOnly | Status::TypeConflict
                ) {
                    row.status = status;
                    row.detail = detail;
                    row.aggregate_base = None;
                }
            }
            self.rows.insert(row.path.clone(), row);
        }
        let mut rows: Vec<_> = std::mem::take(&mut self.rows).into_values().collect();
        aggregate(&mut rows);
        self.rows = rows
            .into_iter()
            .map(|row| (row.path.clone(), row))
            .collect();
        self.rebuild();
    }

    /// Remove ignored rows and update directory summaries without any filesystem I/O.
    pub fn apply_ignores(&mut self, ignores: &[String]) {
        if ignores.is_empty() {
            return;
        }
        let mut rows: Vec<_> = std::mem::take(&mut self.rows)
            .into_values()
            .filter(|row| !is_ignored(&row.path, ignores))
            .collect();
        aggregate(&mut rows);
        self.rows = rows
            .into_iter()
            .map(|row| (row.path.clone(), row))
            .collect();
    }

    pub fn rebuild(&mut self) {
        let selected = self.visible.get(self.selected).cloned();
        let mut directories = Vec::new();
        let mut files = Vec::new();
        self.equal_count = 0;
        for row in self.rows.values() {
            self.equal_count += usize::from(row.status.is_match());
            if row.path.as_os_str().is_empty()
                || row.path == self.directory
                || !(if self.recursive {
                    row.path.starts_with(&self.directory)
                } else {
                    row.path.parent() == Some(self.directory.as_path())
                })
                || (self.differences_only && row.status.is_match())
            {
                continue;
            }
            // BTreeMap already supplies path order. Partitioning preserves that
            // order without repeated tree lookups inside a sort comparator.
            if row.is_directory() {
                directories.push(row.path.clone());
            } else {
                files.push(row.path.clone());
            }
        }
        directories.extend(files);
        self.visible = directories;
        if !self.marked.is_empty() || self.anchor.is_some() {
            let visible: BTreeSet<_> = self.visible.iter().map(PathBuf::as_path).collect();
            self.marked.retain(|path| visible.contains(path.as_path()));
            if self
                .anchor
                .as_ref()
                .is_some_and(|path| !visible.contains(path.as_path()))
            {
                self.anchor = None;
            }
        }
        self.selected = selected
            .and_then(|path| self.visible.iter().position(|item| item == &path))
            .unwrap_or(self.selected)
            .min(self.visible.len().saturating_sub(1));
    }

    pub fn toggle_differences(&mut self) {
        self.differences_only = !self.differences_only;
        self.rebuild();
    }

    pub fn selected_row(&self) -> Option<&Row> {
        self.visible
            .get(self.selected)
            .and_then(|path| self.rows.get(path))
    }

    pub fn clear_marks(&mut self) {
        self.marked.clear();
        self.anchor = None;
    }

    pub fn select_index(&mut self, index: usize) {
        self.clear_marks();
        self.selected = index.min(self.visible.len().saturating_sub(1));
    }

    pub fn move_selection(&mut self, amount: isize) {
        self.select_index(self.selected.saturating_add_signed(amount));
    }

    pub fn extend_selection(&mut self, amount: isize) {
        let Some(current) = self.visible.get(self.selected).cloned() else {
            return;
        };
        let anchor = self.anchor.get_or_insert(current);
        let start = self
            .visible
            .iter()
            .position(|path| path == anchor)
            .unwrap_or(self.selected);
        self.selected = self
            .selected
            .saturating_add_signed(amount)
            .min(self.visible.len().saturating_sub(1));
        self.marked = self.visible[start.min(self.selected)..=start.max(self.selected)]
            .iter()
            .cloned()
            .collect();
    }

    pub fn copy_paths(&self) -> Vec<PathBuf> {
        if self.marked.is_empty() {
            self.visible
                .get(self.selected)
                .cloned()
                .into_iter()
                .collect()
        } else {
            self.visible
                .iter()
                .filter(|path| self.marked.contains(*path))
                .cloned()
                .collect()
        }
    }

    pub fn enter(&mut self, path: &Path) {
        self.clear_marks();
        self.directory = path.into();
        self.visible.clear();
        self.selected = 0;
        self.rebuild();
    }

    pub fn parent(&mut self) {
        self.clear_marks();
        let previous = self.directory.clone();
        if self.directory.pop() {
            self.visible.clear();
            self.rebuild();
            self.selected = self
                .visible
                .iter()
                .position(|path| path == &previous)
                .unwrap_or(0);
        }
    }

    pub fn next_difference(&mut self, backwards: bool) {
        self.clear_marks();
        let length = self.visible.len();
        for step in 1..=length {
            let index = if backwards {
                (self.selected + length - step) % length
            } else {
                (self.selected + step) % length
            };
            if !self.rows[&self.visible[index]].status.is_match() {
                self.selected = index;
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comparison::filesystem::{Entry, Kind};

    fn row(path: &str, kind: Kind, status: Status) -> Row {
        Row {
            path: path.into(),
            left: Some(Entry {
                kind,
                size: None,
                modified: None,
            }),
            right: None,
            status,
            detail: String::new(),
            aggregate_base: None,
        }
    }

    #[test]
    fn streamed_status_updates_keep_counts_without_rebuilding_unchanged_membership() {
        let mut browser = Browser::default();
        assert!(browser.update(row("file", Kind::File, Status::Pending)));
        browser.rebuild();
        let visible = browser.visible.clone();
        assert_eq!(browser.equal_count, 0);
        assert!(!browser.update(row("file", Kind::File, Status::Equal)));
        assert_eq!(browser.equal_count, 1);
        assert_eq!(browser.visible, visible);
        assert!(!browser.update(row("file", Kind::File, Status::Different)));
        assert_eq!(browser.equal_count, 0);
        assert!(browser.update(row("file", Kind::Directory, Status::Pending)));
        browser.rebuild();
        assert_eq!(browser.visible, visible);
        browser.toggle_differences();
        assert!(browser.update(row("file", Kind::Directory, Status::Equal)));
        browser.rebuild();
        assert_eq!(browser.equal_count, 1);
        assert!(browser.visible.is_empty());
        assert!(browser.update(row("file", Kind::Directory, Status::Pending)));
        browser.rebuild();
        assert_eq!(browser.equal_count, 0);
        assert_eq!(browser.visible, visible);
    }

    #[test]
    fn equal_totals_include_hidden_and_nested_entries_and_reset_on_replacement() {
        let mut browser = Browser::default();
        for path in ["equal", "nested/child"] {
            browser.update(row(path, Kind::File, Status::Equal));
        }
        browser.rebuild();
        assert_eq!(browser.equal_count, 2);
        assert_eq!(browser.visible.len(), 1);
        browser.toggle_differences();
        assert!(browser.visible.is_empty());
        assert_eq!(browser.equal_count, 2);
        browser.rows.clear();
        browser.rebuild();
        assert_eq!(browser.equal_count, 0);
    }

    #[test]
    fn ignoring_children_reaggregates_without_losing_directory_uncertainty_or_errors() {
        for original in [
            Status::Equal,
            Status::Pending,
            Status::Error,
            Status::TypeConflict,
        ] {
            let mut parent = row("parent", Kind::Directory, original);
            parent.right = Some(Entry {
                kind: if original == Status::TypeConflict {
                    Kind::File
                } else {
                    Kind::Directory
                },
                size: None,
                modified: None,
            });
            parent.detail = if original == Status::Error {
                "Listing denied".into()
            } else {
                String::new()
            };
            let mut rows = vec![
                parent,
                row("parent/ignored", Kind::Directory, Status::Error),
                row("parent/ignored/file", Kind::File, Status::Different),
                row("parent/ignored-other", Kind::File, Status::Equal),
            ];
            aggregate(&mut rows);
            let mut browser = Browser {
                rows: rows
                    .into_iter()
                    .map(|row| (row.path.clone(), row))
                    .collect(),
                ..Browser::default()
            };
            for _ in 0..2 {
                browser.apply_ignores(&["parent/ignored".into()]);
                assert_eq!(browser.rows.len(), 2);
                assert_eq!(browser.rows[Path::new("parent")].status, original);
                assert_eq!(
                    browser.rows[Path::new("parent/ignored-other")].status,
                    Status::Equal
                );
                if original == Status::Error {
                    assert_eq!(browser.rows[Path::new("parent")].detail, "Listing denied");
                }
            }
        }
    }

    #[test]
    fn missing_side_navigation_filters_and_selection_are_shared() {
        let mut browser = Browser::default();
        for row in [
            row("only", Kind::Directory, Status::LeftOnly),
            row("only/file", Kind::File, Status::LeftOnly),
            row("equal", Kind::File, Status::Equal),
        ] {
            browser.rows.insert(row.path.clone(), row);
        }
        browser.rebuild();
        assert_eq!(
            browser.visible,
            vec![PathBuf::from("only"), PathBuf::from("equal")]
        );
        browser.enter(Path::new("only"));
        assert_eq!(browser.visible, vec![PathBuf::from("only/file")]);
        browser.parent();
        assert_eq!(browser.selected_row().unwrap().path, Path::new("only"));
        browser.differences_only = true;
        browser.rebuild();
        assert_eq!(browser.visible.len(), 1);
        browser.recursive = true;
        browser.rebuild();
        assert_eq!(browser.visible.len(), 2);
        browser.next_difference(false);
        assert_eq!(browser.selected, 1);
    }

    #[test]
    fn toggle_filters_equal_files_and_folders_but_retains_every_uncertain_status() {
        let mut browser = Browser::default();
        for (index, status) in [
            Status::Pending,
            Status::Equal,
            Status::Different,
            Status::LeftOnly,
            Status::RightOnly,
            Status::TypeConflict,
            Status::Unverified,
            Status::Error,
            Status::QuickMatch,
        ]
        .into_iter()
        .enumerate()
        {
            for (prefix, kind) in [("file", Kind::File), ("folder", Kind::Directory)] {
                let entry = row(&format!("{prefix}-{index}"), kind, status);
                browser.rows.insert(entry.path.clone(), entry);
            }
        }
        browser.rebuild();
        let all = browser.visible.clone();
        assert_eq!(all.len(), 18);
        assert_eq!(browser.equal_count, 4);
        browser.selected = browser
            .visible
            .iter()
            .position(|path| path == Path::new("folder-2"))
            .unwrap();
        browser.toggle_differences();
        assert_eq!(browser.visible.len(), 14);
        assert!(
            browser
                .visible
                .iter()
                .all(|path| !browser.rows[path].status.is_match())
        );
        assert_eq!(browser.selected_row().unwrap().path, Path::new("folder-2"));
        browser.toggle_differences();
        assert_eq!(browser.visible, all);
        assert_eq!(browser.selected_row().unwrap().path, Path::new("folder-2"));
        assert_eq!(browser.rows.len(), 18); // Filtering never alters comparison data.
    }

    #[test]
    fn toggle_applies_to_nested_and_recursive_views_without_hiding_changed_parents() {
        let mut browser = Browser::default();
        for entry in [
            row("changed", Kind::Directory, Status::Different),
            row("unchanged", Kind::Directory, Status::Equal),
            row("changed/same-file", Kind::File, Status::Equal),
            row("changed/same-empty-folder", Kind::Directory, Status::Equal),
            row("changed/different-file", Kind::File, Status::Different),
            row("changed/nested", Kind::Directory, Status::Different),
            row("changed/nested/file", Kind::File, Status::Different),
        ] {
            browser.rows.insert(entry.path.clone(), entry);
        }
        browser.enter(Path::new("changed"));
        assert_eq!(browser.visible.len(), 4);
        browser.toggle_differences();
        assert_eq!(
            browser.visible,
            vec![
                PathBuf::from("changed/nested"),
                PathBuf::from("changed/different-file")
            ]
        );
        browser.recursive = true;
        browser.rebuild();
        assert_eq!(browser.visible.len(), 3);
        assert!(
            browser
                .visible
                .contains(&PathBuf::from("changed/nested/file"))
        );
        browser.toggle_differences();
        assert_eq!(browser.visible.len(), 5);
        browser.recursive = false;
        browser.toggle_differences();
        browser.parent();
        assert_eq!(browser.visible, vec![PathBuf::from("changed")]);
    }

    #[test]
    fn range_selection_includes_anchor_reverses_and_survives_streamed_rows() {
        let mut browser = Browser::default();
        for name in ["b", "c", "d"] {
            let row = row(name, Kind::File, Status::Different);
            browser.rows.insert(row.path.clone(), row);
        }
        browser.rebuild();
        browser.extend_selection(1);
        assert_eq!(
            browser.copy_paths(),
            vec![PathBuf::from("b"), PathBuf::from("c")]
        );
        browser.extend_selection(1);
        assert_eq!(browser.marked.len(), 3);
        browser.extend_selection(-1);
        assert_eq!(browser.marked.len(), 2);
        // An earlier row arriving must not move either the cursor or the anchor.
        let added = row("a", Kind::File, Status::Equal);
        browser.rows.insert(added.path.clone(), added);
        browser.rebuild();
        assert_eq!(browser.selected_row().unwrap().path, Path::new("c"));
        browser.extend_selection(-1);
        assert_eq!(browser.copy_paths(), vec![PathBuf::from("b")]);
        browser.extend_selection(-1);
        assert_eq!(
            browser.copy_paths(),
            vec![PathBuf::from("a"), PathBuf::from("b")]
        );
        browser.toggle_differences();
        assert_eq!(browser.copy_paths(), vec![PathBuf::from("b")]);
        browser.move_selection(1);
        assert!(browser.marked.is_empty());
        assert_eq!(browser.copy_paths(), vec![PathBuf::from("c")]);
        browser.extend_selection(-1);
        browser.enter(Path::new("folder"));
        assert!(browser.marked.is_empty());
        assert!(browser.copy_paths().is_empty());
    }

    #[test]
    fn shift_up_can_select_files_and_folders_and_clamps_at_edges() {
        let mut browser = Browser::default();
        for entry in [
            row("folder", Kind::Directory, Status::Different),
            row("file", Kind::File, Status::Different),
        ] {
            browser.rows.insert(entry.path.clone(), entry);
        }
        browser.rebuild();
        browser.select_index(1);
        browser.extend_selection(-1);
        browser.extend_selection(-1);
        assert_eq!(browser.marked.len(), 2);
        browser.parent();
        assert!(browser.marked.is_empty());
        browser.extend_selection(1);
        browser.rows.remove(Path::new("folder"));
        browser.rebuild();
        assert!(!browser.marked.contains(Path::new("folder")));
    }

    #[test]
    fn copy_updates_preserve_parent_inventory_uncertainty_and_unrelated_results() {
        for base in [
            Status::Pending,
            Status::Error,
            Status::Unverified,
            Status::QuickMatch,
            Status::Equal,
        ] {
            let mut browser = Browser::default();
            for entry in [
                row("parent", Kind::Directory, base),
                row("parent/copied", Kind::File, Status::Equal),
                row("parent/untouched", Kind::File, Status::Different),
            ] {
                browser.rows.insert(entry.path.clone(), entry);
            }
            browser.rebuild();
            let untouched = format!("{:?}", browser.rows[Path::new("parent/untouched")]);
            let mut parent = row("parent", Kind::Directory, Status::Different);
            parent.aggregate_base = Some((Status::Equal, String::new()));
            browser.apply_copy_updates(
                &["parent/copied".into()],
                Ok(vec![
                    parent,
                    row("parent/copied", Kind::File, Status::Different),
                ]),
            );
            assert_eq!(
                browser.rows[Path::new("parent")].status,
                if base.is_match() {
                    Status::Different
                } else {
                    base
                }
            );
            assert_eq!(
                format!("{:?}", browser.rows[Path::new("parent/untouched")]),
                untouched
            );
        }
    }

    #[test]
    fn failed_copy_recheck_invalidates_old_matches_without_discarding_other_rows() {
        let mut browser = Browser::default();
        for entry in [
            row("parent", Kind::Directory, Status::Equal),
            row("parent/copied", Kind::File, Status::Equal),
            row("parent/untouched", Kind::File, Status::Equal),
        ] {
            browser.rows.insert(entry.path.clone(), entry);
        }
        browser.rebuild();
        browser.apply_copy_updates(
            &["parent/copied".into()],
            Err("cancelled after partial write".into()),
        );
        assert_eq!(
            browser.rows[Path::new("parent/copied")].status,
            Status::Unverified
        );
        assert_eq!(browser.rows[Path::new("parent")].status, Status::Unverified);
        assert_eq!(
            browser.rows[Path::new("parent/untouched")].status,
            Status::Equal
        );
        assert_eq!(browser.equal_count, 1);
    }

    #[test]
    fn empty_navigation_is_safe() {
        let mut browser = Browser::default();
        browser.next_difference(false);
        browser.next_difference(true);
        browser.move_selection(-1);
        browser.extend_selection(1);
        browser.extend_selection(-1);
        browser.parent();
        assert!(browser.selected_row().is_none());
    }
}
