//! ## FileList
//!
//! `FileList` component renders a file list tab

use tuirealm::command::{Cmd, CmdResult, Direction, Position};
use tuirealm::component::Component;
use tuirealm::props::{
    AttrValue, Attribute, Borders, Color, HorizontalAlignment, PropValue, Props, QueryResult,
    SpanStatic, Style, Table, TextModifiers, Title,
};
use tuirealm::ratatui::text::{Line, Span};
use tuirealm::ratatui::widgets::{List as TuiList, ListDirection, ListItem, ListState};
use tuirealm::state::{State, StateValue};

pub const FILE_LIST_CMD_SELECT_ALL: &str = "A";
pub const FILE_LIST_CMD_DESELECT_ALL: &str = "D";
const PROP_DOT_DOT: &str = "dot_dot";
pub const PROP_ROW_INDICES: &str = "row_indices";
pub const PROP_VISUAL_INDEX: &str = "visual_index";

/// OwnStates contains states for this component
#[derive(Clone, Default)]
struct OwnStates {
    list_index: usize, // Index of selected element in list
    list_len: usize,   // Length of the list
    dot_dot: bool,
    row_indices: Vec<Option<usize>>,
}

impl OwnStates {
    /// Initialize list states
    pub fn init_list_states(&mut self, len: usize, has_dot_dot: bool) {
        self.dot_dot = has_dot_dot;
        self.list_len = len + if has_dot_dot { 1 } else { 0 };
        self.row_indices = (0..len).map(Some).collect();
        self.fix_list_index();
    }

    pub fn set_row_indices(&mut self, row_indices: Vec<Option<usize>>) {
        self.row_indices = row_indices;
        self.list_len = self.row_indices.len() + if self.dot_dot { 1 } else { 0 };
        self.fix_list_index();
    }

    /// Incremenet list index.
    /// If `can_rewind` is `true` the index rewinds when boundary is reached
    pub fn incr_list_index(&mut self, can_rewind: bool) {
        let Some(next) = self.next_selectable_down(can_rewind) else {
            return;
        };
        self.list_index = next;
    }

    pub fn real_index(&self) -> Option<usize> {
        if self.dot_dot {
            self.row_indices
                .get(self.list_index.saturating_sub(1))
                .copied()
                .flatten()
        } else {
            self.row_indices.get(self.list_index).copied().flatten()
        }
    }

    /// Decrement list index
    /// If `can_rewind` is `true` the index rewinds when boundary is reached
    pub fn decr_list_index(&mut self, can_rewind: bool) {
        let Some(next) = self.next_selectable_up(can_rewind) else {
            return;
        };
        self.list_index = next;
    }

    pub fn list_index_at_first(&mut self) {
        self.list_index = (0..self.list_len())
            .find(|idx| self.is_selectable(*idx))
            .unwrap_or(0);
    }

    pub fn list_index_at_last(&mut self) {
        self.list_index = (0..self.list_len())
            .rev()
            .find(|idx| self.is_selectable(*idx))
            .unwrap_or(0);
    }

    /// Returns the length of the file list, which is actually the capacity of the selection vector
    pub fn list_len(&self) -> usize {
        self.list_len
    }

    /// Keep index if possible, otherwise set to lenght - 1
    fn fix_list_index(&mut self) {
        if self.list_index >= self.list_len() && self.list_len() > 0 {
            self.list_index = self.list_len() - 1;
        } else if self.list_len() == 0 {
            self.list_index = 0;
        }

        if !self.is_selectable(self.list_index) {
            self.list_index_at_first();
        }
    }

    fn is_selectable(&self, idx: usize) -> bool {
        if idx >= self.list_len() {
            return false;
        }
        if self.dot_dot && idx == 0 {
            return true;
        }
        let row_idx = if self.dot_dot {
            idx.saturating_sub(1)
        } else {
            idx
        };
        self.row_indices.get(row_idx).is_some_and(Option::is_some)
    }

    fn next_selectable_down(&self, can_rewind: bool) -> Option<usize> {
        if self.list_len() == 0 {
            return None;
        }
        let mut idx = self.list_index;
        for _ in 0..self.list_len() {
            if idx + 1 < self.list_len() {
                idx += 1;
            } else if can_rewind {
                idx = 0;
            } else {
                return None;
            }
            if self.is_selectable(idx) {
                return Some(idx);
            }
        }
        None
    }

