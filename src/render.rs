//! Terminal rendering module.
//!
//! This module provides [`TerminalRenderer`], which handles efficient rendering of
//! terminal content using GPUI's text and drawing systems.
//!
//! # Rendering Pipeline
//!
//! The renderer processes the terminal grid in two phases per row:
//!
//! ```text
//! Terminal Grid → layout_row()                  → paint loop
//!                  │                                │
//!                  ├─ Collect background quads      ├─ Default background
//!                  │                                │
//!                  ├─ Batch cells into runs:        ├─ Per-row backgrounds
//!                  │   break on style change,       │   (HiDPI pixel-snapped)
//!                  │   column gap, complex cell,    │
//!                  │   or decorative cell           ├─ One shape_line(...) per run
//!                  │                                │   with force_width = cell_width
//!                  └─ Flag each run with            │   for monospace runs
//!                     force_cell_width              │
//!                                                   └─ Cursor (HiDPI pixel-snapped)
//! ```
//!
//! # Box-drawing connectivity
//!
//! Box-drawing glyphs (U+2500–U+257F) only connect cleanly across cells when each
//! glyph is placed at exactly `cell_width * column_index`. The renderer achieves
//! this by passing `Some(cell_width)` as the `force_width` argument to GPUI's
//! [`shape_line`](gpui::WindowTextSystem::shape_line). GPUI then clamps every
//! glyph's x position to `glyph_index * cell_width`, eliminating the sub-pixel
//! drift that would otherwise leave gaps between corner / line / T-junction
//! characters in TUIs like `htop`, `vim`, and `lazygit`.
//!
//! Forcing is applied per *run*, not globally:
//!
//! - **Forced runs** (`force_cell_width = true`): single-codepoint cells with
//!   identical styling — ASCII, box-drawing, most program output, and most
//!   nerdfont icons (Devicons, Font Awesome, Codicons, Material, etc.). Each
//!   glyph is pinned to the cell grid.
//! - **Natural-advance runs** (`force_cell_width = false`): runs containing
//!   wide chars (CJK), grapheme clusters with combining marks, ZWJ emoji
//!   sequences, or Powerline glyphs (U+E0A0..=U+E0D7 — full Powerline +
//!   Powerline-Extra block). Shaped at the font's natural advance to avoid
//!   clipping multi-cell or oversized glyphs.
//!
//! Lone-Powerline runs are an exception: they're isolated into a single-cell
//! run and force-clamped separately during paint so the surrounding ASCII run
//! can stay forced and the powerline glyph still aligns with the cell grid.
//! This is what makes Powerline pills (e.g. U+E0B6 cap + colored bg cell)
//! work — the cap glyph is positioned exactly at the column boundary.
//!
//! # Vertical alignment
//!
//! Vertical alignment of full-em decorative glyphs (Powerline pill caps,
//! similar nerdfont separators) against adjacent background quads is the
//! subtlest part of the renderer. The invariant we want is:
//!
//! > The glyph's `padding_top` (the gap GPUI inserts between the row's top
//! > and the glyph's ink top) is **uniform across every row** at every font
//! > size and HiDPI scale factor.
//!
//! Without that invariant, alternating rows draw their pill cap glyph slightly
//! above or below the bg quad, which the eye reads as a flickering or
//! mis-aligned pill at certain sizes/zoom factors.
//!
//! GPUI computes `padding_top = (line_height - ascent - descent) / 2` inside
//! [`ShapedLine::paint`](gpui::ShapedLine::paint) — see
//! `~/.cargo/registry/.../gpui-0.2.2/src/text_system/line.rs:207`. Whatever
//! `line_height` we pass becomes the centring height. We achieve uniformity
//! by snapping `cell_height` once at the top of [`TerminalRenderer::paint`]
//! to the nearest physical pixel, and then deriving every row's geometry
//! from that single snapped value.
//!
//! Concretely: [`TerminalRenderer::measure_cell`] stores
//! `cell_height = (ascent + descent) * line_height_multiplier` *without*
//! rounding. At the start of `paint` we compute
//! `snapped_cell_height = snap_pixels_round(cell_height, scale_factor)`,
//! which makes `snapped_cell_height * scale_factor` an integer count of
//! physical pixels. Row N's top is then
//! `snapped_origin_y + snapped_cell_height * N` — both factors land on
//! physical-pixel boundaries, so every row top does too, with no per-row
//! floor-rounding drift. The bg quad height *is* `snapped_cell_height`, and
//! the same value is passed as `line_height` to
//! [`ShapedLine::paint`](gpui::ShapedLine::paint). GPUI's `padding_top`
//! therefore evaluates to a single value across every row,
//! `(snapped_cell_height - ascent - descent) / 2`, bounded by half a
//! physical pixel in absolute value (because round-to-nearest minimises the
//! deviation in both directions).
//!
//! On the X axis adjacent quads can have *different* colours (different fg
//! cells), so we accept ±1 physical pixel overlap (floor / ceil) rather than
//! risk a sub-pixel gap. On the Y axis adjacent rows stack continuously and
//! all have the same snapped height — uniform stacking with neither overlap
//! nor gap, regardless of font size or scale factor.
//!
//! # Optimizations
//!
//! 1. **Background Merging**: Adjacent cells with the same background color are
//!    merged into single rectangles, reducing the number of quads to paint.
//!
//! 2. **Text Batching**: Adjacent cells with identical styling are grouped into
//!    [`BatchedTextRun`]s and shaped *once per run* (rather than once per cell)
//!    via [`gpui::WindowTextSystem::shape_line`].
//!
//! 3. **Default Background Skip**: Cells with the default background color don't
//!    generate separate background rectangles — the full-bounds default-background
//!    quad covers them.
//!
//! 4. **Cell Measurement**: Font metrics are measured once using the '│' (BOX
//!    DRAWINGS LIGHT VERTICAL) character and cached for consistent cell dimensions.
//!
//! 5. **HiDPI Pixel Snapping**: All four edges of background quads, the cursor,
//!    and per-run text origins are snapped to physical-pixel boundaries against
//!    [`Window::scale_factor`](gpui::Window::scale_factor). X uses floor / ceil
//!    (overlap-tolerant for differently-coloured neighbours); Y derives a
//!    once-per-paint `snapped_cell_height` (round-to-nearest physical pixel)
//!    and stacks every row at integer multiples of it — uniform rows, see
//!    "Vertical alignment" above.
//!
//! # Cell Dimensions
//!
//! Cell size is calculated from actual font metrics using the '│' character,
//! which spans the full cell height in properly designed terminal fonts:
//!
//! - **Width**: Measured from shaped '│' character
//! - **Height**: `(ascent + descent) × line_height_multiplier`
//!
//! The `line_height_multiplier` (default 1.0) can be adjusted to add extra
//! vertical space if needed for specific fonts.
//!
//! # Example
//!
//! ```ignore
//! use gpui::px;
//! use gpui_terminal::{ColorPalette, TerminalRenderer};
//!
//! let renderer = TerminalRenderer::new(
//!     "JetBrains Mono".to_string(),
//!     px(14.0),
//!     1.0,  // line height multiplier
//!     ColorPalette::default(),
//! );
//! ```

