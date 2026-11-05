//! [`Screen`] on top of alacritty_terminal's `Term`.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{Config, Term, TermDamage, TermMode};
use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor, Processor};

use crate::backend::{Screen, ScreenEvent};
use crate::model::{
    CellPos, Color, CursorPos, Dirty, Modes, RenderRow, Run, SelectionKind, Style, TermSize,
};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Collects what the emulator asks for while parsing.
#[derive(Clone, Default)]
struct Listener {
    events: Arc<Mutex<Vec<ScreenEvent>>>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let mapped = match event {
            Event::Title(title) => Some(ScreenEvent::Title(title)),
            Event::ResetTitle => Some(ScreenEvent::Title(String::new())),
            Event::PtyWrite(text) => Some(ScreenEvent::Reply(text.into_bytes())),
            _ => None,
        };
        if let Some(event) = mapped {
            lock(&self.events).push(event);
        }
    }
}

/// Grid size as alacritty_terminal wants it.
struct GridSize {
    columns: usize,
    lines: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.lines
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

impl From<TermSize> for GridSize {
    fn from(size: TermSize) -> Self {
        Self {
            columns: usize::from(size.cols),
            lines: usize::from(size.rows),
        }
    }
}

/// An alacritty_terminal `Term` and its parser.
pub struct AlacrittyScreen {
    term: Term<Listener>,
    parser: Processor,
    listener: Listener,
    size: TermSize,
    all_dirty: bool,
}

impl AlacrittyScreen {
    /// A screen of `size` keeping `scrollback` lines of history.
    pub fn new(size: TermSize, scrollback: usize) -> Self {
        let listener = Listener::default();
        let config = Config {
            scrolling_history: scrollback,
            ..Config::default()
        };
        Self {
            term: Term::new(config, &GridSize::from(size), listener.clone()),
            parser: Processor::new(),
            listener,
            size,
            all_dirty: true,
        }
    }

    /// The grid line shown on viewport `row`.
    fn line(&self, row: u16) -> Line {
        let offset = i32::try_from(self.term.grid().display_offset()).unwrap_or(0);
        Line(i32::from(row) - offset)
    }

