use jiff::ToSpan;
use jiff::civil::{Date, Time};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, Clear, Padding, Paragraph, Row as TableRow, Table, TableState, Wrap,
};

use super::state::{ACTS, Board, Overlay, Pane, Row, Scope, local_start};
use crate::cli::output::{clean, keymap_help};
use crate::domain::keymap::{Action, Keymap, State as InputState};
use crate::domain::{Event, NewReminder, Priority, reminder_due_is_overdue};
use crate::tui::state::{Item, PRIORITIES};

const WIDE: u16 = 100;
const MEDIUM: u16 = 64;
const ROOMY: u16 = 130;
const LISTS_WIDTH: u16 = 22;
const DETAILS_WIDTH: u16 = 34;
const ROOMY_DETAILS_WIDTH: u16 = 42;
const LIST_COLUMN: u16 = 14;
const DUE_COLUMN: u16 = 16;

const MUTED: Style = Style::new().add_modifier(Modifier::DIM);
const STRONG: Style = Style::new().add_modifier(Modifier::BOLD);
const HIGHLIGHT: Style = Style::new().add_modifier(Modifier::REVERSED);

#[derive(Debug, Clone, Copy)]
struct Theme {
    color: bool,
}

impl Theme {
    fn paint(self, color: Color) -> Style {
        if self.color {
            Style::new().fg(color)
        } else {
            Style::new()
        }
    }

    fn badge(self, color: Color) -> Style {
        if self.color {
            Style::new()
                .bg(color)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD)
        } else {
            STRONG.add_modifier(Modifier::REVERSED)
        }
    }
}

pub(crate) fn draw(frame: &mut Frame<'_>, board: &Board, color: bool) {
    let theme = Theme { color };
    let [header, input, info, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    draw_header(frame, header, board, theme);
    draw_input(frame, input, board, theme);
    draw_info(frame, info, board, theme);
    draw_body(frame, body, board, theme);
    draw_footer(frame, footer, board, theme);
    match board.overlay() {
        Some(Overlay::Help) => draw_help(frame, body, board),
        Some(Overlay::ConfirmDelete) => draw_confirm_delete(frame, body, board, theme),
        Some(overlay) => draw_menu(frame, body, board, overlay, theme),
        None => {}
    }
}

fn draw_header(frame: &mut Frame<'_>, area: Rect, board: &Board, theme: Theme) {
    let (label, tone, hint) = match board.mode() {
        InputState::Browse => ("BROWSE", Color::Blue, "letters are actions"),
        InputState::Filter => ("FILTER", Color::Yellow, "typing filters"),
        InputState::Capture => ("CAPTURE", Color::Magenta, "typing adds a reminder"),
    };
    let date = board.now().strftime("%a %-d %b %Y").to_string();
    let [left, right] = Layout::horizontal([
        Constraint::Min(1),
        Constraint::Length(width(date.chars().count() + 1)),
    ])
    .areas(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("nib ", STRONG),
            Span::styled(format!(" {label} "), theme.badge(tone)),
            Span::styled(format!("  {hint}"), MUTED),
        ])),
        left,
    );
    frame.render_widget(
        Paragraph::new(Span::styled(date, MUTED)).right_aligned(),
        right,
    );
}

fn draw_input(frame: &mut Frame<'_>, area: Rect, board: &Board, theme: Theme) {
    let matches = board.match_count();
    let mut counts = format!("{matches} result{}", if matches == 1 { "" } else { "s" });
    if !board.selected().is_empty() {
        counts = format!("{counts} · {} selected", board.selected().len());
    }
    let (glyph, tone, text, caret, right) = match board.mode() {
        InputState::Browse => {
            let applied = clean(board.applied());
            let text = if applied.is_empty() {
                Span::styled("press / to filter", MUTED)
            } else {
                Span::raw(applied)
            };
            ("/", Color::Blue, text, None, counts)
        }
        InputState::Filter => (
            "/",
            Color::Yellow,
            Span::raw(clean(&board.query().text)),
            Some(board.query().caret),
            counts,
        ),
        InputState::Capture => (
            "+",
            Color::Magenta,
            Span::raw(clean(&board.capture().text)),
            Some(board.capture().caret),
            format!("adds to {}", capture_list(board)),
        ),
    };
    let [left, right_area] = Layout::horizontal([
        Constraint::Min(1),
        Constraint::Length(width(right.chars().count() + 1)),
    ])
    .areas(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{glyph} "),
                theme.paint(tone).add_modifier(Modifier::BOLD),
            ),
            text.clone(),
        ])),
        left,
    );
    frame.render_widget(
        Paragraph::new(Span::styled(right, MUTED)).right_aligned(),
        right_area,
    );
    if let Some(caret) = caret {
        let before: String = text.content.chars().take(caret).collect();
        let offset = width(Line::raw(before).width());
        let x = left
            .x
            .saturating_add(2)
            .saturating_add(offset)
            .min(left.right().saturating_sub(1));
        frame.set_cursor_position((x, left.y));
    }
}

