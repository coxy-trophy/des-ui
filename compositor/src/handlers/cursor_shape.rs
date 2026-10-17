//! cursor-shape: an application names the pointer it wants (an I-beam over text, a
//! hand over a link) instead of drawing one itself. The compositor draws three of
//! its own: the arrow, the I-beam and the hand. Every other name gets the arrow.

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::element::memory::MemoryRenderBuffer,
    },
    delegate_cursor_shape,
    input::pointer::CursorIcon,
    utils::{Logical, Point, Transform},
};

use crate::DesComp;

delegate_cursor_shape!(DesComp);

// Cursor shapes may be asked for a drawing tablet's pen too; there is none to draw.
impl smithay::wayland::tablet_manager::TabletSeatHandler for DesComp {}

/// The pointer, top-left corner at the hotspot. `X` is the outline, `.` the fill.
const ARROW: [&str; 17] = [
    "X           ",
    "XX          ",
    "X.X         ",
    "X..X        ",
    "X...X       ",
    "X....X      ",
    "X.....X     ",
    "X......X    ",
    "X.......X   ",
    "X........X  ",
    "X.........X ",
    "X......XXXXX",
    "X...X..X    ",
    "X..XX..X    ",
    "X.X  X..X   ",
    "XX   X..X   ",
    "X     XX    ",
];

/// Over text: the hotspot is the middle of the stem.
const BEAM: [&str; 17] = [
    "XXX XXX",
    "X..X..X",
    "XXX.XXX",
    "  X.X  ",
    "  X.X  ",
    "  X.X  ",
    "  X.X  ",
    "  X.X  ",
    "  X.X  ",
    "  X.X  ",
    "  X.X  ",
    "  X.X  ",
    "  X.X  ",
    "  X.X  ",
    "XXX.XXX",
    "X..X..X",
    "XXX XXX",
];

/// Over a link: the hotspot is the tip of the finger.
const HAND: [&str; 17] = [
    "    XX       ",
    "   X..X      ",
    "   X..X      ",
    "   X..X      ",
    "   X..XXX    ",
    "   X..X..XX  ",
    "XX X..X..X.X ",
    "X..X..X..X..X",
    "X...X......X.X",
    " X.........X.X",
    "  X..........X",
    "  X..........X",
    "   X........X ",
    "   X........X ",
    "    X......X  ",
    "    X......X  ",
    "    XXXXXXXX  ",
];

/// Over a divider between windows: a double arrow, its hotspot in the middle.
const RESIZE: [&str; 9] = [
    "    X     X    ",
    "   XX     XX   ",
    "  X.X     X.X  ",
    " X..XXXXXXX..X ",
    "X.............X",
    " X..XXXXXXX..X ",
    "  X.X     X.X  ",
    "   XX     XX   ",
    "    X     X    ",
];

/// Which of the compositor's own pointers stands for a named one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Arrow,
    Beam,
    Hand,
    Resize,
}

pub fn shape_for(icon: CursorIcon) -> Shape {
    match icon {
        CursorIcon::Text | CursorIcon::VerticalText => Shape::Beam,
        CursorIcon::Pointer => Shape::Hand,
        CursorIcon::EwResize | CursorIcon::ColResize | CursorIcon::EResize | CursorIcon::WResize => Shape::Resize,
        _ => Shape::Arrow,
    }
}

/// The compositor's pointers, ready to draw, each with its hotspot.
pub struct Cursors {
    arrow: MemoryRenderBuffer,
    beam: MemoryRenderBuffer,
    hand: MemoryRenderBuffer,
    resize: MemoryRenderBuffer,
}

impl Cursors {
    pub fn new() -> Cursors {
        Cursors { arrow: buffer(&ARROW), beam: buffer(&BEAM), hand: buffer(&HAND), resize: buffer(&RESIZE) }
    }

    /// The picture for a named pointer, and where in it the pointer's point is.
    pub fn get(&self, icon: CursorIcon) -> (&MemoryRenderBuffer, Point<i32, Logical>) {
        match shape_for(icon) {
            Shape::Arrow => (&self.arrow, (0, 0).into()),
            Shape::Beam => (&self.beam, (3, 8).into()),
            Shape::Hand => (&self.hand, (5, 0).into()),
            Shape::Resize => (&self.resize, (7, 4).into()),
        }
    }
}

/// Rows may be ragged; each is padded to the widest with transparency.
fn buffer(rows: &[&str]) -> MemoryRenderBuffer {
    let width = rows.iter().map(|row| row.len()).max().unwrap_or(1);
    let mut data = Vec::with_capacity(width * rows.len() * 4);
    for row in rows {
        let cells = row.bytes().chain(std::iter::repeat(b' ')).take(width);
        for cell in cells {
            // Argb8888 is stored as B, G, R, A.
            data.extend_from_slice(match cell {
                b'X' => &[0, 0, 0, 255],
                b'.' => &[255, 255, 255, 255],
                _ => &[0, 0, 0, 0],
            });
        }
    }
    MemoryRenderBuffer::from_slice(&data, Fourcc::Argb8888, (width as i32, rows.len() as i32), 1, Transform::Normal, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_and_links_have_their_own_pointers_and_the_rest_the_arrow() {
        assert_eq!(shape_for(CursorIcon::Text), Shape::Beam);
        assert_eq!(shape_for(CursorIcon::Pointer), Shape::Hand);
        assert_eq!(shape_for(CursorIcon::Default), Shape::Arrow);
        assert_eq!(shape_for(CursorIcon::Wait), Shape::Arrow);
        assert_eq!(shape_for(CursorIcon::ColResize), Shape::Resize);
    }

    #[test]
    fn every_hotspot_lies_on_the_picture() {
        for (rows, hotspot) in [(&ARROW[..], (0, 0)), (&BEAM[..], (3, 8)), (&HAND[..], (5, 0)), (&RESIZE[..], (7, 4))] {
            let width = rows.iter().map(|row| row.len()).max().unwrap();
            assert!(hotspot.0 < width && hotspot.1 < rows.len());
            let cell = rows[hotspot.1].as_bytes().get(hotspot.0).copied().unwrap_or(b' ');
            assert_ne!(cell, b' ', "hotspot on a transparent cell");
        }
    }
}
