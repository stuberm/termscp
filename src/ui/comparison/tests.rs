use super::*;
use crate::comparison::profiles::Endpoint;

#[test]
fn folders_duplicate_and_move_profiles_without_touching_endpoints() {
    let root = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    let profile = Profile::new(
        "Website".into(),
        source.path().to_str().unwrap(),
        destination.path().to_str().unwrap(),
        vec![".git".into()],
    )
    .unwrap();
    app.store.save(profile.clone()).unwrap();
    press(&mut app, KeyCode::Char('f'));
    for character in "Work".chars() {
        press(&mut app, KeyCode::Char(character));
    }
    press(&mut app, KeyCode::Enter);
    assert!(app.store.folders.contains_key("Work"));
    app.mode = Mode::Profiles { selected: 2 }; // New, Work, Website.
    press(&mut app, KeyCode::Char('m'));
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.store.folder_of("Website"), Some("Work"));
    app.mode = Mode::Profiles { selected: 1 };
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.home_folder.as_deref(), Some("Work"));
    app.mode = Mode::Profiles { selected: 1 };
    press(&mut app, KeyCode::Char('c'));
    let Mode::Editor(editor) = &app.mode else {
        panic!("expected duplicate editor");
    };
    assert!(editor.original.is_none());
    let copy = editor.profile().unwrap();
    assert_eq!(copy.name, "Website copy");
    assert_eq!(copy.source, profile.source);
    assert_eq!(copy.destination, profile.destination);
    assert_eq!(copy.ignores, profile.ignores);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.store.folder_of("Website copy"), Some("Work"));
    app.mode = Mode::Profiles { selected: 1 };
    press(&mut app, KeyCode::Char('c'));
    let Mode::Editor(editor) = &app.mode else {
        panic!("expected duplicate editor");
    };
    assert_eq!(editor.profile().unwrap().name, "Website copy 2");
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Backspace);
    assert!(app.home_folder.is_none());
    assert_eq!(std::fs::read_dir(source.path()).unwrap().count(), 0);
    assert_eq!(std::fs::read_dir(destination.path()).unwrap().count(), 0);
}

#[test]
fn folder_storage_survives_reload_rename_and_delete() {
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    let profile = Profile::new(
        "Original".into(),
        root.path().to_str().unwrap(),
        root.path().to_str().unwrap(),
        vec![],
    )
    .unwrap();
    app.store.save(profile.clone()).unwrap();
    app.store.create_folder("Work".into()).unwrap();
    assert!(app.store.create_folder("Work".into()).is_err());
    assert!(app.store.create_folder("Nested/Folder".into()).is_err());
    assert!(app.store.move_profile("Original", Some("Missing")).is_err());
    app.store.move_profile("Original", Some("Work")).unwrap();
    let mut renamed = profile;
    renamed.name = "Renamed".into();
    app.store.update("Original", renamed).unwrap();
    let mut loaded = ProfileStore::load(root.path().join("comparisons.json")).unwrap();
    assert_eq!(loaded.folder_of("Renamed"), Some("Work"));
    loaded.move_profile("Renamed", None).unwrap();
    assert_eq!(loaded.folder_of("Renamed"), None);
    loaded.move_profile("Renamed", Some("Work")).unwrap();
    loaded.delete("Renamed").unwrap();
    let loaded = ProfileStore::load(root.path().join("comparisons.json")).unwrap();
    assert!(loaded.folders["Work"].is_empty());
}

#[test]
fn folder_deletion_confirms_and_preserves_profiles() {
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    let profile = Profile::new(
        "Keep".into(),
        root.path().to_str().unwrap(),
        root.path().to_str().unwrap(),
        vec![],
    )
    .unwrap();
    app.store.save(profile.clone()).unwrap();
    app.store.create_folder("Work".into()).unwrap();
    app.store.move_profile("Keep", Some("Work")).unwrap();
    app.mode = Mode::Profiles { selected: 1 };
    press(&mut app, KeyCode::Char('d'));
    assert!(matches!(&app.mode, Mode::DeleteFolder(name) if name == "Work"));
    press(&mut app, KeyCode::Esc);
    assert!(app.store.folders.contains_key("Work"));
    app.mode = Mode::Profiles { selected: 1 };
    press(&mut app, KeyCode::Char('d'));
    press(&mut app, KeyCode::Char('y'));
    let loaded = ProfileStore::load(root.path().join("comparisons.json")).unwrap();
    assert!(!loaded.folders.contains_key("Work"));
    assert_eq!(loaded.profiles["Keep"], profile);
    assert_eq!(loaded.folder_of("Keep"), None);
    assert!(app.store.delete_folder("Missing").is_err());
    app.store.create_folder("Empty".into()).unwrap();
    app.store.delete_folder("Empty").unwrap();
    assert!(app.store.folders.is_empty());
}

