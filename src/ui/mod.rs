//! Pure rendering for every TUI screen.
//!
//! [`draw`] and the screen modules take `&App`; they never mutate state. This
//! keeps layout predictable and lets tests render any screen against a
//! [`ratatui::backend::TestBackend`] without a terminal.

pub mod board;
pub mod browse;
pub mod help;
pub mod queue;
pub mod settings;
pub mod splash;
pub mod theme;

use std::time::Duration;

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Padding, Paragraph};
use ratatui::Frame;

use crate::app::{App, Screen};
use theme::Theme;

/// Minimum supported terminal width.
pub const MIN_WIDTH: u16 = 80;
/// Minimum supported terminal height.
pub const MIN_HEIGHT: u16 = 24;

/// Draw the whole interface.
pub fn draw(frame: &mut Frame, app: &App) {
    let theme = Theme::new(app.color);
    let area = frame.area();

    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        draw_too_small(frame, area, &theme);
        return;
    }

    match app.screen {
        Screen::Splash => splash::draw(frame, area, app, &theme),
        Screen::Help => {
            draw_chrome(frame, area, app, app.help_return, &theme);
            help::draw(frame, area, app, &theme);
        }
        screen => draw_chrome(frame, area, app, screen, &theme),
    }
}

fn draw_chrome(frame: &mut Frame, area: Rect, app: &App, screen: Screen, theme: &Theme) {
    let layout = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(area);

    draw_header(frame, layout[0], app, theme);
    draw_body(frame, layout[1], app, screen, theme);
    draw_status(frame, layout[2], app, theme);
    draw_footer(frame, layout[3], app, theme);
}

/// The area between the header and the status/footer rows.
#[must_use]
pub fn body_area(area: Rect) -> Rect {
    let layout = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(area);
    layout[1]
}

fn draw_body(frame: &mut Frame, area: Rect, app: &App, screen: Screen, theme: &Theme) {
    match screen {
        Screen::Board => board::draw(frame, area, app, theme),
        Screen::Level | Screen::Stream | Screen::Subject | Screen::Material => {
            browse::draw_menu(frame, area, app, theme);
        }
        Screen::Browse => browse::draw(frame, area, app, theme),
        Screen::Queue => queue::draw(frame, area, app, theme),
        Screen::Settings => settings::draw(frame, area, app, theme),
        Screen::Splash | Screen::Help => {}
    }
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(area);

    draw_brand_row(frame, rows[0], app, theme);
    draw_breadcrumb_row(frame, rows[1], app, theme);
    rule(frame, rows[2], theme);
}

fn draw_brand_row(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let columns = Layout::horizontal([Constraint::Min(20), Constraint::Length(20)]).split(area);

    let brand = Line::from(vec![
        Span::raw(" "),
        Span::styled("pu-get", theme.brand_style()),
        Span::styled(
            format!(" v{}", env!("CARGO_PKG_VERSION")),
            theme.dim_style(),
        ),
    ]);
    frame.render_widget(Paragraph::new(brand), columns[0]);

    if let Some(badge) = header_badge(app, theme) {
        frame.render_widget(
            Paragraph::new(badge).alignment(Alignment::Right),
            columns[1],
        );
    }
}

fn header_badge(app: &App, theme: &Theme) -> Option<Line<'static>> {
    let conflicts = app.queue.conflicts.len();
    if conflicts > 0 {
        let label = if conflicts == 1 {
            "1 conflict".to_owned()
        } else {
            format!("{conflicts} conflicts")
        };
        return Some(Line::from(vec![
            Span::styled("! ", theme.error_style()),
            Span::styled(label, theme.error_style()),
            Span::raw(" "),
        ]));
    }

    if app.screen != Screen::Queue {
        let active = app.active_downloads();
        if active > 0 {
            return Some(Line::from(vec![
                Span::styled("● ", theme.accent_style()),
                Span::styled(format!("{active} active"), theme.dim_style()),
                Span::raw(" "),
            ]));
        }
    }

    None
}

