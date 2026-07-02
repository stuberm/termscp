use std::collections::HashMap;

use remotefs::File;
use tuirealm::props::{
    AttrValue, Attribute, Color, HorizontalAlignment, LineStatic, PropPayload, PropValue,
    SpanStatic, TextModifiers, Title,
};
use tuirealm::ratatui::style::Stylize;
use tuirealm::terminal::TerminalAdapter;

use super::super::browser::FileExplorerTab;
use super::super::components::transfer::file_list::PROP_ROW_INDICES;
use super::super::{FileTransferActivity, Id, ui_result};
use crate::utils::fmt::fmt_path_elide_ex;

#[derive(Clone, Copy)]
enum FolderCompareStatus {
    Equal,
    Changed,
}

impl FileTransferActivity {
    /// Update host bridge file list
    pub(in crate::ui::activities::filetransfer) fn update_host_bridge_filelist(&mut self) {
        self.reload_host_bridge_dir();
        self.reload_host_bridge_filelist();
    }

    /// Update host bridge file list
    pub(in crate::ui::activities::filetransfer) fn reload_host_bridge_filelist(&mut self) {
        self.reload_aligned_filelists();
    }

    /// Update remote file list
    pub(in crate::ui::activities::filetransfer) fn update_remote_filelist(&mut self) {
        self.reload_remote_dir();
        self.reload_remote_filelist();
    }

    pub(in crate::ui::activities::filetransfer) fn get_tab_hostname(&self) -> String {
        match self.browser.tab() {
            FileExplorerTab::HostBridge | FileExplorerTab::FindHostBridge => {
                self.get_hostbridge_hostname()
            }
            FileExplorerTab::Remote | FileExplorerTab::FindRemote => self.get_remote_hostname(),
        }
    }

    pub(in crate::ui::activities::filetransfer) fn terminal_prompt(&self) -> String {
        const TERM_CYAN: &str = "\x1b[36m";
        const TERM_GREEN: &str = "\x1b[32m";
        const TERM_YELLOW: &str = "\x1b[33m";
        const TERM_RESET: &str = "\x1b[0m";

        let panel = self.browser.tab();
        match panel {
            FileExplorerTab::HostBridge | FileExplorerTab::FindHostBridge => {
                let username = self
                    .context()
                    .host_bridge_params()
                    .and_then(|params| {
                        params
                            .username()
                            .map(|u| format!("{TERM_CYAN}{u}{TERM_RESET}@"))
                    })
                    .unwrap_or_default();
                let hostname = self.get_hostbridge_hostname();
                format!(
                    "{username}{TERM_GREEN}{hostname}:{TERM_YELLOW}{}{TERM_RESET}$ ",
                    fmt_path_elide_ex(
                        self.host_bridge().wrkdir.as_path(),
                        0,
                        hostname.len() + 3 // 3 because of '/…/'
                    )
                )
            }
            FileExplorerTab::Remote | FileExplorerTab::FindRemote => {
                let username = self
                    .context()
                    .remote_params()
                    .and_then(|params| {
                        params
                            .username()
                            .map(|u| format!("{TERM_CYAN}{u}{TERM_RESET}@"))
                    })
                    .unwrap_or_default();
                let hostname = self.get_remote_hostname();
                let fmt_path = fmt_path_elide_ex(
                    self.remote().wrkdir.as_path(),
                    0,
                    hostname.len() + 3, // 3 because of '/…/'
                );
                let fmt_path = if fmt_path.starts_with('/') {
                    fmt_path
                } else {
                    format!("/{}", fmt_path)
                };

                format!("{username}{TERM_GREEN}{hostname}:{TERM_YELLOW}{fmt_path}{TERM_RESET}$ ",)
            }
        }
    }

    pub(in crate::ui::activities::filetransfer) fn reload_remote_filelist(&mut self) {
        self.reload_aligned_filelists();
    }

