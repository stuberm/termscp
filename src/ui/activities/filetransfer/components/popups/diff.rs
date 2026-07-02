use tuirealm::command::{Cmd, CmdResult, Direction, Position};
use tuirealm::component::{AppComponent, Component};
use tuirealm::event::{Event, Key, KeyEvent, KeyModifiers, NoUserEvent};
use tuirealm::props::{AttrValue, Attribute, BorderType, Borders, Color, Props, QueryResult};
use tuirealm::ratatui::layout::{Constraint, Direction as LayoutDirection, Layout, Rect};
use tuirealm::ratatui::style::{Modifier, Style};
use tuirealm::ratatui::text::{Line, Span};
use tuirealm::ratatui::widgets::{Block, Borders as TuiBorders, Paragraph, Wrap};
use tuirealm::state::State;

use crate::ui::activities::filetransfer::diff::{DiffApplyDirection, DiffKind, DiffRow, DiffView};
use crate::ui::activities::filetransfer::{Msg, UiMsg};

fn first_changed_row(view: &DiffView) -> Option<usize> {
    view.rows.iter().position(|row| row.kind != DiffKind::Equal)
}

pub struct DiffPopup {
    props: Props,
    view: DiffView,
    offset: usize,
    selected: usize,
}

impl DiffPopup {
    pub fn new(view: DiffView, border_color: Color) -> Self {
        let mut props = Props::default();
        props.set(
            Attribute::Borders,
            AttrValue::Borders(
                Borders::default()
                    .modifiers(BorderType::Rounded)
                    .color(border_color),
            ),
        );
        Self {
            props,
            selected: first_changed_row(&view).unwrap_or(0),
            view,
            offset: 0,
        }
    }

    fn visible_height(area: Rect) -> usize {
        area.height.saturating_sub(2) as usize
    }

    fn max_offset(&self, area: Rect) -> usize {
        self.view
            .rows
            .len()
            .saturating_sub(Self::visible_height(area))
    }

    fn scroll_down(&mut self, amount: usize) {
        self.offset = self
            .offset
            .saturating_add(amount)
            .min(self.view.rows.len().saturating_sub(1));
        self.selected = self.selected.max(self.offset);
    }

    fn scroll_up(&mut self, amount: usize) {
        self.offset = self.offset.saturating_sub(amount);
        self.selected = self.selected.min(self.offset.saturating_add(amount));
    }

    fn select_next_change(&mut self) {
        let mut index = self.selected.saturating_add(1);
        while index < self.view.rows.len()
            && self.view.rows[index].kind != DiffKind::Equal
            && self.view.rows[self.selected].kind != DiffKind::Equal
        {
            index += 1;
        }

        if let Some(index) = self
            .view
            .rows
            .iter()
            .enumerate()
            .skip(index)
            .find_map(|(index, row)| (row.kind != DiffKind::Equal).then_some(index))
        {
            self.selected = index;
            self.ensure_selected_visible();
        }
    }

    fn select_previous_change(&mut self) {
        let mut index = self.selected;
        while index > 0 && self.view.rows[index - 1].kind != DiffKind::Equal {
            index -= 1;
        }

        if let Some(index) = self
            .view
            .rows
            .iter()
            .enumerate()
            .take(index)
            .rev()
            .find_map(|(index, row)| (row.kind != DiffKind::Equal).then_some(index))
        {
            let mut hunk_start = index;
            while hunk_start > 0 && self.view.rows[hunk_start - 1].kind != DiffKind::Equal {
                hunk_start -= 1;
            }
            self.selected = hunk_start;
            self.ensure_selected_visible();
        }
    }

    fn ensure_selected_visible(&mut self) {
        if self.selected < self.offset {
            self.offset = self.selected;
        }
    }

    fn selected_hunk_contains(&self, row_index: usize) -> bool {
        if self
            .view
            .rows
            .get(self.selected)
            .is_none_or(|row| row.kind == DiffKind::Equal)
        {
            return row_index == self.selected;
        }

        let mut start = self.selected;
        while start > 0 && self.view.rows[start - 1].kind != DiffKind::Equal {
            start -= 1;
        }
        let mut end = self.selected + 1;
        while end < self.view.rows.len() && self.view.rows[end].kind != DiffKind::Equal {
            end += 1;
        }

        (start..end).contains(&row_index)
    }

    fn row_style(kind: DiffKind) -> Style {
        match kind {
            DiffKind::Equal => Style::default(),
            DiffKind::Added => Style::default().fg(Color::LightGreen),
            DiffKind::Removed => Style::default().fg(Color::LightRed),
        }
    }

    fn line_for(row: &DiffRow, left: bool, selected: bool) -> Line<'static> {
        let (no, text, marker) = if left {
            (
                row.left_no,
                row.left.as_str(),
                if row.kind == DiffKind::Removed {
                    "-"
                } else {
                    " "
                },
            )
        } else {
            (
                row.right_no,
                row.right.as_str(),
                if row.kind == DiffKind::Added {
                    "+"
                } else {
                    " "
                },
            )
        };
        let number = no.map_or_else(|| "     ".to_string(), |n| format!("{n:>5}"));
        let cursor = if selected { "▶" } else { " " };
        let style = if selected {
            Self::row_style(row.kind).add_modifier(Modifier::REVERSED)
        } else {
            Self::row_style(row.kind)
        };
        Line::from(vec![
            Span::styled(cursor, style),
            Span::styled(number, style.fg(Color::DarkGray)),
            Span::styled(" ", style),
            Span::styled(marker.to_string(), style),
            Span::styled(" ", style),
            Span::styled(text.to_string(), style),
        ])
    }

    fn pane<'a>(&self, title: String, lines: Vec<Line<'a>>, border_color: Color) -> Paragraph<'a> {
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(TuiBorders::ALL)
                    .border_style(Style::default().fg(border_color))
                    .title(title),
            )
            .wrap(Wrap { trim: false })
    }
}

