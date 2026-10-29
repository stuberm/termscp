use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::{Constraint, Layout, Rect};
use tuirealm::ratatui::style::{Color, Modifier, Style};
use tuirealm::ratatui::text::{Line, Span};
use tuirealm::ratatui::widgets::{
    Block, Borders, Cell, Clear, Paragraph, Row as TableRow, Table, TableState, Wrap,
};
use unicode_width::UnicodeWidthStr;

use super::{App, Mode};
use crate::comparison::Status;
use crate::comparison::filesystem::{Entry, Kind};

fn status_style(status: Status) -> Style {
    Style::default().fg(match status {
        Status::Equal => Color::DarkGray,
        Status::Different | Status::TypeConflict => Color::Yellow,
        Status::LeftOnly => Color::Cyan,
        Status::RightOnly => Color::Magenta,
        Status::Pending | Status::Unverified => Color::Gray,
        Status::Error => Color::Red,
    })
}

fn block(title: impl Into<String>) -> Block<'static> {
    Block::default().borders(Borders::ALL).title(title.into())
}

/// File names/remote error text can contain terminal control characters. Render them visibly.
fn safe(text: &str) -> String {
    text.chars()
        .flat_map(|character| {
            if character.is_control() && character != '\t' {
                character.escape_default().collect::<Vec<_>>()
            } else if character == '\t' {
                vec![' ', ' ', ' ', ' ']
            } else {
                vec![character]
            }
        })
        .collect()
}

fn file_cell(name: &str, entry: &Option<Entry>) -> String {
    match entry {
        None => "—".into(),
        Some(entry) => format!(
            "{}{}",
            safe(name),
            match entry.kind {
                Kind::Directory => "/",
                Kind::Symlink => " @",
                Kind::Special => " [special]",
                Kind::File => "",
            }
        ),
    }
}