#[test]
fn folder_picker_and_delete_confirmation_render_at_small_sizes() {
    use tuirealm::ratatui::{Terminal, backend::TestBackend};
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    for number in 0..30 {
        app.store
            .create_folder(format!("Folder {number:02}"))
            .unwrap();
    }
    for (width, height) in [(100, 30), (20, 8), (1, 1)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        app.mode = Mode::MoveProfile {
            profile: "Example".into(),
            selected: 0,
        };
        terminal.draw(|frame| view::draw(frame, &app)).unwrap();
        if width == 100 {
            let rendered: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(tuirealm::ratatui::buffer::Cell::symbol)
                .collect();
            assert!(rendered.contains("Available folders"));
            assert!(rendered.contains("Root (no folder)"));
            assert!(rendered.contains("Folder 00"));
        }
        app.mode = Mode::MoveProfile {
            profile: "Example".into(),
            selected: 30,
        };
        terminal.draw(|frame| view::draw(frame, &app)).unwrap();
        if width == 100 {
            let rendered: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(tuirealm::ratatui::buffer::Cell::symbol)
                .collect();
            assert!(rendered.contains("Folder 29"));
        }
        app.mode = Mode::DeleteFolder("Folder 00".into());
        terminal.draw(|frame| view::draw(frame, &app)).unwrap();
        if width == 100 {
            let rendered: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(tuirealm::ratatui::buffer::Cell::symbol)
                .collect();
            assert!(rendered.contains("No profiles or endpoint files will be deleted"));
        }
    }
}

#[test]
fn move_picker_selects_current_folder_and_can_cancel_or_return_to_root() {
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    let profile = Profile::new(
        "Keep".into(),
        root.path().to_str().unwrap(),
        root.path().to_str().unwrap(),
        vec![],
    )
    .unwrap();
    app.store.save(profile).unwrap();
    app.store.create_folder("Alpha".into()).unwrap();
    app.store.create_folder("Work".into()).unwrap();
    app.store.move_profile("Keep", Some("Work")).unwrap();
    app.home_folder = Some("Work".into());
    app.mode = Mode::Profiles { selected: 1 };
    press(&mut app, KeyCode::Char('m'));
    assert!(matches!(app.mode, Mode::MoveProfile { selected: 2, .. }));
    press(&mut app, KeyCode::Home);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.store.folder_of("Keep"), Some("Work"));
    app.mode = Mode::Profiles { selected: 1 };
    press(&mut app, KeyCode::Char('m'));
    press(&mut app, KeyCode::Home);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.store.folder_of("Keep"), None);
    app.home_folder = None;
    app.mode = Mode::Profiles { selected: 3 };
    press(&mut app, KeyCode::Char('m'));
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.store.folder_of("Keep"), Some("Alpha"));
}

fn app(directory: &std::path::Path) -> App {
    App {
        browser: Browser::default(),
        store: ProfileStore::load(directory.join("comparisons.json")).unwrap(),
        home_folder: None,
        profile: None,
        worker: None,
        mode: Mode::Profiles { selected: 0 },
        generation: 0,
        busy: false,
        loading_diff: false,
        message: String::new(),
        keyring: false,
        prompt_password: false,
        quit: false,
    }
}

fn press(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}

#[test]
fn ignore_selected_persists_exact_relative_path_and_removes_subtree() {
    use crate::comparison::{
        Row,
        filesystem::{Entry, Kind},
    };
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    let profile = Profile::new(
        "ignore test".into(),
        root.path().to_str().unwrap(),
        root.path().to_str().unwrap(),
        vec!["*.tmp".into()],
    )
    .unwrap();
    app.store.save(profile.clone()).unwrap();
    app.profile = Some(profile);
    app.mode = Mode::Browse;
    for path in ["nested", "nested/file", "other"] {
        let row = Row {
            path: path.into(),
            left: Some(Entry {
                kind: if path == "nested" {
                    Kind::Directory
                } else {
                    Kind::File
                },
                size: None,
            }),
            right: None,
            status: Status::Pending,
            detail: String::new(),
            aggregate_base: None,
        };
        app.browser.rows.insert(row.path.clone(), row);
    }
    app.browser.rebuild();
    assert_eq!(
        app.browser.selected_row().unwrap().path,
        std::path::Path::new("nested")
    );
    let generation = app.generation;
    let busy = app.busy;
    press(&mut app, KeyCode::Char('i'));
    assert_eq!(app.generation, generation);
    assert_eq!(app.busy, busy);
    assert_eq!(
        app.browser.rows[std::path::Path::new("other")].status,
        Status::Pending
    );
    assert!(app.message.contains("no rescan"));
    assert_eq!(
        app.profile.as_ref().unwrap().ignores,
        vec!["*.tmp", "nested"]
    );
    assert_eq!(app.browser.visible, vec![std::path::PathBuf::from("other")]);
    assert_eq!(
        ProfileStore::load(root.path().join("comparisons.json"))
            .unwrap()
            .profiles["ignore test"]
            .ignores,
        vec!["*.tmp", "nested"]
    );
    assert!(app.message.contains("saved"));
}

