//! Folder comparison and confirmed bidirectional text merging.
#[cfg(test)]
mod copy_tests;
#[cfg(test)]
mod diff_return_tests;
mod editor;
#[cfg(test)]
mod quick_tests;
mod redraw;
mod state;
#[cfg(test)]
mod tests;
mod theme;
mod timing;
#[cfg(test)]
mod timing_tests;
mod view;

use std::collections::BTreeMap;
use std::io::{self, Stdout};
use std::path::PathBuf;
use std::sync::mpsc::TryRecvError;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use editor::Editor;
use state::Browser;
use theme::{Theme, ThemeSettings};
use timing::{ComparisonTimer, Outcome, duration_label};
use tuirealm::ratatui::Terminal;
use tuirealm::ratatui::backend::CrosstermBackend;

use crate::cli::CompareArgs;
use crate::comparison::connection::Credentials;
use crate::comparison::profiles::{Profile, ProfileStore};
use crate::comparison::settings::QuickCheck;
use crate::comparison::text::TextView;
use crate::comparison::worker::{Event as WorkerEvent, Worker};
use crate::comparison::{ComparisonMode, Row, ScanEvent, Status, diagnostics};
use crate::system::environment;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Source,
    Destination,
}

#[derive(Clone)]
enum HomeEntry {
    New,
    Folder(String),
    Profile(String),
}

enum Mode {
    Settings {
        previous: Box<Mode>,
        themes: bool,
        selected: usize,
    },
    CreateFolder {
        value: String,
    },
    MoveProfile {
        profile: String,
        selected: usize,
    },
    DeleteFolder(String),
    Browse,
    Copying,
    Profiles {
        selected: usize,
    },
    Editor(Editor),
    Delete(String),
    Password {
        profile: Profile,
        passwords: Box<Credentials>,
        side: Side,
        value: String,
    },
    Diff {
        path: PathBuf,
        view: TextView,
        selected: usize,
    },
    Help,
}

#[derive(Debug, PartialEq, Eq)]
enum PendingCopy {
    Diff { to_right: bool, whole: bool },
    Entries { paths: Vec<PathBuf>, to_right: bool },
}

struct App {
    theme: ThemeSettings,
    browser: Browser,
    store: ProfileStore,
    home_folder: Option<String>,
    profile: Option<Profile>,
    worker: Option<Worker>,
    mode: Mode,
    generation: u64,
    busy: bool,
    comparison_timer: ComparisonTimer,
    comparison_mode: ComparisonMode,
    loading_diff: bool,
    message: String,
    keyring: bool,
    prompt_password: bool,
    quit: bool,
    pending_copy: Option<PendingCopy>,
}

pub fn run(args: CompareArgs, keyring: bool) -> Result<(), Box<dyn std::error::Error>> {
    crate::system::require_native_store(keyring)?;
    let config = environment::init_config_dir()?.ok_or("No configuration directory available")?;
    let mut store = ProfileStore::load(config.join("comparisons.json"))?;
    let initial = if let Some(name) = args.profile {
        if !args.roots.is_empty() || args.save.is_some() || !args.ignore.is_empty() {
            return Err(
                "--profile cannot be combined with folder roots, --save or --ignore".into(),
            );
        }
        Some(
            store
                .profiles
                .get(&name)
                .cloned()
                .ok_or_else(|| format!("Unknown comparison profile: {name}"))?,
        )
    } else {
        match args.roots.as_slice() {
            [left, right] => {
                let profile = Profile::new(
                    args.save
                        .clone()
                        .unwrap_or_else(|| "Unsaved comparison".into()),
                    left,
                    right,
                    args.ignore,
                )?;
                if args.save.is_some() {
                    if store.profiles.contains_key(&profile.name) {
                        return Err("Profile already exists; edit it in the profile picker".into());
                    }
                    store.save(profile.clone())?;
                }
                Some(profile)
            }
            [] if args.save.is_none() && args.ignore.is_empty() => None,
            _ => return Err("Provide both folder roots, or use --profile NAME".into()),
        }
    };
    let mut app = App {
        theme: ThemeSettings::load(config.join("comparison-ui.json")),
        browser: Browser {
            differences_only: args.differences,
            ..Browser::default()
        },
        store,
        home_folder: None,
        profile: None,
        worker: None,
        mode: Mode::Profiles { selected: 0 },
        generation: 0,
        busy: false,
        comparison_timer: ComparisonTimer::default(),
        comparison_mode: ComparisonMode::default(),
        loading_diff: false,
        message: "Select New comparison or open a saved comparison with Enter".into(),
        keyring,
        prompt_password: args.password_prompt,
        quit: false,
        pending_copy: None,
    };
    if let Some(profile) = initial {
        app.open(profile);
    }
    let mut terminal = Screen::new()?;
    let mut redraw = redraw::Redraw::default();
    while !app.quit {
        if app.poll_worker() {
            redraw.request(false);
        }
        if redraw.due(
            Instant::now(),
            matches!(app.comparison_timer, ComparisonTimer::Running(_)),
        ) {
            terminal.terminal.draw(|frame| view::draw(frame, &app))?;
        }
        if event::poll(Duration::from_millis(50))? {
            // Drain queued repeats before polling scan results and redrawing again.
            // Otherwise every arrow key pays for a full browser rebuild while a
            // scan is streaming results, allowing input to fall further behind.
            // Bound the batch so continuous input cannot starve worker updates.
            for _ in 0..64 {
                match event::read()? {
                    Event::Key(key) if key.kind != KeyEventKind::Release => {
                        app.key(key);
                        redraw.request(true);
                    }
                    Event::Resize(_, _) => redraw.request(true),
                    _ => {}
                }
                if app.quit || !event::poll(Duration::ZERO)? {
                    break;
                }
            }
        }
    }
    Ok(())
}