pub(super) fn draw(frame: &mut Frame<'_>, app: &App) {
    if matches!(
        app.mode,
        Mode::Profiles { .. }
            | Mode::CreateFolder { .. }
            | Mode::MoveProfile { .. }
            | Mode::DeleteFolder(_)
    ) {
        let selected = if let Mode::Profiles { selected } = app.mode {
            selected
        } else {
            0
        };
        draw_home(frame, app, selected);
        match &app.mode {
            Mode::CreateFolder { value } => dialog(
                frame,
                &format!(
                    "Folder name (one level only):\n> {}\n{}\nEnter create · Esc cancel",
                    safe(value),
                    safe(&app.message)
                ),
                " Create folder ",
            ),
            Mode::MoveProfile { profile, selected } => {
                draw_move_picker(frame, app, profile, *selected)
            }
            Mode::DeleteFolder(name) => dialog(
                frame,
                &format!(
                    "Delete folder '{}'?\nIts {} profiles will return to the root list.\nNo profiles or endpoint files will be deleted.\n\ny: confirm · n/Esc: cancel",
                    safe(name),
                    app.store.folders.get(name).map_or(0, Vec::len)
                ),
                " Delete folder ",
            ),
            _ => {}
        }
        return;
    }
    if let Mode::Editor(editor) = &app.mode {
        draw_editor(frame, app, editor);
        return;
    }
    let areas = Layout::vertical([
        Constraint::Length(4),
        Constraint::Min(1),
        Constraint::Length(4),
    ])
    .split(frame.area());
    let name = app
        .profile
        .as_ref()
        .map(|profile| profile.name.as_str())
        .unwrap_or("Profiles");
    let roots = app.profile.as_ref().map(|profile| format!("SOURCE      {}\nDESTINATION {}", safe(&profile.source.description()), safe(&profile.destination.description())))
        .unwrap_or_else(|| "Create a folder comparison: select source and destination\nEither endpoint may be local or remote; copying in file diff requires confirmation".into());
    frame.render_widget(
        Paragraph::new(roots).block(block(format!(" sidemirror / {} / COMPARE ", safe(name)))),
        areas[0],
    );
    match &app.mode {
        Mode::Diff { view, selected } => {
            let height = areas[1].height.saturating_sub(3) as usize;
            let offset = selected.saturating_sub(height / 2);
            let rows = view
                .lines
                .iter()
                .skip(offset)
                .take(height.max(1))
                .enumerate()
                .map(|(index, line)| {
                    let in_chunk = line.changed
                        && view.lines
                            [(offset + index).min(*selected)..=(offset + index).max(*selected)]
                            .iter()
                            .all(|line| line.changed);
                    let value = |side: &Option<(usize, String)>, left: bool| {
                        let Some((number, text)) = side else {
                            return Line::default();
                        };
                        let highlights = if left {
                            &view.highlights.left
                        } else {
                            &view.highlights.right
                        };
                        let mut spans = vec![Span::styled(
                            format!("{number:>5}  "),
                            Style::default().fg(Color::DarkGray),
                        )];
                        if let Some(tokens) = highlights.get(number.saturating_sub(1)) {
                            spans.extend(tokens.iter().map(|token| {
                                Span::styled(
                                    safe(&token.text),
                                    Style::default().fg(Color::Rgb(
                                        token.color[0],
                                        token.color[1],
                                        token.color[2],
                                    )),
                                )
                            }));
                        } else {
                            spans.push(Span::raw(safe(text)));
                        }
                        Line::from(spans)
                    };
                    TableRow::new(vec![
                        Cell::from(value(&line.left, true)),
                        Cell::from(if line.changed { "≠" } else { " " })
                            .style(Style::default().fg(Color::Yellow)),
                        Cell::from(value(&line.right, false)),
                    ])
                    .style(if in_chunk {
                        Style::default().bg(Color::DarkGray)
                    } else if line.changed {
                        Style::default().bg(Color::Rgb(48, 36, 30))
                    } else {
                        Style::default()
                    })
                });
            let table = Table::new(
                rows,
                [
                    Constraint::Percentage(48),
                    Constraint::Length(3),
                    Constraint::Percentage(48),
                ],
            )
            .header(
                TableRow::new([
                    format!("SOURCE · {}", view.left_document.encoding.label()),
                    String::new(),
                    format!("DESTINATION · {}", view.right_document.encoding.label()),
                ])
                .style(Style::default().add_modifier(Modifier::BOLD)),
            )
            .block(block(format!(" Text diff: {} ", safe(&view.title))))
            .row_highlight_style(Style::default().bg(Color::DarkGray));
            let mut state =
                TableState::default().with_selected(Some(selected.saturating_sub(offset)));
            frame.render_stateful_widget(table, areas[1], &mut state);
        }
        _ => draw_browser(frame, app, areas[1]),
    }
    let hint = match &app.mode {
        Mode::CreateFolder { .. } => "Enter create · Esc cancel",
        Mode::MoveProfile { .. } => "↑↓ select folder · Enter move · Esc cancel",
        Mode::DeleteFolder(_) => "y delete folder only · n/Esc cancel",
        Mode::Profiles { .. } => {
            "↑↓ select  Enter open/create  n new  e edit  d delete  Esc back  q quit"
        }
        Mode::Editor(_) => {
            "Tab field  F2 source/F3 destination  Space/←→ protocol/options  Ctrl-U clear  Enter save & open"
        }
        Mode::Delete(_) => "y delete profile only  n/Esc cancel",
        Mode::Password { .. } => {
            "Enter connect · Esc cancel · password saved in OS keychain after successful login"
        }
        Mode::Diff { .. } => {
            "↑↓/jk scroll  PgUp/PgDn  Home/End  n/p hunk  F5 →/F6 ← chunk  F7 →/F8 ← file  Esc back"
        }
        Mode::Help => "Press any key to return",
        Mode::Browse => {
            "↑↓/jk select  Enter open  ←/Backspace parent  d all/differences  f recursive  n/p difference\ni ignore selected  r refresh  c cancel  o profiles  s save/edit  a password/reconnect  ? help  q quit"
        }
    };
    frame.render_widget(
        Paragraph::new(format!("{}\n{}", safe(&app.message), hint)).wrap(Wrap { trim: false }),
        areas[2],
    );
    match &app.mode {
        Mode::Delete(name) => dialog(
            frame,
            &format!(
                "Delete profile '{}'?\nThis does not delete any files.\n\ny: confirm · n/Esc: cancel",
                safe(name)
            ),
            " Delete profile ",
        ),
        Mode::Password { value, side, .. } => dialog(
            frame,
            &format!(
                "{} password / default protocol secret: {}\n{}\nEnter to continue (blank reuses credentials); Esc to cancel",
                if *side == super::Side::Source {
                    "Source"
                } else {
                    "Destination"
                },
                "•".repeat(value.chars().count()),
                "Native OS credential storage required"
            ),
            " Remote authentication ",
        ),
        Mode::Help => dialog(
            frame,
            "Folder comparison\n\nEnter: descend into paired directories or open a file diff.\nMissing sides remain placeholders; browsing never creates folders.\nd: show all entries / only differences (also includes unknown/errors).\nf: direct children / all descendants of the current directory.\nn / p: next / previous non-equal entry (wraps in folder view).\ni: ignore selected file/folder on both sides (saved with saved profiles).\nr: refresh; c: cooperative cancellation after current I/O finishes.\no: profiles; s: save/edit current profile; a: password/reconnect.\nSymlinks/special files are not followed or verified.\nMatching bytes, not timestamps, determine regular-file equality.\nText diff supports UTF-8 / Windows-1252 up to 5 MiB / 10,000 lines per side.\n\nPress any key to return.",
            " Keyboard help ",
        ),
        _ => {}
    }
}