fn draw_breadcrumb_row(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let parts = header_breadcrumb(app);
    let welcome = app.screen == Screen::Board && app.breadcrumb().is_empty();
    let mut spans = vec![Span::raw(" ")];

    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" › ", theme.dim_style()));
        }
        let style = if !welcome && index + 1 == parts.len() {
            theme.accent_style()
        } else {
            theme.dim_style()
        };
        spans.push(Span::styled(part.clone(), style));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn header_breadcrumb(app: &App) -> Vec<String> {
    match app.screen {
        Screen::Splash => Vec::new(),
        Screen::Board => {
            let breadcrumb = app.breadcrumb();
            if breadcrumb.is_empty() {
                vec!["Official study material for Karnataka, CBSE and NCERT".to_owned()]
            } else {
                breadcrumb
            }
        }
        Screen::Level | Screen::Stream | Screen::Subject | Screen::Material | Screen::Browse => {
            let breadcrumb = app.breadcrumb();
            if breadcrumb.is_empty() {
                vec![app.screen.title().to_owned()]
            } else {
                breadcrumb
            }
        }
        Screen::Queue => vec!["Downloads".to_owned()],
        Screen::Settings => vec!["Settings".to_owned()],
        Screen::Help => vec!["Help".to_owned()],
    }
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let columns = Layout::horizontal([Constraint::Min(20), Constraint::Length(30)]).split(area);

    let (text, style) = if app.status.is_empty() {
        (app.status_summary(), theme.dim_style())
    } else if app.status_error {
        (app.status.clone(), theme.error_style())
    } else {
        (app.status.clone(), theme.accent_style())
    };

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(format!(" {text}"), style))),
        columns[0],
    );

    if let Some(hints) = persistent_hints(app) {
        frame.render_widget(
            Paragraph::new(hint_line(&hints, theme, "   ")).alignment(Alignment::Right),
            columns[1],
        );
    }
}

fn persistent_hints(app: &App) -> Option<Vec<(&'static str, &'static str)>> {
    if app.screen == Screen::Help {
        return None;
    }
    if app.screen == Screen::Browse && (app.search_active || app.year_picker_open) {
        return None;
    }
    if app.screen == Screen::Queue && !app.queue.conflicts.is_empty() {
        return None;
    }
    if app.screen == Screen::Settings && app.settings.editing {
        return None;
    }

    if app.screen == Screen::Settings {
        Some(vec![("?", "help"), ("q", "quit")])
    } else {
        Some(vec![("?", "help"), ("s", "settings"), ("q", "quit")])
    }
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let hints = footer_hints(app);
    frame.render_widget(Paragraph::new(hint_line(&hints, theme, "   ")), area);
}

fn hint_line(
    hints: &[(&'static str, &'static str)],
    theme: &Theme,
    separator: &'static str,
) -> Line<'static> {
    let mut spans = Vec::with_capacity(hints.len() * 4);

    for (index, (key, label)) in hints.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(separator, theme.dim_style()));
        }
        spans.push(Span::styled(*key, theme.key_style()));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(*label, theme.dim_style()));
    }

    Line::from(spans)
}

fn footer_hints(app: &App) -> Vec<(&'static str, &'static str)> {
    if app.screen == Screen::Help {
        return vec![("Esc", "close"), ("q", "close")];
    }

    if app.screen == Screen::Settings && app.settings.editing {
        return vec![("Enter", "save"), ("Esc", "cancel"), ("Ctrl-U", "clear")];
    }

    if app.screen == Screen::Queue && !app.queue.conflicts.is_empty() {
        return vec![
            ("s", "skip"),
            ("r", "rename"),
            ("o", "overwrite"),
            ("c", "cancel"),
        ];
    }

    if app.screen == Screen::Browse {
        if app.search_active {
            return vec![
                ("Type", "to filter"),
                ("Enter", "apply"),
                ("Esc", "cancel"),
                ("Up/Down", "move"),
            ];
        }
        if app.year_picker_open {
            return vec![("j/k", "move"), ("Enter", "apply"), ("Esc", "cancel")];
        }

        return vec![
            ("Space", "select"),
            ("d", "download"),
            ("/", "search"),
            ("y", "year"),
            ("t", "latest"),
            ("D", "queue"),
            ("Esc", "back"),
        ];
    }

    match app.screen {
        Screen::Board | Screen::Level | Screen::Stream | Screen::Subject | Screen::Material => {
            vec![
                ("j/k", "move"),
                ("Enter", "select"),
                ("Esc", "back"),
                ("D", "downloads"),
            ]
        }
        Screen::Queue => vec![
            ("j/k", "move"),
            ("p", "pause"),
            ("r", "retry"),
            ("c", "cancel"),
            ("Esc", "back"),
        ],
        Screen::Settings => vec![
            ("j/k", "move"),
            ("Enter", "edit"),
            ("Left/Right", "adjust"),
            ("Esc", "back"),
        ],
        Screen::Splash | Screen::Help | Screen::Browse => Vec::new(),
    }
}