fn capture_list(board: &Board) -> String {
    match board.capture_preview() {
        Some(Ok(NewReminder {
            list: Some(list), ..
        })) => clean(list),
        _ => board
            .config()
            .task
            .list
            .as_deref()
            .map_or_else(|| "the default list".to_owned(), clean),
    }
}

fn draw_info(frame: &mut Frame<'_>, area: Rect, board: &Board, theme: Theme) {
    let line = if board.mode() == InputState::Capture && board.status().is_none() {
        capture_preview(board, theme)
    } else if let Some(error) = board.filter_error() {
        Line::styled(format!("  {}", clean(error)), theme.paint(Color::Red))
    } else if let Some(status) = board.status() {
        let tone = if status.error {
            Color::Red
        } else {
            Color::Green
        };
        Line::styled(format!("  {}", clean(&status.text)), theme.paint(tone))
    } else {
        Line::styled("─".repeat(usize::from(area.width)), MUTED)
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn capture_preview(board: &Board, theme: Theme) -> Line<'static> {
    match board.capture_preview() {
        None => Line::styled(
            "  title, then a due date, !low|!medium|!high, and #list, in any order",
            MUTED,
        ),
        Some(Err(error)) => Line::styled(format!("  {}", clean(error)), theme.paint(Color::Red)),
        Some(Ok(reminder)) => {
            let mut spans = vec![
                Span::raw("  "),
                Span::styled(clean(&reminder.title), STRONG),
            ];
            if let Some(date) = reminder.due {
                let mut due = date.to_string();
                if let Some(time) = reminder.due_time {
                    due.push(' ');
                    due.push_str(&time.strftime("%H:%M").to_string());
                }
                spans.push(Span::styled(
                    format!("  due {due}"),
                    theme.paint(Color::Yellow),
                ));
            }
            if let Some(priority) = reminder.priority {
                spans.push(Span::styled(
                    format!("  priority {}", priority.as_str()),
                    theme.paint(Color::Red),
                ));
            }
            spans.push(Span::styled(
                format!("  list {}", capture_list(board)),
                theme.paint(Color::Magenta),
            ));
            Line::from(spans)
        }
    }
}

fn draw_body(frame: &mut Frame<'_>, area: Rect, board: &Board, theme: Theme) {
    let show_details = board.preview() && area.width >= WIDE;
    if area.width >= MEDIUM {
        let mut constraints = vec![Constraint::Length(LISTS_WIDTH), Constraint::Min(1)];
        if show_details {
            constraints.push(Constraint::Length(if area.width >= ROOMY {
                ROOMY_DETAILS_WIDTH
            } else {
                DETAILS_WIDTH
            }));
        }
        let areas = Layout::horizontal(constraints).split(area);
        draw_lists(frame, areas[0], board, theme);
        draw_items(frame, areas[1], board, theme);
        if show_details {
            draw_details(frame, areas[2], board, theme);
        }
    } else if board.focus() == Pane::Lists {
        draw_lists(frame, area, board, theme);
    } else {
        draw_items(frame, area, board, theme);
    }
}

fn pane_block(title: String, focused: bool, theme: Theme) -> Block<'static> {
    let style = if focused {
        theme.paint(Color::Blue).add_modifier(Modifier::BOLD)
    } else {
        MUTED
    };
    Block::new()
        .borders(Borders::ALL)
        .border_style(if focused {
            theme.paint(Color::Blue)
        } else {
            MUTED
        })
        .title(Span::styled(title, style))
}

fn draw_lists(frame: &mut Frame<'_>, area: Rect, board: &Board, theme: Theme) {
    let focused = board.focus() == Pane::Lists && board.overlay().is_none();
    let block = pane_block(" Lists ".into(), focused, theme);
    let scopes = board.scopes();
    let mut rows = Vec::new();
    let mut selected = board.scope_cursor();
    for (index, scope) in scopes.iter().enumerate() {
        if matches!(scope, Scope::View(0)) {
            rows.push(TableRow::new([Cell::from(Span::styled("views", MUTED))]));
            if index <= selected {
                selected += 1;
            }
        }
        let name = clean(&board.scope_name(scope));
        let count = board.scope_count(scope).to_string();
        let style = match scope {
            Scope::Events => theme.paint(Color::Magenta),
            Scope::View(_) => theme.paint(Color::Cyan),
            _ => Style::new(),
        };
        rows.push(TableRow::new([
            Cell::from(Span::styled(name, style)),
            Cell::from(Line::styled(count, MUTED).right_aligned()),
        ]));
    }
    let table = Table::new(rows, [Constraint::Min(1), Constraint::Length(4)])
        .block(block)
        .row_highlight_style(if focused { HIGHLIGHT } else { STRONG });
    let mut state = TableState::new().with_selected(Some(selected));
    frame.render_stateful_widget(table, area, &mut state);
}

