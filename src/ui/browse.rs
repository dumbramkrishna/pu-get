//! Level, stream, subject, material and PDF-list screens.

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Clear, List, ListItem, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use crate::app::{App, Screen};
use crate::ui::{self, theme::Theme};

/// Draw one of the intermediate selection screens.
pub fn draw_menu(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let (title, items, empty) = match app.screen {
        Screen::Level => (
            "Levels",
            app.levels(),
            "This board has no levels in the catalog.",
        ),
        Screen::Stream => (
            "Streams",
            app.stream_options(),
            "This level has no streams. Press Esc to go back.",
        ),
        Screen::Subject => (
            "Subjects",
            app.subjects(),
            "No subjects are catalogued for this selection.",
        ),
        Screen::Material => (
            "Material",
            app.materials(),
            "No material is catalogued for this subject.",
        ),
        _ => return,
    };

    let block = ui::block(title, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(empty)
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true })
                .style(theme.dim_style()),
            ui::centered_rect(inner.width, 4, inner),
        );
        return;
    }

    let cursor = app.current_cursor();
    let width = inner.width as usize;
    let height = inner.height as usize;
    let start = ui::window_start(cursor, items.len(), height);

    let rows: Vec<ListItem<'_>> = items
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(index, item)| {
            let selected = index == cursor;
            let marker_style = if selected {
                theme.selected_text_style()
            } else {
                theme.accent_style()
            };
            let label_style = if selected {
                theme.selected_text_style()
            } else {
                theme.text_style()
            };
            let count_style = if selected {
                theme.selected_secondary_style()
            } else {
                theme.dim_style()
            };

            let marker = ui::selected_marker(selected);
            let count = format!("{} PDFs", ui::format_count(item.count));
            let gap = width
                .saturating_sub(marker.chars().count() + item.label.chars().count() + count.len())
                .max(1);

            ListItem::new(Line::from(vec![
                Span::styled(marker, marker_style),
                Span::styled(item.label.clone(), label_style),
                Span::raw(" ".repeat(gap)),
                Span::styled(count, count_style),
            ]))
            .style(if selected {
                theme.selected_style()
            } else {
                ui::plain_style()
            })
        })
        .collect();

    frame.render_widget(List::new(rows), inner);
}

/// Draw the PDF table and its filter bar.
pub fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let block = ui::block("PDFs", theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 3 || inner.width == 0 {
        return;
    }

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .split(inner);

    draw_filter_bar(frame, chunks[0], app, theme);
    ui::rule(frame, chunks[1], theme);

    let entries = app.filtered();
    if entries.is_empty() {
        draw_empty_state(frame, chunks[2], app, theme);
    } else {
        draw_table(frame, chunks[2], app, &entries, theme);
    }

    if app.year_picker_open {
        draw_year_picker(frame, area, app, theme);
    }
}

fn draw_filter_bar(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let columns = Layout::horizontal([Constraint::Min(20), Constraint::Length(36)]).split(area);

    let search_label_style = if app.search_active {
        theme.accent_style()
    } else {
        theme.dim_style()
    };

    let mut search_spans = vec![Span::styled("Search", search_label_style), Span::raw("  ")];

    if app.search.is_empty() {
        search_spans.push(Span::styled(
            if app.search_active {
                "Type to filter"
            } else {
                "press / to filter"
            },
            theme.dim_style(),
        ));
    } else {
        search_spans.push(Span::styled(app.search.clone(), theme.text_style()));
    }

    if app.search_active {
        search_spans.push(Span::styled("_", theme.accent_style()));
    }

    frame.render_widget(Paragraph::new(Line::from(search_spans)), columns[0]);

    let year = app
        .year_filter
        .map_or_else(|| "All".to_owned(), |year| year.to_string());
    let mut filter_spans = vec![
        Span::styled("Year ", theme.dim_style()),
        Span::styled(year, theme.text_style()),
        Span::raw("   "),
        Span::styled("Latest ", theme.dim_style()),
        Span::styled(
            if app.latest_only { "On" } else { "Off" },
            if app.latest_only {
                theme.text_style()
            } else {
                theme.dim_style()
            },
        ),
    ];

    if !app.selected.is_empty() {
        filter_spans.push(Span::raw("   "));
        filter_spans.push(Span::styled(
            format!("{} selected", ui::format_count(app.selected.len())),
            theme.key_style(),
        ));
    }

    frame.render_widget(
        Paragraph::new(Line::from(filter_spans)).alignment(Alignment::Right),
        columns[1],
    );
}