    fn reload_aligned_filelists(&mut self) {
        let width = self
            .context_mut()
            .terminal()
            .raw()
            .size()
            .map(|x| (x.width / 2) - 2)
            .unwrap_or(0) as usize;

        let local_hostname = self.get_hostbridge_hostname();
        let local_title = format!(
            "{local_hostname}:{} ",
            fmt_path_elide_ex(
                self.host_bridge().wrkdir.as_path(),
                width,
                local_hostname.len() + 3
            )
        );
        let remote_hostname = self.get_remote_hostname();
        let remote_title = format!(
            "{}:{} ",
            remote_hostname,
            fmt_path_elide_ex(
                self.remote().wrkdir.as_path(),
                width,
                remote_hostname.len() + 3
            )
        );

        let local_files_snapshot = self.host_bridge().iter_files().cloned().collect::<Vec<_>>();
        let remote_files_snapshot = self.remote().iter_files().cloned().collect::<Vec<_>>();
        let local_entries: HashMap<String, (File, usize)> = local_files_snapshot
            .into_iter()
            .enumerate()
            .map(|(index, file)| (file.name().to_string(), (file, index)))
            .collect();
        let remote_entries: HashMap<String, (File, usize)> = remote_files_snapshot
            .into_iter()
            .enumerate()
            .map(|(index, file)| (file.name().to_string(), (file, index)))
            .collect();

        let mut names: Vec<String> = local_entries
            .keys()
            .chain(remote_entries.keys())
            .cloned()
            .collect();
        names.sort_by_cached_key(|name| name.to_lowercase());
        names.dedup();

        let blank_row = || vec![LineStatic::from(SpanStatic::from(" "))];
        let mut local_files = Vec::with_capacity(names.len());
        let mut remote_files = Vec::with_capacity(names.len());
        let mut local_indices = Vec::with_capacity(names.len());
        let mut remote_indices = Vec::with_capacity(names.len());

        for name in names {
            let status = self.compare_aligned_entries(
                local_entries.get(&name).map(|(file, _)| file),
                remote_entries.get(&name).map(|(file, _)| file),
            );

            if let Some((file, index)) = local_entries.get(&name) {
                local_files.push(self.filelist_row(file, Id::ExplorerHostBridge, status));
                local_indices.push(Some(*index));
            } else {
                local_files.push(blank_row());
                local_indices.push(None);
            }

            if let Some((file, index)) = remote_entries.get(&name) {
                remote_files.push(self.filelist_row(file, Id::ExplorerRemote, status));
                remote_indices.push(Some(*index));
            } else {
                remote_files.push(blank_row());
                remote_indices.push(None);
            }
        }

        let to_payload = |indices: Vec<Option<usize>>| {
            PropPayload::Vec(
                indices
                    .into_iter()
                    .map(|index| PropValue::Isize(index.map(|idx| idx as isize).unwrap_or(-1)))
                    .collect(),
            )
        };

        ui_result(self.app.attr(
            &Id::ExplorerHostBridge,
            Attribute::Content,
            AttrValue::Table(local_files),
        ));
        ui_result(self.app.attr(
            &Id::ExplorerHostBridge,
            Attribute::Custom(PROP_ROW_INDICES),
            AttrValue::Payload(to_payload(local_indices)),
        ));
        ui_result(self.app.attr(
            &Id::ExplorerHostBridge,
            Attribute::Title,
            AttrValue::Title(Title::from(local_title).alignment(HorizontalAlignment::Left)),
        ));
        ui_result(self.app.attr(
            &Id::ExplorerRemote,
            Attribute::Content,
            AttrValue::Table(remote_files),
        ));
        ui_result(self.app.attr(
            &Id::ExplorerRemote,
            Attribute::Custom(PROP_ROW_INDICES),
            AttrValue::Payload(to_payload(remote_indices)),
        ));
        ui_result(self.app.attr(
            &Id::ExplorerRemote,
            Attribute::Title,
            AttrValue::Title(Title::from(remote_title).alignment(HorizontalAlignment::Left)),
        ));
    }

    fn compare_aligned_entries(
        &mut self,
        local: Option<&File>,
        remote: Option<&File>,
    ) -> FolderCompareStatus {
        let (Some(local), Some(remote)) = (local, remote) else {
            return FolderCompareStatus::Changed;
        };

        if local.is_dir() && remote.is_dir() {
            return FolderCompareStatus::Equal;
        }
        if !local.is_file() || !remote.is_file() {
            return FolderCompareStatus::Changed;
        }
        if local.metadata().size != remote.metadata().size {
            return FolderCompareStatus::Changed;
        }
        FolderCompareStatus::Equal
    }

    fn filelist_row(&self, file: &File, id: Id, status: FolderCompareStatus) -> Vec<LineStatic> {
        let (marker, color) = match status {
            FolderCompareStatus::Equal => ("= ", Color::LightGreen),
            FolderCompareStatus::Changed => ("≠ ", Color::LightYellow),
        };
        let mut span = match id {
            Id::ExplorerHostBridge => SpanStatic::from(self.host_bridge().fmt_file(file)),
            Id::ExplorerRemote => SpanStatic::from(self.remote().fmt_file(file)),
            _ => SpanStatic::from(file.name().to_string()),
        };
        let enqueued = match id {
            Id::ExplorerHostBridge => self.host_bridge().enqueued().contains_key(file.path()),
            Id::ExplorerRemote => self.remote().enqueued().contains_key(file.path()),
            _ => false,
        };
        if enqueued {
            span.style = span.style.add_modifier(
                TextModifiers::REVERSED | TextModifiers::UNDERLINED | TextModifiers::ITALIC,
            );
        }

        vec![
            LineStatic::from(SpanStatic::raw(marker).bold().fg(color)),
            LineStatic::from(span),
        ]
    }

