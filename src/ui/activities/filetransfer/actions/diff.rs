use std::io::{Read, Write};
use std::path::Path;

use super::SelectedFile;
use crate::ui::activities::filetransfer::diff::{DiffApplyDirection, DiffView, apply_change};
use crate::ui::activities::filetransfer::{FileTransferActivity, Id};

const MAX_DIFF_BYTES: u64 = 5 * 1024 * 1024;
const MAX_DIFF_LINES: usize = 3_000;

impl FileTransferActivity {
    pub(crate) fn action_diff_files(&mut self) -> Result<DiffView, String> {
        let (left_path, right_path) = self.diff_paths()?;
        self.diff_view_for_paths(left_path.as_path(), right_path.as_path())
    }

    pub(crate) fn action_apply_diff_change(
        &mut self,
        row_index: usize,
        direction: DiffApplyDirection,
    ) -> Result<DiffView, String> {
        let (left_path, right_path) = self.diff_paths()?;
        let left = self.read_text_file(&Id::ExplorerHostBridge, left_path.as_path())?;
        let right = self.read_text_file(&Id::ExplorerRemote, right_path.as_path())?;
        let updated = apply_change(&left, &right, row_index, direction)?;

        match direction {
            DiffApplyDirection::LeftToRight => self.write_text_file(
                &Id::ExplorerRemote,
                right_path.as_path(),
                updated.as_bytes(),
            )?,
            DiffApplyDirection::RightToLeft => self.write_text_file(
                &Id::ExplorerHostBridge,
                left_path.as_path(),
                updated.as_bytes(),
            )?,
        }

        self.diff_view_for_paths(left_path.as_path(), right_path.as_path())
    }

    fn diff_paths(&mut self) -> Result<(std::path::PathBuf, std::path::PathBuf), String> {
        let local = match self.get_local_selected_entries() {
            SelectedFile::One(file) => file,
            SelectedFile::Many(_) => return Err("Select one local file to compare".to_string()),
            SelectedFile::None => return Err("Select a local file to compare".to_string()),
        };
        let remote = match self.get_remote_selected_entries() {
            SelectedFile::One(file) => file,
            SelectedFile::Many(_) => return Err("Select one remote file to compare".to_string()),
            SelectedFile::None => return Err("Select a remote file to compare".to_string()),
        };

        if local.is_dir() || remote.is_dir() {
            return Err("File diff is available for regular text files only".to_string());
        }
        if local.metadata().size > MAX_DIFF_BYTES || remote.metadata().size > MAX_DIFF_BYTES {
            return Err(format!(
                "File diff supports files up to {} MiB",
                MAX_DIFF_BYTES / 1024 / 1024
            ));
        }

        Ok((local.path().to_path_buf(), remote.path().to_path_buf()))
    }

    fn diff_view_for_paths(
        &mut self,
        left_path: &Path,
        right_path: &Path,
    ) -> Result<DiffView, String> {
        let left = self.read_text_file(&Id::ExplorerHostBridge, left_path)?;
        let right = self.read_text_file(&Id::ExplorerRemote, right_path)?;
        let left_lines = left.lines().count();
        let right_lines = right.lines().count();
        if left_lines > MAX_DIFF_LINES || right_lines > MAX_DIFF_LINES {
            return Err(format!(
                "File diff supports files up to {MAX_DIFF_LINES} lines per side"
            ));
        }

        Ok(DiffView::new(
            left_path.to_path_buf(),
            right_path.to_path_buf(),
            &left,
            &right,
        ))
    }

    fn read_text_file(&mut self, id: &Id, path: &Path) -> Result<String, String> {
        let mut reader = match id {
            Id::ExplorerHostBridge => self.browser.local_pane_mut().fs.open_file(path),
            Id::ExplorerRemote => self.browser.remote_pane_mut().fs.open_file(path),
            _ => return Err("Invalid diff source".to_string()),
        }
        .map_err(|err| format!("Could not open {}: {err}", path.display()))?;

        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .map_err(|err| format!("Could not read {}: {err}", path.display()))?;

        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn write_text_file(&mut self, id: &Id, path: &Path, bytes: &[u8]) -> Result<(), String> {
        let fs = match id {
            Id::ExplorerHostBridge => &mut self.browser.local_pane_mut().fs,
            Id::ExplorerRemote => &mut self.browser.remote_pane_mut().fs,
            _ => return Err("Invalid diff target".to_string()),
        };
        let file = fs
            .stat(path)
            .map_err(|err| format!("Could not stat {}: {err}", path.display()))?;
        let mut writer = fs
            .create_file(path, file.metadata())
            .map_err(|err| format!("Could not write {}: {err}", path.display()))?;
        writer
            .write_all(bytes)
            .map_err(|err| format!("Could not write {}: {err}", path.display()))?;
        fs.finalize_write(writer)
            .map_err(|err| format!("Could not finish writing {}: {err}", path.display()))
    }
}
