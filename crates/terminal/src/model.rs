//! Terminal data shared by the session, the backends and the view: sizes,
//! colors, styled runs of cells and frames.

/// Size of a terminal grid in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermSize {
    /// Columns.
    pub cols: u16,
    /// Rows.
    pub rows: u16,
}

impl TermSize {
    /// A size; zero dimensions are raised to 1.
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            cols: cols.max(1),
            rows: rows.max(1),
        }
    }
}

/// A cell color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    /// The theme's default foreground or background.
    #[default]
    Default,
    /// A palette color: 0-15 come from the theme, 16-255 are the xterm cube
    /// and gray ramp.
    Indexed(u8),
    /// A 24-bit color.
    Rgb(u8, u8, u8),
}

/// Text attributes of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    /// Foreground.
    pub fg: Color,
    /// Background.
    pub bg: Color,
    /// Bold.
    pub bold: bool,
    /// Dim.
    pub dim: bool,
    /// Italic.
    pub italic: bool,
    /// Underlined (any underline style).
    pub underline: bool,
    /// Struck through.
    pub strike: bool,
    /// Foreground and background swapped.
    pub inverse: bool,
    /// Hidden text.
    pub hidden: bool,
    /// Inside the selection.
    pub selected: bool,
    /// One double-width character (occupies two cells).
    pub wide: bool,
}

/// Consecutive cells with the same style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    /// The text (one character per cell; a wide character is one run).
    pub text: String,
    /// Its style.
    pub style: Style,
}

/// One visible row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderRow {
    /// Row in the viewport, 0 at the top.
    pub row: u16,
    /// Runs from the first column.
    pub runs: Vec<Run>,
}

/// Where the cursor is drawn, in viewport cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorPos {
    /// Row in the viewport.
    pub row: u16,
    /// Column.
    pub col: u16,
}

/// Terminal modes that change how input is encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modes {
    /// Cursor keys send `ESC O` sequences (DECCKM).
    pub app_cursor: bool,
    /// Pastes are wrapped in `ESC [200~ … ESC [201~`.
    pub bracketed_paste: bool,
    /// The alternate screen is active (`less`, `vim`).
    pub alt_screen: bool,
}

/// How a mouse selection grows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionKind {
    /// Character by character.
    Simple,
    /// Whole words.
    Word,
    /// Whole lines.
    Line,
}

/// A cell in the viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellPos {
    /// Row in the viewport.
    pub row: u16,
    /// Column.
    pub col: u16,
}

/// What changed on screen since the last frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dirty {
    /// Everything (resize, scroll, clear).
    All,
    /// Only these viewport rows.
    Rows(Vec<u16>),
}

/// An update for the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Terminal id.
    pub id: u32,
    /// Sequence number; the view acknowledges it.
    pub seq: u64,
    /// Grid size.
    pub size: TermSize,
    /// Whether `rows` holds every row (the view replaces all rows).
    pub full: bool,
    /// Changed rows.
    pub rows: Vec<RenderRow>,
    /// Cursor, if visible in the viewport.
    pub cursor: Option<CursorPos>,
    /// Lines scrolled back from the bottom (0 = following output).
    pub display_offset: u32,
    /// Lines of scrollback available.
    pub history: u32,
    /// Title set by the program (empty if none).
    pub title: String,
    /// Exit code once the shell has exited (`Some(None)` if unknown).
    pub exited: Option<Option<i32>>,
}