    pub(in crate::ui::activities::filetransfer) fn update_progress_bar(
        &mut self,
        filename: String,
    ) {
        // Update the partial bar with the current file progress. The filename
        // goes into the gauge *label*, not a block title: in a multi-file
        // transfer the partial bar omits its top border to join the seam with the
        // full bar, but any top-positioned title forces a 1-row top inset in
        // `Block::inner`, which would shrink the partial bar to one inner row
        // while the full bar keeps two — making the two gauges unequal in height.
        ui_result(self.app.attr(
            &Id::TransferProgressBarPartial,
            Attribute::Text,
            AttrValue::String(format!("{filename} — {}", self.transfer.progress)),
        ));
        ui_result(self.app.attr(
            &Id::TransferProgressBarPartial,
            Attribute::Value,
            AttrValue::Payload(PropPayload::Single(PropValue::F64(
                self.transfer.progress.calc_partial_progress(),
            ))),
        ));
        ui_result(self.app.attr(
            &Id::TransferProgressBarPartial,
            Attribute::Title,
            AttrValue::Title(Title::from(filename).alignment(HorizontalAlignment::Center)),
        ));
        // Update the full bar with the overall progress (only for multi-file transfers)
        if !self.transfer.progress.is_single_file() {
            ui_result(self.app.attr(
                &Id::TransferProgressBarFull,
                Attribute::Value,
                AttrValue::Payload(PropPayload::Single(PropValue::F64(
                    self.transfer.progress.calc_full_progress(),
                ))),
            ));
            ui_result(
                self.app.attr(
                    &Id::TransferProgressBarFull,
                    Attribute::Title,
                    AttrValue::Title(
                        Title::from(format!(
                            "Total {}",
                            self.transfer.progress.file_count_display()
                        ))
                        .alignment(HorizontalAlignment::Center),
                    ),
                ),
            );
        }
    }

    /// Update the progress bar to reflect the pre-transfer scan state.
    ///
    /// Shows how many directories and files have been discovered so far and keeps
    /// the progress value at `0.0` since the total is not yet known.
    pub(in crate::ui::activities::filetransfer) fn update_scan_progress(
        &mut self,
        dirs: usize,
        files: usize,
    ) {
        // During the scan only the partial bar is rendered (the progress model
        // reports `is_single_file()`), so write the scan text there.
        ui_result(self.app.attr(
            &Id::TransferProgressBarPartial,
            Attribute::Text,
            AttrValue::String(format!("Scanning… {dirs} dirs, {files} files")),
        ));
        ui_result(self.app.attr(
            &Id::TransferProgressBarPartial,
            Attribute::Value,
            AttrValue::Payload(PropPayload::Single(PropValue::F64(0.0))),
        ));
    }

    /// Finalize find process
    pub(in crate::ui::activities::filetransfer) fn finalize_find(&mut self) {
        // Set found to none
        self.browser.del_found();
        // Restore tab
        let new_tab = match self.browser.tab() {
            FileExplorerTab::FindHostBridge => FileExplorerTab::HostBridge,
            FileExplorerTab::FindRemote => FileExplorerTab::Remote,
            _ => FileExplorerTab::HostBridge,
        };
        // Give focus to new tab
        match new_tab {
            FileExplorerTab::HostBridge => {
                ui_result(self.app.active(&Id::ExplorerHostBridge));
            }
            FileExplorerTab::Remote => {
                ui_result(self.app.active(&Id::ExplorerRemote));
            }
            FileExplorerTab::FindHostBridge | FileExplorerTab::FindRemote => {
                ui_result(self.app.active(&Id::ExplorerFind));
            }
        }
        self.browser.change_tab(new_tab);
    }

    pub(in crate::ui::activities::filetransfer) fn update_find_list(&mut self) {
        let files: Vec<Vec<LineStatic>> = self
            .found()
            .unwrap()
            .iter_files()
            .map(|x| {
                let mut span = SpanStatic::from(self.found().unwrap().fmt_file(x));
                if self.found().unwrap().enqueued().contains_key(x.path()) {
                    span.style = span.style.add_modifier(
                        TextModifiers::REVERSED | TextModifiers::UNDERLINED | TextModifiers::ITALIC,
                    );
                }
                vec![LineStatic::from(span)]
            })
            .collect();
        ui_result(self.app.attr(
            &Id::ExplorerFind,
            Attribute::Content,
            AttrValue::Table(files),
        ));
    }

    pub(in crate::ui::activities::filetransfer) fn update_browser_file_list(&mut self) {
        match self.browser.tab() {
            FileExplorerTab::HostBridge | FileExplorerTab::FindHostBridge => {
                self.update_host_bridge_filelist()
            }
            FileExplorerTab::Remote | FileExplorerTab::FindRemote => self.update_remote_filelist(),
        }
    }

    pub(in crate::ui::activities::filetransfer) fn reload_browser_file_list(&mut self) {
        match self.browser.tab() {
            FileExplorerTab::HostBridge | FileExplorerTab::FindHostBridge => {
                self.reload_host_bridge_filelist()
            }
            FileExplorerTab::Remote | FileExplorerTab::FindRemote => self.reload_remote_filelist(),
        }
    }

    pub(in crate::ui::activities::filetransfer) fn update_browser_file_list_swapped(&mut self) {
        match self.browser.tab() {
            FileExplorerTab::HostBridge | FileExplorerTab::FindHostBridge => {
                self.update_remote_filelist()
            }
            FileExplorerTab::Remote | FileExplorerTab::FindRemote => {
                self.update_host_bridge_filelist()
            }
        }
    }
}
