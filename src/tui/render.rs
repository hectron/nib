use jiff::ToSpan;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, Padding, Paragraph, Row as TableRow, Table, TableState, Wrap,
};

use super::keys;
use super::state::{ACTS, Action, Item, Mode, PRIORITIES, Row, State};
use crate::cli::output::clean;
use crate::domain::reminder_due_is_overdue;

const PREVIEW_WIDTH: u16 = 40;
const DUE_WIDTH: u16 = 16;
const LIST_WIDTH: u16 = 14;

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
}

const MUTED: Style = Style::new().add_modifier(Modifier::DIM);
const STRONG: Style = Style::new().add_modifier(Modifier::BOLD);

pub(crate) fn draw(frame: &mut Frame<'_>, state: &State, color: bool) {
    let theme = Theme { color };
    let [prompt, status, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    draw_prompt(frame, prompt, state, theme);
    draw_status(frame, status, state, theme);
    let (results, side) = if body.width >= PREVIEW_WIDTH * 2 {
        let [results, side] =
            Layout::horizontal([Constraint::Min(1), Constraint::Length(PREVIEW_WIDTH)])
                .spacing(1)
                .areas(body);
        (results, Some(side))
    } else {
        (body, None)
    };
    if state.mode() == Mode::Filter {
        draw_results(frame, results, state, theme);
        if let Some(side) = side {
            draw_preview(frame, side, state, theme);
        }
    } else {
        if side.is_some() {
            draw_results(frame, results, state, theme);
        }
        draw_menu(frame, side.unwrap_or(results), state, theme);
    }
    draw_footer(frame, footer, state, theme);
}

fn draw_prompt(frame: &mut Frame<'_>, area: Rect, state: &State, theme: Theme) {
    let query = clean(state.query());
    let mut counts = format!("{} matches", state.match_count());
    if !state.selected().is_empty() {
        counts = format!("{} selected  {counts}", state.selected().len());
    }
    let [left, right] = Layout::horizontal([
        Constraint::Min(1),
        Constraint::Length(u16::try_from(counts.chars().count() + 1).unwrap_or(u16::MAX)),
    ])
    .areas(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("> ", theme.paint(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(query.clone()),
        ])),
        left,
    );
    frame.render_widget(
        Paragraph::new(Span::styled(counts, MUTED)).right_aligned(),
        right,
    );
    if state.mode() == Mode::Filter {
        let before: String = query.chars().take(state.caret()).collect();
        let offset = u16::try_from(Line::raw(before).width()).unwrap_or(u16::MAX);
        let x = left
            .x
            .saturating_add(2)
            .saturating_add(offset)
            .min(left.right().saturating_sub(1));
        frame.set_cursor_position((x, left.y));
    }
}