fn draw_table(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    entries: &[crate::catalog::CatalogEntry<'_>],
    theme: &Theme,
) {
    if area.height == 0 {
        return;
    }

    let header = Row::new(vec![
        Cell::from(" "),
        Cell::from("Title"),
        Cell::from("Year"),
        Cell::from("Lang"),
        Cell::from("Size"),
        Cell::from("Source"),
    ])
    .style(theme.header_style());

    let visible = (area.height as usize).saturating_sub(1);
    if visible == 0 {
        return;
    }

    let start = ui::window_start(app.browse_cursor, entries.len(), visible);
    let rows: Vec<Row<'_>> = entries
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .map(|(index, item)| {
            let entry = item.entry;
            let checked = app.selected.contains(&entry.url);
            let row_selected = index == app.browse_cursor;

            let marker_style = if row_selected {
                theme.selected_secondary_style()
            } else if checked {
                theme.accent_style()
            } else {
                theme.dim_style()
            };
            let title_style = if row_selected {
                theme.selected_text_style()
            } else {
                theme.text_style()
            };
            let meta_style = if row_selected {
                theme.selected_secondary_style()
            } else {
                theme.dim_style()
            };

            let cells = vec![
                Cell::from(if checked { "●" } else { "○" }).style(marker_style),
                Cell::from(entry.title.clone()).style(title_style),
                Cell::from(
                    entry
                        .year
                        .map_or_else(|| "-".to_owned(), |year| year.to_string()),
                )
                .style(meta_style),
                Cell::from(entry.language.code().to_ascii_uppercase()).style(meta_style),
                Cell::from(
                    entry
                        .size_bytes
                        .map_or_else(|| "-".to_owned(), ui::human_bytes),
                )
                .style(meta_style),
                Cell::from(entry.source.clone()).style(meta_style),
            ];

            let row = Row::new(cells);
            if row_selected {
                row.style(theme.selected_style())
            } else {
                row
            }
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Min(20),
            Constraint::Length(4),
            Constraint::Length(4),
            Constraint::Length(9),
            Constraint::Length(13),
        ],
    )
    .header(header)
    .column_spacing(1);

    frame.render_widget(table, area);
}

fn draw_empty_state(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let message = if !app.search.trim().is_empty() {
        format!(
            "No PDFs match \"{}\".\nPress Esc to clear the search.",
            app.search.trim()
        )
    } else if let Some(year) = app.year_filter {
        format!("No PDFs for {year}.\nPress y to choose another year.")
    } else {
        "No PDFs are catalogued for this selection.\nPress Esc to go back.".to_owned()
    };

    frame.render_widget(
        Paragraph::new(message)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .style(theme.dim_style()),
        ui::centered_rect(area.width.saturating_sub(8), 4, area),
    );
}

fn draw_year_picker(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let years = app.available_years();
    let wanted = years.len().saturating_add(3);
    let height = u16::try_from(wanted)
        .unwrap_or(u16::MAX)
        .min(area.height.saturating_sub(2));
    let rect = ui::centered_rect(42, height, area);
    frame.render_widget(Clear, rect);

    let block = ui::block("Year", theme);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let len = years.len() + 1;
    let width = inner.width as usize;
    let visible = inner.height as usize;
    let start = ui::window_start(app.year_cursor, len, visible);

    let items: Vec<ListItem<'_>> = (0..len)
        .skip(start)
        .take(visible)
        .map(|index| {
            let selected = index == app.year_cursor;
            let year = index.checked_sub(1).and_then(|offset| years.get(offset));
            let current = match year {
                Some(year) => app.year_filter == Some(*year),
                None => app.year_filter.is_none(),
            };
            let label = year.map_or_else(|| "All years".to_owned(), u16::to_string);

            let marker_style = if selected {
                theme.selected_text_style()
            } else {
                theme.accent_style()
            };
            let label_style = if selected {
                theme.selected_text_style()
            } else {
                theme.text_style()
            };
            let check_style = if selected {
                theme.selected_secondary_style()
            } else {
                theme.success_style()
            };

            let marker = ui::selected_marker(selected);
            let check = if current { "✓" } else { "" };
            let gap = width
                .saturating_sub(marker.chars().count() + label.chars().count() + 1)
                .max(1);

            ListItem::new(Line::from(vec![
                Span::styled(marker, marker_style),
                Span::styled(label, label_style),
                Span::raw(" ".repeat(gap)),
                Span::styled(check, check_style),
            ]))
            .style(if selected {
                theme.selected_style()
            } else {
                ui::plain_style()
            })
        })
        .collect();

    frame.render_widget(List::new(items), inner);
}
