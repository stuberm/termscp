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

#[test]
fn browser_shift_selection_and_confirmed_function_keys_copy_both_directions() {
    use tuirealm::ratatui::Terminal;
    use tuirealm::ratatui::backend::TestBackend;

    let root = tempfile::tempdir().unwrap();
    let left = tempfile::tempdir().unwrap();
    let right = tempfile::tempdir().unwrap();
    std::fs::create_dir(left.path().join("folder")).unwrap();
    std::fs::write(left.path().join("folder/child"), "nested").unwrap();
    std::fs::write(left.path().join("file"), b"binary\0\r\n").unwrap();
    let mut app = tests::app(root.path());
    app.open(
        Profile::new(
            "copy".into(),
            left.path().to_str().unwrap(),
            right.path().to_str().unwrap(),
            vec![],
        )
        .unwrap(),
    );
    wait(&mut app);
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
    assert_eq!(app.browser.marked.len(), 2);
    press(&mut app, KeyCode::F(7));
    assert!(
        matches!(&app.pending_copy, Some(PendingCopy::Entries { paths, to_right: true }) if paths.len() == 2)
    );
    // Confirmation and selection remain readable, even on narrow terminals.
    for (width, height) in [(120, 35), (20, 8), (1, 1)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| view::draw(frame, &app)).unwrap();
        if width == 120 {
            let rendered: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(tuirealm::ratatui::buffer::Cell::symbol)
                .collect();
            assert!(rendered.contains("Confirm copy"));
            assert!(rendered.contains("LEFT → RIGHT"));
            assert!(rendered.contains("2 selected"));
        }
    }
    press(&mut app, KeyCode::Esc);
    assert_eq!(std::fs::read_dir(right.path()).unwrap().count(), 0);
    press(&mut app, KeyCode::F(7));
    press(&mut app, KeyCode::Char('y'));
    assert!(matches!(app.mode, Mode::Copying));
    // Navigation/re-entry cannot interrupt an in-flight write.
    press(&mut app, KeyCode::Char('o'));
    press(&mut app, KeyCode::F(4));
    assert!(matches!(app.mode, Mode::Copying));
    wait(&mut app);
    assert!(matches!(app.mode, Mode::Browse));
    assert!(app.message.contains("Copied 3 entries"), "{}", app.message);
    assert_eq!(
        std::fs::read(right.path().join("file")).unwrap(),
        b"binary\0\r\n"
    );
    assert_eq!(
        std::fs::read_to_string(right.path().join("folder/child")).unwrap(),
        "nested"
    );
    assert_eq!(app.comparison_mode, ComparisonMode::Quick);
    press(&mut app, KeyCode::F(5));
    wait(&mut app);
    assert!(
        app.browser
            .rows
            .values()
            .all(|row| row.status == Status::Equal)
    );
    press(&mut app, KeyCode::End); // Plain movement clears marks; copy focused row.
    assert!(app.browser.marked.is_empty());
    std::fs::write(right.path().join("file"), "reverse").unwrap();
    press(&mut app, KeyCode::F(8));
    assert!(
        matches!(&app.pending_copy, Some(PendingCopy::Entries { paths, to_right: false }) if paths == &vec![PathBuf::from("file")])
    );
    press(&mut app, KeyCode::Char('y'));
    wait(&mut app);
    assert_eq!(
        std::fs::read_to_string(left.path().join("file")).unwrap(),
        "reverse"
    );
    press(&mut app, KeyCode::End);
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT));
    assert_eq!(app.browser.marked.len(), 2);
    press(&mut app, KeyCode::Esc);
    assert!(app.browser.marked.is_empty());
}

