use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::comparison::{Row, Status, aggregate, is_ignored};

#[derive(Default)]
pub(super) struct Browser {
    pub rows: BTreeMap<PathBuf, Row>,
    pub visible: Vec<PathBuf>,
    pub directory: PathBuf,
    pub selected: usize,
    pub differences_only: bool,
    pub recursive: bool,
}

impl Browser {
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
        for row in self.rows.values().filter(|row| {
            !row.path.as_os_str().is_empty()
                && row.path != self.directory
                && if self.recursive {
                    row.path.starts_with(&self.directory)
                } else {
                    row.path.parent() == Some(self.directory.as_path())
                }
                && (!self.differences_only || row.status != Status::Equal)
        }) {
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

    pub fn move_selection(&mut self, amount: isize) {
        self.selected = self
            .selected
            .saturating_add_signed(amount)
            .min(self.visible.len().saturating_sub(1));
    }

    pub fn enter(&mut self, path: &Path) {
        self.directory = path.into();
        self.visible.clear();
        self.selected = 0;
        self.rebuild();
    }

    pub fn parent(&mut self) {
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
        let length = self.visible.len();
        for step in 1..=length {
            let index = if backwards {
                (self.selected + length - step) % length
            } else {
                (self.selected + step) % length
            };
            if self.rows[&self.visible[index]].status != Status::Equal {
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
            left: Some(Entry { kind, size: None }),
            right: None,
            status,
            detail: String::new(),
            aggregate_base: None,
        }
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
        assert_eq!(all.len(), 16);
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
                .all(|path| browser.rows[path].status != Status::Equal)
        );
        assert_eq!(browser.selected_row().unwrap().path, Path::new("folder-2"));
        browser.toggle_differences();
        assert_eq!(browser.visible, all);
        assert_eq!(browser.selected_row().unwrap().path, Path::new("folder-2"));
        assert_eq!(browser.rows.len(), 16); // Filtering never alters comparison data.
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
    fn empty_navigation_is_safe() {
        let mut browser = Browser::default();
        browser.next_difference(false);
        browser.next_difference(true);
        browser.move_selection(-1);
        browser.parent();
        assert!(browser.selected_row().is_none());
    }
}
