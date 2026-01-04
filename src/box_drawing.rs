//! Custom box-drawing character rendering.
//!
//! This module provides programmatic rendering for Unicode box-drawing characters
//! (U+2500-U+257F) using GPUI's path-building primitives instead of font glyphs.
//! This approach eliminates anti-aliasing artifacts and ensures perfect alignment
//! at cell boundaries.
//!
//! # Supported Characters
//!
//! - Light and heavy horizontal/vertical lines
//! - Corners (light, heavy, and mixed)
//! - T-junctions and crosses
//! - Double-line variants
//! - Rounded corners
//! - Dashed lines
//!
//! # Example
//!
//! ```ignore
//! use gpui_terminal::box_drawing;
//!
//! // Check if a character is a box-drawing character
//! if box_drawing::is_box_drawing_char('┌') {
//!     // Draw it programmatically instead of as text
//!     box_drawing::draw_box_character('┌', bounds, color, cell_width, window);
//! }
//! ```

use gpui::{point, Bounds, Hsla, PathBuilder, Pixels, Point, Window, px};

/// Calculate line thicknesses rounded to integer pixels to avoid aliasing.
fn calculate_thickness(cell_width: Pixels) -> (Pixels, Pixels) {
    let cell_width_f32: f32 = cell_width.into();
    // Round to nearest integer pixel
    let light = (cell_width_f32 * 0.15).round().max(1.0);
    let heavy = (cell_width_f32 * 0.28).round().max(2.0);
    (px(light), px(heavy))
}

/// Line weight for box-drawing segments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineWeight {
    /// Single thin line (~1-2px)
    Light,
    /// Single thick line (~2-3px)
    Heavy,
    /// Two parallel thin lines
    Double,
}

/// Describes which segments a box-drawing character uses.
///
/// Each segment extends from the cell center to one of the four edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoxSegments {
    /// Segment from center to top edge
    pub top: Option<LineWeight>,
    /// Segment from center to bottom edge
    pub bottom: Option<LineWeight>,
    /// Segment from center to left edge
    pub left: Option<LineWeight>,
    /// Segment from center to right edge
    pub right: Option<LineWeight>,
}

impl BoxSegments {
    const fn new(
        top: Option<LineWeight>,
        bottom: Option<LineWeight>,
        left: Option<LineWeight>,
        right: Option<LineWeight>,
    ) -> Self {
        Self { top, bottom, left, right }
    }

    const fn horizontal(weight: LineWeight) -> Self {
        Self::new(None, None, Some(weight), Some(weight))
    }

    const fn vertical(weight: LineWeight) -> Self {
        Self::new(Some(weight), Some(weight), None, None)
    }

    const fn corner_tl(weight: LineWeight) -> Self {
        Self::new(None, Some(weight), None, Some(weight))
    }

    const fn corner_tr(weight: LineWeight) -> Self {
        Self::new(None, Some(weight), Some(weight), None)
    }

    const fn corner_bl(weight: LineWeight) -> Self {
        Self::new(Some(weight), None, None, Some(weight))
    }

    const fn corner_br(weight: LineWeight) -> Self {
        Self::new(Some(weight), None, Some(weight), None)
    }

    const fn t_left(weight: LineWeight) -> Self {
        Self::new(Some(weight), Some(weight), None, Some(weight))
    }

    const fn t_right(weight: LineWeight) -> Self {
        Self::new(Some(weight), Some(weight), Some(weight), None)
    }

    const fn t_top(weight: LineWeight) -> Self {
        Self::new(None, Some(weight), Some(weight), Some(weight))
    }

    const fn t_bottom(weight: LineWeight) -> Self {
        Self::new(Some(weight), None, Some(weight), Some(weight))
    }

    const fn cross(weight: LineWeight) -> Self {
        Self::new(Some(weight), Some(weight), Some(weight), Some(weight))
    }
}

use LineWeight::*;

/// Returns true if the character is in the box-drawing Unicode range (U+2500-U+257F).
#[inline]
pub fn is_box_drawing_char(ch: char) -> bool {
    let code = ch as u32;
    code >= 0x2500 && code <= 0x257F
}