/// Draw a dim horizontal rule across `area`.
pub fn rule(frame: &mut Frame, area: Rect, theme: &Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let line = "─".repeat(area.width as usize);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(line, theme.border_style()))),
        area,
    );
}

fn draw_too_small(frame: &mut Frame, area: Rect, theme: &Theme) {
    let message = format!(
        "Terminal too small\npu-get needs at least {MIN_WIDTH}x{MIN_HEIGHT}\ncurrent: {}x{}\n\nResize the window, or press q to quit",
        area.width, area.height
    );

    frame.render_widget(
        Paragraph::new(message)
            .alignment(Alignment::Center)
            .style(theme.error_style()),
        area,
    );
}

/// Rounded panel with one column of horizontal breathing room.
#[must_use]
pub fn block(title: &str, theme: &Theme) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style())
        .padding(Padding::new(1, 1, 0, 0))
        .title(Span::styled(format!(" {title} "), theme.title_style()))
}

/// Centre a fixed-size rectangle inside `area`, clamped to fit.
#[must_use]
pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width.saturating_sub(2)).max(1);
    let height = height.min(area.height.saturating_sub(2)).max(1);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width, height)
}

/// First visible row for a cursor-driven list, keeping the cursor on screen.
#[must_use]
pub fn window_start(cursor: usize, len: usize, height: usize) -> usize {
    if height == 0 || len <= height {
        return 0;
    }

    let cursor = cursor.min(len - 1);
    let start = cursor.saturating_sub(height - 1);
    start.min(len - height)
}

/// Selected-row marker.
#[must_use]
pub fn selected_marker(selected: bool) -> &'static str {
    if selected {
        "› "
    } else {
        "  "
    }
}

/// Human-readable binary byte count.
#[must_use]
pub fn human_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;

    let value = bytes as f64;
    if value >= GIB {
        format!("{:.1} GiB", value / GIB)
    } else if value >= MIB {
        format!("{:.1} MiB", value / MIB)
    } else if value >= KIB {
        format!("{:.1} KiB", value / KIB)
    } else {
        format!("{bytes} B")
    }
}

/// Group thousands, for counts such as 6,844.
#[must_use]
pub fn format_count(value: usize) -> String {
    let digits = value.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);

    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            formatted.push(',');
        }
        formatted.push(character);
    }

    formatted
}

/// Transfer speed, or `-` when the sample has no useful rate yet.
#[must_use]
pub fn format_speed(bytes_per_second: f64) -> String {
    if !bytes_per_second.is_finite() || bytes_per_second <= 0.0 {
        "-".to_owned()
    } else {
        format!("{}/s", human_bytes(bytes_per_second.round() as u64))
    }
}

/// Compact ETA.
#[must_use]
pub fn format_eta(eta: Option<Duration>) -> String {
    let Some(eta) = eta else {
        return "-".to_owned();
    };

    let seconds = eta.as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3_600 {
        format!("{:02}:{:02}", seconds / 60, seconds % 60)
    } else {
        format!(
            "{:02}:{:02}:{:02}",
            seconds / 3_600,
            (seconds % 3_600) / 60,
            seconds % 60
        )
    }
}

/// Filled and total cells for a text progress bar.
#[must_use]
pub fn progress_parts(fraction: f64, width: usize) -> (usize, usize) {
    let width = width.max(1);
    let filled = (fraction.clamp(0.0, 1.0) * width as f64).round() as usize;
    (filled.min(width), width)
}