    fn next_selectable_up(&self, can_rewind: bool) -> Option<usize> {
        if self.list_len() == 0 {
            return None;
        }
        let mut idx = self.list_index;
        for _ in 0..self.list_len() {
            if idx > 0 {
                idx -= 1;
            } else if can_rewind {
                idx = self.list_len() - 1;
            } else {
                return None;
            }
            if self.is_selectable(idx) {
                return Some(idx);
            }
        }
        None
    }
}

#[derive(Default)]
pub struct FileList {
    props: Props,
    states: OwnStates,
}

impl FileList {
    pub fn foreground(mut self, fg: Color) -> Self {
        self.attr(Attribute::Foreground, AttrValue::Color(fg));
        self
    }

    pub fn background(mut self, bg: Color) -> Self {
        self.attr(Attribute::Background, AttrValue::Color(bg));
        self
    }

    pub fn borders(mut self, b: Borders) -> Self {
        self.attr(Attribute::Borders, AttrValue::Borders(b));
        self
    }

    pub fn title(mut self, t: Title) -> Self {
        self.attr(Attribute::Title, AttrValue::Title(t));
        self
    }

    pub fn highlight_color(mut self, c: Color) -> Self {
        self.attr(
            Attribute::HighlightStyle,
            AttrValue::Style(Style::default().fg(c).add_modifier(TextModifiers::REVERSED)),
        );
        self
    }

    pub fn rows(mut self, rows: Table) -> Self {
        self.attr(Attribute::Content, AttrValue::Table(rows));
        self
    }

    /// If enabled, show `..` entry at the beginning of the list
    pub fn dot_dot(mut self, show: bool) -> Self {
        self.attr(Attribute::Custom(PROP_DOT_DOT), AttrValue::Flag(show));
        self
    }

    pub fn visual_index(&self) -> usize {
        self.states.list_index
    }

    /// Returns the value of the `dot_dot` property
    fn has_dot_dot(&self) -> bool {
        self.props
            .get(Attribute::Custom(PROP_DOT_DOT))
            .and_then(AttrValue::as_flag)
            .unwrap_or(false)
    }
}