fn draw_move_picker(frame: &mut Frame<'_>, app: &App, profile: &str, selected: usize) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    let areas = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(3),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(format!(
            "Move '{}'\nSelect a destination. This only changes profile organization.",
            safe(profile)
        )),
        areas[0],
    );
    let current = app.store.folder_of(profile);
    let root = TableRow::new(vec![
        "Root (no folder)".to_string(),
        if current.is_none() {
            "Current".into()
        } else {
            String::new()
        },
    ]);
    let folders = app.store.folders.keys().map(|name| {
        TableRow::new(vec![
            safe(name),
            if current == Some(name.as_str()) {
                "Current".into()
            } else {
                String::new()
            },
        ])
    });
    let table = Table::new(
        std::iter::once(root).chain(folders),
        [Constraint::Percentage(80), Constraint::Percentage(20)],
    )
    .block(block(" Available folders "))
    .row_highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("› ");
    frame.render_stateful_widget(
        table,
        areas[1],
        &mut TableState::default().with_selected(Some(selected)),
    );
    frame.render_widget(
        Paragraph::new(format!(
            "{}\n↑↓/jk select · Home/End · Enter move · Esc cancel",
            safe(&app.message)
        ))
        .wrap(Wrap { trim: false }),
        areas[2],
    );
}

fn input_style(selected: bool) -> Style {
    if selected {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    }
}

/// Keep the insertion point visible, including wide Unicode and masked passwords.
fn input_tail(text: &str, width: u16, active: bool) -> (String, u16) {
    let text = safe(text);
    if !active {
        return (text, 0);
    }
    let available = width.saturating_sub(1) as usize;
    let mut used = 0;
    let mut start = text.len();
    for (index, _) in text.char_indices().rev() {
        // Measure the suffix as a string so emoji/ZWJ sequences use terminal width.
        let length = text[index..].width();
        if length > available {
            break;
        }
        used = length;
        start = index;
    }
    (text[start..].to_string(), used as u16)
}

