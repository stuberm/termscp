use std::path::Path;

use tuirealm::ratatui::Terminal;
use tuirealm::ratatui::backend::TestBackend;

use super::*;

fn press(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn wait(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.busy {
        assert!(Instant::now() < deadline, "{}", app.message);
        app.poll_worker();
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn write(root: &Path, name: &str, bytes: &[u8]) {
    std::fs::write(root.join(name), bytes).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(root.join(name))
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(100))
        .unwrap();
}

fn render(app: &App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(120, 35)).unwrap();
    terminal.draw(|frame| view::draw(frame, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(tuirealm::ratatui::buffer::Cell::symbol)
        .collect()
}

#[test]
fn profile_opens_quick_f5_verifies_all_files_and_r_returns_to_quick() {
    let config = tempfile::tempdir().unwrap();
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    write(a.path(), "file", b"old\n");
    write(b.path(), "file", b"new\n");
    write(a.path(), "line-endings", b"same\r\n");
    write(b.path(), "line-endings", b"same\n");
    let mut app = tests::app(config.path());
    app.open(
        Profile::new(
            "quick".into(),
            a.path().to_str().unwrap(),
            b.path().to_str().unwrap(),
            vec![],
        )
        .unwrap(),
    );
    wait(&mut app);
    assert_eq!(app.comparison_mode, ComparisonMode::Quick);
    assert_eq!(
        app.browser.rows[Path::new("file")].status,
        Status::QuickMatch
    );
    assert_eq!(
        app.browser.rows[Path::new("line-endings")].status,
        Status::Different
    );
    assert!(app.message.contains("contents not verified"));
    let screen = render(&app);
    assert!(screen.contains("Quick · size + modified time"));
    assert!(screen.contains("≈ quick match"));
    assert!(screen.contains("F5 full compare"));
    assert_eq!(app.browser.equal_count, 1);
    press(&mut app, KeyCode::Char('d'));
    assert_eq!(app.browser.visible, vec![PathBuf::from("line-endings")]);
    press(&mut app, KeyCode::Char('d'));

    // Full comparison must relist, not merely revisit a selection or reuse quick results.
    write(a.path(), "new-file", b"x");
    let generation = app.generation;
    press(&mut app, KeyCode::F(5));
    assert!(app.busy);
    assert_eq!(app.generation, generation + 1);
    assert_eq!(app.comparison_mode, ComparisonMode::Full);
    assert!(app.browser.rows.is_empty());
    wait(&mut app);
    assert_eq!(
        app.browser.rows[Path::new("file")].status,
        Status::Different
    );
    assert_eq!(
        app.browser.rows[Path::new("line-endings")].status,
        Status::Equal
    );
    assert_eq!(
        app.browser.rows[Path::new("new-file")].status,
        Status::LeftOnly
    );
    assert!(app.message.contains("content-verified"));
    assert!(render(&app).contains("Full · normalized contents"));

    // Quick refresh intentionally replaces, rather than mixes with, full results.
    press(&mut app, KeyCode::Char('r'));
    wait(&mut app);
    assert_eq!(app.comparison_mode, ComparisonMode::Quick);
    assert_eq!(
        app.browser.rows[Path::new("file")].status,
        Status::QuickMatch
    );
    assert_eq!(
        app.browser.rows[Path::new("line-endings")].status,
        Status::Different
    );
}

#[test]
fn mode_switches_during_scanning_keep_only_current_generation_results() {
    let config = tempfile::tempdir().unwrap();
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    write(a.path(), "file", b"old");
    write(b.path(), "file", b"new");
    let mut app = tests::app(config.path());
    app.open(
        Profile::new(
            "switch".into(),
            a.path().to_str().unwrap(),
            b.path().to_str().unwrap(),
            vec![],
        )
        .unwrap(),
    );
    let initial = app.generation;
    // Queue a full scan without waiting for the initial inventory, then replace it.
    press(&mut app, KeyCode::F(5));
    press(&mut app, KeyCode::Char('r'));
    assert_eq!(app.generation, initial + 2);
    wait(&mut app);
    assert_eq!(app.comparison_mode, ComparisonMode::Quick);
    assert_eq!(
        app.browser.rows[Path::new("file")].status,
        Status::QuickMatch
    );
    assert!(app.message.contains("metadata only"));
    let old_label = app.comparison_timer.label();
    assert!(!app.finish_comparison(initial + 1, vec![], Duration::from_secs(99), false));
    assert_eq!(
        app.browser.rows[Path::new("file")].status,
        Status::QuickMatch
    );
    assert_eq!(app.comparison_timer.label(), old_label);
    press(&mut app, KeyCode::F(5));
    wait(&mut app);
    assert_eq!(
        app.browser.rows[Path::new("file")].status,
        Status::Different
    );
}

#[test]
fn saved_profile_checkboxes_drive_initial_refresh_and_copy_checks_without_affecting_full() {
    let config = tempfile::tempdir().unwrap();
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    write(a.path(), "file", b"old\n");
    write(b.path(), "file", b"new\n");
    std::fs::OpenOptions::new()
        .write(true)
        .open(b.path().join("file"))
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(200))
        .unwrap();
    let mut app = tests::app(config.path());
    let profile = Profile::new(
        "criteria".into(),
        a.path().to_str().unwrap(),
        b.path().to_str().unwrap(),
        vec![],
    )
    .unwrap();
    app.store.save(profile.clone()).unwrap();
    app.edit(Some(profile));
    let Mode::Editor(editor) = &mut app.mode else {
        panic!("editor");
    };
    editor.field = editor.quick_timestamp_index();
    press(&mut app, KeyCode::Char(' '));
    let screen = render(&app);
    assert!(screen.contains("[x] Size"));
    assert!(screen.contains("[ ] Timestamp"));
    press(&mut app, KeyCode::Enter); // Save and open using size-only.
    wait(&mut app);
    assert_eq!(app.profile.as_ref().unwrap().quick_check, QuickCheck::Size);
    assert_eq!(
        ProfileStore::load(config.path().join("comparisons.json"))
            .unwrap()
            .profiles["criteria"]
            .quick_check,
        QuickCheck::Size
    );
    assert_eq!(
        app.browser.rows[Path::new("file")].status,
        Status::QuickMatch
    );
    assert!(render(&app).contains("Quick · size only"));
    // Copy in size-only mode: different write timestamps must not force a mismatch.
    let timer = app.comparison_timer.label();
    press(&mut app, KeyCode::F(7));
    press(&mut app, KeyCode::Char('y'));
    wait(&mut app);
    assert_eq!(
        app.browser.rows[Path::new("file")].status,
        Status::QuickMatch
    );
    assert_eq!(app.comparison_timer.label(), timer);
    // Refresh commands must capture the requested criteria, not the initial pool default.
    write(b.path(), "file", b"different length\n"); // Same timestamp as source, different size.
    app.profile.as_mut().unwrap().quick_check = QuickCheck::Timestamp;
    press(&mut app, KeyCode::Char('r'));
    wait(&mut app);
    assert_eq!(
        app.browser.rows[Path::new("file")].status,
        Status::QuickMatch
    );
    assert!(render(&app).contains("Quick · modified time only"));
    app.profile.as_mut().unwrap().quick_check = QuickCheck::Both;
    press(&mut app, KeyCode::Char('r'));
    wait(&mut app);
    assert_eq!(
        app.browser.rows[Path::new("file")].status,
        Status::Different
    );
    app.profile.as_mut().unwrap().quick_check = QuickCheck::Size;
    write(b.path(), "file", b"new\n");
    press(&mut app, KeyCode::Char('r'));
    wait(&mut app);
    assert_eq!(
        app.browser.rows[Path::new("file")].status,
        Status::QuickMatch
    );
    press(&mut app, KeyCode::F(5));
    wait(&mut app);
    assert_eq!(
        app.browser.rows[Path::new("file")].status,
        Status::Different
    );
    assert_eq!(app.profile.as_ref().unwrap().quick_check, QuickCheck::Size);
    press(&mut app, KeyCode::Char('r'));
    wait(&mut app);
    assert_eq!(
        app.browser.rows[Path::new("file")].status,
        Status::QuickMatch
    );
    // Duplicating from the home screen must retain opt-in criteria.
    press(&mut app, KeyCode::Char('o'));
    app.mode = Mode::Profiles { selected: 1 };
    press(&mut app, KeyCode::Char('c'));
    let Mode::Editor(editor) = &app.mode else {
        panic!("duplicate editor");
    };
    assert_eq!(editor.quick_check, QuickCheck::Size);
    assert!(editor.original.is_none());
    press(&mut app, KeyCode::Enter);
    wait(&mut app);
    assert_eq!(app.store.profiles.len(), 2);
    assert!(
        app.store
            .profiles
            .values()
            .all(|profile| profile.quick_check == QuickCheck::Size)
    );
}

#[test]
fn disabling_both_checkboxes_runs_full_on_open_refresh_and_copy_without_rescanning() {
    let config = tempfile::tempdir().unwrap();
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    write(a.path(), "file", b"old\n");
    write(b.path(), "file", b"new\n");
    write(a.path(), "line-endings", b"same\r\n");
    write(b.path(), "line-endings", b"same\n");
    let mut app = tests::app(config.path());
    let profile = Profile::new(
        "no quick checks".into(),
        a.path().to_str().unwrap(),
        b.path().to_str().unwrap(),
        vec![],
    )
    .unwrap();
    app.edit(Some(profile));
    let Mode::Editor(editor) = &mut app.mode else {
        panic!("editor");
    };
    editor.field = editor.quick_size_index();
    press(&mut app, KeyCode::Char(' '));
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Char(' '));
    assert!(render(&app).contains("[ ] Size  [ ] Timestamp"));
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.comparison_mode, ComparisonMode::Full); // Even while connecting.
    assert!(app.message.contains("Full"));
    wait(&mut app);
    assert_eq!(
        app.browser.rows[Path::new("file")].status,
        Status::Different
    );
    assert_eq!(
        app.browser.rows[Path::new("line-endings")].status,
        Status::Equal
    );
    assert!(app.message.contains("content-verified"));
    assert!(render(&app).contains("Full · normalized contents"));
    let profile = ProfileStore::load(config.path().join("comparisons.json"))
        .unwrap()
        .profiles["no quick checks"]
        .clone();
    assert_eq!(profile.quick_check, QuickCheck::None);
    // Copies use full verification of only the selected path and retain the snapshot/timer.
    write(b.path(), "line-endings", b"changed externally\n");
    let timer = app.comparison_timer.label();
    let scan_id = app.generation;
    press(&mut app, KeyCode::F(7));
    press(&mut app, KeyCode::Char('y'));
    wait(&mut app);
    assert_eq!(app.generation, scan_id + 1); // Copy request only, no new scan.
    assert_eq!(app.browser.rows[Path::new("file")].status, Status::Equal);
    assert_eq!(
        app.browser.rows[Path::new("line-endings")].status,
        Status::Equal
    ); // Retained unrelated snapshot.
    assert_eq!(app.comparison_timer.label(), timer);
    press(&mut app, KeyCode::Char('r'));
    wait(&mut app);
    assert_eq!(app.comparison_mode, ComparisonMode::Full);
    assert_eq!(
        app.browser.rows[Path::new("line-endings")].status,
        Status::Different
    );
    press(&mut app, KeyCode::F(5));
    wait(&mut app);
    assert_eq!(app.comparison_mode, ComparisonMode::Full);
    assert_eq!(
        app.browser.rows[Path::new("line-endings")].status,
        Status::Different
    );
    app.open(profile); // Persisted None also drives worker startup, without the editor.
    wait(&mut app);
    assert_eq!(app.comparison_mode, ComparisonMode::Full);
    assert_eq!(
        app.browser.rows[Path::new("line-endings")].status,
        Status::Different
    );
    // A queued r during connection must not fall back to the default quick policy.
    press(&mut app, KeyCode::Char('o'));
    app.open(app.store.profiles["no quick checks"].clone());
    press(&mut app, KeyCode::Char('r'));
    wait(&mut app);
    assert_eq!(app.comparison_mode, ComparisonMode::Full);
    assert_eq!(
        app.browser.rows[Path::new("line-endings")].status,
        Status::Different
    );
}

#[test]
fn quick_matches_skip_difference_navigation_and_filter_updates() {
    use crate::comparison::filesystem::{Entry, Kind};
    let entry = Entry {
        kind: Kind::File,
        size: Some(1),
        modified: None,
    };
    let row = |path: &str, status| Row {
        path: path.into(),
        left: Some(entry.clone()),
        right: Some(entry.clone()),
        status,
        detail: String::new(),
        aggregate_base: None,
    };
    let mut browser = Browser::default();
    for (path, status) in [
        ("a", Status::QuickMatch),
        ("b", Status::Equal),
        ("c", Status::Unverified),
    ] {
        browser.update(row(path, status));
    }
    browser.rebuild();
    browser.next_difference(false);
    assert_eq!(browser.selected_row().unwrap().path, Path::new("c"));
    browser.toggle_differences();
    assert_eq!(browser.visible, vec![PathBuf::from("c")]);
    assert_eq!(browser.equal_count, 2);
    assert!(browser.update(row("a", Status::Different)));
    browser.rebuild();
    assert_eq!(
        browser.visible,
        vec![PathBuf::from("a"), PathBuf::from("c")]
    );
    assert!(!browser.update(row("b", Status::QuickMatch)));
    assert_eq!(browser.equal_count, 1);
}