/// Returns the horizontal line weight if the character has a continuous horizontal line
/// (both left and right segments with matching weight).
pub fn get_horizontal_weight(ch: char) -> Option<LineWeight> {
    let segments = get_box_segments(ch)?;
    match (segments.left, segments.right) {
        (Some(l), Some(r)) if l == r => Some(l),
        _ => None,
    }
}

/// Returns the vertical line weight if the character has a continuous vertical line
/// (both top and bottom segments with matching weight).
pub fn get_vertical_weight(ch: char) -> Option<LineWeight> {
    let segments = get_box_segments(ch)?;
    match (segments.top, segments.bottom) {
        (Some(t), Some(b)) if t == b => Some(t),
        _ => None,
    }
}

/// Returns true if the character extends to the left edge.
pub fn extends_left(ch: char) -> bool {
    get_box_segments(ch).map(|s| s.left.is_some()).unwrap_or(false)
}

/// Returns true if the character extends to the right edge.
pub fn extends_right(ch: char) -> bool {
    get_box_segments(ch).map(|s| s.right.is_some()).unwrap_or(false)
}

/// Returns the box segments for a Unicode box-drawing character.
///
/// Returns `None` if the character is not a recognized box-drawing character.
pub fn get_box_segments(ch: char) -> Option<BoxSegments> {
    let code = ch as u32;
    if code < 0x2500 || code > 0x257F {
        return None;
    }

    // Table indexed by (code - 0x2500)
    Some(match code {
        // ─ Light horizontal
        0x2500 => BoxSegments::horizontal(Light),
        // ━ Heavy horizontal
        0x2501 => BoxSegments::horizontal(Heavy),
        // │ Light vertical
        0x2502 => BoxSegments::vertical(Light),
        // ┃ Heavy vertical
        0x2503 => BoxSegments::vertical(Heavy),

        // Dashed lines (render as solid for now)
        0x2504 => BoxSegments::horizontal(Light), // ┄
        0x2505 => BoxSegments::horizontal(Heavy), // ┅
        0x2506 => BoxSegments::vertical(Light),   // ┆
        0x2507 => BoxSegments::vertical(Heavy),   // ┇
        0x2508 => BoxSegments::horizontal(Light), // ┈
        0x2509 => BoxSegments::horizontal(Heavy), // ┉
        0x250A => BoxSegments::vertical(Light),   // ┊
        0x250B => BoxSegments::vertical(Heavy),   // ┋

        // Light corners
        0x250C => BoxSegments::corner_tl(Light), // ┌
        0x2510 => BoxSegments::corner_tr(Light), // ┐
        0x2514 => BoxSegments::corner_bl(Light), // └
        0x2518 => BoxSegments::corner_br(Light), // ┘

        // Heavy-right corners
        0x250D => BoxSegments::new(None, Some(Light), None, Some(Heavy)), // ┍
        0x2511 => BoxSegments::new(None, Some(Light), Some(Heavy), None), // ┑
        0x2515 => BoxSegments::new(Some(Light), None, None, Some(Heavy)), // ┕
        0x2519 => BoxSegments::new(Some(Light), None, Some(Heavy), None), // ┙

        // Heavy-down/up corners
        0x250E => BoxSegments::new(None, Some(Heavy), None, Some(Light)), // ┎
        0x2512 => BoxSegments::new(None, Some(Heavy), Some(Light), None), // ┒
        0x2516 => BoxSegments::new(Some(Heavy), None, None, Some(Light)), // ┖
        0x251A => BoxSegments::new(Some(Heavy), None, Some(Light), None), // ┚

        // Heavy corners
        0x250F => BoxSegments::corner_tl(Heavy), // ┏
        0x2513 => BoxSegments::corner_tr(Heavy), // ┓
        0x2517 => BoxSegments::corner_bl(Heavy), // ┗
        0x251B => BoxSegments::corner_br(Heavy), // ┛

        // Light T-junctions
        0x251C => BoxSegments::t_left(Light),   // ├
        0x2524 => BoxSegments::t_right(Light),  // ┤
        0x252C => BoxSegments::t_top(Light),    // ┬
        0x2534 => BoxSegments::t_bottom(Light), // ┴

        // Mixed T-junctions (light vertical, heavy horizontal)
        0x251D => BoxSegments::new(Some(Light), Some(Light), None, Some(Heavy)), // ┝
        0x2525 => BoxSegments::new(Some(Light), Some(Light), Some(Heavy), None), // ┥
        0x252D => BoxSegments::new(None, Some(Light), Some(Heavy), Some(Light)), // ┭
        0x252E => BoxSegments::new(None, Some(Light), Some(Light), Some(Heavy)), // ┮
        0x2535 => BoxSegments::new(Some(Light), None, Some(Heavy), Some(Light)), // ┵
        0x2536 => BoxSegments::new(Some(Light), None, Some(Light), Some(Heavy)), // ┶

        // Mixed T-junctions (heavy vertical, light horizontal)
        0x251E => BoxSegments::new(Some(Heavy), Some(Light), None, Some(Light)), // ┞
        0x251F => BoxSegments::new(Some(Light), Some(Heavy), None, Some(Light)), // ┟
        0x2526 => BoxSegments::new(Some(Heavy), Some(Light), Some(Light), None), // ┦
        0x2527 => BoxSegments::new(Some(Light), Some(Heavy), Some(Light), None), // ┧
        0x252F => BoxSegments::new(None, Some(Heavy), Some(Light), Some(Light)), // ┯
        0x2537 => BoxSegments::new(Some(Heavy), None, Some(Light), Some(Light)), // ┷

        // Heavy T-junctions
        0x2520 => BoxSegments::new(Some(Heavy), Some(Heavy), None, Some(Light)), // ┠
        0x2521 => BoxSegments::new(Some(Heavy), Some(Light), None, Some(Heavy)), // ┡
        0x2522 => BoxSegments::new(Some(Light), Some(Heavy), None, Some(Heavy)), // ┢
        0x2523 => BoxSegments::t_left(Heavy), // ┣
        0x2528 => BoxSegments::new(Some(Heavy), Some(Heavy), Some(Light), None), // ┨
        0x2529 => BoxSegments::new(Some(Heavy), Some(Light), Some(Heavy), None), // ┩
        0x252A => BoxSegments::new(Some(Light), Some(Heavy), Some(Heavy), None), // ┪
        0x252B => BoxSegments::t_right(Heavy), // ┫
        0x2530 => BoxSegments::new(None, Some(Heavy), Some(Heavy), Some(Light)), // ┰
        0x2531 => BoxSegments::new(None, Some(Heavy), Some(Light), Some(Heavy)), // ┱
        0x2532 => BoxSegments::new(None, Some(Light), Some(Heavy), Some(Heavy)), // ┲
        0x2533 => BoxSegments::t_top(Heavy), // ┳
        0x2538 => BoxSegments::new(Some(Heavy), None, Some(Heavy), Some(Light)), // ┸
        0x2539 => BoxSegments::new(Some(Heavy), None, Some(Light), Some(Heavy)), // ┹
        0x253A => BoxSegments::new(Some(Light), None, Some(Heavy), Some(Heavy)), // ┺
        0x253B => BoxSegments::t_bottom(Heavy), // ┻

        // Light cross
        0x253C => BoxSegments::cross(Light), // ┼

        // Mixed crosses
        0x253D => BoxSegments::new(Some(Light), Some(Light), Some(Heavy), Some(Light)), // ┽
        0x253E => BoxSegments::new(Some(Light), Some(Light), Some(Light), Some(Heavy)), // ┾
        0x253F => BoxSegments::new(Some(Light), Some(Light), Some(Heavy), Some(Heavy)), // ┿
        0x2540 => BoxSegments::new(Some(Heavy), Some(Light), Some(Light), Some(Light)), // ╀
        0x2541 => BoxSegments::new(Some(Light), Some(Heavy), Some(Light), Some(Light)), // ╁
        0x2542 => BoxSegments::new(Some(Heavy), Some(Heavy), Some(Light), Some(Light)), // ╂
        0x2543 => BoxSegments::new(Some(Heavy), Some(Light), Some(Heavy), Some(Light)), // ╃
        0x2544 => BoxSegments::new(Some(Heavy), Some(Light), Some(Light), Some(Heavy)), // ╄
        0x2545 => BoxSegments::new(Some(Light), Some(Heavy), Some(Heavy), Some(Light)), // ╅
        0x2546 => BoxSegments::new(Some(Light), Some(Heavy), Some(Light), Some(Heavy)), // ╆
        0x2547 => BoxSegments::new(Some(Heavy), Some(Heavy), Some(Heavy), Some(Light)), // ╇
        0x2548 => BoxSegments::new(Some(Heavy), Some(Heavy), Some(Light), Some(Heavy)), // ╈
        0x2549 => BoxSegments::new(Some(Heavy), Some(Light), Some(Heavy), Some(Heavy)), // ╉
        0x254A => BoxSegments::new(Some(Light), Some(Heavy), Some(Heavy), Some(Heavy)), // ╊

        // Heavy cross
        0x254B => BoxSegments::cross(Heavy), // ╋

        // More dashed (render as solid)
        0x254C => BoxSegments::horizontal(Light), // ╌
        0x254D => BoxSegments::horizontal(Heavy), // ╍
        0x254E => BoxSegments::vertical(Light),   // ╎
        0x254F => BoxSegments::vertical(Heavy),   // ╏

        // Double lines
        0x2550 => BoxSegments::horizontal(Double), // ═
        0x2551 => BoxSegments::vertical(Double),   // ║

        // Double corners
        0x2554 => BoxSegments::corner_tl(Double), // ╔
        0x2557 => BoxSegments::corner_tr(Double), // ╗
        0x255A => BoxSegments::corner_bl(Double), // ╚
        0x255D => BoxSegments::corner_br(Double), // ╝

        // Mixed single/double corners
        0x2552 => BoxSegments::new(None, Some(Light), None, Some(Double)), // ╒
        0x2553 => BoxSegments::new(None, Some(Double), None, Some(Light)), // ╓
        0x2555 => BoxSegments::new(None, Some(Light), Some(Double), None), // ╕
        0x2556 => BoxSegments::new(None, Some(Double), Some(Light), None), // ╖
        0x2558 => BoxSegments::new(Some(Light), None, None, Some(Double)), // ╘
        0x2559 => BoxSegments::new(Some(Double), None, None, Some(Light)), // ╙
        0x255B => BoxSegments::new(Some(Light), None, Some(Double), None), // ╛
        0x255C => BoxSegments::new(Some(Double), None, Some(Light), None), // ╜

        // Double T-junctions
        0x2560 => BoxSegments::t_left(Double),   // ╠
        0x2563 => BoxSegments::t_right(Double),  // ╣
        0x2566 => BoxSegments::t_top(Double),    // ╦
        0x2569 => BoxSegments::t_bottom(Double), // ╩

        // Mixed single/double T-junctions
        0x255E => BoxSegments::new(Some(Light), Some(Light), None, Some(Double)), // ╞
        0x255F => BoxSegments::new(Some(Double), Some(Double), None, Some(Light)), // ╟
        0x2561 => BoxSegments::new(Some(Light), Some(Light), Some(Double), None), // ╡
        0x2562 => BoxSegments::new(Some(Double), Some(Double), Some(Light), None), // ╢
        0x2564 => BoxSegments::new(None, Some(Light), Some(Double), Some(Double)), // ╤
        0x2565 => BoxSegments::new(None, Some(Double), Some(Light), Some(Light)), // ╥
        0x2567 => BoxSegments::new(Some(Light), None, Some(Double), Some(Double)), // ╧
        0x2568 => BoxSegments::new(Some(Double), None, Some(Light), Some(Light)), // ╨

        // Double cross
        0x256C => BoxSegments::cross(Double), // ╬

        // Mixed single/double crosses
        0x256A => BoxSegments::new(Some(Light), Some(Light), Some(Double), Some(Double)), // ╪
        0x256B => BoxSegments::new(Some(Double), Some(Double), Some(Light), Some(Light)), // ╫

        // Rounded corners
        0x256D => BoxSegments::corner_tl(Light), // ╭
        0x256E => BoxSegments::corner_tr(Light), // ╮
        0x256F => BoxSegments::corner_br(Light), // ╯
        0x2570 => BoxSegments::corner_bl(Light), // ╰

        // Diagonals - not supported with this segment model, skip
        0x2571 => return None, // ╱
        0x2572 => return None, // ╲
        0x2573 => return None, // ╳

        // Half lines
        0x2574 => BoxSegments::new(None, None, Some(Light), None), // ╴ left
        0x2575 => BoxSegments::new(Some(Light), None, None, None), // ╵ up
        0x2576 => BoxSegments::new(None, None, None, Some(Light)), // ╶ right
        0x2577 => BoxSegments::new(None, Some(Light), None, None), // ╷ down
        0x2578 => BoxSegments::new(None, None, Some(Heavy), None), // ╸ heavy left
        0x2579 => BoxSegments::new(Some(Heavy), None, None, None), // ╹ heavy up
        0x257A => BoxSegments::new(None, None, None, Some(Heavy)), // ╺ heavy right
        0x257B => BoxSegments::new(None, Some(Heavy), None, None), // ╻ heavy down

        // Mixed half lines
        0x257C => BoxSegments::new(None, None, Some(Light), Some(Heavy)), // ╼
        0x257D => BoxSegments::new(Some(Light), Some(Heavy), None, None), // ╽
        0x257E => BoxSegments::new(None, None, Some(Heavy), Some(Light)), // ╾
        0x257F => BoxSegments::new(Some(Heavy), Some(Light), None, None), // ╿

        _ => return None,
    })
}