#[test]
fn ignoring_during_scan_preserves_worker_and_filters_queued_and_final_results() {
    use crate::comparison::{
        Row,
        filesystem::{Entry, Kind},
    };
    use std::path::Path;
    use std::time::Instant;
    let config = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    for (root, content) in [(source.path(), "old"), (destination.path(), "changed")] {
        std::fs::create_dir_all(root.join("parent/ignored")).unwrap();
        std::fs::write(root.join("parent/ignored/file"), content).unwrap();
        std::fs::write(root.join("parent/keep"), "same").unwrap();
    }
    let mut app = app(config.path());
    let profile = Profile::new(
        "live".into(),
        source.path().to_str().unwrap(),
        destination.path().to_str().unwrap(),
        vec![],
    )
    .unwrap();
    app.store.save(profile.clone()).unwrap();
    app.start(profile, Credentials::default());
    // Seed already displayed rows while worker updates (including its final snapshot) queue up.
    for (path, kind, status) in [
        ("parent/ignored", Kind::Directory, Status::Pending),
        ("parent/keep", Kind::File, Status::Equal),
    ] {
        let entry = Entry { kind, size: None };
        app.browser.rows.insert(
            path.into(),
            Row {
                path: path.into(),
                left: Some(entry.clone()),
                right: Some(entry),
                status,
                detail: String::new(),
                aggregate_base: None,
            },
        );
    }
    app.browser.enter(Path::new("parent"));
    let generation = app.generation;
    press(&mut app, KeyCode::Char('i'));
    assert_eq!(app.generation, generation);
    assert!(app.busy && app.worker.is_some());
    assert_eq!(app.browser.directory, Path::new("parent"));
    assert_eq!(
        app.browser.rows[Path::new("parent/keep")].status,
        Status::Equal
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.busy {
        assert!(
            Instant::now() < deadline,
            "scan must finish without restarting"
        );
        app.poll_worker();
        assert!(
            app.browser
                .rows
                .keys()
                .all(|path| !path.starts_with("parent/ignored"))
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(app.generation, generation);
    assert_eq!(app.browser.rows[Path::new("parent")].status, Status::Equal);
    assert_eq!(
        app.browser.rows[Path::new("parent/keep")].status,
        Status::Equal
    );
    // Ignoring the remaining file after completion must not mark other results pending.
    press(&mut app, KeyCode::Char('i'));
    assert!(!app.busy);
    assert_eq!(app.generation, generation);
    assert_eq!(app.browser.rows[Path::new("parent")].status, Status::Equal);
    assert!(app.browser.visible.is_empty());
    assert_eq!(
        app.store.profiles["live"].ignores,
        vec!["parent/ignored", "parent/keep"]
    );
    // Only an explicit refresh restarts scanning and uses the updated ignore rules.
    press(&mut app, KeyCode::Char('r'));
    assert_eq!(app.generation, generation + 1);
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.busy {
        assert!(Instant::now() < deadline);
        app.poll_worker();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(app.browser.rows.len(), 1);
    assert_eq!(app.browser.rows[Path::new("parent")].status, Status::Equal);
}

#[test]
fn ignore_unsaved_comparison_does_not_save_or_broaden_wildcard_names() {
    use crate::comparison::{
        Row,
        filesystem::{Entry, Kind},
    };
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    app.profile = Some(
        Profile::new(
            "unsaved".into(),
            root.path().to_str().unwrap(),
            root.path().to_str().unwrap(),
            vec![],
        )
        .unwrap(),
    );
    app.mode = Mode::Browse;
    for name in ["file?.txt", "file*.txt", "file;other", "plain.txt"] {
        app.browser.rows.clear();
        app.browser.rows.insert(
            name.into(),
            Row {
                path: name.into(),
                left: Some(Entry {
                    kind: Kind::File,
                    size: None,
                }),
                right: None,
                status: Status::LeftOnly,
                detail: String::new(),
                aggregate_base: None,
            },
        );
        app.browser.rebuild();
        press(&mut app, KeyCode::Char('i'));
        if name != "plain.txt" {
            assert!(app.profile.as_ref().unwrap().ignores.is_empty());
            assert!(app.message.contains("literally"));
        }
    }
    assert_eq!(app.profile.as_ref().unwrap().ignores, vec!["plain.txt"]);
    assert!(app.store.profiles.is_empty());
    assert!(app.message.contains("session only"));
}

#[test]
fn native_credentials_cannot_be_disabled_by_legacy_profiles_or_f4() {
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    let mut profile = Profile::new(
        "remember".into(),
        "sftp://alice@server/root",
        root.path().to_str().unwrap(),
        vec![],
    )
    .unwrap();
    let mut legacy = serde_json::to_value(&profile).unwrap();
    legacy["remember_passwords"] = serde_json::json!(false);
    profile = serde_json::from_value(legacy).unwrap();
    app.edit(Some(profile.clone()));
    press(&mut app, KeyCode::F(4));
    let Mode::Editor(editor) = &app.mode else {
        panic!("expected editor");
    };
    profile = editor.profile().unwrap();
    assert!(
        serde_json::to_value(&profile)
            .unwrap()
            .get("remember_passwords")
            .is_none()
    );
    app.store.save(profile.clone()).unwrap();
    let restored = ProfileStore::load(root.path().join("comparisons.json")).unwrap();
    assert_eq!(restored.profiles["remember"], profile);
}

#[test]
fn escape_from_profiles_does_not_reopen_previous_comparison() {
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    app.profile = Some(
        Profile::new(
            "previous".into(),
            root.path().to_str().unwrap(),
            root.path().to_str().unwrap(),
            vec![],
        )
        .unwrap(),
    );
    app.mode = Mode::Browse;
    press(&mut app, KeyCode::Char('o'));
    assert!(matches!(app.mode, Mode::Profiles { .. }));
    app.home_folder = Some("folder".into());
    press(&mut app, KeyCode::Esc);
    assert!(app.home_folder.is_none());
    assert!(!app.quit);
    assert!(matches!(app.mode, Mode::Profiles { .. }));
    press(&mut app, KeyCode::Esc);
    assert!(app.quit);
    assert!(matches!(app.mode, Mode::Profiles { .. }));
}

#[test]
fn home_new_comparison_saves_opens_and_updates_the_same_session() {
    let root = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let updated = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    press(&mut app, KeyCode::Enter); // Explicit New comparison row.
    let Mode::Editor(editor) = &mut app.mode else {
        panic!("expected comparison settings");
    };
    editor.name = "website".into();
    editor.source.location = source.path().display().to_string();
    editor.destination.location = destination.path().display().to_string();
    press(&mut app, KeyCode::Enter);
    assert!(matches!(app.mode, Mode::Browse));
    assert_eq!(app.store.profiles.len(), 1);
    assert_eq!(app.profile.as_ref(), app.store.profiles.get("website"));
    press(&mut app, KeyCode::Char('o'));
    press(&mut app, KeyCode::Down); // First saved session after New comparison.
    press(&mut app, KeyCode::Char('e'));
    let Mode::Editor(editor) = &mut app.mode else {
        panic!("expected edit settings");
    };
    assert_eq!(editor.original.as_deref(), Some("website"));
    editor.destination.location = updated.path().display().to_string();
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(matches!(app.mode, Mode::Browse));
    assert_eq!(app.store.profiles.len(), 1);
    let expected = Endpoint::Local {
        path: updated.path().canonicalize().unwrap(),
    };
    assert_eq!(app.profile.as_ref().unwrap().destination, expected);
    assert_eq!(
        ProfileStore::load(root.path().join("comparisons.json"))
            .unwrap()
            .profiles["website"]
            .destination,
        expected
    );
}

#[test]
fn compare_key_toggles_all_and_differences_without_restarting_the_worker() {
    use crate::comparison::filesystem::{Entry, Kind};
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    app.mode = Mode::Browse;
    for (name, kind, status) in [
        ("same-folder", Kind::Directory, Status::Equal),
        ("same-file", Kind::File, Status::Equal),
        ("different-file", Kind::File, Status::Different),
    ] {
        let entry = Entry { kind, size: None };
        let row = crate::comparison::Row {
            path: name.into(),
            left: Some(entry.clone()),
            right: Some(entry),
            status,
            detail: String::new(),
            aggregate_base: None,
        };
        app.browser.rows.insert(row.path.clone(), row);
    }
    app.browser.rebuild();
    let generation = app.generation;
    press(&mut app, KeyCode::Char('d'));
    assert!(app.browser.differences_only);
    assert_eq!(
        app.browser.visible,
        vec![std::path::PathBuf::from("different-file")]
    );
    press(&mut app, KeyCode::Char('d'));
    assert!(!app.browser.differences_only);
    assert_eq!(app.browser.visible.len(), 3);
    assert_eq!(app.generation, generation);
    assert!(app.worker.is_none());
}

#[test]
fn new_comparison_cannot_overwrite_an_existing_name() {
    let root = tempfile::tempdir().unwrap();
    let profile = Profile::new(
        "existing".into(),
        root.path().to_str().unwrap(),
        root.path().to_str().unwrap(),
        vec![],
    )
    .unwrap();
    let mut app = app(root.path());
    app.store.save(profile.clone()).unwrap();
    app.edit(None);
    let Mode::Editor(editor) = &mut app.mode else {
        panic!("expected editor");
    };
    editor.name = "existing".into();
    editor.source.location = root.path().display().to_string();
    editor.destination.location = root.path().display().to_string();
    press(&mut app, KeyCode::Enter);
    assert!(matches!(app.mode, Mode::Editor(_)));
    assert!(app.message.contains("already exists"));
    assert_eq!(app.store.profiles["existing"], profile);
    assert!(app.worker.is_none());
}

#[test]
fn saving_s3_settings_keeps_session_tokens_for_the_worker_not_the_profile() {
    use super::editor::{EndpointType, Field};
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    app.prompt_password = true; // Stop at the prompt, without making a network connection.
    app.edit(None);
    let Mode::Editor(editor) = &mut app.mode else {
        panic!("expected editor");
    };
    editor.name = "S3 review".into();
    editor.source.kind = EndpointType::Protocol(crate::filetransfer::FileTransferProtocol::AwsS3);
    editor.source.location = "/prefix".into();
    editor.source.fields.insert(Field::Bucket, "bucket".into());
    editor
        .source
        .fields
        .insert(Field::SecretKey, "secret-key-value".into());
    editor
        .source
        .fields
        .insert(Field::SecurityToken, "security-token-value".into());
    editor
        .source
        .fields
        .insert(Field::SessionToken, "session-token-value".into());
    editor.destination.location = root.path().display().to_string();
    press(&mut app, KeyCode::Enter);
    let Mode::Password { passwords, .. } = &app.mode else {
        panic!("expected authentication prompt");
    };
    assert_eq!(
        passwords.source.secret_access_key.as_deref(),
        Some("secret-key-value")
    );
    assert_eq!(
        passwords.source.security_token.as_deref(),
        Some("security-token-value")
    );
    assert_eq!(
        passwords.source.session_token.as_deref(),
        Some("session-token-value")
    );
    let contents = std::fs::read_to_string(root.path().join("comparisons.json")).unwrap();
    for secret in [
        "secret-key-value",
        "security-token-value",
        "session-token-value",
    ] {
        assert!(!contents.contains(secret));
    }
    press(&mut app, KeyCode::Esc);
    assert!(app.worker.is_none());
}

#[test]
fn remote_password_prompts_are_separate_and_never_saved() {
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    app.prompt_password = true;
    let profile = Profile::new(
        "remote".into(),
        "sftp://source/folder",
        "sftp://destination/folder",
        vec![],
    )
    .unwrap();
    app.open(profile.clone());
    let Mode::Password { side, .. } = &app.mode else {
        panic!("expected source password prompt");
    };
    assert!(*side == Side::Source);
    for character in "source-secret".chars() {
        press(&mut app, KeyCode::Char(character));
    }
    press(&mut app, KeyCode::Enter);
    let Mode::Password {
        side,
        passwords,
        value,
        ..
    } = &app.mode
    else {
        panic!("expected destination password prompt");
    };
    assert!(*side == Side::Destination);
    assert_eq!(passwords.source.password.as_deref(), Some("source-secret"));
    assert!(passwords.destination.password.is_none());
    assert!(value.is_empty());
    for character in "destination-secret".chars() {
        press(&mut app, KeyCode::Char(character));
    }
    press(&mut app, KeyCode::Esc);
    assert!(app.worker.is_none()); // No live connections made by the prompt test.
    assert!(app.store.profiles.is_empty());
    let serialized = serde_json::to_string(&profile).unwrap();
    assert!(!serialized.contains("secret"));
}
