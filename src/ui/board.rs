//! Board selection screen.

use std::collections::HashSet;

use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;
use crate::catalog::canonical_level;
use crate::ui::{self, theme::Theme};

/// Draw the board list.
pub fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let block = ui::block("Boards", theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let boards = app.boards();
    if boards.is_empty() {
        let message = format!(
            "No catalog files were found.\n\nAdd a catalog to {}\nor run `pu-get update` or `pu-get crawl`.",
            app.paths.catalog_dir.display()
        );
        frame.render_widget(
            Paragraph::new(message)
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true })
                .style(theme.dim_style()),
            ui::centered_rect(inner.width, 6, inner),
        );
        return;
    }

    let width = inner.width as usize;
    let visible = ((inner.height as usize + 1) / 3).max(1);
    let start = ui::window_start(app.board_cursor, boards.len(), visible);
    let end = (start + visible).min(boards.len());

    let mut items: Vec<ListItem<'_>> = Vec::new();
    for (position, index) in (start..end).enumerate() {
        let board = &boards[index];
        let selected = index == app.board_cursor;
        let marker_style = if selected {
            theme.selected_text_style()
        } else {
            theme.accent_style()
        };
        let name_style = if selected {
            theme.selected_text_style()
        } else {
            theme.text_style()
        };
        let secondary_style = if selected {
            theme.selected_secondary_style()
        } else {
            theme.dim_style()
        };

        let marker = ui::selected_marker(selected);
        let count = format!("{} PDFs", ui::format_count(board.count));
        let gap = width
            .saturating_sub(marker.chars().count() + board.label.chars().count() + count.len())
            .max(1);

        let title = Line::from(vec![
            Span::styled(marker, marker_style),
            Span::styled(board.label.clone(), name_style),
            Span::raw(" ".repeat(gap)),
            Span::styled(count, secondary_style),
        ]);

        let levels = board_levels(app, &board.value);
        let detail = Line::from(vec![Span::raw("  "), Span::styled(levels, secondary_style)]);

        items.push(ListItem::new(vec![title, detail]).style(if selected {
            theme.selected_style()
        } else {
            ui::plain_style()
        }));

        if position + 1 < visible && index + 1 < boards.len() {
            items.push(ListItem::new(Line::from("")));
        }
    }

    frame.render_widget(List::new(items), inner);
}

fn board_levels(app: &App, board_id: &str) -> String {
    let Some(board) = app.catalog.find_board(board_id) else {
        return "No material yet".to_owned();
    };

    let mut seen = HashSet::new();
    let mut labels = Vec::new();
    let mut numbers = Vec::new();
    let mut all_numeric = true;

    for entry in &board.entries {
        let canonical = canonical_level(&entry.level);
        if !seen.insert(canonical.clone()) {
            continue;
        }

        if let Ok(number) = canonical.parse::<u16>() {
            numbers.push(number);
        } else {
            all_numeric = false;
        }
        labels.push(short_level_label(&entry.level));
    }

    if labels.is_empty() {
        return "No material yet".to_owned();
    }

    if all_numeric {
        numbers.sort_unstable();
        let contiguous = numbers.len() >= 3
            && numbers
                .windows(2)
                .all(|pair| pair[1] == pair[0].saturating_add(1));
        if contiguous {
            return format!(
                "Classes {}-{}",
                numbers.first().copied().unwrap_or(0),
                numbers.last().copied().unwrap_or(0)
            );
        }
        return numbers
            .iter()
            .map(|number| format!("Class {number}"))
            .collect::<Vec<_>>()
            .join(" · ");
    }

    labels.join(" · ")
}

fn short_level_label(level: &str) -> String {
    let canonical = canonical_level(level);
    match canonical.as_str() {
        "sslc" => "SSLC".to_owned(),
        "puc1" => "PUC 1".to_owned(),
        "puc2" => "PUC 2".to_owned(),
        _ => match canonical.parse::<u16>() {
            Ok(number) => format!("Class {number}"),
            Err(_) => level.trim().to_owned(),
        },
    }
}