/// Returns true if the character is a rounded corner (U+256D-U+2570).
#[inline]
fn is_rounded_corner(ch: char) -> bool {
    let code = ch as u32;
    code >= 0x256D && code <= 0x2570
}

/// Draws a horizontal line spanning multiple cells.
///
/// This draws a single continuous path from start_x to end_x at the vertical center,
/// eliminating gaps between adjacent cells.
pub fn draw_horizontal_span(
    start_x: Pixels,
    end_x: Pixels,
    cy: Pixels,
    weight: LineWeight,
    cell_width: Pixels,
    color: Hsla,
    window: &mut Window,
) {
    let (light_thickness, heavy_thickness) = calculate_thickness(cell_width);
    let thickness = get_thickness(weight, light_thickness, heavy_thickness);

    draw_continuous_line(
        point(start_x, cy),
        point(end_x, cy),
        weight,
        thickness,
        true,
        color,
        window,
    );
}

/// Draws a vertical line spanning multiple cells.
pub fn draw_vertical_span(
    cx: Pixels,
    start_y: Pixels,
    end_y: Pixels,
    weight: LineWeight,
    cell_width: Pixels,
    color: Hsla,
    window: &mut Window,
) {
    let (light_thickness, heavy_thickness) = calculate_thickness(cell_width);
    let thickness = get_thickness(weight, light_thickness, heavy_thickness);

    draw_continuous_line(
        point(cx, start_y),
        point(cx, end_y),
        weight,
        thickness,
        false,
        color,
        window,
    );
}