fn draw_items(frame: &mut Frame<'_>, area: Rect, board: &Board, theme: Theme) {
    let focused = board.focus() == Pane::Items && board.overlay().is_none();
    let title = format!(
        " {} · {} ",
        clean(&board.scope_name(&board.current_scope())),
        board.match_count()
    );
    let block = pane_block(title, focused, theme);
    let rows = board.rows();
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(" Nothing matches.", MUTED)).block(block),
            area,
        );
        return;
    }
    let today = board.now().date();
    let mut list_width = 0;
    let mut due_width = 0;
    let table_rows: Vec<TableRow<'_>> = rows
        .iter()
        .map(|row| {
            let cells = match row {
                Row::Reminder(item) => reminder_cells(item, board, theme),
                Row::Event(event) => event_cells(event, board, today, theme),
                Row::Add(text) => [
                    Span::raw(" "),
                    Span::styled("+", theme.paint(Color::Cyan)),
                    Span::styled(
                        format!("Add reminder: {}", clean(text)),
                        theme.paint(Color::Cyan),
                    ),
                    Span::raw(""),
                    Span::styled("enter", MUTED),
                    Span::raw(""),
                ],
            };
            list_width = list_width.max(cells[3].width());
            due_width = due_width.max(cells[4].width());
            TableRow::new(cells.map(Cell::from))
        })
        .collect();
    let capped = |value: usize, cap: u16| width(value).min(cap);
    let table = Table::new(
        table_rows,
        [
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(capped(list_width, LIST_COLUMN)),
            Constraint::Length(capped(due_width, DUE_COLUMN)),
            Constraint::Length(3),
        ],
    )
    .block(block)
    .column_spacing(1)
    .row_highlight_style(if focused || board.mode() != InputState::Browse {
        HIGHLIGHT
    } else {
        STRONG
    });
    let mut state = TableState::new().with_selected(Some(board.cursor()));
    frame.render_stateful_widget(table, area, &mut state);
}

fn reminder_cells(item: &Item, board: &Board, theme: Theme) -> [Span<'static>; 6] {
    let marker = if board.is_selected(&item.id) {
        "+"
    } else {
        " "
    };
    let (check, title) = if item.completed {
        ("✓", MUTED.add_modifier(Modifier::CROSSED_OUT))
    } else {
        (" ", Style::new())
    };
    [
        Span::styled(
            marker,
            theme.paint(Color::Yellow).add_modifier(Modifier::BOLD),
        ),
        Span::styled(check, MUTED),
        Span::styled(clean(&item.title), title),
        Span::styled(
            clean(item.list.as_deref().unwrap_or_default()),
            theme.paint(Color::Magenta),
        ),
        reminder_due(item, board, theme),
        priority_span(item.priority, theme),
    ]
}

fn event_cells(event: &Event, board: &Board, today: Date, theme: Theme) -> [Span<'static>; 6] {
    [
        Span::raw(" "),
        Span::styled("◆", theme.paint(Color::Magenta)),
        Span::raw(clean(&event.title)),
        Span::styled(clean(&event.calendar), theme.paint(Color::Magenta)),
        Span::styled(event_when(event, board, today), MUTED),
        Span::raw(""),
    ]
}

fn event_when(event: &Event, board: &Board, today: Date) -> String {
    if event.all_day {
        day_label(event.start.date(), today)
    } else {
        let start = local_start(event, board.now());
        format!(
            "{} {}",
            day_label(start.date(), today),
            start.time().strftime("%H:%M")
        )
    }
}

fn event_details_when(event: &Event, board: &Board) -> String {
    if event.all_day {
        format!("{} all day", event.start.date())
    } else {
        let start = local_start(event, board.now());
        format!("{} {}", start.date(), start.time().strftime("%H:%M"))
    }
}

fn day_label(date: Date, today: Date) -> String {
    if date == today {
        "today".into()
    } else if today.checked_add(1.day()).ok() == Some(date) {
        "tomorrow".into()
    } else if today.checked_sub(1.day()).ok() == Some(date) {
        "yesterday".into()
    } else if date.year() == today.year() {
        date.strftime("%b %-d").to_string()
    } else {
        date.to_string()
    }
}

fn due_text(date: Date, time: Option<Time>, today: Date) -> String {
    let mut label = day_label(date, today);
    if let Some(time) = time {
        label.push(' ');
        label.push_str(&time.strftime("%H:%M").to_string());
    }
    label
}

fn reminder_due(item: &Item, board: &Board, theme: Theme) -> Span<'static> {
    let Some(date) = item.due else {
        return Span::raw("");
    };
    let today = board.now().date();
    let style = if !item.completed
        && reminder_due_is_overdue(item.due, item.due_time, item.due_at, board.now())
    {
        theme.paint(Color::Red)
    } else if date == today {
        theme.paint(Color::Yellow)
    } else {
        MUTED
    };
    Span::styled(due_text(date, item.due_time, today), style)
}

