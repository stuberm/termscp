use tuirealm::ratatui::Terminal;
use tuirealm::ratatui::backend::TestBackend;

use super::*;

fn wait_for_worker(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.busy {
        assert!(
            Instant::now() < deadline,
            "comparison worker did not finish"
        );
        app.poll_worker();
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn a_completed_worker_does_not_request_repainting_an_idle_screen() {
    let directory = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("file"), b"same\r\n").unwrap();
    std::fs::write(destination.path().join("file"), b"same\n").unwrap();
    let mut app = tests::app(directory.path());
    assert!(!app.poll_worker());
    app.start(
        Profile::new(
            "idle".into(),
            source.path().to_str().unwrap(),
            destination.path().to_str().unwrap(),
            vec![],
        )
        .unwrap(),
        Credentials::default(),
    );
    wait_for_worker(&mut app);
    assert_eq!(app.comparison_mode, ComparisonMode::Quick);
    assert_eq!(app.browser.equal_count, 0); // Raw sizes differ, even though CRLF/LF normalizes.
    assert!(!app.poll_worker());
    app.key(KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE));
    wait_for_worker(&mut app);
    assert_eq!(app.comparison_mode, ComparisonMode::Full);
    assert_eq!(app.browser.equal_count, 1);
    for _ in 0..20 {
        assert!(!app.poll_worker());
    }
}

#[test]
fn finished_total_is_authoritative_persistent_and_guarded_against_old_jobs() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = tests::app(directory.path());
    app.generation = 1;
    app.busy = true;
    app.comparison_timer
        .start(Instant::now() - Duration::from_secs(30));
    assert!(app.finish_comparison(1, vec![], Duration::from_millis(12345), false));
    assert!(!app.busy);
    assert_eq!(
        app.comparison_timer.label(),
        "Comparison: completed in 12.345s"
    );
    assert!(app.message.starts_with("Scan complete in 12.345s"));
    app.message = "Some unrelated selection/ignore message".into();
    assert_eq!(
        app.comparison_timer.label(),
        "Comparison: completed in 12.345s"
    );

    app.generation = 2;
    app.busy = true;
    app.comparison_timer.start(Instant::now());
    assert!(!app.finish_comparison(1, vec![], Duration::from_secs(99), true));
    assert!(app.busy);
    assert!(matches!(app.comparison_timer, ComparisonTimer::Running(_)));
}

#[test]
fn cancelled_pending_and_error_results_are_not_labeled_completed() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = tests::app(directory.path());
    for (status, cancelled, expected) in [
        (Status::Equal, true, Outcome::Cancelled),
        (Status::Error, true, Outcome::Cancelled),
        (Status::Error, false, Outcome::Incomplete),
        (Status::Pending, false, Outcome::Incomplete),
        // Intentional unverified symlinks/special files do not mean the scan
        // itself failed to finish; the existing safety warning still applies.
        (Status::Unverified, false, Outcome::Completed),
    ] {
        let row = Row {
            path: "nested/file".into(),
            left: None,
            right: None,
            status,
            detail: "test error".into(),
            aggregate_base: None,
        };
        app.finish_comparison(0, vec![row], Duration::from_millis(1250), cancelled);
        assert!(
            matches!(app.comparison_timer, ComparisonTimer::Finished { elapsed, outcome }
            if elapsed == Duration::from_millis(1250) && outcome == expected)
        );
        if expected != Outcome::Completed {
            assert!(!app.message.starts_with("Scan complete"));
        }
    }
    let row = Row {
        path: "".into(),
        left: None,
        right: None,
        status: Status::Error,
        detail: "root listing denied".into(),
        aggregate_base: None,
    };
    app.finish_comparison(0, vec![row], Duration::from_millis(1250), false);
    assert_eq!(
        app.message,
        "Scan incomplete after 1.250s: root listing denied"
    );
}

#[test]
fn new_comparisons_and_refreshes_reset_timing_but_text_diffs_do_not() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    std::fs::write(a.path().join("file"), "abc").unwrap();
    std::fs::write(b.path().join("file"), "abc").unwrap();
    let mut app = tests::app(a.path());
    let profile = Profile::new(
        "timing".into(),
        a.path().to_str().unwrap(),
        b.path().to_str().unwrap(),
        vec![],
    )
    .unwrap();
    app.start(profile, Credentials::default());
    assert!(matches!(app.comparison_timer, ComparisonTimer::Running(_)));
    wait_for_worker(&mut app);
    assert!(matches!(
        app.comparison_timer,
        ComparisonTimer::Finished {
            outcome: Outcome::Completed,
            ..
        }
    ));
    let completed = app.comparison_timer.label();
    app.browse_key(KeyCode::Enter);
    wait_for_worker(&mut app);
    assert!(matches!(app.mode, Mode::Diff { .. }));
    assert_eq!(app.comparison_timer.label(), completed);
    assert!(app.refresh());
    assert!(matches!(app.comparison_timer, ComparisonTimer::Running(_)));
    wait_for_worker(&mut app);
    assert!(matches!(
        app.comparison_timer,
        ComparisonTimer::Finished {
            outcome: Outcome::Completed,
            ..
        }
    ));
}

#[test]
fn connection_failures_stop_the_timer_without_claiming_a_completed_scan() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let profile = Profile::new(
        "failure".into(),
        a.path().to_str().unwrap(),
        b.path().to_str().unwrap(),
        vec![],
    )
    .unwrap();
    let mut app = tests::app(b.path());
    std::fs::remove_dir(a.path()).unwrap();
    app.start(profile, Credentials::default());
    wait_for_worker(&mut app);
    assert!(matches!(
        app.comparison_timer,
        ComparisonTimer::Finished {
            outcome: Outcome::Failed,
            ..
        }
    ));
    assert!(app.message.starts_with("Connection failed:"));
}

#[test]
fn timing_remains_visible_above_file_controls_after_progress_messages_change() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = tests::app(directory.path());
    app.mode = Mode::Browse;
    app.busy = true;
    app.comparison_timer.start(Instant::now());
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal.draw(|frame| view::draw(frame, &app)).unwrap();
    let rendered: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(tuirealm::ratatui::buffer::Cell::symbol)
        .collect();
    assert!(rendered.contains("Comparison: running · "));
    assert!(rendered.contains("elapsed"));
    app.busy = false;
    app.comparison_timer
        .finish(Duration::from_millis(12345), Outcome::Completed);
    app.message = "An unrelated message without the elapsed time".into();
    for (width, height) in [(100, 24), (40, 12), (20, 8), (1, 1)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| view::draw(frame, &app)).unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(tuirealm::ratatui::buffer::Cell::symbol)
            .collect();
        if width >= 40 {
            assert!(rendered.contains("Comparison: completed in 12.345s"));
            assert!(rendered.contains("Show: "));
        }
    }
}
