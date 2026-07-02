use std::io::Read;

use super::SelectedFile;
use crate::ui::activities::filetransfer::diff::DiffView;
use crate::ui::activities::filetransfer::{FileTransferActivity, Id};

const MAX_DIFF_BYTES: u64 = 5 * 1024 * 1024;
const MAX_DIFF_LINES: usize = 3_000;

impl FileTransferActivity {
    pub(crate) fn action_diff_files(&mut self) -> Result<DiffView, String> {
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

        let left_path = local.path().to_path_buf();
        let right_path = remote.path().to_path_buf();
        let left = self.read_text_file(&Id::ExplorerHostBridge, &left_path)?;
        let right = self.read_text_file(&Id::ExplorerRemote, &right_path)?;
        let left_lines = left.lines().count();
        let right_lines = right.lines().count();
        if left_lines > MAX_DIFF_LINES || right_lines > MAX_DIFF_LINES {
            return Err(format!(
                "File diff supports files up to {MAX_DIFF_LINES} lines per side"
            ));
        }

        Ok(DiffView::new(left_path, right_path, &left, &right))
    }

    fn read_text_file(&mut self, id: &Id, path: &std::path::Path) -> Result<String, String> {
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
}