fn priority_span(priority: Option<Priority>, theme: Theme) -> Span<'static> {
    match priority {
        Some(Priority::High) => Span::styled("!!!", theme.paint(Color::Red)),
        Some(Priority::Medium) => Span::styled("!!", theme.paint(Color::Yellow)),
        Some(Priority::Low) => Span::styled("!", theme.paint(Color::Cyan)),
        None => Span::raw(""),
    }
}

fn draw_details(frame: &mut Frame<'_>, area: Rect, board: &Board, theme: Theme) {
    let block = pane_block(" Details ".into(), false, theme).padding(Padding::horizontal(1));
    let lines = match board.current() {
        Some(Row::Reminder(item)) => reminder_details(item, board, theme),
        Some(Row::Event(event)) => event_details(event, board, theme),
        Some(Row::Add(text)) => vec![
            Line::styled("New reminder", STRONG),
            Line::raw(clean(text)),
            Line::raw(""),
            Line::styled(
                "Parsed like quick-add: a trailing due date, !priority, and #list. Press enter to add it.",
                MUTED,
            ),
        ],
        None => vec![Line::styled("Nothing matches the filter.", MUTED)],
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn field(name: &str, value: Span<'static>) -> Line<'static> {
    Line::from(vec![Span::styled(format!("{name:<9}"), MUTED), value])
}

fn notes_lines(lines: &mut Vec<Line<'static>>, notes: Option<&str>) {
    if let Some(notes) = notes.filter(|notes| !notes.trim().is_empty()) {
        lines.push(Line::raw(""));
        lines.extend(notes.lines().map(|line| Line::raw(clean(line))));
    }
}

fn reminder_details(item: &Item, board: &Board, theme: Theme) -> Vec<Line<'static>> {
    let mut lines = vec![Line::styled(clean(&item.title), STRONG)];
    if let Some(list) = &item.list {
        lines.push(field(
            "list",
            Span::styled(clean(list), theme.paint(Color::Magenta)),
        ));
    }
    let due = if let Some(date) = item.due {
        let mut due = reminder_due(item, board, theme);
        if due.content != date.to_string() {
            due.content = format!("{} ({date})", due.content).into();
        }
        due
    } else {
        Span::styled("none", MUTED)
    };
    lines.push(field("due", due));
    lines.push(field(
        "priority",
        item.priority.map_or_else(
            || Span::styled("none", MUTED),
            |priority| Span::raw(priority.as_str()),
        ),
    ));
    if item.completed {
        lines.push(field("status", Span::raw("completed")));
    }
    lines.push(field("alarms", yes_no(item.extras.alarms)));
    lines.push(field("repeats", yes_no(item.extras.repeats)));
    if let Some(url) = &item.url {
        lines.push(field(
            "link",
            Span::styled(clean(url), theme.paint(Color::Cyan)),
        ));
    }
    notes_lines(&mut lines, item.notes.as_deref());
    lines
}

fn yes_no(value: bool) -> Span<'static> {
    if value {
        Span::raw("yes")
    } else {
        Span::styled("no", MUTED)
    }
}

fn event_details(event: &Event, board: &Board, theme: Theme) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::styled(clean(&event.title), STRONG),
        field("when", Span::raw(event_details_when(event, board))),
        field(
            "calendar",
            Span::styled(clean(&event.calendar), theme.paint(Color::Magenta)),
        ),
    ];
    if let Some(location) = &event.location {
        lines.push(field("location", Span::raw(clean(location))));
    }
    if let Some(url) = &event.meeting_url {
        lines.push(field(
            "meeting",
            Span::styled(clean(url), theme.paint(Color::Cyan)),
        ));
    }
    notes_lines(&mut lines, event.notes.as_deref());
    lines.push(Line::raw(""));
    lines.push(Line::styled("Events are read-only.", MUTED));
    lines
}