fn draw_status(frame: &mut Frame<'_>, area: Rect, state: &State, theme: Theme) {
    let line = if let Some(error) = state.filter_error() {
        Line::styled(format!("  {}", clean(error)), theme.paint(Color::Red))
    } else if let Some(status) = state.status() {
        let tone = if status.error {
            Color::Red
        } else {
            Color::Green
        };
        Line::styled(format!("  {}", clean(&status.text)), theme.paint(tone))
    } else {
        Line::styled(
            "─".repeat(usize::from(area.width)),
            MUTED.add_modifier(Modifier::DIM),
        )
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_results(frame: &mut Frame<'_>, area: Rect, state: &State, theme: Theme) {
    let rows = state.rows();
    if rows.is_empty() {
        frame.render_widget(Paragraph::new(Span::styled("  No matches.", MUTED)), area);
        return;
    }
    let mut list_width = 0;
    let mut due_width = 0;
    let table_rows: Vec<TableRow<'_>> = rows
        .iter()
        .map(|row| {
            let cells = match row {
                Row::Item(item) => {
                    let marker = if state.is_selected(&item.id) {
                        "+"
                    } else {
                        " "
                    };
                    let title_style = if item.completed {
                        MUTED.add_modifier(Modifier::CROSSED_OUT)
                    } else {
                        Style::new()
                    };
                    [
                        Span::styled(marker, theme.paint(Color::Yellow)),
                        Span::styled(clean(&item.title), title_style),
                        Span::styled(
                            clean(item.list.as_deref().unwrap_or_default()),
                            theme.paint(Color::Magenta),
                        ),
                        due_span(item, state, theme),
                    ]
                }
                Row::Add(text) => [
                    Span::styled("+", theme.paint(Color::Cyan)),
                    Span::styled(
                        format!("Add reminder: {}", clean(text)),
                        theme.paint(Color::Cyan),
                    ),
                    Span::raw(""),
                    Span::styled("enter", MUTED),
                ],
            };
            list_width = list_width.max(cells[2].width());
            due_width = due_width.max(cells[3].width());
            TableRow::new(cells.map(Cell::from))
        })
        .collect();
    let width = |value: usize, cap: u16| u16::try_from(value).unwrap_or(cap).min(cap);
    let table = Table::new(
        table_rows,
        [
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(width(list_width, LIST_WIDTH)),
            Constraint::Length(width(due_width, DUE_WIDTH)),
        ],
    )
    .column_spacing(1)
    .row_highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    let mut table_state = TableState::new().with_selected(Some(state.cursor()));
    frame.render_stateful_widget(table, area, &mut table_state);
}

fn due_span(item: &Item, state: &State, theme: Theme) -> Span<'static> {
    let Some(date) = item.due else {
        return Span::raw("");
    };
    let today = state.now().date();
    let mut label = if date == today {
        "today".to_owned()
    } else if today.checked_add(1.day()).ok() == Some(date) {
        "tomorrow".to_owned()
    } else if today.checked_sub(1.day()).ok() == Some(date) {
        "yesterday".to_owned()
    } else {
        date.to_string()
    };
    if let Some(time) = item.due_time {
        label.push(' ');
        label.push_str(&time.strftime("%H:%M").to_string());
    }
    let style = if !item.completed
        && reminder_due_is_overdue(item.due, item.due_time, item.due_at, state.now())
    {
        theme.paint(Color::Red)
    } else if date == today {
        theme.paint(Color::Yellow)
    } else {
        MUTED
    };
    Span::styled(label, style)
}

fn draw_preview(frame: &mut Frame<'_>, area: Rect, state: &State, theme: Theme) {
    let block = Block::new()
        .borders(Borders::LEFT)
        .border_style(MUTED)
        .padding(Padding::horizontal(1));
    let lines = match state.current() {
        Some(Row::Item(item)) => item_preview(item, state, theme),
        Some(Row::Add(text)) => vec![
            Line::styled("New reminder", STRONG),
            Line::raw(clean(text)),
            Line::raw(""),
            Line::styled(
                "Parsed like quick-add: a trailing due date, !priority, and #list.",
                MUTED,
            ),
        ],
        None => Vec::new(),
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn item_preview(item: &Item, state: &State, theme: Theme) -> Vec<Line<'static>> {
    let field = |name: &str, value: Span<'static>| {
        Line::from(vec![Span::styled(format!("{name:<9}"), MUTED), value])
    };
    let mut lines = vec![Line::styled(clean(&item.title), STRONG)];
    if let Some(list) = &item.list {
        lines.push(field(
            "list",
            Span::styled(clean(list), theme.paint(Color::Magenta)),
        ));
    }
    if item.due.is_some() {
        let mut due = due_span(item, state, theme);
        if let Some(date) = item.due
            && due.content != date.to_string()
            && item.due_time.is_none()
        {
            due.content = format!("{} ({date})", due.content).into();
        }
        lines.push(field("due", due));
    }
    if let Some(priority) = item.priority {
        lines.push(field("priority", Span::raw(priority.as_str())));
    }
    if item.completed {
        lines.push(field("status", Span::raw("completed")));
    }
    if let Some(url) = &item.url {
        lines.push(field("url", Span::raw(clean(url))));
    }
    if let Some(notes) = item
        .notes
        .as_deref()
        .filter(|notes| !notes.trim().is_empty())
    {
        lines.push(Line::raw(""));
        lines.extend(notes.lines().map(|line| Line::raw(clean(line))));
    }
    lines
}

fn draw_menu(frame: &mut Frame<'_>, area: Rect, state: &State, theme: Theme) {
    let targets = state.targets();
    let what = match targets {
        [only] => state
            .item(only)
            .map_or_else(|| clean(only), |item| clean(&item.title)),
        many => format!("{} reminders", many.len()),
    };
    let (title, options): (String, Vec<(String, String)>) = match state.mode() {
        Mode::Priority => (
            format!("Priority for {what}"),
            PRIORITIES
                .iter()
                .map(|(key, name, _)| (key.to_string(), (*name).to_owned()))
                .collect(),
        ),
        Mode::Move => (
            format!("Move {what} to"),
            state
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
        Mode::ConfirmDelete => {
            draw_confirm_delete(frame, area, state, &what, theme);
            return;
        }
        Mode::Act | Mode::Filter => (
            format!("Act on {what}"),
            ACTS.iter()
                .map(|(key, name)| (key.to_string(), (*name).to_owned()))
                .collect(),
        ),
    };
    let hidden = hidden_line(state, theme);
    let lines: Vec<Line<'_>> = options
        .into_iter()
        .enumerate()
        .map(|(index, (key, name))| {
            let line = Line::from(vec![
                Span::styled(
                    format!("{key}  "),
                    theme.paint(Color::Cyan).add_modifier(Modifier::BOLD),
                ),
                Span::raw(name),
            ]);
            if index == state.menu_cursor() {
                line.style(Style::new().add_modifier(Modifier::REVERSED))
            } else {
                line
            }
        })
        .chain(hidden)
        .collect();
    let skip = state
        .menu_cursor()
        .saturating_sub(usize::from(area.height.saturating_sub(2)));
    let block = Block::new()
        .borders(Borders::LEFT)
        .border_style(MUTED)
        .padding(Padding::horizontal(1))
        .title(Span::styled(title, STRONG));
    frame.render_widget(
        Paragraph::new(lines.into_iter().skip(skip).collect::<Vec<_>>())
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn hidden_line(state: &State, theme: Theme) -> Option<Line<'static>> {
    let hidden = state.hidden_target_count();
    (hidden > 0).then(|| {
        Line::styled(
            format!("{hidden} selected not shown by the current query."),
            theme.paint(Color::Yellow),
        )
    })
}

fn draw_confirm_delete(frame: &mut Frame<'_>, area: Rect, state: &State, what: &str, theme: Theme) {
    let key = |text: String| {
        Span::styled(
            format!("{text}  "),
            theme.paint(Color::Cyan).add_modifier(Modifier::BOLD),
        )
    };
    let mut lines = Vec::new();
    if state.delete_needs_count() {
        let count = state.targets().len();
        lines.push(Line::from(vec![
            Span::raw(format!("Type {count} and press enter: ")),
            Span::styled(state.confirm_input().to_owned(), STRONG),
        ]));
    } else {
        lines.push(Line::from(vec![
            key("y".into()),
            Span::raw("delete  "),
            key("n".into()),
            Span::raw("keep"),
        ]));
    }
    if let Some(hidden) = hidden_line(state, theme) {
        lines.push(hidden);
    }
    let partial = state.partial_target_count();
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
    let block = Block::new()
        .borders(Borders::LEFT)
        .border_style(MUTED)
        .padding(Padding::horizontal(1))
        .title(Span::styled(format!("Delete {what}?"), STRONG));
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_footer(frame: &mut Frame<'_>, area: Rect, state: &State, theme: Theme) {
    let hints: &[(Action, &str)] = match state.mode() {
        Mode::Filter => &[
            (Action::Accept, "pick"),
            (Action::ToggleDown, "select"),
            (Action::OpenMenu, "act"),
            (Action::Down, "move"),
            (Action::Cancel, "cancel"),
        ],
        Mode::ConfirmDelete => &[(Action::Cancel, "keep")],
        Mode::Act | Mode::Priority | Mode::Move => {
            &[(Action::Accept, "choose"), (Action::Cancel, "back")]
        }
    };
    let mut spans = Vec::new();
    for (action, text) in hints {
        if let Some(key) = keys::label(*action) {
            if !spans.is_empty() {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(key, theme.paint(Color::Cyan)));
            spans.push(Span::styled(format!(" {text}"), MUTED));
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::tui::state::tests::{data, now, type_text};

    fn screen(state: &State, width: u16) -> String {
        sized_screen(state, width, 9)
    }

    fn sized_screen(state: &State, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame, state, false)).unwrap();
        terminal.backend().to_string()
    }

    #[test]
    fn the_prompt_shows_results_a_preview_and_the_add_row() {
        let mut items = data();
        items.items[0].notes = Some("2% or oat".into());
        let (mut state, _) = State::new(items, "", now());
        type_text(&mut state, "buy");
        state.update(Action::ToggleDown);
        state.update(Action::Up);
        assert_snapshot!(screen(&state, 84), @r#"
        "> buy                                                          1 selected  2 matches"
        "────────────────────────────────────────────────────────────────────────────────────"
        "+ Buy oat milk                   Home today │ Buy oat milk                          "
        "  Buy eggs                       Work       │ list     Home                         "
        "+ Add reminder: buy                   enter │ due      today (2026-10-30)           "
        "                                            │ priority high                         "
        "                                            │                                       "
        "                                            │ 2% or oat                             "
        "enter pick  tab select  ctrl-o act  ctrl-n move  esc cancel                         "
        "#);
    }

    #[test]
    fn the_act_menu_replaces_the_preview() {
        let (mut state, _) = State::new(data(), "", now());
        state.update(Action::ToggleDown);
        state.update(Action::ToggleDown);
        state.update(Action::OpenMenu);
        state.update(Action::Down);
        assert_snapshot!(screen(&state, 84), @r#"
        ">                                                              2 selected  3 matches"
        "────────────────────────────────────────────────────────────────────────────────────"
        "+ Buy oat milk                Home today    │Act on 2 reminders                     "
        "+ Buy eggs                    Work          │ x  done                               "
        "  File report                 Work tomorrow │ p  priority                           "
        "                                            │ m  move                               "
        "                                            │ d  delete                             "
        "                                            │                                       "
        "enter choose  esc back                                                              "
        "#);
    }

    #[test]
    fn narrow_terminals_hide_the_preview_and_show_errors_inline() {
        let (state, _) = State::new(data(), "pri:urgent", now());
        assert_snapshot!(screen(&state, 60), @r#"
        "> pri:urgent                                       3 matches"
        "  invalid value "urgent" for pri filter; expected high, medi"
        "  Buy oat milk                                 Home today   "
        "  Buy eggs                                     Work         "
        "  File report                                  Work tomorrow"
        "                                                            "
        "                                                            "
        "                                                            "
        "enter pick  tab select  ctrl-o act  ctrl-n move  esc cancel "
        "#);
    }

    #[test]
    fn delete_confirmation_counts_hidden_selections_and_warns_about_undo() {
        let (mut state, _) = State::new(data(), "", now());
        state.update(Action::ToggleDown);
        type_text(&mut state, "eggs");
        state.update(Action::ToggleDown);
        state.update(Action::OpenMenu);
        state.update(Action::Hotkey('d'));
        assert_snapshot!(screen(&state, 84), @r#"
        "> eggs                                                         2 selected  1 matches"
        "────────────────────────────────────────────────────────────────────────────────────"
        "+ Buy eggs                       Work       │Delete 2 reminders?                    "
        "+ Add reminder: eggs                  enter │ y  delete  n  keep                    "
        "                                            │ 1 selected not shown by the current   "
        "                                            │ query.                                "
        "                                            │ Undo re-creates deleted reminders     "
        "                                            │ with new ids.                         "
        "esc keep                                                                            "
        "#);
    }

    #[test]
    fn deleting_many_asks_for_the_count() {
        let mut items = data();
        items.items.push(crate::tui::state::tests::item(
            "rem-4",
            "Water plants",
            "Home",
            None,
            None,
        ));
        let (mut state, _) = State::new(items, "", now());
        for _ in 0..4 {
            state.update(Action::ToggleDown);
        }
        state.update(Action::OpenMenu);
        state.update(Action::Hotkey('d'));
        state.update(Action::Hotkey('4'));
        assert_snapshot!(screen(&state, 60), @r#"
        ">                                      4 selected  4 matches"
        "────────────────────────────────────────────────────────────"
        "│Delete 4 reminders?                                        "
        "│ Type 4 and press enter: 4                                 "
        "│ Undo re-creates deleted reminders with new ids.           "
        "│                                                           "
        "│                                                           "
        "│                                                           "
        "esc keep                                                    "
        "#);
    }

    #[test]
    fn a_partial_backup_asks_for_the_count_and_says_what_undo_loses() {
        let mut items = data();
        items.items[1].backup_partial = true;
        let (mut state, _) = State::new(items, "", now());
        state.update(Action::ToggleDown);
        state.update(Action::ToggleDown);
        state.update(Action::OpenMenu);
        state.update(Action::Hotkey('d'));
        assert_snapshot!(screen(&state, 60), @r#"
        ">                                      2 selected  3 matches"
        "────────────────────────────────────────────────────────────"
        "│Delete 2 reminders?                                        "
        "│ Type 2 and press enter:                                   "
        "│ 1 has alarms, repeats, or links that undo can't bring     "
        "│ back.                                                     "
        "│ Undo re-creates deleted reminders with new ids.           "
        "│                                                           "
        "esc keep                                                    "
        "#);
    }

    #[test]
    fn menus_count_selections_hidden_by_the_query() {
        let (mut state, _) = State::new(data(), "", now());
        state.update(Action::ToggleDown);
        type_text(&mut state, "eggs");
        state.update(Action::ToggleDown);
        state.update(Action::OpenMenu);
        state.update(Action::Hotkey('p'));
        assert_snapshot!(sized_screen(&state, 84, 10), @r#"
        "> eggs                                                         2 selected  1 matches"
        "────────────────────────────────────────────────────────────────────────────────────"
        "+ Buy eggs                       Work       │Priority for 2 reminders               "
        "+ Add reminder: eggs                  enter │ h  high                               "
        "                                            │ m  medium                             "
        "                                            │ l  low                                "
        "                                            │ n  none                               "
        "                                            │ 1 selected not shown by the current   "
        "                                            │ query.                                "
        "enter choose  esc back                                                              "
        "#);
    }
}