use crate::colors::ColorPalette;
use crate::event::GpuiEventProxy;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point as AlacPoint};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::Color;
use gpui::{
    App, Bounds, Edges, Font, FontFeatures, FontStyle, FontWeight, Hsla, Pixels, Point,
    SharedString, Size, TextRun, UnderlineStyle, Window, px, quad, transparent_black,
};

/// Snap a logical pixel value down to the nearest physical pixel boundary.
///
/// Used for the *left* edge of background quads / cursor / run origins so adjacent
/// quads share a physical-pixel boundary on HiDPI displays. Without this, fractional
/// `cell_width` causes adjacent backgrounds to bleed through with thin gaps or
/// overlap by a sub-pixel.
#[inline]
fn snap_pixels_floor(value: Pixels, scale_factor: f32) -> Pixels {
    if !(scale_factor.is_finite() && scale_factor > 0.0) {
        return value.floor();
    }
    let scaled: f32 = f32::from(value) * scale_factor;
    px(scaled.floor() / scale_factor)
}

/// Snap a logical pixel value up to the nearest physical pixel boundary.
///
/// Used for the *right* edge of background quads / cursor so they meet the next
/// quad's snapped left edge with no sub-pixel gap.
#[inline]
fn snap_pixels_ceil(value: Pixels, scale_factor: f32) -> Pixels {
    if !(scale_factor.is_finite() && scale_factor > 0.0) {
        return value.ceil();
    }
    let scaled: f32 = f32::from(value) * scale_factor;
    px(scaled.ceil() / scale_factor)
}

/// Snap a logical pixel value to the *nearest* physical pixel boundary
/// (banker's rounding via `round`).
///
/// Used to snap `cell_height` in the paint pass: rounding to nearest minimises
/// the maximum padding error between the glyph's natural ink extent
/// (`ascent + descent`) and the row height passed to GPUI's text paint as
/// `line_height`. See "Vertical alignment" in the module docs for why this
/// matters for Powerline pill caps.
#[inline]
fn snap_pixels_round(value: Pixels, scale_factor: f32) -> Pixels {
    if !(scale_factor.is_finite() && scale_factor > 0.0) {
        return px(f32::from(value).round());
    }
    let scaled: f32 = f32::from(value) * scale_factor;
    px(scaled.round() / scale_factor)
}

/// Returns `true` for Unicode codepoints in the Powerline / Symbols-for-Powerline
/// private-use range (U+E0A0–U+E0D7).
///
/// Covers the full Powerline + Powerline-Extra block:
///
/// - **U+E0A0..=U+E0A3** — branch, line-number, column-number glyphs.
/// - **U+E0B0..=U+E0B7** — original triangles / rounded separators.
/// - **U+E0B8..=U+E0BF** — Powerline-Extra trapezoids.
/// - **U+E0C0..=U+E0C8, U+E0CA, U+E0CC..=U+E0D7** — Powerline-Extra hex / pixel
///   patterns / honeycomb / IO-style separators.
///
/// These glyphs (pill caps, triangle/circle separators, etc. used by Powerline,
/// Starship, oh-my-posh and similar) are typically rendered with a foreground
/// color *different from* the surrounding text (matching the next cell's
/// background to construct a "pill"). Style change alone would break the run,
/// but classifying them as decorative also gives them their own single-cell run
/// — the paint loop then force-clamps that lone run to `cell_width` so the
/// glyph aligns to the cell grid even though `force_cell_width` is `false` on
/// the run itself.
///
/// Other nerdfont families (Devicons, Font Awesome, Codicons, Material, etc.)
/// are *not* recognised as decorative: they are typically designed at one
/// em-width and share the surrounding text's foreground color, so they go
/// through the regular forced-width path which gives them better column
/// alignment.
#[inline]
fn is_powerline_char(ch: char) -> bool {
    matches!(ch as u32, 0xE0A0..=0xE0D7)
}

/// A batched run of text with consistent styling.
///
/// This struct groups adjacent terminal cells with identical visual attributes
/// to reduce the number of text rendering calls.
#[derive(Debug, Clone)]
pub struct BatchedTextRun {
    /// The text content to render
    pub text: String,

    /// Starting column position
    pub start_col: usize,

    /// Number of grid cells the run spans (excluding wide-char spacer cells).
    ///
    /// This is independent of `text.chars().count()`: a complex grapheme cluster
    /// (emoji with VS16, ZWJ sequence) appends multiple `char`s into `text` while
    /// occupying a single cell.
    pub cell_count: usize,

    /// Row position
    pub row: usize,

    /// Foreground color
    pub fg_color: Hsla,

    /// Background color
    pub bg_color: Hsla,

    /// Bold flag
    pub bold: bool,

    /// Italic flag
    pub italic: bool,

    /// Underline flag
    pub underline: bool,

    /// Whether each glyph in this run should be clamped to exactly `cell_width`
    /// during shaping (via GPUI's `shape_line(force_width=...)`).
    ///
    /// `true` for runs containing only single-codepoint, non-decorative cells —
    /// this is what makes box-drawing characters connect cleanly across cells.
    /// `false` for runs containing emoji / ZWJ sequences / wide chars / powerline
    /// glyphs, which are rendered at their natural advance.
    pub force_cell_width: bool,
}

/// Background rectangle to paint.
///
/// Represents a rectangular region with a solid color background.
#[derive(Debug, Clone)]
pub struct BackgroundRect {
    /// Starting column position
    pub start_col: usize,

    /// Ending column position (exclusive)
    pub end_col: usize,

    /// Row position
    pub row: usize,

    /// Background color
    pub color: Hsla,
}

impl BackgroundRect {
    /// Check if this rectangle can be merged with another.
    ///
    /// Two rectangles can be merged if they:
    /// - Are on the same row
    /// - Have the same color
    /// - Are horizontally adjacent
    fn can_merge_with(&self, other: &Self) -> bool {
        self.row == other.row && self.color == other.color && self.end_col == other.start_col
    }
}