/// Draws only the vertical components of a box-drawing character.
///
/// Use this after drawing a horizontal span to add the vertical parts
/// (top/bottom segments) without redrawing the horizontal line.
pub fn draw_vertical_components(
    ch: char,
    bounds: Bounds<Pixels>,
    color: Hsla,
    cell_width: Pixels,
    window: &mut Window,
) {
    let Some(segments) = get_box_segments(ch) else {
        return;
    };

    // Skip if no vertical components
    if segments.top.is_none() && segments.bottom.is_none() {
        return;
    }

    let cx = bounds.origin.x + bounds.size.width / 2.0;
    let cy = bounds.origin.y + bounds.size.height / 2.0;

    let (light_thickness, heavy_thickness) = calculate_thickness(cell_width);

    let top_edge = bounds.origin.y;
    let bottom_edge = bounds.origin.y + bounds.size.height;

    // Check if vertical segments have same weight
    let v_weight = match (segments.top, segments.bottom) {
        (Some(t), Some(b)) if t == b => Some(t),
        (Some(t), None) => Some(t),
        (None, Some(b)) => Some(b),
        _ => None,
    };

    if let Some(weight) = v_weight {
        let thickness = get_thickness(weight, light_thickness, heavy_thickness);
        let start_y = if segments.top.is_some() { top_edge } else { cy };
        let end_y = if segments.bottom.is_some() { bottom_edge } else { cy };

        draw_continuous_line(
            point(cx, start_y),
            point(cx, end_y),
            weight,
            thickness,
            false,
            color,
            window,
        );
    } else {
        // Different weights - draw separately
        if let Some(weight) = segments.top {
            let thickness = get_thickness(weight, light_thickness, heavy_thickness);
            draw_continuous_line(
                point(cx, top_edge),
                point(cx, cy),
                weight,
                thickness,
                false,
                color,
                window,
            );
        }
        if let Some(weight) = segments.bottom {
            let thickness = get_thickness(weight, light_thickness, heavy_thickness);
            draw_continuous_line(
                point(cx, cy),
                point(cx, bottom_edge),
                weight,
                thickness,
                false,
                color,
                window,
            );
        }
    }
}