/// Style used for unselected rows.
#[must_use]
pub fn plain_style() -> Style {
    Style::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::sample_app;
    use crate::catalog::MaterialType;
    use crate::download::{DownloadEvent, JobId};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn render(app: &App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    fn browse_app() -> App {
        let mut app = sample_app();
        app.selection.board = Some("kseeb".to_owned());
        app.selection.level = Some("SSLC".to_owned());
        app.selection.subject = Some("Mathematics".to_owned());
        app.selection.kind = Some(MaterialType::PreviousYearPaper);
        app.screen = Screen::Browse;
        app
    }

    #[test]
    fn splash_shows_logo_and_version() {
        let mut app = sample_app();
        app.screen = Screen::Splash;
        let screen = render(&app, 80, 24);
        assert!(screen.contains("pu-get"));
        assert!(screen.contains("v0.0.1"));
    }

    #[test]
    fn board_screen_lists_curated_boards() {
        let app = sample_app();
        let screen = render(&app, 80, 24);
        assert!(screen.contains("Boards"));
        assert!(screen.contains("Karnataka State Board"));
        assert!(screen.contains("Karnataka Pre-University"));
        assert!(screen.contains("PUC 1"));
        assert!(screen.contains("D downloads"));
    }

    #[test]
    fn level_screen_uses_friendly_level_labels() {
        let mut app = sample_app();
        app.screen = Screen::Level;
        app.selection.board = Some("kseeb".to_owned());
        let screen = render(&app, 80, 24);
        assert!(screen.contains("Levels"));
        assert!(screen.contains("SSLC (Class 10)"));
    }

    #[test]
    fn browse_screen_renders_the_pdf_table() {
        let app = browse_app();
        let screen = render(&app, 80, 24);
        assert!(screen.contains("PDFs"));
        assert!(screen.contains("SSLC Mathematics Previous Year Paper"));
        assert!(screen.contains("KSEAB"));
        assert!(screen.contains("Latest"));
        assert!(screen.contains("D queue"));
        assert!(screen.contains("q quit"));
    }

    #[test]
    fn queue_screen_renders_progress_rows() {
        let mut app = sample_app();
        app.screen = Screen::Queue;
        app.handle_download_event(DownloadEvent::Queued {
            id: JobId(1),
            title: "Physics Textbook".to_owned(),
        });
        app.handle_download_event(DownloadEvent::Progress {
            id: JobId(1),
            progress: crate::download::ProgressSnapshot {
                downloaded: 512 * 1024,
                total: Some(1024 * 1024),
                bytes_per_second: 256.0 * 1024.0,
                eta: Some(Duration::from_secs(2)),
            },
        });

        let screen = render(&app, 80, 24);
        assert!(screen.contains("Downloads"));
        assert!(screen.contains("Queue"));
        assert!(screen.contains("Physics Textbook"));
        assert!(screen.contains("50%"));
    }

    #[test]
    fn retry_rows_use_compact_messages() {
        let mut app = sample_app();
        app.screen = Screen::Queue;
        let id = JobId(9);
        app.handle_download_event(DownloadEvent::Queued {
            id,
            title: "Physics Textbook".to_owned(),
        });
        app.handle_download_event(DownloadEvent::Retrying {
            id,
            attempt: 2,
            max_attempts: 4,
            delay: Duration::from_secs(1),
            error: "request timed out after 60 second(s)".to_owned(),
        });

        let screen = render(&app, 80, 24);
        assert!(screen.contains("retrying"));
        assert!(screen.contains("Timed out"));
        assert!(screen.contains("retry 2/4 in 1s"));
        assert!(!screen.contains("60 second(s)"));
    }

    #[test]
    fn help_is_a_centred_overlay() {
        let mut app = sample_app();
        app.open_help();
        let screen = render(&app, 80, 24);
        assert!(screen.contains("Keyboard shortcuts"));
        assert!(screen.contains("fuzzy search"));
    }

    #[test]
    fn settings_lists_editable_fields() {
        let mut app = sample_app();
        app.screen = Screen::Settings;
        let screen = render(&app, 80, 24);
        assert!(screen.contains("Settings"));
        assert!(screen.contains("DOWNLOADS"));
        assert!(screen.contains("PREFERENCES"));
        assert!(screen.contains("Download directory"));
        assert!(screen.contains("Folder template"));
    }

    #[test]
    fn too_small_terminals_get_a_clear_message() {
        let app = sample_app();
        let screen = render(&app, 60, 18);
        assert!(screen.contains("Terminal too small"));
        assert!(screen.contains("80x24"));
    }

    #[test]
    fn window_start_keeps_the_cursor_visible() {
        assert_eq!(window_start(0, 100, 10), 0);
        assert_eq!(window_start(9, 100, 10), 0);
        assert_eq!(window_start(10, 100, 10), 1);
        assert_eq!(window_start(99, 100, 10), 90);
        assert_eq!(window_start(5, 3, 10), 0);
        assert_eq!(window_start(5, 10, 0), 0);
    }

    #[test]
    fn formatting_helpers_are_stable() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1024), "1.0 KiB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MiB");
        assert_eq!(format_count(6_844), "6,844");
        assert_eq!(format_count(391), "391");
        assert_eq!(format_speed(0.0), "-");
        assert_eq!(format_eta(None), "-");
        assert_eq!(format_eta(Some(Duration::from_secs(42))), "42s");
        assert_eq!(format_eta(Some(Duration::from_secs(125))), "02:05");
        assert_eq!(format_eta(Some(Duration::from_secs(3_725))), "01:02:05");
        assert_eq!(progress_parts(0.5, 10), (5, 10));
        assert_eq!(progress_parts(2.0, 10), (10, 10));
    }
}