/// Terminal renderer with font settings and cell dimensions.
///
/// This struct manages the rendering of terminal content, including text,
/// backgrounds, and cursor. It maintains font metrics and provides the
/// [`paint`](Self::paint) method for drawing the terminal grid.
///
/// # Font Metrics
///
/// Cell dimensions are calculated from actual font measurements via
/// [`measure_cell`](Self::measure_cell). This ensures accurate character
/// positioning regardless of the font used.
///
/// # Usage
///
/// The renderer is typically used internally by [`TerminalView`](crate::TerminalView),
/// but can also be used directly for custom rendering:
///
/// ```ignore
/// // Measure cell dimensions (call once per font change)
/// renderer.measure_cell(window);
///
/// // Paint the terminal grid
/// renderer.paint(bounds, padding, &term, window, cx);
/// ```
///
/// # Performance
///
/// For optimal performance:
/// - Call `measure_cell` only when font settings change
/// - The `paint` method is designed to be called every frame
/// - Background and text batching minimize GPU draw calls
#[derive(Clone)]
pub struct TerminalRenderer {
    /// Font family name (e.g., "Fira Code", "Menlo")
    pub font_family: String,

    /// Font size in pixels
    pub font_size: Pixels,

    /// Width of a single character cell
    pub cell_width: Pixels,

    /// Height of a single character cell (line height)
    pub cell_height: Pixels,

    /// Multiplier for line height to accommodate tall glyphs
    pub line_height_multiplier: f32,

    /// Color palette for resolving terminal colors
    pub palette: ColorPalette,
}

impl TerminalRenderer {
    /// Creates a new terminal renderer with the given font settings and color palette.
    ///
    /// # Arguments
    ///
    /// * `font_family` - The name of the font family to use
    /// * `font_size` - The font size in pixels
    /// * `line_height_multiplier` - Multiplier for line height (e.g., 1.2 for 20% extra)
    /// * `palette` - The color palette to use for terminal colors
    ///
    /// # Returns
    ///
    /// A new `TerminalRenderer` instance with default cell dimensions.
    ///
    /// # Examples
    ///
    /// ```
    /// use gpui::px;
    /// use gpui_terminal::render::TerminalRenderer;
    /// use gpui_terminal::ColorPalette;
    ///
    /// let renderer = TerminalRenderer::new("Fira Code".to_string(), px(14.0), 1.0, ColorPalette::default());
    /// ```
    pub fn new(
        font_family: String,
        font_size: Pixels,
        line_height_multiplier: f32,
        palette: ColorPalette,
    ) -> Self {
        // Default cell dimensions - will be measured on first paint
        // Using 0.6 as approximate em-width ratio for monospace fonts
        let cell_width = font_size * 0.6;
        let cell_height = font_size * 1.4; // Line height with some spacing

        Self {
            font_family,
            font_size,
            cell_width,
            cell_height,
            line_height_multiplier,
            palette,
        }
    }

    /// Measure cell dimensions based on actual font metrics.
    ///
    /// This method measures the actual width and height of characters
    /// using the GPUI text system. It uses the '│' (BOX DRAWINGS LIGHT VERTICAL)
    /// character which spans the full cell height in properly designed terminal fonts.
    ///
    /// # Arguments
    ///
    /// * `window` - The GPUI window for text system access
    pub fn measure_cell(&mut self, window: &mut Window) {
        // Measure using '│' (U+2502, BOX DRAWINGS LIGHT VERTICAL)
        // This character spans the full cell height in terminal fonts, making it
        // ideal for measuring exact cell dimensions used by TUIs
        let font = Font {
            family: self.font_family.clone().into(),
            features: FontFeatures::default(),
            fallbacks: None,
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
        };

        let text_run = TextRun {
            len: "│".len(),
            font,
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };

        // Shape the box-drawing character to get cell metrics
        let shaped = window
            .text_system()
            .shape_line("│".into(), self.font_size, &[text_run], None);

        // Get the width from the shaped line
        if shaped.width > px(0.0) {
            self.cell_width = shaped.width;
        }

        // Calculate height from ascent + descent with optional multiplier.
        //
        // We deliberately do *not* round / ceil here. Using the exact metric
        // means `cell_height` matches the glyph's natural ink extent (a full-em
        // glyph fills exactly one cell), which is what makes Powerline pills
        // and similar full-em decorative glyphs visually align with adjacent
        // background quads. Per-row Y pixel snapping in `paint` handles the
        // physical-pixel quantization separately.
        let raw_line_height = shaped.ascent + shaped.descent;
        if raw_line_height > px(0.0) {
            // `.max(px(1.0))` is a sanity floor for fonts that report tiny
            // metrics — avoids degenerate zero-height cells.
            self.cell_height = (raw_line_height * self.line_height_multiplier).max(px(1.0));
        }
    }

