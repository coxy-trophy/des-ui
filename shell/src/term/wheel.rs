use eframe::egui::{Event, Key, Modifiers, MouseWheelUnit};
use vt100::{MouseProtocolEncoding, MouseProtocolMode};

#[derive(Clone, Copy, PartialEq)]
enum Route {
    Local,
    Mouse(MouseProtocolMode, MouseProtocolEncoding),
    Arrows(bool),
}

/// Fractional wheel gestures belong to their pane and input route.
#[derive(Default)]
pub(super) struct WheelState {
    remainder: f32,
    route: Option<Route>,
}

impl WheelState {
    pub(super) fn reset(&mut self) {
        self.remainder = 0.0;
        self.route = None;
    }

    pub(super) fn handle(
        &mut self,
        screen: &mut vt100::Screen,
        event: &Event,
        cell: (u16, u16),
        cell_height: f32,
        rows: u16,
    ) -> Vec<u8> {
        let Event::MouseWheel { unit, delta, modifiers } = event else {
            return Vec::new();
        };
        // Some backends already rotate Shift+wheel into the horizontal axis.
        let vertical = if modifiers.shift && delta.y == 0.0 { delta.x } else { delta.y };
        if vertical == 0.0 || !vertical.is_finite() {
            return Vec::new();
        }
        let mode = screen.mouse_protocol_mode();
        let route = if modifiers.shift || screen.scrollback() > 0 {
            Route::Local
        } else if mode != MouseProtocolMode::None {
            Route::Mouse(mode, screen.mouse_protocol_encoding())
        } else if screen.alternate_screen() {
            Route::Arrows(screen.application_cursor())
        } else {
            Route::Local
        };
        if self.route != Some(route) {
            self.reset();
            self.route = Some(route);
        }
        let lines = match unit {
            MouseWheelUnit::Point => vertical / cell_height.max(1.0),
            MouseWheelUnit::Line => vertical,
            MouseWheelUnit::Page => vertical * f32::from(rows),
        };
        let total = self.remainder + lines;
        if !total.is_finite() {
            self.reset();
            return Vec::new();
        }
        self.remainder = total.fract();
        // Discard oversized whole-event backlog; one input cannot monopolize a frame.
        let ticks = total.trunc().clamp(-256.0, 256.0) as i32;
        if ticks == 0 {
            return Vec::new();
        }
        if route == Route::Local {
            let current = screen.scrollback();
            let target = if ticks > 0 {
                current.saturating_add(ticks as usize)
            } else {
                current.saturating_sub(ticks.unsigned_abs() as usize)
            };
            screen.set_scrollback(target);
            return Vec::new();
        }
        let bytes = match route {
            Route::Mouse(mode, encoding) => mouse_bytes(mode, encoding, ticks > 0, cell, *modifiers),
            Route::Arrows(app_cursor) => super::key_bytes(
                if ticks > 0 { Key::ArrowUp } else { Key::ArrowDown },
                *modifiers,
                app_cursor,
            ).unwrap_or_default(),
            Route::Local => unreachable!(),
        };
        bytes.repeat(ticks.unsigned_abs() as usize)
    }
}