    fn point(&self, cell: CellPos) -> Point {
        let last = usize::from(self.size.cols.saturating_sub(1));
        Point::new(self.line(cell.row), Column(usize::from(cell.col).min(last)))
    }
}

fn color(color: AnsiColor) -> Color {
    match color {
        AnsiColor::Spec(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
        AnsiColor::Indexed(index) => Color::Indexed(index),
        AnsiColor::Named(named) => {
            let value = named as usize;
            let dim_black = NamedColor::DimBlack as usize;
            let dim_white = NamedColor::DimWhite as usize;
            if value < 16 {
                Color::Indexed(u8::try_from(value).unwrap_or(7))
            } else if (dim_black..=dim_white).contains(&value) {
                Color::Indexed(u8::try_from(value - dim_black).unwrap_or(7))
            } else {
                // Foreground, background, cursor and their bright/dim forms.
                Color::Default
            }
        }
    }
}

fn style(cell: &Cell, selected: bool) -> Style {
    let flags = cell.flags;
    Style {
        fg: color(cell.fg),
        bg: color(cell.bg),
        bold: flags.contains(Flags::BOLD),
        dim: flags.contains(Flags::DIM),
        italic: flags.contains(Flags::ITALIC),
        underline: flags.intersects(Flags::ALL_UNDERLINES),
        strike: flags.contains(Flags::STRIKEOUT),
        inverse: flags.contains(Flags::INVERSE),
        hidden: flags.contains(Flags::HIDDEN),
        selected,
        wide: flags.contains(Flags::WIDE_CHAR),
    }
}

fn is_spacer(cell: &Cell) -> bool {
    cell.flags
        .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
}

impl Screen for AlacrittyScreen {
    fn feed(&mut self, bytes: &[u8]) -> Vec<ScreenEvent> {
        self.parser.advance(&mut self.term, bytes);
        std::mem::take(&mut *lock(&self.listener.events))
    }

    fn resize(&mut self, size: TermSize) {
        self.size = size;
        self.term.resize(GridSize::from(size));
        self.all_dirty = true;
    }

    fn take_dirty(&mut self) -> Dirty {
        let dirty = if self.all_dirty {
            Dirty::All
        } else {
            match self.term.damage() {
                TermDamage::Full => Dirty::All,
                TermDamage::Partial(lines) => Dirty::Rows(
                    lines
                        .map(|damage| u16::try_from(damage.line).unwrap_or(u16::MAX))
                        .collect(),
                ),
            }
        };
        self.term.reset_damage();
        self.all_dirty = false;
        dirty
    }

    fn render_row(&self, row: u16) -> RenderRow {
        let line = self.line(row);
        let selection = self
            .term
            .selection
            .as_ref()
            .and_then(|s| s.to_range(&self.term));
        let cells = &self.term.grid()[line];
        let mut runs: Vec<Run> = Vec::new();
        for col in 0..usize::from(self.size.cols) {
            let cell = &cells[Column(col)];
            if is_spacer(cell) {
                continue;
            }
            let selected = selection
                .as_ref()
                .is_some_and(|range| range.contains(Point::new(line, Column(col))));
            let style = style(cell, selected);
            let ch = if cell.c == '\0' { ' ' } else { cell.c };
            match runs.last_mut() {
                Some(run) if run.style == style && !style.wide => run.text.push(ch),
                _ => runs.push(Run {
                    text: ch.to_string(),
                    style,
                }),
            }
        }
        RenderRow { row, runs }
    }

    fn row_text(&self, row: u16) -> String {
        let cells = &self.term.grid()[self.line(row)];
        (0..usize::from(self.size.cols))
            .map(|col| {
                let cell = &cells[Column(col)];
                if is_spacer(cell) || cell.c == '\0' {
                    ' '
                } else {
                    cell.c
                }
            })
            .collect()
    }

    fn cursor(&self) -> Option<CursorPos> {
        if !self.term.mode().contains(TermMode::SHOW_CURSOR) {
            return None;
        }
        let point = self.term.grid().cursor.point;
        let offset = i32::try_from(self.term.grid().display_offset()).unwrap_or(0);
        let row = u16::try_from(point.line.0 + offset).ok()?;
        (row < self.size.rows).then(|| CursorPos {
            row,
            col: u16::try_from(point.column.0).unwrap_or(0),
        })
    }

    fn scroll(&mut self, lines: i32) {
        self.term.scroll_display(Scroll::Delta(lines));
        self.all_dirty = true;
    }

    fn scroll_to_bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
        self.all_dirty = true;
    }

    fn scroll_state(&self) -> (u32, u32) {
        let grid = self.term.grid();
        (
            u32::try_from(grid.display_offset()).unwrap_or(u32::MAX),
            u32::try_from(grid.history_size()).unwrap_or(u32::MAX),
        )
    }

    fn modes(&self) -> Modes {
        let mode = self.term.mode();
        Modes {
            app_cursor: mode.contains(TermMode::APP_CURSOR),
            bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
            alt_screen: mode.contains(TermMode::ALT_SCREEN),
        }
    }

    fn select(&mut self, anchor: CellPos, head: CellPos, kind: SelectionKind) {
        let ty = match kind {
            SelectionKind::Simple => SelectionType::Simple,
            SelectionKind::Word => SelectionType::Semantic,
            SelectionKind::Line => SelectionType::Lines,
        };
        let (a, h) = (self.point(anchor), self.point(head));
        let forward = h >= a;
        let mut selection = Selection::new(ty, a, if forward { Side::Left } else { Side::Right });
        selection.update(h, if forward { Side::Right } else { Side::Left });
        self.term.selection = Some(selection);
        self.all_dirty = true;
    }

    fn clear_selection(&mut self) {
        self.term.selection = None;
        self.all_dirty = true;
    }

    fn selection_text(&self) -> Option<String> {
        self.term
            .selection_to_string()
            .filter(|text| !text.is_empty())
    }
}
