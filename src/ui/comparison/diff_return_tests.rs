use std::path::Path;

use super::*;

fn press(app: &mut App, key: KeyCode) {
    app.key(KeyEvent::new(key, KeyModifiers::NONE));
}

fn wait(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.busy {
        assert!(Instant::now() < deadline, "{}", app.message);
        app.poll_worker();
        std::thread::sleep(Duration::from_millis(1));
    }
}

struct Fixture {
    _config: tempfile::TempDir,
    left: tempfile::TempDir,
    right: tempfile::TempDir,
    app: App,
}

impl Fixture {
    fn new(mode: ComparisonMode) -> Self {
        let config = tempfile::tempdir().unwrap();
        let left = tempfile::tempdir().unwrap();
        let right = tempfile::tempdir().unwrap();
        for (root, bytes) in [(left.path(), "old\n"), (right.path(), "new text\n")] {
            std::fs::create_dir(root.join("folder")).unwrap();
            std::fs::write(root.join("folder/file"), bytes).unwrap();
            std::fs::write(root.join("folder/other"), format!("unrelated {bytes}")).unwrap();
        }
        let mut app = tests::app(config.path());
        app.open(
            Profile::new(
                "return".into(),
                left.path().to_str().unwrap(),
                right.path().to_str().unwrap(),
                vec![],
            )
            .unwrap(),
        );
        wait(&mut app);
        if mode == ComparisonMode::Full {
            press(&mut app, KeyCode::F(5));
            wait(&mut app);
        }
        app.browser.enter(Path::new("folder"));
        app.browser.recursive = true;
        app.browser.toggle_differences();
        assert_eq!(
            app.browser.selected_row().unwrap().path,
            Path::new("folder/file")
        );
        Self {
            _config: config,
            left,
            right,
            app,
        }
    }

    fn open_diff(&mut self) {
        press(&mut self.app, KeyCode::Enter);
        wait(&mut self.app);
        assert!(matches!(self.app.mode, Mode::Diff { .. }));
    }
}

#[test]
fn read_only_diff_return_preserves_quick_and_full_results_without_rescanning() {
    for mode in [ComparisonMode::Quick, ComparisonMode::Full] {
        for key in [KeyCode::Esc, KeyCode::Char('q')] {
            let mut fixture = Fixture::new(mode);
            let app = &mut fixture.app;
            app.browser.marked.insert("folder/file".into());
            let rows = format!("{:?}", app.browser.rows);
            let visible = app.browser.visible.clone();
            let marked = app.browser.marked.clone();
            let selected = app.browser.selected;
            let timer = app.comparison_timer.label();
            fixture.open_diff();
            // A cancelled copy prompt is still read-only navigation.
            press(&mut fixture.app, KeyCode::F(7));
            assert!(fixture.app.pending_copy.is_some());
            press(&mut fixture.app, KeyCode::Esc);
            assert!(matches!(fixture.app.mode, Mode::Diff { .. }));
            // Visiting settings must not disturb the retained comparison.
            press(&mut fixture.app, KeyCode::F(4));
            press(&mut fixture.app, KeyCode::Esc);
            std::fs::write(fixture.left.path().join("folder/new-file"), "new").unwrap();
            let app = &mut fixture.app;
            let generation = app.generation;
            press(app, key);
            assert!(matches!(app.mode, Mode::Browse));
            assert!(!app.busy && !app.quit);
            assert_eq!(app.generation, generation);
            assert_eq!(format!("{:?}", app.browser.rows), rows);
            assert_eq!(app.browser.visible, visible);
            assert_eq!(app.browser.marked, marked);
            assert_eq!(app.browser.selected, selected);
            assert_eq!(app.browser.directory, Path::new("folder"));
            assert!(app.browser.recursive && app.browser.differences_only);
            assert_eq!(app.comparison_mode, mode);
            assert_eq!(app.comparison_timer.label(), timer);
            assert!(!app.poll_worker());
            // Explicit refresh still works and discovers external changes.
            press(app, KeyCode::Char('r'));
            wait(app);
            assert!(app.browser.rows.contains_key(Path::new("folder/new-file")));
        }
    }
}

#[test]
fn confirmed_diff_copy_updates_only_its_file_without_restarting_on_return() {
    let mut fixture = Fixture::new(ComparisonMode::Full);
    fixture.open_diff();
    std::fs::write(fixture.left.path().join("folder/new-file"), "new").unwrap();
    let app = &mut fixture.app;
    let timer = app.comparison_timer.label();
    let untouched = format!("{:?}", app.browser.rows[Path::new("folder/other")]);
    press(app, KeyCode::F(7));
    press(app, KeyCode::Char('y'));
    // Escape cannot leave an in-flight write.
    press(app, KeyCode::Esc);
    assert!(app.busy && matches!(app.mode, Mode::Diff { .. }));
    wait(app);
    assert!(matches!(&app.mode, Mode::Diff { path, .. } if path == Path::new("folder/file")));
    assert_eq!(
        app.browser.rows[Path::new("folder/file")].status,
        Status::Equal
    );
    assert_eq!(
        format!("{:?}", app.browser.rows[Path::new("folder/other")]),
        untouched
    );
    assert_eq!(
        app.browser.rows[Path::new("folder")].status,
        Status::Different
    );
    assert!(!app.browser.rows.contains_key(Path::new("folder/new-file")));
    // The copied file is now hidden by Differences only. Further diff copies
    // must keep targeting it, not the browser's newly selected next sibling.
    press(app, KeyCode::F(8));
    press(app, KeyCode::Char('y'));
    wait(app);
    assert!(matches!(&app.mode, Mode::Diff { path, .. } if path == Path::new("folder/file")));
    assert_eq!(
        std::fs::read(fixture.right.path().join("folder/file")).unwrap(),
        b"old\n"
    );
    let generation = app.generation;
    press(app, KeyCode::Esc);
    assert!(!app.busy);
    assert_eq!(app.generation, generation);
    assert_eq!(app.comparison_mode, ComparisonMode::Full);
    assert_eq!(app.comparison_timer.label(), timer);
    assert!(!app.browser.rows.contains_key(Path::new("folder/new-file")));
    // Only an explicit refresh discovers unrelated additions.
    press(app, KeyCode::F(5));
    wait(app);
    assert_eq!(
        app.browser.rows[Path::new("folder/file")].status,
        Status::Equal
    );
}

#[test]
fn failed_diff_copy_rechecks_only_its_file_and_keeps_its_error() {
    let mut fixture = Fixture::new(ComparisonMode::Full);
    fixture.open_diff();
    std::fs::write(
        fixture.right.path().join("folder/file"),
        "external change\n",
    )
    .unwrap();
    std::fs::write(fixture.left.path().join("folder/new-file"), "new").unwrap();
    let app = &mut fixture.app;
    let timer = app.comparison_timer.label();
    let generation = app.generation;
    press(app, KeyCode::F(7));
    press(app, KeyCode::Char('y'));
    wait(app);
    assert_eq!(app.generation, generation + 1);
    assert_eq!(app.comparison_timer.label(), timer);
    assert_eq!(app.comparison_mode, ComparisonMode::Full);
    assert_eq!(
        app.browser.rows[Path::new("folder/file")].status,
        Status::Different
    );
    assert!(matches!(app.mode, Mode::Browse));
    assert!(
        app.message.contains("changed since opening the diff"),
        "{}",
        app.message
    );
    assert!(!app.browser.rows.contains_key(Path::new("folder/new-file")));
    assert_eq!(
        std::fs::read(fixture.right.path().join("folder/file")).unwrap(),
        b"external change\n"
    );
}