fn mouse_bytes(
    mode: MouseProtocolMode,
    encoding: MouseProtocolEncoding,
    up: bool,
    (row, col): (u16, u16),
    modifiers: Modifiers,
) -> Vec<u8> {
    let modifier = if mode == MouseProtocolMode::Press {
        0
    } else {
        4 * u32::from(modifiers.shift) + 8 * u32::from(modifiers.alt) + 16 * u32::from(modifiers.ctrl)
    };
    let button = if up { 64 } else { 65 } + modifier;
    let x = u32::from(col) + 1;
    let y = u32::from(row) + 1;
    match encoding {
        MouseProtocolEncoding::Sgr => format!("\x1b[<{button};{x};{y}M").into_bytes(),
        MouseProtocolEncoding::Default if x <= 223 && y <= 223 => {
            vec![0x1b, b'[', b'M', (button + 32) as u8, (x + 32) as u8, (y + 32) as u8]
        }
        MouseProtocolEncoding::Utf8 if x <= 2015 && y <= 2015 => {
            let mut bytes = b"\x1b[M".to_vec();
            for value in [button + 32, x + 32, y + 32] {
                let mut buffer = [0; 4];
                bytes.extend_from_slice(char::from_u32(value).unwrap().encode_utf8(&mut buffer).as_bytes());
            }
            bytes
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(unit: MouseWheelUnit, y: f32, modifiers: Modifiers) -> Event {
        Event::MouseWheel { unit, delta: eframe::egui::vec2(0.0, y), modifiers }
    }
    fn send(state: &mut WheelState, parser: &mut vt100::Parser, event: Event) -> Vec<u8> {
        state.handle(parser.screen_mut(), &event, (0, 0), 20.0, 3)
    }
    #[test]
    fn application_encodings_and_modifiers() {
        let mut p = vt100::Parser::new(3, 120, 20);
        let mut w = WheelState::default();
        p.process(b"\x1b[?1000h");
        assert_eq!(send(&mut w, &mut p, event(MouseWheelUnit::Line, 1.0, Modifiers::NONE)), b"\x1b[M`!!");
        assert_eq!(send(&mut w, &mut p, event(MouseWheelUnit::Line, -1.0, Modifiers::ALT | Modifiers::CTRL)), b"\x1b[My!!");
        p.process(b"\x1b[?1005h");
        assert_eq!(w.handle(p.screen_mut(), &event(MouseWheelUnit::Line, 1.0, Modifiers::NONE), (0, 99), 20.0, 3), b"\x1b[M`\xc2\x84!");
        p.process(b"\x1b[?1006h");
        assert_eq!(send(&mut w, &mut p, event(MouseWheelUnit::Line, -1.0, Modifiers::CTRL)), b"\x1b[<81;1;1M");
        p.process(b"\x1b[?9h");
        assert_eq!(send(&mut w, &mut p, event(MouseWheelUnit::Line, 1.0, Modifiers::ALT | Modifiers::CTRL)), b"\x1b[<64;1;1M");
    }
    #[test]
    fn history_fractional_shift_reset_and_bounds() {
        let mut p = vt100::Parser::new(3, 20, 20);
        p.process(b"a\r\nb\r\nc\r\nd\r\ne\r\nf\r\n");
        let mut w = WheelState::default();
        send(&mut w, &mut p, event(MouseWheelUnit::Point, 10.0, Modifiers::NONE));
        assert_eq!(p.screen().scrollback(), 0);
        send(&mut w, &mut p, event(MouseWheelUnit::Point, 10.0, Modifiers::NONE));
        assert_eq!(p.screen().scrollback(), 1);
        send(&mut w, &mut p, event(MouseWheelUnit::Page, -1.0, Modifiers::NONE));
        assert_eq!(p.screen().scrollback(), 0);
        send(&mut w, &mut p, event(MouseWheelUnit::Point, 10.0, Modifiers::NONE));
        w.reset();
        send(&mut w, &mut p, event(MouseWheelUnit::Point, 10.0, Modifiers::NONE));
        assert_eq!(p.screen().scrollback(), 0);
        w.reset();
        p.process(b"\x1b[?1000h");
        assert!(send(&mut w, &mut p, event(MouseWheelUnit::Line, 1.0, Modifiers::SHIFT)).is_empty());
        assert_eq!(p.screen().scrollback(), 1);
        assert!(send(&mut w, &mut p, event(MouseWheelUnit::Line, -1.0, Modifiers::NONE)).is_empty());
        assert_eq!(p.screen().scrollback(), 0);
        assert!(!send(&mut w, &mut p, event(MouseWheelUnit::Line, 1.0, Modifiers::NONE)).is_empty());
    }
    #[test]
    fn local_clamps_and_reporting_preserves_history() {
        let mut p = vt100::Parser::new(3, 20, 20);
        p.process(b"a\r\nb\r\nc\r\nd\r\ne\r\nf\r\n");
        let mut w = WheelState::default();
        send(&mut w, &mut p, event(MouseWheelUnit::Line, 100.0, Modifiers::NONE));
        let maximum = p.screen().scrollback();
        assert!(maximum > 0 && maximum < 100);
        send(&mut w, &mut p, event(MouseWheelUnit::Line, 100.0, Modifiers::NONE));
        assert_eq!(p.screen().scrollback(), maximum);
        p.process(b"\x1b[?1000h");
        assert!(send(&mut w, &mut p, event(MouseWheelUnit::Line, 1.0, Modifiers::NONE)).is_empty());
        assert_eq!(p.screen().scrollback(), maximum);
        send(&mut w, &mut p, event(MouseWheelUnit::Line, -100.0, Modifiers::NONE));
        assert_eq!(p.screen().scrollback(), 0);
        assert_eq!(w.handle(p.screen_mut(), &event(MouseWheelUnit::Line, 1.0, Modifiers::NONE), (0, 223), 20.0, 3), Vec::<u8>::new());
    }
    #[test]
    fn alternate_fallback_horizontal_and_route_remainder() {
        let mut p = vt100::Parser::new(3, 20, 20);
        let mut w = WheelState::default();
        send(&mut w, &mut p, event(MouseWheelUnit::Point, 10.0, Modifiers::NONE));
        p.process(b"\x1b[?1049h");
        assert!(send(&mut w, &mut p, event(MouseWheelUnit::Point, 10.0, Modifiers::NONE)).is_empty());
        assert_eq!(send(&mut w, &mut p, event(MouseWheelUnit::Point, 10.0, Modifiers::NONE)), b"\x1b[A");
        p.process(b"\x1b[?1h");
        assert_eq!(send(&mut w, &mut p, event(MouseWheelUnit::Line, -1.0, Modifiers::NONE)), b"\x1bOB");
        let horizontal = Event::MouseWheel { unit: MouseWheelUnit::Line, delta: eframe::egui::vec2(2.0, 0.0), modifiers: Modifiers::NONE };
        assert!(send(&mut w, &mut p, horizontal).is_empty());
        assert_eq!(send(&mut w, &mut p, event(MouseWheelUnit::Line, 1e9, Modifiers::NONE)).len(), 256 * 3);
    }
}