fn draw_input(frame: &mut Frame<'_>, text: &str, area: Rect, title: &str, active: bool) {
    let border = block(title);
    let inner = border.inner(area);
    let (text, cursor) = input_tail(text, inner.width, active);
    frame.render_widget(
        Paragraph::new(text)
            .style(input_style(active))
            .block(border),
        area,
    );
    if active && inner.width > 0 && inner.height > 0 {
        frame.set_cursor_position((inner.x + cursor, inner.y));
    }
}

fn draw_editor(frame: &mut Frame<'_>, app: &App, editor: &super::editor::Editor) {
    let areas = Layout::vertical([Constraint::Min(1), Constraint::Length(4)]).split(frame.area());
    let outer = block(if editor.original.is_some() {
        " Edit comparison · save and open "
    } else {
        " New comparison · save and open "
    });
    let inner = outer.inner(areas[0]);
    frame.render_widget(outer, areas[0]);
    let form = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(3),
    ])
    .split(inner);
    draw_input(
        frame,
        &editor.name,
        form[0],
        " Comparison name ",
        editor.field == 0,
    );
    let columns =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(form[1]);
    for (side, endpoint, area, title) in [
        (
            super::Side::Source,
            &editor.source,
            columns[0],
            " Source settings · F2 ",
        ),
        (
            super::Side::Destination,
            &editor.destination,
            columns[1],
            " Destination settings · F3 ",
        ),
    ] {
        let selected = editor.selected(side);
        let inner = block(title).inner(area);
        let lines = endpoint
            .rows()
            .into_iter()
            .enumerate()
            .flat_map(|(index, (label, value))| {
                let active = selected == Some(index);
                let style = input_style(active);
                [
                    Line::styled(label, style),
                    Line::styled(input_tail(&value, inner.width, active).0, style),
                ]
            })
            .collect::<Vec<_>>();
        let scroll = selected
            .map(|index| (index * 2 + 1).saturating_sub(area.height.saturating_sub(3) as usize))
            .unwrap_or(0);
        frame.render_widget(
            Paragraph::new(lines)
                .scroll((scroll as u16, 0))
                .block(block(title)),
            area,
        );
        if let Some(index) = selected {
            let y = (index * 2 + 1).saturating_sub(scroll);
            if inner.width > 0 && y < inner.height as usize {
                let value = &endpoint.rows()[index].1;
                let cursor = input_tail(value, inner.width, true).1;
                frame.set_cursor_position((inner.x + cursor, inner.y + y as u16));
            }
        }
    }
    draw_input(
        frame,
        &editor.ignores,
        form[2],
        " Ignore globs · semicolon-separated ",
        editor.field == editor.ignore_index(),
    );
    frame.render_widget(Paragraph::new(format!("{}\nTab/Shift-Tab fields · F1 name/F2 source/F3 destination · Space/←→ type/options\nPasswords: native OS keychain (required) · Ctrl-U clear · Enter/Ctrl-S save & open · Esc cancel", safe(&app.message)))
        .wrap(Wrap { trim: false }), areas[1]);
}

