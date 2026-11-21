//! Keyboard-shortcut overlay.
//!
//! The shortcut list replaces the body of the screen instead of floating over
//! the list content, so nothing underneath distracts from the key map. The
//! header, status and footer stay visible for context.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::ui::{self, theme::Theme};

/// Draw the help overlay over the body area.
pub fn draw(frame: &mut Frame, area: Rect, _app: &App, theme: &Theme) {
    let body = ui::body_area(area);
    let rect = Rect::new(
        body.x + 1,
        body.y,
        body.width.saturating_sub(2).max(1),
        body.height.max(1),
    );
    frame.render_widget(Clear, rect);

    let block = ui::block(
        &format!("Keyboard shortcuts · v{}", env!("CARGO_PKG_VERSION")),
        theme,
    );
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let columns = Layout::horizontal([
        Constraint::Length(34),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(inner);

    let section = |text: &'static str| Line::from(Span::styled(text, theme.header_style()));
    let row = |key: &'static str, label: &'static str| {
        Line::from(vec![
            Span::styled(format!("{key:<14}"), theme.key_style()),
            Span::styled(label, theme.dim_style()),
        ])
    };

    let left = vec![
        section("NAVIGATION"),
        row("j/k / arrows", "move"),
        row("Enter", "open / download"),
        row("Esc / Bksp", "back"),
        row("Home / End", "first / last"),
        Line::from(""),
        section("BROWSE"),
        row("Space", "select"),
        row("a", "select all"),
        row("/", "fuzzy search"),
        row("y", "year"),
        row("t", "latest only"),
        row("d / Enter", "download"),
        Line::from(""),
        section("SHORTCUTS"),
        row("D", "downloads"),
        row("s", "settings"),
    ];

    let right = vec![
        section("DOWNLOADS"),
        row("p / Enter", "pause / resume"),
        row("r", "retry failed"),
        row("c", "cancel"),
        row("s r o c", "resolve a conflict"),
        Line::from(""),
        section("EVERYWHERE"),
        row("?", "this help"),
        row("q", "quit"),
        row("Ctrl-C", "quit immediately"),
        Line::from(""),
        Line::from(Span::styled(
            "Search, year and latest-only",
            theme.dim_style(),
        )),
        Line::from(Span::styled(
            "changes clear the selection.",
            theme.dim_style(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Downloads go to the folder",
            theme.dim_style(),
        )),
        Line::from(Span::styled("configured in Settings.", theme.dim_style())),
    ];

    frame.render_widget(Paragraph::new(left), columns[0]);
    frame.render_widget(
        Block::default()
            .borders(Borders::LEFT)
            .border_style(theme.border_style()),
        columns[1],
    );
    frame.render_widget(Paragraph::new(right), columns[2]);
}
