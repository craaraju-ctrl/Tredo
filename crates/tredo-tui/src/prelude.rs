//! Shared re-exports used by all tredo-tui modules.
//! Each tab module just does `use crate::prelude::*;`.

pub(crate) use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, Paragraph, Wrap},
    Frame,
};

/// Shared loading spinner — shows a rotating animation based on elapsed time.
pub fn loading_spinner(now: std::time::Instant) -> &'static str {
    let phase = (now.elapsed().as_millis() / 250) % 4;
    match phase {
        0 => "◴",
        1 => "◷",
        2 => "◶",
        _ => "◵",
    }
}

/// Central color theme for the tredo TUI.
/// All tabs should reference `THEME.*` instead of hardcoded colors.
pub(crate) struct Theme {
    pub positive: Color,
    pub negative: Color,
    pub neutral: Color,
    pub brand: Color,
    pub muted: Color,
    pub highlight: Color,
    pub border: Color,
    pub _accent_border: Color,
    pub info: Color,
    pub warning: Color,
    pub danger: Color,
}

pub(crate) const THEME: Theme = Theme {
    positive: Color::Green,
    negative: Color::Red,
    neutral: Color::Yellow,
    brand: Color::Cyan,
    muted: Color::DarkGray,
    highlight: Color::White,
    border: Color::DarkGray,
    _accent_border: Color::Cyan,
    info: Color::Cyan,
    warning: Color::Yellow,
    danger: Color::Red,
};
