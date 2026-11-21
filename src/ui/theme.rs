//! Colour palette and text styles.
//!
//! A single cyan accent carries focus and keys. Selection uses a quiet slate
//! background rather than a saturated fill, which keeps dense screens readable.
//! With `NO_COLOR` every colour becomes the terminal default and selection falls
//! back to reverse video.

use ratatui::style::{Color, Modifier, Style};

/// Visual palette for the whole TUI.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    /// Whether colour is enabled.
    pub enabled: bool,
    /// Primary accent.
    pub accent: Color,
    /// Normal text.
    pub text: Color,
    /// Secondary text.
    pub dim: Color,
    /// Panel borders and rules.
    pub border: Color,
    /// Selected-row foreground.
    pub selected_fg: Color,
    /// Selected-row background.
    pub selected_bg: Color,
    /// Errors.
    pub error: Color,
    /// Successful completion.
    pub success: Color,
}

impl Theme {
    /// Build a palette, honouring `NO_COLOR`.
    #[must_use]
    pub fn new(enabled: bool) -> Self {
        if enabled {
            Self {
                enabled,
                accent: Color::Cyan,
                text: Color::Reset,
                dim: Color::DarkGray,
                border: Color::DarkGray,
                selected_fg: Color::White,
                selected_bg: Color::DarkGray,
                error: Color::LightRed,
                success: Color::LightGreen,
            }
        } else {
            Self {
                enabled,
                accent: Color::Reset,
                text: Color::Reset,
                dim: Color::Reset,
                border: Color::Reset,
                selected_fg: Color::Reset,
                selected_bg: Color::Reset,
                error: Color::Reset,
                success: Color::Reset,
            }
        }
    }

    /// Brand wordmark.
    #[must_use]
    pub fn brand_style(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    /// Accent text.
    #[must_use]
    pub fn accent_style(&self) -> Style {
        Style::default().fg(self.accent)
    }

    /// Key hints.
    #[must_use]
    pub fn key_style(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    /// Panel titles.
    #[must_use]
    pub fn title_style(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    /// Table and column headers.
    #[must_use]
    pub fn header_style(&self) -> Style {
        Style::default().fg(self.dim).add_modifier(Modifier::BOLD)
    }

    /// Plain body text.
    #[must_use]
    pub fn text_style(&self) -> Style {
        Style::default().fg(self.text)
    }

    /// De-emphasised text.
    #[must_use]
    pub fn dim_style(&self) -> Style {
        Style::default().fg(self.dim)
    }

    /// Panel border and horizontal rules.
    #[must_use]
    pub fn border_style(&self) -> Style {
        Style::default().fg(self.border)
    }

    /// Selected row.
    #[must_use]
    pub fn selected_style(&self) -> Style {
        if self.enabled {
            Style::default()
                .fg(self.selected_fg)
                .bg(self.selected_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().add_modifier(Modifier::REVERSED)
        }
    }

    /// Text placed on top of [`Self::selected_style`].
    #[must_use]
    pub fn selected_text_style(&self) -> Style {
        Style::default().add_modifier(Modifier::BOLD)
    }

    /// Secondary text that must stay readable on a selected row.
    #[must_use]
    pub fn selected_secondary_style(&self) -> Style {
        if self.enabled {
            Style::default().fg(self.selected_fg)
        } else {
            Style::default()
        }
    }

    /// Error text.
    #[must_use]
    pub fn error_style(&self) -> Style {
        Style::default().fg(self.error).add_modifier(Modifier::BOLD)
    }

    /// Success text.
    #[must_use]
    pub fn success_style(&self) -> Style {
        Style::default()
            .fg(self.success)
            .add_modifier(Modifier::BOLD)
    }

    /// Active transfer text.
    #[must_use]
    pub fn active_style(&self) -> Style {
        Style::default().fg(self.accent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_mode_uses_the_single_accent() {
        let theme = Theme::new(true);
        assert_eq!(theme.accent, Color::Cyan);
        assert_eq!(theme.selected_style().fg, Some(Color::White));
        assert_eq!(theme.selected_style().bg, Some(Color::DarkGray));
    }

    #[test]
    fn no_colour_mode_resets_colours_and_reverses_selection() {
        let theme = Theme::new(false);
        assert!(!theme.enabled);
        assert_eq!(theme.accent, Color::Reset);
        assert_eq!(theme.selected_style().bg, None);
        assert!(theme
            .selected_style()
            .add_modifier
            .contains(Modifier::REVERSED));
    }
}