#[test]
fn single_browser_file_copies_keep_mode_timer_selection_and_unrelated_rows() {
    use std::path::Path;
    for mode in [ComparisonMode::Quick, ComparisonMode::Full] {
        let config = tempfile::tempdir().unwrap();
        let left = tempfile::tempdir().unwrap();
        let right = tempfile::tempdir().unwrap();
        for (root, bytes) in [
            (left.path(), b"left\0\n".as_slice()),
            (right.path(), b"right\0\n"),
        ] {
            std::fs::create_dir(root.join("outside")).unwrap();
            std::fs::write(root.join("outside/keep"), b"x").unwrap();
            std::fs::write(root.join("file"), bytes).unwrap();
        }
        let mut app = tests::app(config.path());
        app.open(
            Profile::new(
                "single".into(),
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
        press(&mut app, KeyCode::End);
        app.browser.marked.insert("file".into());
        let timer = app.comparison_timer.label();
        let visible = app.browser.visible.clone();
        let selected = app.browser.selected;
        let untouched = format!("{:?}", app.browser.rows[Path::new("outside/keep")]);
        // Neither an unrelated edit nor an addition should be discovered by a copy.
        std::fs::write(left.path().join("outside/keep"), b"external change").unwrap();
        std::fs::write(left.path().join("outside/new"), b"new").unwrap();
        for direction in [7, 8] {
            if direction == 8 {
                std::fs::write(right.path().join("file"), b"reverse\0\n").unwrap();
            }
            let generation = app.generation;
            press(&mut app, KeyCode::F(direction));
            press(&mut app, KeyCode::Char('y'));
            wait(&mut app);
            assert!(matches!(app.mode, Mode::Browse));
            assert_eq!(app.generation, generation + 1);
            assert_eq!(app.comparison_mode, mode);
            assert_eq!(app.comparison_timer.label(), timer);
            assert_eq!(app.browser.visible, visible);
            assert_eq!(app.browser.selected, selected);
            assert_eq!(
                app.browser.marked,
                std::collections::BTreeSet::from([PathBuf::from("file")])
            );
            assert_eq!(
                format!("{:?}", app.browser.rows[Path::new("outside/keep")]),
                untouched
            );
            assert!(!app.browser.rows.contains_key(Path::new("outside/new")));
            assert_eq!(
                std::fs::read(left.path().join("file")).unwrap(),
                std::fs::read(right.path().join("file")).unwrap()
            );
            let file = &app.browser.rows[Path::new("file")];
            assert_eq!(
                file.left.as_ref().unwrap().size,
                file.right.as_ref().unwrap().size
            );
            if mode == ComparisonMode::Full {
                assert_eq!(file.status, Status::Equal);
            }
        }
    }
}

#[test]
fn copying_one_nested_file_updates_new_parent_presence_but_not_uncopied_siblings() {
    use std::path::Path;
    let config = tempfile::tempdir().unwrap();
    let left = tempfile::tempdir().unwrap();
    let right = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(left.path().join("new/sub")).unwrap();
    std::fs::write(left.path().join("new/sub/file"), b"copied").unwrap();
    std::fs::write(left.path().join("new/sub/sibling"), b"not copied").unwrap();
    let mut app = tests::app(config.path());
    app.open(
        Profile::new(
            "nested".into(),
            left.path().to_str().unwrap(),
            right.path().to_str().unwrap(),
            vec![],
        )
        .unwrap(),
    );
    wait(&mut app);
    press(&mut app, KeyCode::F(5));
    wait(&mut app);
    app.browser.enter(Path::new("new/sub"));
    assert_eq!(
        app.browser.selected_row().unwrap().path,
        Path::new("new/sub/file")
    );
    press(&mut app, KeyCode::F(7));
    press(&mut app, KeyCode::Char('y'));
    wait(&mut app);
    for path in ["new", "new/sub"] {
        let row = &app.browser.rows[Path::new(path)];
        assert!(row.left.is_some() && row.right.is_some());
        assert_eq!(row.status, Status::Different); // Uncopied sibling is still left-only.
    }
    assert_eq!(
        app.browser.rows[Path::new("new/sub/file")].status,
        Status::Equal
    );
    assert_eq!(
        app.browser.rows[Path::new("new/sub/sibling")].status,
        Status::LeftOnly
    );
    assert!(!right.path().join("new/sub/sibling").exists());
    assert_eq!(app.browser.directory, Path::new("new/sub"));
}

#[test]
fn failed_copy_rechecks_only_its_paths_and_preserves_error_message() {
    let root = tempfile::tempdir().unwrap();
    let left = tempfile::tempdir().unwrap();
    let right = tempfile::tempdir().unwrap();
    std::fs::write(left.path().join("file"), "source").unwrap();
    std::fs::create_dir(right.path().join("file")).unwrap();
    let mut app = tests::app(root.path());
    app.open(
        Profile::new(
            "copy".into(),
            left.path().to_str().unwrap(),
            right.path().to_str().unwrap(),
            vec![],
        )
        .unwrap(),
    );
    wait(&mut app);
    press(&mut app, KeyCode::F(7));
    press(&mut app, KeyCode::Char('y'));
    wait(&mut app);
    assert!(matches!(app.mode, Mode::Browse));
    assert!(app.message.contains("type conflicts"), "{}", app.message);
    assert_eq!(
        app.browser.selected_row().unwrap().status,
        Status::TypeConflict
    );
    assert!(right.path().join("file").is_dir());
}