/// Draws a box-drawing character into the given cell bounds.
///
/// Returns `true` if the character was drawn, `false` if it's not a box-drawing character.
pub fn draw_box_character(
    ch: char,
    bounds: Bounds<Pixels>,
    color: Hsla,
    cell_width: Pixels,
    window: &mut Window,
) -> bool {
    let Some(segments) = get_box_segments(ch) else {
        return false;
    };

    // Calculate center point
    let cx = bounds.origin.x + bounds.size.width / 2.0;
    let cy = bounds.origin.y + bounds.size.height / 2.0;

    // Calculate line thicknesses (rounded to integer pixels)
    let (light_thickness, heavy_thickness) = calculate_thickness(cell_width);

    // Edge positions
    let left_edge = bounds.origin.x;
    let right_edge = bounds.origin.x + bounds.size.width;
    let top_edge = bounds.origin.y;
    let bottom_edge = bounds.origin.y + bounds.size.height;

    // Handle rounded corners specially with actual curves
    if is_rounded_corner(ch) {
        draw_rounded_corner(ch, bounds, cx, cy, light_thickness, color, window);
        return true;
    }

    // Draw connected paths to avoid gaps at junctions
    // Group by weight and draw continuous paths

    // Check if all horizontal segments have same weight
    let h_weight = match (segments.left, segments.right) {
        (Some(l), Some(r)) if l == r => Some(l),
        (Some(l), None) => Some(l),
        (None, Some(r)) => Some(r),
        _ => None,
    };

    // Check if all vertical segments have same weight
    let v_weight = match (segments.top, segments.bottom) {
        (Some(t), Some(b)) if t == b => Some(t),
        (Some(t), None) => Some(t),
        (None, Some(b)) => Some(b),
        _ => None,
    };

    // Overlap to eliminate sub-pixel gaps between adjacent cells
    let overlap = px(1.0);

    // Draw horizontal path (continuous from left to right through center)
    if let Some(weight) = h_weight {
        let thickness = get_thickness(weight, light_thickness, heavy_thickness);
        // Extend past cell boundaries to overlap with adjacent cells
        let start_x = if segments.left.is_some() { left_edge - overlap } else { cx };
        let end_x = if segments.right.is_some() { right_edge + overlap } else { cx };

        draw_continuous_line(
            point(start_x, cy),
            point(end_x, cy),
            weight,
            thickness,
            true,
            color,
            window,
        );
    } else {
        // Different weights - draw separately
        if let Some(weight) = segments.left {
            let thickness = get_thickness(weight, light_thickness, heavy_thickness);
            draw_continuous_line(
                point(left_edge - overlap, cy),
                point(cx, cy),
                weight,
                thickness,
                true,
                color,
                window,
            );
        }
        if let Some(weight) = segments.right {
            let thickness = get_thickness(weight, light_thickness, heavy_thickness);
            draw_continuous_line(
                point(cx, cy),
                point(right_edge + overlap, cy),
                weight,
                thickness,
                true,
                color,
                window,
            );
        }
    }

    // Draw vertical path (continuous from top to bottom through center)
    if let Some(weight) = v_weight {
        let thickness = get_thickness(weight, light_thickness, heavy_thickness);
        // Extend past cell boundaries to overlap with adjacent cells
        let start_y = if segments.top.is_some() { top_edge - overlap } else { cy };
        let end_y = if segments.bottom.is_some() { bottom_edge + overlap } else { cy };

        draw_continuous_line(
            point(cx, start_y),
            point(cx, end_y),
            weight,
            thickness,
            false,
            color,
            window,
        );
    } else {
        // Different weights - draw separately
        if let Some(weight) = segments.top {
            let thickness = get_thickness(weight, light_thickness, heavy_thickness);
            draw_continuous_line(
                point(cx, top_edge - overlap),
                point(cx, cy),
                weight,
                thickness,
                false,
                color,
                window,
            );
        }
        if let Some(weight) = segments.bottom {
            let thickness = get_thickness(weight, light_thickness, heavy_thickness);
            draw_continuous_line(
                point(cx, cy),
                point(cx, bottom_edge + overlap),
                weight,
                thickness,
                false,
                color,
                window,
            );
        }
    }

    true
}