    /// Layout cells into batched text runs and background rects for a single row.
    ///
    /// Processes a row of terminal cells and groups adjacent cells into runs that
    /// can each be shaped in a single [`shape_line`](gpui::WindowTextSystem::shape_line)
    /// call. Also collects merged background rectangles.
    ///
    /// # Run-breaking rules
    ///
    /// A new run starts whenever any of the following changes between cells:
    ///
    /// - **Style** — fg/bg color, bold, italic, or underline differ.
    /// - **Column gap** — the next cell's column isn't `prev_col + 1`. This is what
    ///   isolates wide-char glyphs (the [`WIDE_CHAR_SPACER`](Flags::WIDE_CHAR_SPACER)
    ///   between them is skipped, leaving a gap of 1).
    /// - **Complex cell** — any cell containing combining marks (zerowidth) or
    ///   flagged [`WIDE_CHAR`](Flags::WIDE_CHAR). Each complex cell becomes its
    ///   own natural-advance run so the grapheme cluster shapes correctly.
    /// - **Decorative cell** — any Powerline-range glyph (U+E0B0..=U+E0D7).
    ///   Isolated so the surrounding ASCII run can stay forced-width.
    ///
    /// The output run's `force_cell_width` flag is `true` only when the run is
    /// free of complex *and* decorative cells.
    ///
    /// # Arguments
    ///
    /// * `row` - The row number
    /// * `cells` - Iterator over (column, [`Cell`]) pairs
    /// * `colors` - Terminal color configuration for resolving [`Color::Indexed`]
    ///
    /// # Returns
    ///
    /// `(backgrounds, text_runs)`:
    /// - `backgrounds` — merged background rectangles for non-default cells.
    /// - `text_runs` — batched text runs ready for per-run shaping in `paint`.
    ///
    /// [`Color::Indexed`]: alacritty_terminal::vte::ansi::Color::Indexed
    pub fn layout_row(
        &self,
        row: usize,
        cells: impl Iterator<Item = (usize, Cell)>,
        colors: &Colors,
    ) -> (Vec<BackgroundRect>, Vec<BatchedTextRun>) {
        let mut backgrounds = Vec::new();
        let mut text_runs = Vec::new();

        let mut current_run: Option<BatchedTextRun> = None;
        let mut current_bg: Option<BackgroundRect> = None;

        // Per-run tracking for complex / decorative cells. These are flushed into
        // `force_cell_width` when the run is closed.
        let mut current_contains_complex = false;
        let mut current_contains_decorative = false;

        // The column at which the next cell must appear to be eligible for the
        // current run. A mismatch (gap, e.g. caused by a wide-char spacer being
        // skipped, or by selection slicing) breaks the run.
        let mut next_expected_column: Option<usize> = None;

        for (col, cell) in cells {
            // Skip wide character spacers — they leave a column gap that breaks
            // the run on the following cell, so the wide-char run stands alone.
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }

            // Extract cell styling.
            let fg_color = self.palette.resolve(cell.fg, colors);
            let bg_color = self.palette.resolve(cell.bg, colors);
            let bold = cell.flags.contains(Flags::BOLD);
            let italic = cell.flags.contains(Flags::ITALIC);
            let underline = cell.flags.contains(Flags::UNDERLINE);

            // Classify the cell.
            //
            // - "Complex" = contains combining marks (zerowidth) or is a wide char.
            //   Such cells get their natural glyph advance instead of force_width,
            //   because clamping a wide-char or emoji-cluster glyph to a single
            //   cell width would clip it.
            // - "Decorative" = a Powerline-range glyph (U+E0B0..=U+E0D7). These
            //   get their own forced single-cell run so adjacent ASCII stays
            //   forced-width and the decorative glyph aligns with the cell grid.
            let has_zerowidth = cell.zerowidth().is_some_and(|z| !z.is_empty());
            let cell_is_complex = has_zerowidth || cell.flags.contains(Flags::WIDE_CHAR);
            let cell_is_decorative = is_powerline_char(cell.c);

            // Get the primary character (substitute space for the alacritty
            // empty-cell sentinel '\0').
            let primary_ch = if cell.c == '\0' { ' ' } else { cell.c };

            // Handle background rectangles (unchanged from previous logic).
            if let Some(ref mut bg_rect) = current_bg {
                if bg_rect.color == bg_color && bg_rect.end_col == col {
                    bg_rect.end_col = col + 1;
                } else {
                    backgrounds.push(bg_rect.clone());
                    current_bg = Some(BackgroundRect {
                        start_col: col,
                        end_col: col + 1,
                        row,
                        color: bg_color,
                    });
                }
            } else {
                current_bg = Some(BackgroundRect {
                    start_col: col,
                    end_col: col + 1,
                    row,
                    color: bg_color,
                });
            }

            // Decide whether to break the current run.
            let style_changed = current_run.as_ref().is_some_and(|run| {
                run.fg_color != fg_color
                    || run.bg_color != bg_color
                    || run.bold != bold
                    || run.italic != italic
                    || run.underline != underline
            });
            let gap_breaks_run = next_expected_column.is_some_and(|expected| expected != col);
            let complex_breaks_run = current_contains_complex || cell_is_complex;
            let decorative_breaks_run = current_contains_decorative || cell_is_decorative;

            let should_break = current_run.is_some()
                && (style_changed
                    || gap_breaks_run
                    || complex_breaks_run
                    || decorative_breaks_run);

            if should_break {
                if let Some(mut run) = current_run.take() {
                    run.force_cell_width =
                        !current_contains_complex && !current_contains_decorative;
                    text_runs.push(run);
                }
                current_contains_complex = false;
                current_contains_decorative = false;
            }

            // Extend or start a new run.
            if let Some(ref mut run) = current_run {
                run.text.push(primary_ch);
                if let Some(zw) = cell.zerowidth() {
                    for &combining in zw {
                        run.text.push(combining);
                    }
                }
                run.cell_count += 1;
            } else {
                let mut text = String::new();
                text.push(primary_ch);
                if let Some(zw) = cell.zerowidth() {
                    for &combining in zw {
                        text.push(combining);
                    }
                }
                current_run = Some(BatchedTextRun {
                    text,
                    start_col: col,
                    cell_count: 1,
                    row,
                    fg_color,
                    bg_color,
                    bold,
                    italic,
                    underline,
                    // Placeholder; finalized when the run is closed.
                    force_cell_width: false,
                });
            }

            current_contains_complex |= cell_is_complex;
            current_contains_decorative |= cell_is_decorative;

            // Wide chars take 2 visual columns but are stored as 1 cell with a
            // following WIDE_CHAR_SPACER. Setting expected = col + 1 means the
            // skipped spacer at col + 1 produces a gap (col + 1 expected vs.
            // col + 2 actual on the next visible cell), which breaks the run —
            // exactly what we want, since wide chars are already complex.
            next_expected_column = Some(col + 1);
        }

        // Finalize and push the trailing run.
        if let Some(mut run) = current_run.take() {
            run.force_cell_width = !current_contains_complex && !current_contains_decorative;
            text_runs.push(run);
        }
        if let Some(bg) = current_bg {
            backgrounds.push(bg);
        }

        // Merge adjacent backgrounds with same color.
        let merged_backgrounds = self.merge_backgrounds(backgrounds);

