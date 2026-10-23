//! What des-hands tells the compositor about the hand in front of the webcam: one
//! line each on its standard output, which the compositor reads.

/// The pointer's buttons a hand can press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
}

/// Which way two fingers swept, as the hand sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Message {
    /// The pointer moves by this much, as fractions of the screen across and down.
    Motion { dx: f32, dy: f32 },
    Press(Button),
    Release(Button),
    /// Two fingers moved up (less than nothing) or down, by this fraction of the view
    /// taken twice.
    Scroll(f32),
    Sweep(Side),
    /// A fist: the pointer rests; or the fist opened again.
    Rest,
    Wake,
    /// A hand came into view, or left it.
    Seen,
    Gone,
}

fn button(word: &str) -> Option<Button> {
    match word {
        "left" => Some(Button::Left),
        "right" => Some(Button::Right),
        _ => None,
    }
}

impl Message {
    pub fn encode(&self) -> String {
        let side = |left: bool| if left { "left" } else { "right" };
        match self {
            Message::Motion { dx, dy } => format!("motion {dx:.5} {dy:.5}\n"),
            Message::Press(which) => format!("press {}\n", side(*which == Button::Left)),
            Message::Release(which) => format!("release {}\n", side(*which == Button::Left)),
            Message::Scroll(by) => format!("scroll {by:.5}\n"),
            Message::Sweep(which) => format!("sweep {}\n", side(*which == Side::Left)),
            Message::Rest => "rest\n".into(),
            Message::Wake => "wake\n".into(),
            Message::Seen => "seen\n".into(),
            Message::Gone => "gone\n".into(),
        }
    }

    /// Reads one line; anything else is `None`. Numbers out of all reason are refused,
    /// so that nothing sends the pointer to the far side of everything.
    pub fn decode(line: &str) -> Option<Message> {
        let mut words = line.split_whitespace();
        let name = words.next()?;
        let mut number = || words.next()?.parse::<f32>().ok().filter(|value| value.is_finite() && value.abs() <= 4.0);
        let message = match name {
            "motion" => Message::Motion { dx: number()?, dy: number()? },
            "scroll" => Message::Scroll(number()?),
            "press" | "release" | "sweep" => {
                let which = words.next()?;
                match name {
                    "press" => Message::Press(button(which)?),
                    "release" => Message::Release(button(which)?),
                    _ => Message::Sweep(if button(which)? == Button::Left { Side::Left } else { Side::Right }),
                }
            }
            "rest" => Message::Rest,
            "wake" => Message::Wake,
            "seen" => Message::Seen,
            "gone" => Message::Gone,
            _ => return None,
        };
        words.next().is_none().then_some(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip() {
        for message in [
            Message::Motion { dx: 0.0125, dy: -0.5 },
            Message::Press(Button::Left),
            Message::Release(Button::Right),
            Message::Scroll(-0.25),
            Message::Sweep(Side::Left),
            Message::Sweep(Side::Right),
            Message::Rest,
            Message::Wake,
            Message::Seen,
            Message::Gone,
        ] {
            assert_eq!(Message::decode(&message.encode()), Some(message));
        }
        assert_eq!(Message::decode("motion 1e9 0"), None, "too far");
        assert_eq!(Message::decode("motion NaN 0"), None);
        assert_eq!(Message::decode("press middle"), None);
        assert_eq!(Message::decode("rest now"), None);
        assert_eq!(Message::decode(""), None);
    }
}