fn draw_footer(frame: &mut Frame<'_>, area: Rect, board: &Board, theme: Theme) {
    let pairs: Vec<(String, String)> = match board.overlay() {
        Some(Overlay::Help) => vec![
            ("up/down".into(), "scroll".into()),
            ("any other key".into(), "close".into()),
        ],
        Some(Overlay::ConfirmDelete) => vec![("esc".into(), "keep".into())],
        Some(_) => vec![
            ("enter".into(), "choose".into()),
            ("esc".into(), "back".into()),
        ],
        None => footer_hints(&board.config().tui.keymap, board.mode()),
    };
    let mut spans = Vec::new();
    for (key, label) in pairs {
        if !spans.is_empty() {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(key, theme.paint(Color::Cyan)));
        spans.push(Span::styled(format!(" {label}"), MUTED));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

pub(crate) fn footer_hints(keymap: &Keymap, state: InputState) -> Vec<(String, String)> {
    let hints: &[(Action, &str)] = match state {
        InputState::Browse => &[
            (Action::Filter, "filter"),
            (Action::Capture, "add"),
            (Action::SelectToggle, "select"),
            (Action::Done, "done"),
            (Action::Edit, "edit"),
            (Action::Delete, "delete"),
            (Action::ActMenu, "act"),
            (Action::NextPane, "pane"),
            (Action::Help, "help"),
            (Action::Quit, "quit"),
        ],
        InputState::Filter => &[
            (Action::Accept, "keep"),
            (Action::Cancel, "clear"),
            (Action::SelectToggle, "select"),
            (Action::ActMenu, "act"),
            (Action::Help, "help"),
        ],
        InputState::Capture => &[
            (Action::Accept, "add"),
            (Action::Cancel, "cancel"),
            (Action::Help, "help"),
        ],
    };
    hints
        .iter()
        .filter_map(|(action, label)| {
            keymap
                .bindings(state)
                .find(|(_, bound)| *bound == [*action])
                .map(|(chord, _)| (chord.to_string(), (*label).to_owned()))
        })
        .collect()
}

fn popup(area: Rect, wanted_width: u16, wanted_height: u16) -> Rect {
    let width = wanted_width.min(area.width.saturating_sub(2)).max(1);
    let height = wanted_height.min(area.height).max(1);
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

fn overlay_block(title: String, theme: Theme) -> Block<'static> {
    Block::new()
        .borders(Borders::ALL)
        .border_style(theme.paint(Color::Blue))
        .padding(Padding::horizontal(1))
        .title(Span::styled(title, STRONG))
}

fn draw_help(frame: &mut Frame<'_>, area: Rect, board: &Board) {
    let state = board.mode();
    let help = keymap_help(&board.config().tui.keymap, state);
    let mut lines: Vec<Line<'static>> = help
        .lines()
        .map(|line| Line::raw(line.to_owned()))
        .collect();
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "In menus: letters or digits choose, up/down and enter pick, esc goes back.",
        MUTED,
    ));
    let target = popup(area, 76, width(lines.len() + 2));
    let visible = usize::from(target.height.saturating_sub(2));
    let scroll = board.help_scroll().min(lines.len().saturating_sub(visible));
    frame.render_widget(Clear, target);
    frame.render_widget(
        Paragraph::new(lines)
            .scroll((width(scroll), 0))
            .block(overlay_block(
                format!(" Keys in {} ", state.as_str().to_uppercase()),
                Theme { color: false },
            )),
        target,
    );
}

fn target_name(board: &Board) -> String {
    match board.targets() {
        [only] => board
            .item(only)
            .map_or_else(|| clean(only), |item| format!("\"{}\"", clean(&item.title))),
        many => format!("{} reminders", many.len()),
    }
}

fn hidden_line(board: &Board, theme: Theme) -> Option<Line<'static>> {
    let hidden = board.hidden_target_count();
    (hidden > 0).then(|| {
        Line::styled(
            format!("{hidden} selected not shown by the current list or filter."),
            theme.paint(Color::Yellow),
        )
    })
}

