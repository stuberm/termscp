use tuirealm::command::{Cmd, CmdResult, Direction, Position};
use tuirealm::component::{AppComponent, Component};
use tuirealm::event::{Event, Key, KeyEvent, NoUserEvent};
use tuirealm::props::{AttrValue, Attribute, BorderType, Borders, Color, Props, QueryResult};
use tuirealm::ratatui::layout::{Constraint, Direction as LayoutDirection, Layout, Rect};
use tuirealm::ratatui::style::Style;
use tuirealm::ratatui::text::{Line, Span};
use tuirealm::ratatui::widgets::{Block, Borders as TuiBorders, Paragraph, Wrap};
use tuirealm::state::State;

use crate::ui::activities::filetransfer::diff::{DiffKind, DiffRow, DiffView};
use crate::ui::activities::filetransfer::{Msg, UiMsg};

pub struct DiffPopup {
    props: Props,
    view: DiffView,
    offset: usize,
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
    }

    fn scroll_up(&mut self, amount: usize) {
        self.offset = self.offset.saturating_sub(amount);
    }

    fn row_style(kind: DiffKind) -> Style {
        match kind {
            DiffKind::Equal => Style::default(),
            DiffKind::Added => Style::default().fg(Color::LightGreen),
            DiffKind::Removed => Style::default().fg(Color::LightRed),
        }
    }

    fn line_for(row: &DiffRow, left: bool) -> Line<'static> {
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
        Line::from(vec![
            Span::styled(number, Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled(marker.to_string(), Self::row_style(row.kind)),
            Span::raw(" "),
            Span::styled(text.to_string(), Self::row_style(row.kind)),
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
        let rows = self
            .view
            .rows
            .iter()
            .skip(self.offset)
            .take(take)
            .collect::<Vec<_>>();
        let left = rows
            .iter()
            .map(|row| Self::line_for(row, true))
            .collect::<Vec<_>>();
        let right = rows
            .iter()
            .map(|row| Self::line_for(row, false))
            .collect::<Vec<_>>();
        let status = format!(
            "{}  ({}/{})  Esc/q close",
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
                code: Key::Down, ..
            }) => {
                self.perform(Cmd::Move(Direction::Down));
                Some(Msg::None)
            }
            Event::Keyboard(KeyEvent { code: Key::Up, .. }) => {
                self.perform(Cmd::Move(Direction::Up));
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