/// Get thickness for a line weight.
fn get_thickness(weight: LineWeight, light: Pixels, heavy: Pixels) -> Pixels {
    match weight {
        LineWeight::Light => light,
        LineWeight::Heavy => heavy,
        LineWeight::Double => light, // Double uses light thickness for each line
    }
}

/// Draws a continuous line, handling double lines specially.
fn draw_continuous_line(
    from: Point<Pixels>,
    to: Point<Pixels>,
    weight: LineWeight,
    thickness: Pixels,
    is_horizontal: bool,
    color: Hsla,
    window: &mut Window,
) {
    match weight {
        LineWeight::Light | LineWeight::Heavy => {
            draw_line(from, to, thickness, color, window);
        }
        LineWeight::Double => {
            // Draw two parallel lines
            let gap = thickness;
            let offset = (thickness + gap) / 2.0;

            if is_horizontal {
                // Offset vertically for horizontal lines
                draw_line(
                    point(from.x, from.y - offset),
                    point(to.x, to.y - offset),
                    thickness,
                    color,
                    window,
                );
                draw_line(
                    point(from.x, from.y + offset),
                    point(to.x, to.y + offset),
                    thickness,
                    color,
                    window,
                );
            } else {
                // Offset horizontally for vertical lines
                draw_line(
                    point(from.x - offset, from.y),
                    point(to.x - offset, to.y),
                    thickness,
                    color,
                    window,
                );
                draw_line(
                    point(from.x + offset, from.y),
                    point(to.x + offset, to.y),
                    thickness,
                    color,
                    window,
                );
            }
        }
    }
}