        (merged_backgrounds, text_runs)
    }

    /// Merge adjacent background rects with same color.
    ///
    /// This optimization reduces the number of rectangles to paint by
    /// combining horizontally adjacent rectangles that share the same color.
    ///
    /// # Arguments
    ///
    /// * `rects` - Vector of background rectangles to merge
    ///
    /// # Returns
    ///
    /// A new vector with merged rectangles
    fn merge_backgrounds(&self, mut rects: Vec<BackgroundRect>) -> Vec<BackgroundRect> {
        if rects.is_empty() {
            return rects;
        }

        let mut merged = Vec::new();
        let mut current = rects.remove(0);

        for rect in rects {
            if current.can_merge_with(&rect) {
                current.end_col = rect.end_col;
            } else {
                merged.push(current);
                current = rect;
            }
        }

        merged.push(current);
        merged
    }

    /// Paint terminal content to the window.
    ///
    /// Draws, in order: the default background covering the full `bounds`
    /// (including padding); then for each row, via
    /// [`layout_row`](Self::layout_row), the merged non-default background
    /// quads followed by one
    /// [`shape_line`](gpui::WindowTextSystem::shape_line) + paint per
    /// [`BatchedTextRun`] (forced runs pass `Some(cell_width)` as
    /// `force_width` so each glyph lands at exactly `column * cell_width` —
    /// that's what makes box-drawing glyphs connect cleanly; lone Powerline
    /// runs are also force-clamped here); and finally the cursor quad.
    ///
    /// At the start of paint, `cell_height` is snapped once to the nearest
    /// physical pixel against
    /// [`Window::scale_factor`](gpui::Window::scale_factor); the resulting
    /// `snapped_cell_height` is used uniformly for every row's top, every
    /// bg quad's height, and the `line_height` argument to
    /// [`ShapedLine::paint`](gpui::ShapedLine::paint). Background and cursor
    /// X edges use floor / ceil snapping (overlap-tolerant for differently
    /// coloured neighbours).
    ///
    /// # Arguments
    ///
    /// * `bounds` - The bounding box to render within
    /// * `padding` - Padding between `bounds` and the cell grid
    /// * `term` - The terminal state
    /// * `window` - The GPUI window
    /// * `cx` - The application context
    pub fn paint(
        &self,
        bounds: Bounds<Pixels>,
        padding: Edges<Pixels>,
        term: &Term<GpuiEventProxy>,
        window: &mut Window,
        cx: &mut App,
    ) {
        // Get terminal dimensions
        let grid = term.grid();
        let num_lines = grid.screen_lines();
        let num_cols = grid.columns();
        let colors = term.colors();

        // Calculate default background color
        let default_bg = self.palette.resolve(
            Color::Named(alacritty_terminal::vte::ansi::NamedColor::Background),
            colors,
        );

        // Paint default background (covers full bounds including padding)
        window.paint_quad(quad(
            bounds,
            px(0.0),
            default_bg,
            Edges::<Pixels>::default(),
            transparent_black(),
            Default::default(),
        ));

        // Calculate origin offset (content starts after padding)
        let origin = Point {
            x: bounds.origin.x + padding.left,
            y: bounds.origin.y + padding.top,
        };

        // Cache the window's logical-to-physical scale factor for pixel snapping.
        let scale_factor = window.scale_factor();

        // Snap `cell_height` once to the nearest physical pixel. This is the
        // key to uniform vertical alignment of decorative glyphs (Powerline
        // pill caps, etc.) across every row at every font size / zoom factor.
        //
        // Why round-to-nearest (and not floor or ceil)?
        //   - With unrounded `cell_height`, per-row floor produces row_heights
        //     that alternate between `floor(cell_height)` and `ceil(cell_height)`
        //     in physical pixels. Passing those varying values as `line_height`
        //     to `ShapedLine.paint` makes GPUI's `padding_top` flip sign between
        //     rows, so pill cap glyphs visibly "stick above" on some rows and
        //     "fall short" on others.
        //   - Round-to-nearest gives a single uniform `line_height`, bounding
        //     `|padding_top|` to ≤ ½ a physical pixel — uniformly across
        //     every row, every font size, every scale factor.
        //
        // We also snap `origin.y` itself so that
        // `snapped_origin_y + snapped_cell_height * N` lands on a physical
        // pixel for every row index `N`.
        let snapped_cell_height =
            snap_pixels_round(self.cell_height, scale_factor).max(px(1.0));
        let snapped_origin_y = snap_pixels_floor(origin.y, scale_factor);

        // Iterate over visible lines.
        for line_idx in 0..num_lines {
            let line = Line(line_idx as i32);

            // Collect cells for this line.
            let cells: Vec<(usize, Cell)> = (0..num_cols)
                .map(|col_idx| {
                    let col = Column(col_idx);
                    let point = AlacPoint::new(line, col);
                    let cell = grid[point].clone();
                    (col_idx, cell)
                })
                .collect();

            // Layout the row into background rectangles and styled text runs.
            let (backgrounds, text_runs) =
                self.layout_row(line_idx, cells.iter().cloned(), colors);

            // Uniform per-row math: every row top is on a physical-pixel
            // boundary, and every row has the same height. There is no
            // "drift" between adjacent rows.
            let y_top = snapped_origin_y + snapped_cell_height * (line_idx as f32);
            let row_height = snapped_cell_height;

            // Paint backgrounds with hidpi-aware pixel snapping so adjacent
            // quads share a physical-pixel boundary and don't bleed/overlap.
            for bg_rect in backgrounds {
                if bg_rect.color == default_bg {
                    continue;
                }

                let raw_start_x = origin.x + self.cell_width * (bg_rect.start_col as f32);
                let raw_end_x = origin.x + self.cell_width * (bg_rect.end_col as f32);
                let start_x = snap_pixels_floor(raw_start_x, scale_factor);
                let end_x = snap_pixels_ceil(raw_end_x, scale_factor);
                let width = (end_x - start_x).max(px(0.0));

                let rect_bounds = Bounds {
                    origin: Point {
                        x: start_x,
                        y: y_top,
                    },
                    size: Size {
                        width,
                        height: row_height,
                    },
                };

                window.paint_quad(quad(
                    rect_bounds,
                    px(0.0),
                    bg_rect.color,
                    Edges::<Pixels>::default(),
                    transparent_black(),
                    Default::default(),
                ));
            }

            // Paint styled text runs.
            //
            // For runs marked `force_cell_width` we pass `Some(self.cell_width)`
            // as `shape_line`'s `force_width` argument. GPUI clamps every glyph's
            // x position to `glyph_index * force_width`, which makes monospace
            // box-drawing glyphs connect cleanly across cells. Complex /
            // decorative runs (emoji, ZWJ sequences, wide chars, powerline
            // glyphs) are shaped at natural advance.
            //
            // The text origin's Y uses the snapped `y_top` from above so glyph
            // ink and the bg quad on the same row share a physical-pixel
            // anchor — that's what makes Powerline pills and other full-em
            // decorative glyphs visually meet adjacent backgrounds without
            // sticking above or falling short.
            for run in text_runs {
                if run.text.is_empty() {
                    continue;
                }

                let font = Font {
                    family: self.font_family.clone().into(),
                    features: FontFeatures::default(),
                    fallbacks: None,
                    weight: if run.bold {
                        FontWeight::BOLD
                    } else {
                        FontWeight::NORMAL
                    },
                    style: if run.italic {
                        FontStyle::Italic
                    } else {
                        FontStyle::Normal
                    },
                };

                let text_run = TextRun {
                    len: run.text.len(),
                    font,
                    color: run.fg_color,
                    background_color: None,
                    underline: if run.underline {
                        Some(UnderlineStyle {
                            thickness: px(1.0),
                            color: Some(run.fg_color),
                            wavy: false,
                        })
                    } else {
                        None
                    },
                    strikethrough: None,
                };

                // Powerline glyphs sit alone in a single-cell run; force-clamp
                // them to cell width even though the run was flagged decorative.
                let is_lone_powerline = run.cell_count == 1
                    && run.text.chars().count() == 1
                    && run
                        .text
                        .chars()
                        .next()
                        .is_some_and(is_powerline_char);
                let force = run.force_cell_width || is_lone_powerline;
                let force_width = if force { Some(self.cell_width) } else { None };

                let text: SharedString = run.text.clone().into();
                let shaped_line =
                    window
                        .text_system()
                        .shape_line(text, self.font_size, &[text_run], force_width);

                let run_origin_x = origin.x + self.cell_width * (run.start_col as f32);
                // Forced runs are already aligned to integer cells; emoji /
                // wide-char runs are floored to a whole pixel to avoid drift.
                let run_x = if force {
                    run_origin_x
                } else {
                    run_origin_x.floor()
                };

                // Origin Y is the row top (no separate `vertical_offset`):
                // GPUI computes `padding_top = (line_height - ascent - descent) / 2`
                // internally (line.rs:207) and uses that for centring, so any
                // extra space from `line_height_multiplier > 1.0` is centred
                // automatically when we pass `snapped_cell_height` as
                // `line_height`. Adding our own offset here would double-count
                // and shift the glyph below its row.
                //
                // `line_height` = `snapped_cell_height` is what makes
                // `padding_top` uniform across every row — see "Vertical
                // alignment" in the module docs for the full reasoning.
                let _ = shaped_line.paint(
                    Point {
                        x: run_x,
                        y: y_top,
                    },
                    snapped_cell_height,
                    window,
                    cx,
                );
            }
        }

        // Paint cursor with pixel-snapped edges on all four sides.
        //
        // X uses floor / ceil (allow ±1 physical pixel overlap with
        // neighbours rather than risk a sub-pixel gap). Y derives from the
        // same `snapped_origin_y` + `snapped_cell_height * row` formula used
        // for row tops, so the cursor's vertical span exactly matches one
        // row's bg quad span — both edges are already on physical pixels.
        let cursor_point = grid.cursor.point;
        let raw_cursor_x = origin.x + self.cell_width * (cursor_point.column.0 as f32);
        let raw_cursor_end_x = raw_cursor_x + self.cell_width;
        let cursor_x = snap_pixels_floor(raw_cursor_x, scale_factor);
        let cursor_end_x = snap_pixels_ceil(raw_cursor_end_x, scale_factor);

        let cursor_y = snapped_origin_y + snapped_cell_height * (cursor_point.line.0 as f32);
        let cursor_y_bot = cursor_y + snapped_cell_height;

        let cursor_color = self.palette.resolve(
            Color::Named(alacritty_terminal::vte::ansi::NamedColor::Cursor),
            colors,
        );

        let cursor_bounds = Bounds {
            origin: Point {
                x: cursor_x,
                y: cursor_y,
            },
            size: Size {
                width: (cursor_end_x - cursor_x).max(px(0.0)),
                height: (cursor_y_bot - cursor_y).max(px(0.0)),
            },
        };

        window.paint_quad(quad(
            cursor_bounds,
            px(0.0),
            cursor_color,
            Edges::<Pixels>::default(),
            transparent_black(),
            Default::default(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_renderer_creation() {
        let renderer = TerminalRenderer::new(
            "Fira Code".to_string(),
            px(14.0),
            1.0,
            ColorPalette::default(),
        );
        assert_eq!(renderer.font_family, "Fira Code");
        assert_eq!(renderer.font_size, px(14.0));
        assert_eq!(renderer.line_height_multiplier, 1.0);
    }

    #[test]
    fn test_background_rect_merge() {
        let black = Hsla::black();

        let rect1 = BackgroundRect {
            start_col: 0,
            end_col: 5,
            row: 0,
            color: black,
        };

        let rect2 = BackgroundRect {
            start_col: 5,
            end_col: 10,
            row: 0,
            color: black,
        };

        assert!(rect1.can_merge_with(&rect2));

        let rect3 = BackgroundRect {
            start_col: 5,
            end_col: 10,
            row: 1,
            color: black,
        };

        assert!(!rect1.can_merge_with(&rect3));
    }

    #[test]
    fn test_merge_backgrounds() {
        let renderer = TerminalRenderer::new(
            "monospace".to_string(),
            px(14.0),
            1.0,
            ColorPalette::default(),
        );
        let black = Hsla::black();

        let rects = vec![
            BackgroundRect {
                start_col: 0,
                end_col: 5,
                row: 0,
                color: black,
            },
            BackgroundRect {
                start_col: 5,
                end_col: 10,
                row: 0,
                color: black,
            },
        ];

        let merged = renderer.merge_backgrounds(rects);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].start_col, 0);
        assert_eq!(merged[0].end_col, 10);
    }

    /// Mirrors arbor's `positioned_runs_keep_cell_width_for_box_drawing_symbols`
    /// (terminal_rendering.rs:1611-1632). Adjacent box-drawing and ASCII cells
    /// with identical styling must collapse into one run flagged with
    /// `force_cell_width = true`, so GPUI's `shape_line(..., force_width=...)`
    /// clamps every glyph to the cell grid and the lines connect cleanly.
    #[test]
    fn layout_row_keeps_cell_width_for_box_drawing_symbols() {
        let renderer = TerminalRenderer::new(
            "monospace".to_string(),
            px(14.0),
            1.0,
            ColorPalette::default(),
        );
        let colors = Colors::default();

        let cell_pipe = Cell {
            c: '\u{2502}', // │ BOX DRAWINGS LIGHT VERTICAL
            ..Cell::default()
        };
        let cell_x = Cell {
            c: 'X',
            ..Cell::default()
        };
        let cells = vec![(0usize, cell_pipe), (1usize, cell_x)];

        let (_bgs, runs) = renderer.layout_row(0, cells.into_iter(), &colors);

        assert_eq!(runs.len(), 1, "box-drawing + ascii must merge into one run");
        assert_eq!(runs[0].text, "\u{2502}X");
        assert!(
            runs[0].force_cell_width,
            "merged box-drawing run should be force-clamped to cell width"
        );
        assert_eq!(runs[0].cell_count, 2);
        assert_eq!(runs[0].start_col, 0);
    }

    /// Powerline glyphs (U+E0B0..=U+E0D7) are decorative — they break the
    /// surrounding ASCII run so their natural advance is preserved, and the
    /// paint loop force-clamps the lone-powerline run on its own. Mirrors
    /// arbor's `force_cell_width_disabled_for_powerline_runs` test.
    #[test]
    fn layout_row_isolates_powerline_glyph_from_ascii_run() {
        let renderer = TerminalRenderer::new(
            "monospace".to_string(),
            px(14.0),
            1.0,
            ColorPalette::default(),
        );
        let colors = Colors::default();

        let cell_pl = Cell {
            c: '\u{e0b0}', // powerline right-arrow
            ..Cell::default()
        };
        let cell_x = Cell {
            c: 'X',
            ..Cell::default()
        };
        let cells = vec![(0usize, cell_pl), (1usize, cell_x)];

        let (_bgs, runs) = renderer.layout_row(0, cells.into_iter(), &colors);

        assert_eq!(runs.len(), 2, "powerline + ascii must split into two runs");
        assert_eq!(runs[0].text, "\u{e0b0}");
        assert!(
            !runs[0].force_cell_width,
            "decorative run is not forced at the run level — paint loop forces lone-powerline runs"
        );
        assert_eq!(runs[1].text, "X");
        assert!(runs[1].force_cell_width);
    }

    /// Wide chars (e.g. CJK ideographs) occupy two cells: the WIDE_CHAR cell
    /// followed by a WIDE_CHAR_SPACER. The spacer is skipped, which leaves a
    /// column gap that breaks the surrounding run. The wide char itself is
    /// classified as complex so its glyph is shaped at natural advance.
    #[test]
    fn layout_row_breaks_run_around_wide_char() {
        let renderer = TerminalRenderer::new(
            "monospace".to_string(),
            px(14.0),
            1.0,
            ColorPalette::default(),
        );
        let colors = Colors::default();

        let cell_a = Cell {
            c: 'A',
            ..Cell::default()
        };
        let cell_wide = Cell {
            c: '漢',
            flags: Flags::WIDE_CHAR,
            ..Cell::default()
        };
        let cell_spacer = Cell {
            flags: Flags::WIDE_CHAR_SPACER,
            ..Cell::default()
        };
        let cell_b = Cell {
            c: 'B',
            ..Cell::default()
        };
        let cells = vec![
            (0usize, cell_a),
            (1usize, cell_wide),
            (2usize, cell_spacer),
            (3usize, cell_b),
        ];

        let (_bgs, runs) = renderer.layout_row(0, cells.into_iter(), &colors);

        // Expect 3 runs: 'A' (forced) | '漢' (complex, not forced) | 'B' (forced).
        assert_eq!(runs.len(), 3);

        assert_eq!(runs[0].text, "A");
        assert_eq!(runs[0].start_col, 0);
        assert!(runs[0].force_cell_width);

        assert_eq!(runs[1].text, "漢");
        assert_eq!(runs[1].start_col, 1);
        assert!(
            !runs[1].force_cell_width,
            "wide-char run must not be force-clamped — its glyph spans two cells"
        );

        assert_eq!(runs[2].text, "B");
        assert_eq!(
            runs[2].start_col, 3,
            "post-wide-char run must start at the spacer's *next* column (not the spacer itself)"
        );
        assert!(runs[2].force_cell_width);
    }

    /// Combining marks (zerowidth) flagged via `Cell::push_zerowidth` mark a
    /// cell as complex so its primary char + combining marks are shaped as a
    /// grapheme cluster at natural advance. Tests the zerowidth detection
    /// branch in `layout_row` (alacritty_terminal Cell::zerowidth).
    #[test]
    fn layout_row_breaks_run_on_combining_marks() {
        let renderer = TerminalRenderer::new(
            "monospace".to_string(),
            px(14.0),
            1.0,
            ColorPalette::default(),
        );
        let colors = Colors::default();

        let cell_a = Cell {
            c: 'A',
            ..Cell::default()
        };
        // 'e' + U+0301 (combining acute accent) → 'é' as a 2-codepoint cluster.
        let mut cell_e_acute = Cell {
            c: 'e',
            ..Cell::default()
        };
        cell_e_acute.push_zerowidth('\u{0301}');

        let cells = vec![(0usize, cell_a), (1usize, cell_e_acute)];

        let (_bgs, runs) = renderer.layout_row(0, cells.into_iter(), &colors);

        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].text, "A");
        assert!(runs[0].force_cell_width);
        assert_eq!(runs[1].text, "e\u{0301}");
        assert!(
            !runs[1].force_cell_width,
            "combining-mark cluster must not be force-clamped — it's shaped as one grapheme"
        );
        assert_eq!(runs[1].cell_count, 1, "complex cell still occupies one cell");
    }

    /// `is_powerline_char` covers the full Powerline + Powerline-Extra block
    /// (U+E0A0..=U+E0D7) and rejects adjacent ranges and unrelated codepoints.
    #[test]
    fn powerline_range_recognition() {
        // Powerline (branch / line-num / col-num glyphs).
        assert!(is_powerline_char('\u{e0a0}')); // branch
        assert!(is_powerline_char('\u{e0a1}')); // line number
        assert!(is_powerline_char('\u{e0a2}')); // padlock
        assert!(is_powerline_char('\u{e0a3}')); // column number

        // Powerline original separators.
        assert!(is_powerline_char('\u{e0b0}')); // right triangle
        assert!(is_powerline_char('\u{e0b6}')); // rounded left semicircle (pill cap)

        // Powerline-Extra trapezoids / hex / pixel patterns.
        assert!(is_powerline_char('\u{e0bc}'));
        assert!(is_powerline_char('\u{e0c0}'));
        assert!(is_powerline_char('\u{e0d4}'));
        assert!(is_powerline_char('\u{e0d7}'));

        // Adjacent ranges and unrelated codepoints.
        assert!(!is_powerline_char('\u{e09f}')); // just below
        assert!(!is_powerline_char('\u{e0d8}')); // just above
        assert!(!is_powerline_char('\u{e700}')); // Devicons (force-clamped, not isolated)
        assert!(!is_powerline_char('\u{f000}')); // Font Awesome
        assert!(!is_powerline_char('A'));
        assert!(!is_powerline_char('\u{2502}')); // box drawing
    }

    /// `snap_pixels_floor` rounds a logical pixel value down so the result
    /// lands on a physical pixel boundary at the given scale factor. At
    /// integer scale factors this is a no-op for already-integer logical
    /// pixels; at fractional scale factors it rounds toward 0.
    #[test]
    fn snap_floor_lands_on_physical_pixel() {
        // scale = 1.0: integer logical → integer physical, no change.
        assert_eq!(snap_pixels_floor(px(0.0), 1.0), px(0.0));
        assert_eq!(snap_pixels_floor(px(20.0), 1.0), px(20.0));
        assert_eq!(snap_pixels_floor(px(20.4), 1.0), px(20.0));

        // scale = 2.0: 20.5 logical = 41 physical → snaps to 41/2 = 20.5.
        // (already on a half-physical-pixel grid)
        assert_eq!(snap_pixels_floor(px(20.5), 2.0), px(20.5));
        // 20.4 logical = 40.8 physical → floor → 40 physical → 20.0 logical.
        assert_eq!(snap_pixels_floor(px(20.4), 2.0), px(20.0));

        // scale = 1.5: 19.0 logical = 28.5 physical → floor → 28 physical
        //   = 28/1.5 = 18.6666… logical.
        let snapped = snap_pixels_floor(px(19.0), 1.5);
        let expected_physical = (f32::from(snapped) * 1.5).round() as i32;
        assert_eq!(expected_physical, 28);
    }

    /// `snap_pixels_ceil` rounds up to the nearest physical pixel.
    #[test]
    fn snap_ceil_lands_on_physical_pixel() {
        assert_eq!(snap_pixels_ceil(px(0.0), 1.0), px(0.0));
        assert_eq!(snap_pixels_ceil(px(20.0), 1.0), px(20.0));
        assert_eq!(snap_pixels_ceil(px(20.4), 1.0), px(21.0));

        // scale = 1.5: 19.0 logical = 28.5 physical → ceil → 29 physical
        //   = 29/1.5 ≈ 19.333 logical.
        let snapped = snap_pixels_ceil(px(19.0), 1.5);
        let expected_physical = (f32::from(snapped) * 1.5).round() as i32;
        assert_eq!(expected_physical, 29);
    }

    /// `snap_pixels_round` rounds to the nearest physical pixel.
    #[test]
    fn snap_round_lands_on_physical_pixel() {
        // scale = 1.0: round to nearest integer logical pixel.
        assert_eq!(snap_pixels_round(px(20.0), 1.0), px(20.0));
        assert_eq!(snap_pixels_round(px(20.4), 1.0), px(20.0));
        assert_eq!(snap_pixels_round(px(20.6), 1.0), px(21.0));

        // scale = 1.5: round logical * 1.5 to nearest integer physical.
        // 19.7 logical = 29.55 physical → round → 30 physical → 30/1.5 = 20.0
        assert_eq!(snap_pixels_round(px(19.7), 1.5), px(20.0));
        // 19.4 logical = 29.1 physical → round → 29 physical → 29/1.5 ≈ 19.333
        let snapped = snap_pixels_round(px(19.4), 1.5);
        let expected_physical = (f32::from(snapped) * 1.5).round() as i32;
        assert_eq!(expected_physical, 29);

        // scale = 2.0: 19.4 logical = 38.8 physical → round → 39 physical
        //   = 39/2 = 19.5 logical.
        assert_eq!(snap_pixels_round(px(19.4), 2.0), px(19.5));
    }

    /// At a fractional scale factor and fractional `cell_height`, the
    /// production rule used in [`TerminalRenderer::paint`] —
    /// `snapped_origin_y + snapped_cell_height * line_idx` — yields uniform
    /// rows: every row top sits on a physical-pixel boundary, every row has
    /// the same height, and there is no floor-rounding drift between adjacent
    /// rows.
    ///
    /// This is the property that keeps `padding_top` (computed inside GPUI's
    /// [`ShapedLine::paint`](gpui::ShapedLine::paint) as
    /// `(line_height - ascent - descent) / 2`) constant across rows, which is
    /// what eliminates the "pill cap sticks above on some rows / falls short
    /// on others" artifact at certain font sizes / zoom factors.
    #[test]
    fn snapped_cell_height_yields_uniform_row_stack() {
        // Use a fractional cell height (mimics ascent + descent without
        // rounding) and a fractional scale factor (mimics 1.5x scaling).
        let cell_height = px(19.7);
        let scale = 1.5;
        let origin_y = px(0.0);

        let snapped_origin_y = snap_pixels_floor(origin_y, scale);
        let snapped_cell_height = snap_pixels_round(cell_height, scale);

        let mut prev_bottom: Option<Pixels> = None;
        for row in 0..50 {
            let y_top = snapped_origin_y + snapped_cell_height * (row as f32);
            let row_height = snapped_cell_height;
            let y_bot = y_top + row_height;

            // Row N's bottom is row N+1's top — continuous stack.
            if let Some(prev_b) = prev_bottom {
                assert_eq!(
                    prev_b, y_top,
                    "row {row} top does not match previous row's bottom"
                );
            }

            // Every row has the same height — uniform.
            assert_eq!(
                row_height, snapped_cell_height,
                "row {row} height differs from snapped cell height"
            );

            // Both edges are on physical pixel boundaries.
            let top_phys = f32::from(y_top) * scale;
            let bot_phys = f32::from(y_bot) * scale;
            assert!(
                (top_phys - top_phys.round()).abs() < 1e-3,
                "row {row} y_top {top_phys} not on a physical pixel"
            );
            assert!(
                (bot_phys - bot_phys.round()).abs() < 1e-3,
                "row {row} y_bot {bot_phys} not on a physical pixel"
            );

            prev_bottom = Some(y_bot);
        }
    }

    /// `padding_top` (GPUI's internal centring offset, computed as
    /// `(line_height - ascent - descent) / 2`) is bounded by half a physical
    /// pixel when we pass `snapped_cell_height` (round-to-nearest) as
    /// `line_height`. This is independent of font size and scale factor —
    /// the property that makes pill cap rendering uniform.
    #[test]
    fn snapped_cell_height_bounds_padding_top() {
        // Sweep a range of (cell_height, scale_factor) combinations.
        for &cell_height_raw in &[15.0_f32, 17.3, 18.0, 19.7, 21.5, 24.2, 28.0] {
            for &scale in &[1.0_f32, 1.25, 1.5, 1.75, 2.0, 2.5] {
                let cell_height = px(cell_height_raw);
                let snapped = snap_pixels_round(cell_height, scale);

                // |snapped_cell_height - cell_height| ≤ 0.5 / scale (logical px)
                let diff = (f32::from(snapped) - cell_height_raw).abs();
                let max_logical_error = 0.5 / scale + 1e-4;
                assert!(
                    diff <= max_logical_error,
                    "cell={cell_height_raw} scale={scale}: snapped {} differs by {diff} > {max_logical_error}",
                    f32::from(snapped),
                );

                // |padding_top| = |snapped - cell_height| / 2 ≤ 0.25/scale logical
                //              = 0.25 physical pixels (a quarter-physical-pixel)
                //              regardless of cell_height or scale_factor.
                let padding_top_physical = (diff / 2.0) * scale;
                assert!(
                    padding_top_physical <= 0.25 + 1e-4,
                    "cell={cell_height_raw} scale={scale}: padding_top {padding_top_physical} physical px > 0.25",
                );
            }
        }
    }
}