impl Component for DiffPopup {
    fn view(&mut self, frame: &mut tuirealm::ratatui::Frame, area: Rect) {
        self.offset = self.offset.min(self.max_offset(area));
        let border_color = self
            .props
            .get(Attribute::Borders)
            .and_then(AttrValue::as_borders)
            .map(|b| b.color)
            .unwrap_or(Color::Reset);
        let chunks = Layout::default()
            .direction(LayoutDirection::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);
        let take = Self::visible_height(area);
        if self.selected >= self.offset.saturating_add(take) {
            self.offset = self.selected.saturating_add(1).saturating_sub(take);
        }
        let rows = self
            .view
            .rows
            .iter()
            .enumerate()
            .skip(self.offset)
            .take(take)
            .collect::<Vec<_>>();
        let left = rows
            .iter()
            .map(|(index, row)| Self::line_for(row, true, self.selected_hunk_contains(*index)))
            .collect::<Vec<_>>();
        let right = rows
            .iter()
            .map(|(index, row)| Self::line_for(row, false, self.selected_hunk_contains(*index)))
            .collect::<Vec<_>>();
        let status = format!(
            "{}  ({}/{})  Tab/↑↓ select  ←/→ or </> copy  Esc/q close",
            self.view.left_title,
            self.offset.saturating_add(1).min(self.view.rows.len()),
            self.view.rows.len()
        );
        frame.render_widget(self.pane(status, left, border_color), chunks[0]);
        frame.render_widget(
            self.pane(self.view.right_title.clone(), right, border_color),
            chunks[1],
        );
    }

    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        self.props.set(attr, value);
    }

    fn query<'a>(&'a self, attr: Attribute) -> Option<QueryResult<'a>> {
        self.props.get_for_query(attr)
    }

    fn state(&self) -> State {
        State::None
    }

    fn perform(&mut self, cmd: Cmd) -> CmdResult {
        match cmd {
            Cmd::Move(Direction::Down) => self.scroll_down(1),
            Cmd::Move(Direction::Up) => self.scroll_up(1),
            Cmd::Scroll(Direction::Down) => self.scroll_down(8),
            Cmd::Scroll(Direction::Up) => self.scroll_up(8),
            Cmd::GoTo(Position::Begin) => self.offset = 0,
            Cmd::GoTo(Position::End) => self.offset = self.view.rows.len().saturating_sub(1),
            _ => return CmdResult::NoChange,
        }
        CmdResult::Changed(State::None)
    }
}

impl AppComponent<Msg, NoUserEvent> for DiffPopup {
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        match ev {
            Event::Keyboard(KeyEvent {
                code: Key::Esc | Key::Char('q'),
                ..
            }) => Some(Msg::Ui(UiMsg::CloseDiffPopup)),
            Event::Keyboard(KeyEvent {
                code: Key::Right | Key::Char('>' | '.'),
                modifiers: KeyModifiers::NONE | KeyModifiers::SHIFT,
            }) => Some(Msg::Ui(UiMsg::DiffApplyChange(
                self.selected,
                DiffApplyDirection::LeftToRight,
            ))),
            Event::Keyboard(KeyEvent {
                code: Key::Left | Key::Char('<' | ','),
                modifiers: KeyModifiers::NONE | KeyModifiers::SHIFT,
            }) => Some(Msg::Ui(UiMsg::DiffApplyChange(
                self.selected,
                DiffApplyDirection::RightToLeft,
            ))),
            Event::Keyboard(KeyEvent { code: Key::Tab, .. }) => {
                self.select_next_change();
                Some(Msg::None)
            }
            Event::Keyboard(KeyEvent {
                code: Key::BackTab, ..
            }) => {
                self.select_previous_change();
                Some(Msg::None)
            }
            Event::Keyboard(KeyEvent {
                code: Key::Down, ..
            }) => {
                self.select_next_change();
                Some(Msg::None)
            }
            Event::Keyboard(KeyEvent { code: Key::Up, .. }) => {
                self.select_previous_change();
                Some(Msg::None)
            }
            Event::Keyboard(KeyEvent {
                code: Key::PageDown,
                ..
            }) => {
                self.perform(Cmd::Scroll(Direction::Down));
                Some(Msg::None)
            }
            Event::Keyboard(KeyEvent {
                code: Key::PageUp, ..
            }) => {
                self.perform(Cmd::Scroll(Direction::Up));
                Some(Msg::None)
            }
            Event::Keyboard(KeyEvent {
                code: Key::Home, ..
            }) => {
                self.perform(Cmd::GoTo(Position::Begin));
                Some(Msg::None)
            }
            Event::Keyboard(KeyEvent { code: Key::End, .. }) => {
                self.perform(Cmd::GoTo(Position::End));
                Some(Msg::None)
            }
            _ => None,
        }
    }
}