/// Draws a single line using PathBuilder.
fn draw_line(
    from: Point<Pixels>,
    to: Point<Pixels>,
    thickness: Pixels,
    color: Hsla,
    window: &mut Window,
) {
    let mut builder = PathBuilder::stroke(thickness);
    builder.move_to(from);
    builder.line_to(to);
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

/// Draws a rounded corner character with an actual curve.
///
/// Rounded corners: ╭ (U+256D), ╮ (U+256E), ╯ (U+256F), ╰ (U+2570)
fn draw_rounded_corner(
    ch: char,
    bounds: Bounds<Pixels>,
    cx: Pixels,
    cy: Pixels,
    thickness: Pixels,
    color: Hsla,
    window: &mut Window,
) {
    // Overlap to eliminate sub-pixel gaps
    let overlap = px(1.0);

    let left = bounds.origin.x - overlap;
    let right = bounds.origin.x + bounds.size.width + overlap;
    let top = bounds.origin.y - overlap;
    let bottom = bounds.origin.y + bounds.size.height + overlap;

    // Radius scales with cell size - use about 40% of half-cell for a nice curve
    let half_w = bounds.size.width / 2.0;
    let half_h = bounds.size.height / 2.0;
    let radius_x = half_w * 0.8;
    let radius_y = half_h * 0.8;

    let mut builder = PathBuilder::stroke(thickness);

    match ch {
        // ╭ Top-left corner: comes from bottom, curves to right
        '\u{256D}' => {
            // Start from bottom edge, go up to curve start
            builder.move_to(point(cx, bottom));
            builder.line_to(point(cx, cy + radius_y));
            // Quadratic curve to the right
            builder.curve_to(point(cx + radius_x, cy), point(cx, cy));
            // Continue to right edge
            builder.line_to(point(right, cy));
        }
        // ╮ Top-right corner: comes from left, curves to bottom
        '\u{256E}' => {
            // Start from left edge, go right to curve start
            builder.move_to(point(left, cy));
            builder.line_to(point(cx - radius_x, cy));
            // Quadratic curve downward
            builder.curve_to(point(cx, cy + radius_y), point(cx, cy));
            // Continue to bottom edge
            builder.line_to(point(cx, bottom));
        }
        // ╯ Bottom-right corner: comes from top, curves to left
        '\u{256F}' => {
            // Start from top edge, go down to curve start
            builder.move_to(point(cx, top));
            builder.line_to(point(cx, cy - radius_y));
            // Quadratic curve to the left
            builder.curve_to(point(cx - radius_x, cy), point(cx, cy));
            // Continue to left edge
            builder.line_to(point(left, cy));
        }
        // ╰ Bottom-left corner: comes from right, curves to top
        '\u{2570}' => {
            // Start from right edge, go left to curve start
            builder.move_to(point(right, cy));
            builder.line_to(point(cx + radius_x, cy));
            // Quadratic curve upward
            builder.curve_to(point(cx, cy - radius_y), point(cx, cy));
            // Continue to top edge
            builder.line_to(point(cx, top));
        }
        _ => return,
    }

    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_box_drawing_char() {
        assert!(is_box_drawing_char('─')); // U+2500
        assert!(is_box_drawing_char('│')); // U+2502
        assert!(is_box_drawing_char('┌')); // U+250C
        assert!(is_box_drawing_char('┼')); // U+253C
        assert!(is_box_drawing_char('╬')); // U+256C
        assert!(is_box_drawing_char('╿')); // U+257F (last)
        assert!(!is_box_drawing_char('A'));
        assert!(!is_box_drawing_char(' '));
        assert!(!is_box_drawing_char('█')); // Block element, not box drawing
    }

    #[test]
    fn test_get_box_segments_horizontal() {
        let seg = get_box_segments('─').unwrap();
        assert_eq!(seg.left, Some(Light));
        assert_eq!(seg.right, Some(Light));
        assert_eq!(seg.top, None);
        assert_eq!(seg.bottom, None);
    }

    #[test]
    fn test_get_box_segments_vertical() {
        let seg = get_box_segments('│').unwrap();
        assert_eq!(seg.left, None);
        assert_eq!(seg.right, None);
        assert_eq!(seg.top, Some(Light));
        assert_eq!(seg.bottom, Some(Light));
    }

    #[test]
    fn test_get_box_segments_corner() {
        let seg = get_box_segments('┌').unwrap();
        assert_eq!(seg.left, None);
        assert_eq!(seg.right, Some(Light));
        assert_eq!(seg.top, None);
        assert_eq!(seg.bottom, Some(Light));
    }

    #[test]
    fn test_get_box_segments_cross() {
        let seg = get_box_segments('┼').unwrap();
        assert_eq!(seg.left, Some(Light));
        assert_eq!(seg.right, Some(Light));
        assert_eq!(seg.top, Some(Light));
        assert_eq!(seg.bottom, Some(Light));
    }

    #[test]
    fn test_get_box_segments_double() {
        let seg = get_box_segments('═').unwrap();
        assert_eq!(seg.left, Some(Double));
        assert_eq!(seg.right, Some(Double));
        assert_eq!(seg.top, None);
        assert_eq!(seg.bottom, None);
    }

    #[test]
    fn test_get_box_segments_invalid() {
        assert!(get_box_segments('A').is_none());
        assert!(get_box_segments(' ').is_none());
    }
}