fn draw_menu(frame: &mut Frame<'_>, area: Rect, board: &Board, overlay: Overlay, theme: Theme) {
    let what = target_name(board);
    let (title, options): (String, Vec<(String, String)>) = match overlay {
        Overlay::Priority => (
            format!(" Priority for {what} "),
            PRIORITIES
                .iter()
                .map(|(key, name, _)| (key.to_string(), (*name).to_owned()))
                .collect(),
        ),
        Overlay::Move => (
            format!(" Move {what} to "),
            board
                .lists()
                .iter()
                .enumerate()
                .map(|(index, list)| {
                    let key = if index < 9 {
                        (index + 1).to_string()
                    } else {
                        " ".to_owned()
                    };
                    (key, clean(&list.name))
                })
                .collect(),
        ),
        _ => (
            format!(" Act on {what} "),
            ACTS.iter()
                .map(|(key, name)| (key.to_string(), (*name).to_owned()))
                .collect(),
        ),
    };
    let hidden = hidden_line(board, theme);
    let extra = usize::from(hidden.is_some());
    let target = popup(area, 48, width(options.len() + extra + 2));
    let inner_height = usize::from(target.height.saturating_sub(2));
    let room = inner_height.saturating_sub(extra).max(1);
    let skip = board.menu_cursor().saturating_sub(room - 1);
    let mut lines: Vec<Line<'static>> = hidden.into_iter().collect();
    lines.extend(
        options
            .into_iter()
            .enumerate()
            .skip(skip)
            .map(|(index, (key, name))| {
                let line = Line::from(vec![
                    Span::styled(
                        format!("{key}  "),
                        theme.paint(Color::Cyan).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(name),
                ]);
                if index == board.menu_cursor() {
                    line.style(HIGHLIGHT)
                } else {
                    line
                }
            }),
    );
    frame.render_widget(Clear, target);
    frame.render_widget(
        Paragraph::new(lines).block(overlay_block(title, theme)),
        target,
    );
}

fn draw_confirm_delete(frame: &mut Frame<'_>, area: Rect, board: &Board, theme: Theme) {
    let key = |text: &str| {
        Span::styled(
            format!("{text}  "),
            theme.paint(Color::Cyan).add_modifier(Modifier::BOLD),
        )
    };
    let mut lines = Vec::new();
    if board.delete_needs_count() {
        lines.push(Line::from(vec![
            Span::raw(format!("Type {} and press enter: ", board.targets().len())),
            Span::styled(board.confirm_input().to_owned(), STRONG),
        ]));
    } else {
        lines.push(Line::from(vec![
            key("y"),
            Span::raw("delete  "),
            key("n"),
            Span::raw("keep"),
        ]));
    }
    if let Some(hidden) = hidden_line(board, theme) {
        lines.push(hidden);
    }
    let partial = board.partial_target_count();
    if partial > 0 {
        let has = if partial == 1 { "has" } else { "have" };
        lines.push(Line::styled(
            format!("{partial} {has} alarms, repeats, or links that undo can't bring back."),
            theme.paint(Color::Yellow),
        ));
    }
    lines.push(Line::styled(
        "Undo re-creates deleted reminders with new ids.",
        MUTED,
    ));
    let inner = usize::from(popup(area, 56, 1).width.saturating_sub(4)).max(1);
    let rows: usize = lines
        .iter()
        .map(|line| line.width().max(1).div_ceil(inner))
        .sum();
    let target = popup(area, 56, width(rows + 2));
    frame.render_widget(Clear, target);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(overlay_block(
                format!(" Delete {}? ", target_name(board)),
                theme,
            )),
        target,
    );
}