fn draw_home(frame: &mut Frame<'_>, app: &App, selected: usize) {
    let areas = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(3),
    ])
    .split(frame.area());
    frame.render_widget(Paragraph::new("sidemirror · Folder comparisons\nChoose a saved comparison or create a new one. Source and destination can each be local or remote."), areas[0]);
    let new = TableRow::new(vec![
        "+ New comparison".to_string(),
        "Select source…".into(),
        "Select destination…".into(),
    ])
    .style(Style::default().fg(Color::Cyan));
    let entries = App::home_entries(&app.store, app.home_folder.as_deref());
    let profiles = entries.into_iter().skip(1).map(|entry| match entry {
        super::HomeEntry::Folder(name) => TableRow::new(vec![
            format!("▸ {}", safe(&name)),
            format!("{} profiles", app.store.folders[&name].len()),
            String::new(),
        ])
        .style(Style::default().fg(Color::Cyan)),
        super::HomeEntry::Profile(name) => {
            let profile = &app.store.profiles[&name];
            TableRow::new(vec![
                safe(&profile.name),
                safe(&profile.source.description()),
                safe(&profile.destination.description()),
            ])
        }
        super::HomeEntry::New => unreachable!(),
    });
    let table = Table::new(
        std::iter::once(new).chain(profiles),
        [
            Constraint::Percentage(26),
            Constraint::Percentage(37),
            Constraint::Percentage(37),
        ],
    )
    .header(
        TableRow::new(["COMPARISON", "SOURCE", "DESTINATION"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    )
    .block(block(format!(
        " Saved comparisons / {} ",
        app.home_folder
            .as_deref()
            .map(safe)
            .unwrap_or_else(|| "All folders".into())
    )))
    .row_highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    );
    frame.render_stateful_widget(
        table,
        areas[1],
        &mut TableState::default().with_selected(Some(selected)),
    );
    frame.render_widget(
        Paragraph::new(format!(
            "{}\n↑↓ select · Enter open · n new · e edit · c duplicate · f folder · m move · d delete · Backspace/Esc back · q quit",
            safe(&app.message)
        ))
        .wrap(Wrap { trim: false }),
        areas[2],
    );
}

fn draw_browser(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let sections = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);
    let option = |label: &str, selected: bool| {
        Span::styled(
            if selected {
                format!("[{label}]")
            } else {
                format!(" {label} ")
            },
            input_style(selected),
        )
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("Show: "),
            option("All files & folders", !app.browser.differences_only),
            Span::raw("  "),
            option("Differences only", app.browser.differences_only),
            Span::raw("  d: toggle"),
        ])),
        sections[0],
    );
    let area = sections[1];
    let height = area.height.saturating_sub(4) as usize;
    let offset = app.browser.selected.saturating_sub(height / 2);
    let rows = app
        .browser
        .visible
        .iter()
        .skip(offset)
        .take(height.max(1))
        .map(|path| {
            let row = &app.browser.rows[path];
            let name = if app.browser.recursive {
                path.strip_prefix(&app.browser.directory)
                    .unwrap_or(path)
                    .display()
                    .to_string()
            } else {
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            };
            TableRow::new(vec![
                Cell::from(file_cell(&name, &row.left)),
                Cell::from(row.status.label()),
                Cell::from(file_cell(&name, &row.right)),
            ])
            .style(status_style(row.status))
        });
    let equal = app
        .browser
        .rows
        .values()
        .filter(|row| row.status == Status::Equal)
        .count();
    let total = app.browser.rows.len();
    let title = format!(
        " /{} · {} · {} · {} equal / {} total{} ",
        safe(&app.browser.directory.display().to_string()),
        if app.browser.differences_only {
            "differences"
        } else {
            "all entries"
        },
        if app.browser.recursive {
            "recursive"
        } else {
            "folder"
        },
        equal,
        total,
        if app.busy { " · working…" } else { "" }
    );
    let table = Table::new(
        rows,
        [
            Constraint::Percentage(42),
            Constraint::Length(18),
            Constraint::Percentage(42),
        ],
    )
    .header(
        TableRow::new(["SOURCE", "STATUS", "DESTINATION"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    )
    .block(block(title))
    .row_highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    );
    frame.render_stateful_widget(
        table,
        area,
        &mut TableState::default().with_selected(Some(app.browser.selected.saturating_sub(offset))),
    );
    if let Some(row) = app.browser.selected_row()
        && !row.detail.is_empty()
        && area.height > 3
    {
        let detail = Rect {
            x: area.x + 1,
            y: area.y + area.height - 2,
            width: area.width.saturating_sub(2),
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(safe(&row.detail)).style(status_style(row.status)),
            detail,
        );
    }
}

