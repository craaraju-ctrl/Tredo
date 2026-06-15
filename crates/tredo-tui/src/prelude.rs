//! Shared re-exports used by all tredo-tui modules.
//! Each tab module just does `use crate::prelude::*;`.

pub(crate) use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, Paragraph, Wrap},
    Frame,
};