impl Component for FileList {
    fn view(
        &mut self,
        frame: &mut tuirealm::ratatui::Frame,
        area: tuirealm::ratatui::layout::Rect,
    ) {
        let title = self
            .props
            .get(Attribute::Title)
            .and_then(AttrValue::as_title)
            .cloned()
            .unwrap_or_else(|| Title::from(String::default()).alignment(HorizontalAlignment::Left));
        let borders = self
            .props
            .get(Attribute::Borders)
            .and_then(AttrValue::as_borders)
            .unwrap_or_default();
        let focus = self
            .props
            .get(Attribute::Focus)
            .and_then(AttrValue::as_flag)
            .unwrap_or(false);
        let div = tui_realm_stdlib::utils::get_block(borders, Some(&title), focus, None);
        // Make list entries
        let init_table_iter = if self.has_dot_dot() {
            vec![vec![tuirealm::props::LineStatic::from(SpanStatic::from(
                "..",
            ))]]
        } else {
            vec![]
        };

        let foreground = self
            .props
            .get(Attribute::Foreground)
            .and_then(AttrValue::as_color)
            .unwrap_or(Color::Reset);
        let background = self
            .props
            .get(Attribute::Background)
            .and_then(AttrValue::as_color)
            .unwrap_or(Color::Reset);
        let list_items: Vec<ListItem> = match self
            .props
            .get(Attribute::Content)
            .and_then(AttrValue::as_table)
        {
            Some(table) => init_table_iter
                .iter()
                .chain(table.iter())
                .map(|row| {
                    let columns: Vec<Span> = row
                        .iter()
                        .flat_map(|line| line.spans.iter())
                        .map(|span| {
                            let mut style = span.style;
                            if style.fg.is_none() {
                                style.fg = Some(foreground);
                            }
                            if style.bg.is_none() {
                                style.bg = Some(background);
                            }
                            Span::styled(span.content.clone(), style)
                        })
                        .collect();
                    ListItem::new(Line::from(columns))
                })
                .collect(),
            _ => Vec::new(),
        };
        let highlight_style = self
            .props
            .get(Attribute::HighlightStyle)
            .and_then(AttrValue::as_style);
        // Make list
        let mut list = TuiList::new(list_items)
            .block(div)
            .direction(ListDirection::TopToBottom);
        if let Some(style) = highlight_style {
            let applied = if focus {
                style
            } else {
                // On blur keep the highlight color on the foreground but drop the
                // REVERSED modifier so the selected row just stands out via fg only.
                let mut s = Style::default();
                if let Some(fg) = style.fg {
                    s = s.fg(fg);
                }
                s
            };
            list = list.highlight_style(applied);
        }
        let mut state: ListState = ListState::default();
        state.select(Some(self.states.list_index));
        frame.render_stateful_widget(list, area, &mut state);
    }

    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        self.props.set(attr, value.clone());
        match attr {
            Attribute::Content => {
                let len = self
                    .props
                    .get(Attribute::Content)
                    .and_then(AttrValue::as_table)
                    .map(std::vec::Vec::len)
                    .unwrap_or(0);
                self.states.init_list_states(len, self.has_dot_dot());
                self.states.fix_list_index();
            }
            Attribute::Custom(PROP_ROW_INDICES) => {
                let row_indices = value
                    .unwrap_payload()
                    .unwrap_vec()
                    .into_iter()
                    .map(|value| match value {
                        PropValue::Isize(idx) if idx >= 0 => Some(idx as usize),
                        _ => None,
                    })
                    .collect();
                self.states.set_row_indices(row_indices);
            }
            Attribute::Custom(PROP_VISUAL_INDEX) => {
                let index = value.unwrap_payload().unwrap_single().unwrap_usize();
                self.states.list_index = index.min(self.states.list_len().saturating_sub(1));
            }
            Attribute::Focus => {
                if value.unwrap_flag() && !self.states.is_selectable(self.states.list_index) {
                    self.states.fix_list_index();
                }
            }
            _ => {}
        }
    }

    fn query<'a>(&'a self, attr: Attribute) -> Option<QueryResult<'a>> {
        self.props.get_for_query(attr)
    }

    fn state(&self) -> State {
        if self.has_dot_dot() && self.states.list_index == 0 {
            return State::Single(StateValue::String("..".to_string()));
        }

        State::Single(StateValue::Usize(self.states.real_index().unwrap_or(0)))
    }

    fn perform(&mut self, cmd: Cmd) -> CmdResult {
        match cmd {
            Cmd::Move(Direction::Down) => {
                let prev = self.states.list_index;
                self.states.incr_list_index(true);
                if prev != self.states.list_index {
                    CmdResult::Changed(self.state())
                } else {
                    CmdResult::NoChange
                }
            }
            Cmd::Move(Direction::Up) => {
                let prev = self.states.list_index;
                self.states.decr_list_index(true);
                if prev != self.states.list_index {
                    CmdResult::Changed(self.state())
                } else {
                    CmdResult::NoChange
                }
            }
            Cmd::Scroll(Direction::Down) => {
                let prev = self.states.list_index;
                (0..8).for_each(|_| self.states.incr_list_index(false));
                if prev != self.states.list_index {
                    CmdResult::Changed(self.state())
                } else {
                    CmdResult::NoChange
                }
            }
            Cmd::Scroll(Direction::Up) => {
                let prev = self.states.list_index;
                (0..8).for_each(|_| self.states.decr_list_index(false));
                if prev != self.states.list_index {
                    CmdResult::Changed(self.state())
                } else {
                    CmdResult::NoChange
                }
            }
            Cmd::GoTo(Position::Begin) => {
                let prev = self.states.list_index;
                self.states.list_index_at_first();
                if prev != self.states.list_index {
                    CmdResult::Changed(self.state())
                } else {
                    CmdResult::NoChange
                }
            }
            Cmd::GoTo(Position::End) => {
                let prev = self.states.list_index;
                self.states.list_index_at_last();
                if prev != self.states.list_index {
                    CmdResult::Changed(self.state())
                } else {
                    CmdResult::NoChange
                }
            }
            Cmd::Toggle => {
                if self.states.list_index == 0 && self.has_dot_dot() {
                    return CmdResult::NoChange;
                }

                let Some(index) = self.states.real_index() else {
                    return CmdResult::NoChange;
                };
                self.states.incr_list_index(false);
                CmdResult::Changed(State::Single(StateValue::Usize(index)))
            }
            _ => CmdResult::NoChange,
        }
    }
}