fn popup(area: Rect, width: u16, height: u16) -> Rect {
    let columns = Layout::horizontal([
        Constraint::Percentage((100 - width) / 2),
        Constraint::Percentage(width),
        Constraint::Min(0),
    ])
    .split(area);
    Layout::vertical([
        Constraint::Percentage((100 - height) / 2),
        Constraint::Percentage(height),
        Constraint::Min(0),
    ])
    .split(columns[1])[1]
}

fn dialog(frame: &mut Frame<'_>, message: &str, title: &str) {
    let area = popup(frame.area(), 85, 75);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(message)
            .wrap(Wrap { trim: false })
            .block(block(title)),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::super::state::Browser;
    use super::*;
    use crate::comparison::profiles::ProfileStore;
    use tuirealm::ratatui::Terminal;
    use tuirealm::ratatui::backend::TestBackend;

    #[test]
    fn compare_filter_toggle_highlights_active_mode_and_filters_both_columns() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = App {
            browser: Browser::default(),
            store: ProfileStore::load(directory.path().join("profiles.json")).unwrap(),
            profile: None,
            worker: None,
            mode: Mode::Browse,
            home_folder: None,
            generation: 0,
            busy: false,
            loading_diff: false,
            message: String::new(),
            keyring: false,
            prompt_password: false,
            quit: false,
            pending_copy: None,
        };
        for (name, kind, status) in [
            ("same-file", Kind::File, Status::Equal),
            ("same-folder", Kind::Directory, Status::Equal),
            ("changed-file", Kind::File, Status::Different),
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
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        for differences_only in [false, true, false] {
            if app.browser.differences_only != differences_only {
                app.browse_key(crossterm::event::KeyCode::Char('d'));
            }
            terminal.draw(|frame| draw(frame, &app)).unwrap();
            let rendered: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(tuirealm::ratatui::buffer::Cell::symbol)
                .collect();
            assert!(rendered.contains(if differences_only {
                "[Differences only]"
            } else {
                "[All files & folders]"
            }));
            assert!(rendered.contains("d: toggle"));
            assert!(rendered.contains("changed-file"));
            assert_eq!(rendered.contains("same-file"), !differences_only);
            assert_eq!(rendered.contains("same-folder"), !differences_only);
            assert_eq!(rendered.matches("changed-file").count(), 2); // Source and destination stay aligned.
        }
    }

    #[test]
    fn home_lists_new_comparison_and_saved_source_destination_pairs() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = ProfileStore::load(directory.path().join("profiles.json")).unwrap();
        store
            .save(
                crate::comparison::profiles::Profile::new(
                    "Servers".into(),
                    "sftp://a/source",
                    "ftps://b/destination",
                    vec![],
                )
                .unwrap(),
            )
            .unwrap();
        let app = App {
            browser: Browser::default(),
            store,
            profile: None,
            worker: None,
            mode: Mode::Profiles { selected: 0 },
            home_folder: None,
            generation: 0,
            busy: false,
            loading_diff: false,
            message: String::new(),
            keyring: false,
            prompt_password: false,
            quit: false,
            pending_copy: None,
        };
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(tuirealm::ratatui::buffer::Cell::symbol)
            .collect();
        for text in [
            "New comparison",
            "Saved comparisons",
            "SOURCE",
            "DESTINATION",
            "Servers",
            "sftp://a/source",
            "ftps://b/destination",
        ] {
            assert!(rendered.contains(text), "missing {text}");
        }
    }

    #[test]
    fn renders_both_settings_panels_masks_secrets_and_handles_small_terminals() {
        use super::super::editor::{Editor, EndpointType, Field};
        let directory = tempfile::tempdir().unwrap();
        let mut editor = Editor::new(None, None);
        editor.source.kind =
            EndpointType::Protocol(crate::filetransfer::FileTransferProtocol::AwsS3);
        editor
            .source
            .fields
            .insert(Field::SecretKey, "do-not-render-source".into());
        editor.destination.kind =
            EndpointType::Protocol(crate::filetransfer::FileTransferProtocol::Sftp);
        editor
            .destination
            .fields
            .insert(Field::Password, "do-not-render-destination".into());
        let mut app = App {
            browser: Browser::default(),
            store: ProfileStore::load(directory.path().join("profiles.json")).unwrap(),
            profile: None,
            worker: None,
            mode: Mode::Editor(editor),
            home_folder: None,
            generation: 0,
            busy: false,
            loading_diff: false,
            message: "validation-error-visible".into(),
            keyring: false,
            prompt_password: false,
            quit: false,
            pending_copy: None,
        };
        for (width, height) in [(160, 40), (80, 24), (40, 12), (1, 1)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
            let rendered: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(tuirealm::ratatui::buffer::Cell::symbol)
                .collect();
            assert!(!rendered.contains("do-not-render-source"));
            assert!(!rendered.contains("do-not-render-destination"));
            if width == 160 {
                for text in [
                    "Source settings",
                    "Destination settings",
                    "Secret access key",
                    "Password",
                    "validation-error-visible",
                ] {
                    assert!(rendered.contains(text));
                }
            }
        }
        let Mode::Editor(editor) = &mut app.mode else {
            unreachable!()
        };
        editor.name = "long name 界".repeat(20);
        editor.ignores = "long/path/界".repeat(20);
        let source_secret = 1 + editor
            .source
            .field_ids()
            .iter()
            .position(|field| *field == Field::SecretKey)
            .unwrap();
        let destination_password = 1
            + editor.source.field_ids().len()
            + editor
                .destination
                .field_ids()
                .iter()
                .position(|field| *field == Field::Password)
                .unwrap();
        let ignore = editor.ignore_index();
        for field in [0, source_secret, destination_password, ignore] {
            for (width, height) in [(120, 30), (40, 16)] {
                let Mode::Editor(editor) = &mut app.mode else {
                    unreachable!()
                };
                editor.field = field;
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal.draw(|frame| draw(frame, &app)).unwrap();
                assert!(
                    terminal.backend().cursor_visible(),
                    "field {field} must show a cursor"
                );
                let position = terminal.backend().cursor_position();
                assert!(position.x < width && position.y < height);
                let symbol = terminal.backend().buffer()[(position.x, position.y)].symbol();
                assert_eq!(
                    symbol, " ",
                    "insertion cursor must follow the visible input tail"
                );
            }
        }
    }

    #[test]
    fn input_tail_uses_display_width_and_keeps_insertion_space() {
        assert_eq!(
            input_tail("prefix界e\u{301}", 5, true),
            ("x界e\u{301}".into(), 4)
        );
        assert_eq!(input_tail(&"•".repeat(100), 10, true), ("•".repeat(9), 9));
        assert_eq!(input_tail("text", 0, true), (String::new(), 0));
        assert_eq!(input_tail("prefix👩‍💻", 3, true), ("👩‍💻".into(), 2));
    }

    #[test]
    fn renders_empty_small_terminal_and_profiles_without_panicking() {
        let directory = tempfile::tempdir().unwrap();
        let app = App {
            browser: Browser::default(),
            store: ProfileStore::load(directory.path().join("profiles.json")).unwrap(),
            profile: None,
            worker: None,
            mode: Mode::Profiles { selected: 0 },
            generation: 0,
            busy: false,
            loading_diff: false,
            home_folder: None,
            message: "Ready".into(),
            keyring: false,
            prompt_password: false,
            quit: false,
            pending_copy: None,
        };
        for (width, height) in [(100, 30), (20, 8), (1, 1)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
        }
        assert_eq!(safe("bad\x1b[31m\nname"), "bad\\u{1b}[31m\\nname");
    }
}
