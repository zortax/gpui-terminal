//! What the renderer draws: a grid of cells.
//!
//! [`GridSource`] is implemented for alacritty's [`Term`], which is what
//! [`TerminalView`](crate::TerminalView) draws. It lets the renderer draw a
//! grid that lives somewhere else too, such as a terminal emulated in another
//! process or on another machine whose cells arrive over a connection.
//! [`CellGrid`] holds such a grid.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::term::color::Colors;

/// A grid of cells the renderer can draw.
pub trait GridSource {
    /// Number of rows on screen.
    fn screen_lines(&self) -> usize;

    /// Number of columns.
    fn columns(&self) -> usize;

    /// The cell at a row and column on screen.
    fn cell(&self, line: usize, column: usize) -> Cell;

    /// Colors set by the program, which override the palette.
    fn colors(&self) -> &Colors;

    /// The cursor's row and column, or `None` while it is hidden.
    fn cursor(&self) -> Option<(usize, usize)>;
}

impl<T> GridSource for Term<T> {
    fn screen_lines(&self) -> usize {
        self.grid().screen_lines()
    }

    fn columns(&self) -> usize {
        self.grid().columns()
    }

    fn cell(&self, line: usize, column: usize) -> Cell {
        self.grid()[Point::new(Line(line as i32), Column(column))].clone()
    }

    fn colors(&self) -> &Colors {
        Term::colors(self)
    }

    fn cursor(&self) -> Option<(usize, usize)> {
        if !self.mode().contains(TermMode::SHOW_CURSOR) {
            return None;
        }
        let point = self.grid().cursor.point;
        Some((point.line.0.max(0) as usize, point.column.0))
    }
}

/// A grid of cells kept outside any emulator, for drawing cells that come
/// from elsewhere.
#[derive(Clone)]
pub struct CellGrid {
    lines: usize,
    columns: usize,
    cells: Vec<Cell>,
    colors: Colors,
    cursor: Option<(usize, usize)>,
}

impl CellGrid {
    /// A blank grid of `lines` rows and `columns` columns.
    pub fn new(lines: usize, columns: usize) -> Self {
        Self {
            lines,
            columns,
            cells: vec![Cell::default(); lines * columns],
            colors: Colors::default(),
            cursor: Some((0, 0)),
        }
    }

    /// Changes the size, keeping the cells that still fit.
    pub fn resize(&mut self, lines: usize, columns: usize) {
        let mut cells = vec![Cell::default(); lines * columns];
        for line in 0..lines.min(self.lines) {
            for column in 0..columns.min(self.columns) {
                cells[line * columns + column] = self.cells[line * self.columns + column].clone();
            }
        }
        self.lines = lines;
        self.columns = columns;
        self.cells = cells;
    }

    /// The cell at a row and column, to change it.
    pub fn cell_mut(&mut self, line: usize, column: usize) -> &mut Cell {
        &mut self.cells[line * self.columns + column]
    }

    /// Replaces a whole row; cells past the end of `cells` become blank.
    pub fn set_line(&mut self, line: usize, cells: impl IntoIterator<Item = Cell>) {
        let row = &mut self.cells[line * self.columns..(line + 1) * self.columns];
        let mut given = cells.into_iter();
        for slot in row {
            *slot = given.next().unwrap_or_default();
        }
    }

    /// Colors the program set, which override the palette.
    pub fn colors_mut(&mut self) -> &mut Colors {
        &mut self.colors
    }

    pub fn set_cursor(&mut self, cursor: Option<(usize, usize)>) {
        self.cursor = cursor;
    }
}

impl GridSource for CellGrid {
    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.columns
    }

    fn cell(&self, line: usize, column: usize) -> Cell {
        self.cells[line * self.columns + column].clone()
    }

    fn colors(&self) -> &Colors {
        &self.colors
    }

    fn cursor(&self) -> Option<(usize, usize)> {
        self.cursor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cell_grid_keeps_what_fits_when_resized() {
        let mut grid = CellGrid::new(2, 3);
        grid.set_line(0, "abc".chars().map(|c| Cell { c, ..Cell::default() }));
        grid.resize(3, 2);
        assert_eq!(grid.cell(0, 0).c, 'a');
        assert_eq!(grid.cell(0, 1).c, 'b');
        assert_eq!(grid.cell(2, 1).c, ' ');
    }
}