/// Restores raw mode and alternate screen on every normal/error return.
struct Screen {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl Screen {
    fn new() -> io::Result<Self> {
        enable_raw_mode()?;
        if let Err(error) = execute!(io::stdout(), EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(error);
        }
        match Terminal::new(CrosstermBackend::new(io::stdout())) {
            Ok(terminal) => Ok(Self { terminal }),
            Err(error) => {
                let _ = disable_raw_mode();
                let _ = execute!(io::stdout(), LeaveAlternateScreen);
                Err(error)
            }
        }
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

impl App {
    fn quick_check(&self) -> QuickCheck {
        self.profile
            .as_ref()
            .map_or_else(QuickCheck::default, |profile| profile.quick_check)
    }

    fn log_display_snapshot(&self, phase: &str) {
        if !diagnostics::enabled() {
            return;
        }
        diagnostics::record("display_snapshot", || {
            serde_json::json!({
                "request_id": self.generation, "phase": phase,
                "directory": self.browser.directory.to_string_lossy(),
                "differences_only": self.browser.differences_only, "recursive": self.browser.recursive,
                "rows": self.browser.rows.len(),
                "selected_path": self.browser.selected_row().map(|row| row.path.to_string_lossy()),
                "diff_path": if let Mode::Diff { path, .. } = &self.mode { Some(path.to_string_lossy()) } else { None },
                "ignores": self.profile.as_ref().map(|profile| &profile.ignores),
            "quick_check": self.quick_check(),
            })
        });
        diagnostics::display_rows(
            self.generation,
            self.comparison_mode,
            self.browser.rows.values(),
            self.quick_check(),
        );
    }

    fn cancel_diff(&mut self) {
        if self.loading_diff {
            if let Some(worker) = &self.worker {
                worker.cancel();
            }
            self.generation += 1;
            self.busy = false;
            self.loading_diff = false;
        }
    }

    fn open(&mut self, profile: Profile) {
        self.open_with_credentials(profile, Credentials::default());
    }

    fn open_with_credentials(&mut self, profile: Profile, credentials: Credentials) {
        self.cancel_diff();
        if self.prompt_password && profile.has_remote() {
            self.authenticate_with_credentials(profile, credentials);
        } else {
            self.start(profile, credentials);
        }
    }

    fn authenticate(&mut self, profile: Profile) {
        if !profile.has_remote() {
            self.message = "Both endpoints are local; no password is needed".into();
            return;
        }
        self.authenticate_with_credentials(profile, Credentials::default());
    }

    fn authenticate_with_credentials(&mut self, profile: Profile, credentials: Credentials) {
        let side = if profile.source.needs_secret_prompt() {
            Side::Source
        } else if profile.destination.needs_secret_prompt() {
            Side::Destination
        } else {
            self.start(profile, credentials);
            return;
        };
        self.mode = Mode::Password {
            profile,
            passwords: Box::new(credentials),
            side,
            value: String::new(),
        };
    }

    fn start(&mut self, profile: Profile, passwords: Credentials) {
        self.comparison_mode = ComparisonMode::default().for_checks(profile.quick_check);
        self.comparison_timer.start(Instant::now());
        self.generation += 1;
        self.worker = Some(Worker::start(
            profile.clone(),
            passwords,
            self.keyring,
            self.generation,
            profile.effective_parallelism(self.theme.parallel_comparisons),
        ));
        self.profile = Some(profile);
        self.browser.clear_marks();
        self.browser.rows.clear();
        self.browser.directory.clear();
        self.browser.selected = 0;
        self.browser.rebuild();
        self.busy = true;
        self.loading_diff = false;
        self.mode = Mode::Browse;
        self.message = format!(
            "Connecting / {} — F5 full comparison; c cancel; Enter file diff; copying requires confirmation",
            self.comparison_mode.label(self.quick_check())
        );
    }

    fn poll_worker(&mut self) -> bool {
        let Some(worker) = &self.worker else {
            return false;
        };
        let mut events = Vec::new();
        let mut disconnected = false;
        for _ in 0..512 {
            match worker.events.try_recv() {
                Ok(event) => events.push(event),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        let changed = events.iter().any(|event| match event {
            WorkerEvent::Scan { id, .. }
            | WorkerEvent::Finished { id, .. }
            | WorkerEvent::Text { id, .. }
            | WorkerEvent::Copied { id, .. }
            | WorkerEvent::Updated { id, .. }
            | WorkerEvent::Failed { id, .. } => *id == self.generation,
        });
        let mut dirty = false;
        for event in events {
            match event {
                WorkerEvent::Scan { id, event } if id == self.generation => match event {
                    ScanEvent::Progress(message) => self.message = message,
                    ScanEvent::Rows(rows) => {
                        for row in rows {
                            if self.profile.as_ref().is_some_and(|profile| {
                                crate::comparison::is_ignored(&row.path, &profile.ignores)
                            }) {
                                continue;
                            }
                            dirty |= self.browser.update(row);
                        }
                    }
                },
                WorkerEvent::Finished {
                    id,
                    rows,
                    cancelled,
                    elapsed,
                } => {
                    dirty |= self.finish_comparison(id, rows, elapsed, cancelled);
                }
                WorkerEvent::Updated { id, paths, rows } if id == self.generation => {
                    let rows = rows.map(|rows| {
                        rows.into_iter()
                            .filter(|row| {
                                !self.profile.as_ref().is_some_and(|profile| {
                                    crate::comparison::is_ignored(&row.path, &profile.ignores)
                                })
                            })
                            .collect()
                    });
                    self.browser.apply_copy_updates(&paths, rows);
                    self.log_display_snapshot("copy_updates");
                }
                WorkerEvent::Text { id, path, view } if id == self.generation => {
                    self.busy = false;
                    self.loading_diff = false;
                    match view {
                        Ok(view) => {
                            let selected =
                                view.lines.iter().position(|line| line.changed).unwrap_or(0);
                            self.mode = Mode::Diff {
                                view,
                                selected,
                                path,
                            };
                            self.message = "Diff · ␍ marks CRLF · [no final newline] marks an unterminated line".into();
                        }
                        Err(error) => {
                            self.mode = Mode::Browse;
                            // Scoped post-copy updates already invalidate any
                            // possibly partial writes; no whole-tree restart.
                            self.message = error;
                        }
                    }
                }
                WorkerEvent::Copied { id, result } if id == self.generation => {
                    self.busy = false;
                    self.loading_diff = false;
                    self.mode = Mode::Browse;
                    self.message = match result {
                        Ok(count) => format!(
                            "Copied {count} entries · only copied paths rechecked; comparison retained"
                        ),
                        Err(error) => error,
                    };
                }
                WorkerEvent::Failed { id, error, elapsed } if id == self.generation => {
                    self.comparison_timer.finish(elapsed, Outcome::Failed);
                    self.busy = false;
                    if matches!(self.mode, Mode::Copying) {
                        self.mode = Mode::Browse;
                    }
                    self.message = format!(
                        "Connection failed: {error} · a for endpoint passwords, o for saved comparisons"
                    );
                }
                _ => {} // Old jobs/profile workers cannot replace current results.
            }
        }
        if dirty {
            self.browser.rebuild();
        }
        if disconnected && self.busy {
            self.comparison_timer.stop(Outcome::Failed);
            self.busy = false;
            self.loading_diff = false;
            self.mode = Mode::Browse;
            self.message = "Connection worker stopped; reopen the profile to reconnect".into();
            return true;
        }
        changed
    }

    fn finish_comparison(
        &mut self,
        id: u64,
        rows: Vec<Row>,
        elapsed: Duration,
        cancelled: bool,
    ) -> bool {
        // An interrupted/replaced scan must not overwrite the next scan's timer.
        if id != self.generation {
            return false;
        }
        self.browser.rows = rows
            .into_iter()
            .map(|row| (row.path.clone(), row))
            .collect::<BTreeMap<_, _>>();
        if let Some(profile) = &self.profile {
            self.browser.apply_ignores(&profile.ignores);
        }
        let outcome = if cancelled {
            Outcome::Cancelled
        } else if self
            .browser
            .rows
            .values()
            .any(|row| matches!(row.status, Status::Error | Status::Pending))
        {
            Outcome::Incomplete
        } else {
            Outcome::Completed
        };
        self.comparison_timer.finish(elapsed, outcome);
        self.busy = false;
        let elapsed = duration_label(elapsed);
        self.message = match outcome {
            Outcome::Cancelled => format!(
                "Cancelled after {elapsed} — partial results are not proof of equality; r to rescan"
            ),
            Outcome::Completed if self.comparison_mode == ComparisonMode::Quick => format!(
                "Scan complete in {elapsed} — metadata only; contents not verified · F5 full comparison"
            ),
            Outcome::Completed => format!(
                "Scan complete in {elapsed} — content-verified regular files; symlinks are not followed"
            ),
            _ => format!(
                "Scan incomplete after {elapsed} — some entries could not be verified; r to rescan"
            ),
        };
        if !cancelled
            && let Some(root) = self.browser.rows.get(std::path::Path::new(""))
            && root.status == Status::Error
        {
            self.message = format!("Scan incomplete after {elapsed}: {}", root.detail);
        }
        self.log_display_snapshot("scan_finished");
        true
    }

    fn key(&mut self, key: KeyEvent) {
        if let Mode::Copying = self.mode {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('c')) {
                if let Some(worker) = &self.worker {
                    worker.cancel();
                }
                self.message =
                    "Cancellation requested; waiting for current I/O; affected entries may need verification".into();
            }
            return;
        }
        if let Some(pending) = self.pending_copy.take() {
            let (to_right, whole) = match pending {
                PendingCopy::Diff { to_right, whole } => (to_right, whole),
                PendingCopy::Entries { paths, to_right } => {
                    if key.code == KeyCode::Char('y') {
                        if let (Some(worker), Some(profile)) = (&mut self.worker, &self.profile) {
                            self.generation += 1;
                            match worker.copy_entries(
                                self.generation,
                                paths,
                                to_right,
                                profile.ignores.clone(),
                            ) {
                                Ok(()) => {
                                    self.comparison_timer.stop(Outcome::Interrupted);
                                    self.loading_diff = false;
                                    self.busy = true;
                                    self.mode = Mode::Copying;
                                    self.message = "Copying — c/Esc cancels after current I/O; do not close until complete".into();
                                }
                                Err(error) => self.message = error,
                            }
                        }
                    } else {
                        self.message = "Copy cancelled".into();
                    }
                    return;
                }
            };
            if key.code == KeyCode::Char('y') {
                if let Mode::Diff {
                    path,
                    view,
                    selected,
                } = &self.mode
                    && let Some(worker) = &self.worker
                {
                    self.generation += 1;
                    match worker.copy(
                        self.generation,
                        path.clone(),
                        view.clone(),
                        *selected,
                        to_right,
                        whole,
                    ) {
                        Ok(()) => {
                            self.busy = true;
                            self.message = "Copying — do not close until complete".into();
                        }
                        Err(error) => self.message = error,
                    }
                }
            } else {
                self.message = "Copy cancelled".into();
            }
            return;
        }
        if self.busy && matches!(self.mode, Mode::Diff { .. }) {
            return;
        }

        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.quit = true;
            return;
        }
        if key.code == KeyCode::F(4)
            && matches!(
                self.mode,
                Mode::Browse | Mode::Profiles { .. } | Mode::Diff { .. }
            )
        {
            self.cancel_diff();
            let previous = Box::new(std::mem::replace(&mut self.mode, Mode::Browse));
            self.mode = Mode::Settings {
                previous,
                themes: false,
                selected: 0,
            };
            return;
        }
        match &mut self.mode {
            Mode::Settings {
                previous,
                themes,
                selected,
            } => match key.code {
                KeyCode::Esc if *themes => {
                    *themes = false;
                    *selected = 0;
                }
                KeyCode::Esc => {
                    self.mode = *std::mem::replace(previous, Box::new(Mode::Browse));
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let count = if *themes { Theme::ALL.len() } else { 2 };
                    *selected = (*selected + 1).min(count - 1);
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    *selected = selected.saturating_sub(1);
                }
                KeyCode::Enter if !*themes && *selected == 0 => {
                    *themes = true;
                    *selected = Theme::ALL
                        .iter()
                        .position(|theme| *theme == self.theme.theme)
                        .unwrap_or(0);
                }
                KeyCode::Left
                | KeyCode::Right
                | KeyCode::Char(' ' | '+' | '-')
                | KeyCode::Enter
                    if !*themes && *selected == 1 =>
                {
                    let parallelism = self
                        .theme
                        .parallel_comparisons
                        .adjusted(!matches!(key.code, KeyCode::Left | KeyCode::Char('-')));
                    self.message = match self.theme.apply_parallelism(parallelism) {
                        Ok(()) => format!(
                            "Default parallel comparisons saved: {} — applies on next open/refresh; profile overrides are unchanged",
                            parallelism.get()
                        ),
                        Err(error) => format!("Could not save parallel comparisons: {error}"),
                    };
                }
                KeyCode::Enter => {
                    let theme = Theme::ALL[*selected];
                    self.message = match self.theme.apply(theme) {
                        Ok(()) => format!("Theme saved: {}", theme.name()),
                        Err(error) => format!("Could not save theme: {error}"),
                    };
                }
                _ => {}
            },
            Mode::Browse => {
                if key.modifiers.contains(KeyModifiers::SHIFT)
                    && matches!(key.code, KeyCode::Up | KeyCode::Down)
                {
                    self.browser
                        .extend_selection(if key.code == KeyCode::Down { 1 } else { -1 });
                } else {
                    self.browse_key(key.code);
                }
            }
            Mode::Copying => unreachable!(),
            Mode::Profiles { selected } => {
                let entries = Self::home_entries(&self.store, self.home_folder.as_deref());
                let entry = entries.get(*selected).cloned();
                match key.code {
                    KeyCode::Down | KeyCode::Char('j') => {
                        *selected = (*selected + 1).min(entries.len().saturating_sub(1))
                    }
                    KeyCode::Up | KeyCode::Char('k') => *selected = selected.saturating_sub(1),
                    KeyCode::Enter => match entry {
                        Some(HomeEntry::New) => self.edit(None),
                        Some(HomeEntry::Folder(folder)) => {
                            self.home_folder = Some(folder);
                            *selected = 0;
                        }
                        Some(HomeEntry::Profile(name)) => {
                            self.open(self.store.profiles[&name].clone())
                        }
                        None => {}
                    },
                    KeyCode::Backspace => {
                        self.home_folder = None;
                        *selected = 0;
                    }
                    KeyCode::Char('f') => {
                        self.mode = Mode::CreateFolder {
                            value: String::new(),
                        }
                    }
                    KeyCode::Char('m') => {
                        if let Some(HomeEntry::Profile(name)) = entry {
                            let selected = self
                                .store
                                .folder_of(&name)
                                .and_then(|folder| {
                                    self.store.folders.keys().position(|name| name == folder)
                                })
                                .map_or(0, |index| index + 1);
                            self.message = "Choose a destination folder".into();
                            self.mode = Mode::MoveProfile {
                                profile: name,
                                selected,
                            };
                        }
                    }
                    KeyCode::Char('c') => {
                        if let Some(HomeEntry::Profile(name)) = entry {
                            let mut profile = self.store.profiles[&name].clone();
                            let base = format!("{name} copy");
                            profile.name = base.clone();
                            let mut number = 2;
                            while self.store.profiles.contains_key(&profile.name) {
                                profile.name = format!("{base} {number}");
                                number += 1;
                            }
                            self.edit(Some(profile));
                        }
                    }
                    KeyCode::Char('n') => self.edit(None),
                    KeyCode::Char('e') => {
                        if let Some(HomeEntry::Profile(name)) = entry {
                            self.edit(Some(self.store.profiles[&name].clone()));
                        }
                    }
                    KeyCode::Char('d') => match entry {
                        Some(HomeEntry::Profile(name)) => self.mode = Mode::Delete(name),
                        Some(HomeEntry::Folder(name)) => self.mode = Mode::DeleteFolder(name),
                        _ => {}
                    },
                    KeyCode::Char('q') => self.quit = true,
                    KeyCode::Esc => {
                        if self.home_folder.take().is_some() {
                            *selected = 0;
                        } else {
                            self.quit = true;
                        }
                    }
                    _ => {}
                }
            }
            Mode::MoveProfile { profile, selected } => match key.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    *selected = (*selected + 1).min(self.store.folders.len())
                }
                KeyCode::Up | KeyCode::Char('k') => *selected = selected.saturating_sub(1),
                KeyCode::Home => *selected = 0,
                KeyCode::End => *selected = self.store.folders.len(),
                KeyCode::Enter => {
                    let folder = if *selected == 0 {
                        None
                    } else {
                        self.store.folders.keys().nth(*selected - 1).cloned()
                    };
                    match self.store.move_profile(profile, folder.as_deref()) {
                        Ok(()) => {
                            self.message =
                                format!("Profile moved to {}", folder.as_deref().unwrap_or("root"));
                            self.mode = Mode::Profiles { selected: 0 };
                        }
                        Err(error) => self.message = error,
                    }
                }
                KeyCode::Esc => self.mode = Mode::Profiles { selected: 0 },
                _ => {}
            },
            Mode::DeleteFolder(name) => match key.code {
                KeyCode::Char('y') => {
                    match self.store.delete_folder(name) {
                        Ok(()) => {
                            if self.home_folder.as_deref() == Some(name.as_str()) {
                                self.home_folder = None;
                            }
                            self.message =
                                "Folder deleted; its profiles are now in the root list".into();
                        }
                        Err(error) => self.message = error,
                    }
                    self.mode = Mode::Profiles { selected: 0 };
                }
                KeyCode::Esc | KeyCode::Char('n') => self.mode = Mode::Profiles { selected: 0 },
                _ => {}
            },
            Mode::CreateFolder { value } => match key.code {
                KeyCode::Enter => {
                    let result = self.store.create_folder(value.clone());
                    match result {
                        Ok(()) => {
                            self.message = "Profile organization saved".into();
                            self.mode = Mode::Profiles { selected: 0 };
                        }
                        Err(error) => self.message = error,
                    }
                }
                KeyCode::Esc => self.mode = Mode::Profiles { selected: 0 },
                KeyCode::Backspace => {
                    value.pop();
                }
                KeyCode::Char(character)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    value.push(character)
                }
                _ => {}
            },
            Mode::Editor(editor) => {
                if key.code == KeyCode::Enter
                    || (key.code == KeyCode::Char('s')
                        && key.modifiers.contains(KeyModifiers::CONTROL))
                {
                    self.save_editor();
                } else if key.code == KeyCode::Esc {
                    self.mode = Mode::Profiles { selected: 0 };
                } else {
                    editor.key(key);
                }
            }
            Mode::Delete(name) => match key.code {
                KeyCode::Char('y') => {
                    match self.store.delete(name) {
                        Ok(()) => {
                            self.message = "Profile deleted (folder contents untouched)".into()
                        }
                        Err(error) => self.message = error,
                    }
                    self.mode = Mode::Profiles { selected: 0 };
                }
                KeyCode::Esc | KeyCode::Char('n') => self.mode = Mode::Profiles { selected: 0 },
                _ => {}
            },
            Mode::Password {
                profile,
                passwords,
                side,
                value,
            } => match key.code {
                KeyCode::Enter => {
                    let password = std::mem::take(value);
                    if !password.is_empty() {
                        if *side == Side::Source {
                            passwords.source.password = Some(password);
                        } else {
                            passwords.destination.password = Some(password);
                        }
                    }
                    if *side == Side::Source && profile.destination.needs_secret_prompt() {
                        *side = Side::Destination;
                    } else {
                        let profile = profile.clone();
                        let passwords = std::mem::take(passwords);
                        self.start(profile, *passwords);
                    }
                }
                KeyCode::Backspace => {
                    value.pop();
                }
                KeyCode::Char(character)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    value.push(character)
                }
                KeyCode::Esc => {
                    self.mode = if self.profile.is_some() {
                        Mode::Browse
                    } else {
                        Mode::Profiles { selected: 0 }
                    }
                }
                _ => {}
            },
            Mode::Diff { view, selected, .. } => match key.code {
                KeyCode::F(direction @ 5..=8) => {
                    let whole = matches!(direction, 7 | 8);
                    if !whole && !view.lines.get(*selected).is_some_and(|line| line.changed) {
                        self.message = "Select a changed chunk first (n/p)".into();
                    } else {
                        let to_right = matches!(direction, 5 | 7);
                        self.pending_copy = Some(PendingCopy::Diff { to_right, whole });
                        self.message = format!(
                            "Overwrite {} {}? y confirm / any other key cancel. Remote writes are not atomic.",
                            if to_right {
                                "RIGHT from LEFT"
                            } else {
                                "LEFT from RIGHT"
                            },
                            if whole {
                                "entire file"
                            } else {
                                "selected chunk"
                            }
                        );
                    }
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.mode = Mode::Browse;
                    self.message =
                        "Comparison retained · r quick refresh · F5 full comparison".into();
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    *selected = (*selected + 1).min(view.lines.len().saturating_sub(1))
                }
                KeyCode::Up | KeyCode::Char('k') => *selected = selected.saturating_sub(1),
                KeyCode::PageDown => {
                    *selected = (*selected + 20).min(view.lines.len().saturating_sub(1))
                }
                KeyCode::PageUp => *selected = selected.saturating_sub(20),
                KeyCode::Home => *selected = 0,
                KeyCode::End => *selected = view.lines.len().saturating_sub(1),
                KeyCode::Char('n') => {
                    if let Some(index) =
                        ((selected.saturating_add(1))..view.lines.len()).find(|&index| {
                            view.lines[index].changed
                                && (index == 0 || !view.lines[index - 1].changed)
                        })
                    {
                        *selected = index;
                    }
                }
                KeyCode::Char('p') => {
                    if let Some(index) = (0..*selected).rev().find(|&index| {
                        view.lines[index].changed && (index == 0 || !view.lines[index - 1].changed)
                    }) {
                        *selected = index;
                    }
                }
                _ => {}
            },
            Mode::Help => self.mode = Mode::Browse,
        }
    }

    fn home_entries(store: &ProfileStore, folder: Option<&str>) -> Vec<HomeEntry> {
        let mut entries = vec![HomeEntry::New];
        if folder.is_none() {
            entries.extend(store.folders.keys().cloned().map(HomeEntry::Folder));
        }
        entries.extend(
            store
                .profiles
                .keys()
                .filter(|name| store.folder_of(name) == folder)
                .cloned()
                .map(HomeEntry::Profile),
        );
        entries
    }

    fn edit(&mut self, profile: Option<Profile>) {
        let original = profile
            .as_ref()
            .filter(|profile| self.store.profiles.get(&profile.name) == Some(*profile))
            .map(|profile| profile.name.clone());
        self.mode = Mode::Editor(Editor::new(profile.as_ref(), original));
        self.message = "Independent source/destination settings · F2 source, F3 destination · Space/←→ type/options · Enter saves and opens".into();
    }

    fn save_editor(&mut self) {
        let Mode::Editor(editor) = &self.mode else {
            return;
        };
        let original = editor.original.clone();
        let credentials = editor.credentials();
        let profile = match editor.profile() {
            Ok(profile) => profile,
            Err(error) => {
                self.message = error;
                return;
            }
        };
        let is_new = original.is_none();
        let result = if let Some(original) = original {
            self.store.update(&original, profile.clone())
        } else if self.store.profiles.contains_key(&profile.name) {
            Err("That comparison name already exists; edit the saved comparison instead".into())
        } else {
            self.store.save(profile.clone())
        };
        match result {
            Ok(()) => {
                if is_new
                    && self.home_folder.is_some()
                    && let Err(error) = self
                        .store
                        .move_profile(&profile.name, self.home_folder.as_deref())
                {
                    self.message = error;
                    return;
                }
                self.open_with_credentials(profile, credentials);
            }
            Err(error) => self.message = error,
        }
    }

    fn ignore_selected(&mut self) {
        if self.loading_diff {
            return;
        }
        let Some(row) = self.browser.selected_row() else {
            return;
        };
        let path = row.path.clone();
        let Some(mut profile) = self.profile.clone() else {
            return;
        };
        let pattern = path
            .components()
            .map(|part| part.as_os_str().to_str())
            .collect::<Option<Vec<_>>>()
            .map(|parts| parts.join("/"));
        let Some(pattern) =
            pattern.filter(|value| !value.is_empty() && !value.contains(['*', '?', ';']))
        else {
            self.message =
                "Cannot represent this path literally in ignore globs; use s to edit rules".into();
            return;
        };
        if !profile.ignores.contains(&pattern) {
            profile.ignores.push(pattern.clone());
        }
        let saved = self.store.profiles.get(&profile.name) == self.profile.as_ref();
        if saved && let Err(error) = self.store.update(&profile.name, profile.clone()) {
            self.message = error;
            return;
        }
        self.browser.apply_ignores(&profile.ignores);
        self.profile = Some(profile);
        self.browser.rebuild();
        self.log_display_snapshot("ignore_applied");
        self.message = format!(
            "Ignoring {pattern} on both sides{} · comparison kept; no rescan",
            if saved {
                " (saved)"
            } else {
                " (session only; s to save)"
            }
        );
    }

    fn refresh(&mut self) -> bool {
        self.refresh_as(ComparisonMode::Quick)
    }

    fn refresh_as(&mut self, mode: ComparisonMode) -> bool {
        let mode = mode.for_checks(self.quick_check());
        let started = Instant::now();
        self.generation += 1;
        if let (Some(worker), Some(profile)) = (&mut self.worker, &self.profile) {
            match worker.refresh(
                self.generation,
                profile.ignores.clone(),
                profile.effective_parallelism(self.theme.parallel_comparisons),
                mode,
                profile.quick_check,
            ) {
                Ok(()) => {
                    self.comparison_mode = mode;
                    self.comparison_timer.start(started);
                    self.busy = true;
                    self.loading_diff = false;
                    self.browser.rows.clear();
                    self.browser.rebuild();
                    self.message = format!(
                        "{} — entries appear as folders are listed",
                        mode.label(profile.quick_check)
                    );
                    return true;
                }
                Err(error) => {
                    self.comparison_timer
                        .finish(started.elapsed(), Outcome::Failed);
                    self.message = error;
                }
            }
        }
        self.comparison_timer.stop(Outcome::Failed);
        self.busy = false;
        self.loading_diff = false;
        false
    }

    fn browse_key(&mut self, key: KeyCode) {
        if matches!(key, KeyCode::Char('o' | 's' | 'a' | '?')) {
            self.cancel_diff();
        }
        match key {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Down | KeyCode::Char('j') => self.browser.move_selection(1),
            KeyCode::Up | KeyCode::Char('k') => self.browser.move_selection(-1),
            KeyCode::PageDown => self.browser.move_selection(20),
            KeyCode::PageUp => self.browser.move_selection(-20),
            KeyCode::Home => self.browser.select_index(0),
            KeyCode::End => self
                .browser
                .select_index(self.browser.visible.len().saturating_sub(1)),
            KeyCode::F(direction @ 7..=8) => {
                let paths = self.browser.copy_paths();
                if paths.is_empty() {
                    self.message = "Select a file or folder first".into();
                } else if self.worker.is_none() {
                    self.message = "Reopen the comparison to reconnect before copying".into();
                } else {
                    self.cancel_diff();
                    let to_right = direction == 7;
                    self.message = format!(
                        "Copy {} selected file(s)/folder(s) {}? Existing files will be overwritten; folders copy recursively (ignores honored). y confirm / any other key cancel. Remote writes are not atomic; no rollback.",
                        paths.len(),
                        if to_right {
                            "LEFT → RIGHT"
                        } else {
                            "RIGHT → LEFT"
                        },
                    );
                    self.pending_copy = Some(PendingCopy::Entries { paths, to_right });
                }
            }
            KeyCode::Char('n') => self.browser.next_difference(false),
            KeyCode::Char('p') => self.browser.next_difference(true),
            KeyCode::Char('d') => {
                self.browser.toggle_differences();
                self.log_display_snapshot("filter_changed");
            }
            KeyCode::Char('f') => {
                self.browser.recursive = !self.browser.recursive;
                self.browser.rebuild();
                self.log_display_snapshot("recursive_view_changed");
            }
            KeyCode::Backspace | KeyCode::Left => {
                self.browser.parent();
                self.log_display_snapshot("navigate_parent");
            }
            KeyCode::Enter | KeyCode::Right => {
                if let Some(row) = self.browser.selected_row().cloned() {
                    if row.is_directory() {
                        self.browser.enter(&row.path);
                        self.log_display_snapshot("navigate_directory");
                    } else if row.is_file_pair() && !self.loading_diff {
                        self.comparison_timer.stop(Outcome::Interrupted);
                        self.generation += 1;
                        if let Some(worker) = &mut self.worker {
                            match worker.diff(self.generation, row.path) {
                                Ok(()) => {
                                    self.busy = true;
                                    self.loading_diff = true;
                                    self.message = "Opening file diff (interrupting folder scan if active) — Esc to cancel".into();
                                }
                                Err(error) => self.message = error,
                            }
                        }
                    } else {
                        self.message = format!("{} · {}", row.status.label(), row.detail);
                    }
                }
            }
            KeyCode::Char('i') => self.ignore_selected(),
            KeyCode::Char('r') => {
                self.refresh();
            }
            KeyCode::F(5) => {
                self.refresh_as(ComparisonMode::Full);
            }
            KeyCode::Esc if !self.busy && !self.browser.marked.is_empty() => {
                self.browser.clear_marks();
                self.message = "Selection cleared".into();
            }
            KeyCode::Esc | KeyCode::Char('c') => {
                if let Some(worker) = &self.worker {
                    worker.cancel();
                }
                if self.loading_diff {
                    self.generation += 1;
                    self.busy = false;
                    self.loading_diff = false;
                }
                self.message =
                    "Cancellation requested (takes effect after the current I/O operation)".into();
            }
            KeyCode::Char('o') => self.mode = Mode::Profiles { selected: 0 },
            KeyCode::Char('s') => self.edit(self.profile.clone()),
            KeyCode::Char('a') => {
                if let Some(profile) = self.profile.clone() {
                    self.authenticate(profile);
                }
            }
            KeyCode::Char('?') => self.mode = Mode::Help,
            _ => {}
        }
    }
}