fn width(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::domain::Config;
    use crate::tui::board::state::tests::board_with;
    use crate::tui::board::state::{Input, MenuKey, TextEdit};
    use crate::tui::state::tests::data;

    fn screen(board: &Board, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame, board, false)).unwrap();
        terminal.backend().to_string()
    }

    fn board() -> Board {
        let mut items = data();
        items.items[0].notes = Some("2% or oat".into());
        items.items[0].url = Some("https://example.com/milk".into());
        items.items[0].extras.alarms = true;
        board_with(items, Config::default())
    }

    fn act(board: &mut Board, action: Action) {
        board.update(Input::Action(action));
    }

    fn type_text(board: &mut Board, text: &str) {
        for character in text.chars() {
            board.update(Input::Text(TextEdit::Insert(character)));
        }
    }

    #[test]
    fn browse_shows_lists_items_and_details() {
        let mut board = board();
        act(&mut board, Action::SelectToggle);
        assert_snapshot!(screen(&board, 110, 12), @r#"
        "nib  BROWSE   letters are actions                                                              Fri 30 Oct 2026"
        "/ press / to filter                                                                     5 results · 1 selected"
        "──────────────────────────────────────────────────────────────────────────────────────────────────────────────"
        "┌ Lists ─────────────┐┌ All · 5 ───────────────────────────────────────────┐┌ Details ───────────────────────┐"
        "│All                5││+   Buy oat milk         Home     today          !!!││ Buy oat milk                   │"
        "│Today              2││    Buy eggs             Work                       ││ list     Home                  │"
        "│Home               1││    File report          Work     tomorrow       !! ││ due      today (2026-10-30)    │"
        "│Work               2││  ◆ Standup              Work     today 09:00       ││ priority high                  │"
        "│Events             2││  ◆ Lunch with Sam       Personal tomorrow 12:00    ││ alarms   yes                   │"
        "│                    ││                                                    ││ repeats  no                    │"
        "└────────────────────┘└────────────────────────────────────────────────────┘└────────────────────────────────┘"
        "/ filter  a add  tab select  x done  e edit  delete delete  ctrl-o act  ] pane  ? help  q quit                "
        
        "#);
    }

    #[test]
    fn filter_mode_narrows_the_items_and_offers_an_add_row() {
        let mut board = board();
        act(&mut board, Action::Filter);
        type_text(&mut board, "buy");
        assert_snapshot!(screen(&board, 110, 9), @r#"
        "nib  FILTER   typing filters                                                                   Fri 30 Oct 2026"
        "/ buy                                                                                                2 results"
        "──────────────────────────────────────────────────────────────────────────────────────────────────────────────"
        "┌ Lists ─────────────┐┌ All · 2 ───────────────────────────────────────────┐┌ Details ───────────────────────┐"
        "│All                5││    Buy oat milk                      Home today !!!││ Buy oat milk                   │"
        "│Today              2││    Buy eggs                          Work          ││ list     Home                  │"
        "│Home               1││  + Add reminder: buy                      enter    ││ due      today (2026-10-30)    │"
        "└────────────────────┘└────────────────────────────────────────────────────┘└────────────────────────────────┘"
        "enter keep  esc clear  tab select  ctrl-o act  f1 help                                                        "
        
        "#);
    }

    #[test]
    fn capture_mode_previews_the_parsed_reminder() {
        let mut board = board();
        act(&mut board, Action::Capture);
        type_text(&mut board, "Pay rent tomorrow !high #Work");
        assert_snapshot!(screen(&board, 110, 8), @r#"
        "nib  CAPTURE   typing adds a reminder                                                          Fri 30 Oct 2026"
        "+ Pay rent tomorrow !high #Work                                                                   adds to Work"
        "  Pay rent  due 2026-10-31  priority high  list Work                                                          "
        "┌ Lists ─────────────┐┌ All · 5 ───────────────────────────────────────────┐┌ Details ───────────────────────┐"
        "│All                5││    Buy oat milk         Home     today          !!!││ Buy oat milk                   │"
        "│Today              2││    Buy eggs             Work                       ││ list     Home                  │"
        "└────────────────────┘└────────────────────────────────────────────────────┘└────────────────────────────────┘"
        "enter add  esc cancel  f1 help                                                                                "
        
        "#);
    }

    #[test]
    fn delete_confirmation_counts_hidden_selections() {
        let mut board = board();
        act(&mut board, Action::SelectToggle);
        act(&mut board, Action::Filter);
        type_text(&mut board, "eggs");
        act(&mut board, Action::SelectToggle);
        act(&mut board, Action::Accept);
        act(&mut board, Action::Delete);
        assert_snapshot!(screen(&board, 110, 12), @r#"
        "nib  BROWSE   letters are actions                                                              Fri 30 Oct 2026"
        "/ eggs                                                                                   1 result · 2 selected"
        "──────────────────────────────────────────────────────────────────────────────────────────────────────────────"
        "┌ Lists ─────────────┐┌ All · 1 ───────────────────────────────────────────┐┌ Details ───────────────────────┐"
        "│All                5││+   ┌ Delete 2 reminders? ─────────────────────────────────┐ggs                       │"
        "│Today              2││    │ y  delete  n  keep                                   │    Work                  │"
        "│Home               1││    │ 1 selected not shown by the current list or filter.  │    none                  │"
        "│Work               2││    │ Undo re-creates deleted reminders with new ids.      │ity none                  │"
        "│Events             2││    └──────────────────────────────────────────────────────┘s   no                    │"
        "│                    ││                                                    ││ repeats  no                    │"
        "└────────────────────┘└────────────────────────────────────────────────────┘└────────────────────────────────┘"
        "esc keep                                                                                                      "
        
        "#);
    }

    #[test]
    fn event_details_show_the_meeting_link_and_notes() {
        let mut board = board();
        act(&mut board, Action::Bottom);
        act(&mut board, Action::Up);
        assert_snapshot!(screen(&board, 110, 13), @r#"
        "nib  BROWSE   letters are actions                                                              Fri 30 Oct 2026"
        "/ press / to filter                                                                                  5 results"
        "──────────────────────────────────────────────────────────────────────────────────────────────────────────────"
        "┌ Lists ─────────────┐┌ All · 5 ───────────────────────────────────────────┐┌ Details ───────────────────────┐"
        "│All                5││    Buy oat milk         Home     today          !!!││ Standup                        │"
        "│Today              2││    Buy eggs             Work                       ││ when     2026-10-30 09:00      │"
        "│Home               1││    File report          Work     tomorrow       !! ││ calendar Work                  │"
        "│Work               2││  ◆ Standup              Work     today 09:00       ││ meeting  https://zoom.us/j/1   │"
        "│Events             2││  ◆ Lunch with Sam       Personal tomorrow 12:00    ││                                │"
        "│                    ││                                                    ││ Daily check-in.                │"
        "│                    ││                                                    ││                                │"
        "└────────────────────┘└────────────────────────────────────────────────────┘└────────────────────────────────┘"
        "/ filter  a add  tab select  x done  e edit  delete delete  ctrl-o act  ] pane  ? help  q quit                "
        
        "#);
    }

    #[test]
    fn medium_terminals_drop_the_details_pane() {
        let board = board();
        assert_snapshot!(screen(&board, 80, 8), @r#"
        "nib  BROWSE   letters are actions                                Fri 30 Oct 2026"
        "/ press / to filter                                                    5 results"
        "────────────────────────────────────────────────────────────────────────────────"
        "┌ Lists ─────────────┐┌ All · 5 ───────────────────────────────────────────────┐"
        "│All                5││    Buy oat milk             Home     today          !!!│"
        "│Today              2││    Buy eggs                 Work                       │"
        "└────────────────────┘└────────────────────────────────────────────────────────┘"
        "/ filter  a add  tab select  x done  e edit  delete delete  ctrl-o act  ] pane  "
        
        "#);
    }

    #[test]
    fn narrow_terminals_show_only_the_focused_pane() {
        let mut board = board();
        assert_snapshot!(screen(&board, 50, 8), @r#"
        "nib  BROWSE   letters are actions  Fri 30 Oct 2026"
        "/ press / to filter                      5 results"
        "──────────────────────────────────────────────────"
        "┌ All · 5 ───────────────────────────────────────┐"
        "│    Buy oat milk     Home     today          !!!│"
        "│    Buy eggs         Work                       │"
        "└────────────────────────────────────────────────┘"
        "/ filter  a add  tab select  x done  e edit  delet"
        
        "#);
        act(&mut board, Action::PrevPane);
        assert_snapshot!(screen(&board, 50, 8), @r#"
        "nib  BROWSE   letters are actions  Fri 30 Oct 2026"
        "/ press / to filter                      5 results"
        "──────────────────────────────────────────────────"
        "┌ Lists ─────────────────────────────────────────┐"
        "│All                                            5│"
        "│Today                                          2│"
        "└────────────────────────────────────────────────┘"
        "/ filter  a add  tab select  x done  e edit  delet"
        
        "#);
    }

    #[test]
    fn help_is_generated_from_the_active_keymap() {
        let mut board = board();
        act(&mut board, Action::Capture);
        act(&mut board, Action::Help);
        assert_snapshot!(screen(&board, 90, 14), @r#"
        "nib  CAPTURE   typing adds a reminder                                      Fri 30 Oct 2026"
        "+                                                                 adds to the default list"
        "  title, then a due date, !low|!medium|!high, and #list, in any order                     "
        "┌ Lists┌ Keys in CAPTURE ─────────────────────────────────────────────────────────┐──────┐"
        "│All   │ alt-h      -> priority-high: set high priority                           │   !!!│"
        "│Today │ alt-l      -> priority-low: set low priority                             │      │"
        "│Home  │ alt-m      -> priority-medium: set medium priority                       │   !! │"
        "│Work  │ alt-n      -> priority-none: clear priority                              │      │"
        "│Events│ ctrl-c     -> cancel: cancel                                             │00    │"
        "│      │ ctrl-n     -> down: move down                                            │      │"
        "│      │ ctrl-o     -> act-menu: open the action menu                             │      │"
        "│      │ ctrl-p     -> up: move up                                                │      │"
        "└──────└──────────────────────────────────────────────────────────────────────────┘──────┘"
        "up/down scroll  any other key close                                                       "
        
        "#);
    }

    #[test]
    fn the_footer_follows_rebound_keys() {
        let mut config = Config::default();
        config
            .tui
            .keymap
            .bind(InputState::Browse, "/".parse().unwrap(), vec![Action::None]);
        config.tui.keymap.bind(
            InputState::Browse,
            "s".parse().unwrap(),
            vec![Action::Filter],
        );
        let board = board_with(data(), config);
        assert_snapshot!(screen(&board, 110, 5), @r#"
        "nib  BROWSE   letters are actions                                                              Fri 30 Oct 2026"
        "/ press / to filter                                                                                  5 results"
        "──────────────────────────────────────────────────────────────────────────────────────────────────────────────"
        "┌ Lists ─────────────┐┌ All · 5 ───────────────────────────────────────────┐┌ Details ───────────────────────┐"
        "s filter  a add  tab select  x done  e edit  delete delete  ctrl-o act  ] pane  ? help  q quit                "
        
        "#);
    }

    #[test]
    fn move_menu_lists_writable_lists() {
        let mut board = board();
        act(&mut board, Action::Move);
        board.update(Input::Menu(MenuKey::Down));
        assert_snapshot!(screen(&board, 110, 10), @r#"
        "nib  BROWSE   letters are actions                                                              Fri 30 Oct 2026"
        "/ press / to filter                                                                                  5 results"
        "──────────────────────────────────────────────────────────────────────────────────────────────────────────────"
        "┌ Lists ─────────────┐┌ All · 5 ───────────────────────────────────────────┐┌ Details ───────────────────────┐"
        "│All                5││    Buy ┌ Move "Buy oat milk" to ──────────────────────┐uy oat milk                   │"
        "│Today              2││    Buy │ 1  Home                                      │ist     Home                  │"
        "│Home               1││    File│ 2  Work                                      │ue      today (2026-10-30)    │"
        "│Work               2││  ◆ Stan└──────────────────────────────────────────────┘riority high                  │"
        "└────────────────────┘└────────────────────────────────────────────────────┘└────────────────────────────────┘"
        "enter choose  esc back                                                                                        "
        
        "#);
    }
}
