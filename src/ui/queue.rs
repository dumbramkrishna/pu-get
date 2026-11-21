//! Download queue screen.

use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::state::{JobStatus, JobView};
use crate::app::App;
use crate::ui::{self, theme::Theme};

/// Draw the queue and any pending conflict prompt.
pub fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let block = ui::block("Queue", theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.queue.jobs.is_empty() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled("Queue is empty", theme.title_style())),
                Line::from(Span::styled(
                    "Pick PDFs with Space, then press d.",
                    theme.dim_style(),
                )),
            ])
            .alignment(Alignment::Center),
            ui::centered_rect(inner.width, 3, inner),
        );
    } else {
        draw_jobs(frame, inner, app, theme);
    }

    if !app.queue.conflicts.is_empty() {
        draw_conflict(frame, area, app, theme);
    }
}

fn draw_jobs(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let visible = ((area.height as usize + 1) / 3).max(1);
    let start = ui::window_start(app.queue.cursor, app.queue.jobs.len(), visible);
    let bar_width = ((area.width as usize) / 3).clamp(16, 30);
    let end = (start + visible).min(app.queue.jobs.len());

    let mut items: Vec<ListItem<'_>> = Vec::new();
    for (position, index) in (start..end).enumerate() {
        let job = &app.queue.jobs[index];
        let selected = index == app.queue.cursor;
        let title_style = if selected {
            theme.selected_text_style()
        } else {
            theme.text_style()
        };
        let secondary_style = if selected {
            theme.selected_secondary_style()
        } else {
            theme.dim_style()
        };
        let status_style = job_status_style(job.status, selected, theme);

        let progress = progress_tag(job);
        let first = Line::from(vec![
            Span::styled(ui::selected_marker(selected), theme.accent_style()),
            Span::styled(status_glyph(job.status), status_style),
            Span::raw(" "),
            Span::styled(format!("{:<9}", job.status.label()), status_style),
            Span::styled(
                progress,
                if selected {
                    theme.selected_secondary_style()
                } else {
                    theme.text_style()
                },
            ),
            Span::styled(job.title.clone(), title_style),
        ]);

        let (filled, total) = ui::progress_parts(job.progress.fraction(), bar_width);
        let second = Line::from(vec![
            Span::raw("     "),
            Span::styled("█".repeat(filled), theme.accent_style()),
            Span::styled("░".repeat(total - filled), secondary_style),
            Span::raw("  "),
            Span::styled(job_detail(job), secondary_style),
        ]);

        items.push(ListItem::new(vec![first, second]).style(if selected {
            theme.selected_style()
        } else {
            ui::plain_style()
        }));

        if position + 1 < visible && index + 1 < app.queue.jobs.len() {
            items.push(ListItem::new(Line::from("")));
        }
    }

    frame.render_widget(List::new(items), area);
}

fn progress_tag(job: &JobView) -> String {
    let Some(total) = job.progress.total else {
        return "      ".to_owned();
    };

    if total == 0 {
        return "      ".to_owned();
    }

    format!("{:>3.0}%  ", job.progress.fraction() * 100.0)
}

fn job_detail(job: &JobView) -> String {
    match job.status {
        JobStatus::Running => {
            let size = job.progress.total.map_or_else(
                || format!("{} downloaded", ui::human_bytes(job.progress.downloaded)),
                |total| {
                    format!(
                        "{} / {}",
                        ui::human_bytes(job.progress.downloaded),
                        ui::human_bytes(total)
                    )
                },
            );
            format!(
                "{size}  ·  {}  ·  ETA {}",
                ui::format_speed(job.progress.bytes_per_second),
                ui::format_eta(job.progress.eta)
            )
        }
        JobStatus::Queued => "Waiting for a free slot".to_owned(),
        JobStatus::Paused => format!(
            "{} downloaded  ·  p to resume",
            ui::human_bytes(job.progress.downloaded)
        ),
        JobStatus::Retrying => job
            .note
            .clone()
            .unwrap_or_else(|| "Waiting to retry".to_owned()),
        JobStatus::Conflict => "Waiting for your decision".to_owned(),
        JobStatus::Completed => format!("{}  ·  saved", ui::human_bytes(job.progress.downloaded)),
        JobStatus::Failed => match &job.note {
            Some(note) => format!("{note}  ·  r to retry"),
            None => "Failed  ·  r to retry".to_owned(),
        },
        JobStatus::Skipped | JobStatus::Cancelled => job
            .note
            .clone()
            .unwrap_or_else(|| job.status.label().to_owned()),
    }
}

fn status_glyph(status: JobStatus) -> &'static str {
    match status {
        JobStatus::Queued => "○",
        JobStatus::Running => "●",
        JobStatus::Paused => "‖",
        JobStatus::Retrying => "↻",
        JobStatus::Conflict => "!",
        JobStatus::Completed => "✓",
        JobStatus::Failed => "✕",
        JobStatus::Skipped => "–",
        JobStatus::Cancelled => "✕",
    }
}

fn job_status_style(status: JobStatus, selected: bool, theme: &Theme) -> Style {
    match status {
        JobStatus::Failed | JobStatus::Conflict => theme.error_style(),
        JobStatus::Completed => theme.success_style(),
        JobStatus::Running | JobStatus::Retrying => theme.active_style(),
        JobStatus::Queued | JobStatus::Paused | JobStatus::Skipped | JobStatus::Cancelled => {
            if selected {
                theme.selected_secondary_style()
            } else {
                theme.dim_style()
            }
        }
    }
}

fn draw_conflict(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let Some(prompt) = app.queue.conflicts.front() else {
        return;
    };

    let title = app
        .queue
        .jobs
        .iter()
        .find(|job| job.id == prompt.id)
        .map_or("Download", |job| job.title.as_str());

    let height = 10.min(area.height.saturating_sub(2));
    let rect = ui::centered_rect(68, height, area);
    frame.render_widget(Clear, rect);

    let block = ui::block(
        &format!("Resolve conflict · {} waiting", app.queue.conflicts.len()),
        theme,
    );
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let expected = prompt
        .expected_size
        .map_or_else(|| "unknown".to_owned(), ui::human_bytes);

    let lines = vec![
        Line::from(Span::styled(title, theme.text_style())),
        Line::from(Span::styled(
            prompt.path.display().to_string(),
            theme.dim_style(),
        )),
        Line::from(Span::styled(
            format!(
                "existing {}  ·  expected {expected}",
                ui::human_bytes(prompt.existing_size)
            ),
            theme.dim_style(),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("s", theme.key_style()),
            Span::styled(" skip   ", theme.dim_style()),
            Span::styled("r", theme.key_style()),
            Span::styled(" rename   ", theme.dim_style()),
            Span::styled("o", theme.key_style()),
            Span::styled(" overwrite   ", theme.dim_style()),
            Span::styled("c", theme.key_style()),
            Span::styled(" cancel", theme.dim_style()),
        ]),
    ];

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
