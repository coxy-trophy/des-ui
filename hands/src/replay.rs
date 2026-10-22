//! Recordings of a hand: one line a frame, so that gestures can be tried again on
//! the same movements without a camera. A recording begins `picture <width> <height>`;
//! then each frame is its time in seconds and either `-`, no hand, or how sure the
//! model is of the hand, which hand it is, and the 21 joints' x and y. A line
//! beginning `#` is a note, such as which gesture follows.

use crate::geometry::Point;
use crate::gestures::{Event, Gestures, Pose};
use crate::hand::Hand;

/// One frame, as a line of a recording.
pub fn line(time: f64, hand: Option<&Hand>) -> String {
    let Some(hand) = hand else {
        return format!("{time:.3} -");
    };
    let mut line = format!("{time:.3} {:.3} {:.3}", hand.presence, hand.right);
    for joint in &hand.joints {
        line.push_str(&format!(" {:.1} {:.1}", joint.x, joint.y));
    }
    line
}

/// Reads a recording: the picture's size, then each frame.
pub fn parse(text: &str) -> Result<((f32, f32), Vec<(f64, Option<Hand>)>), String> {
    // Lines beginning `#` say what the hand was asked to do, for whoever reads them.
    let mut lines = text.lines().filter(|line| !line.trim().is_empty() && !line.starts_with('#'));
    let size: Vec<f32> = lines
        .next()
        .and_then(|line| line.strip_prefix("picture "))
        .map(|rest| rest.split_whitespace().filter_map(|word| word.parse().ok()).collect())
        .unwrap_or_default();
    let [width, height] = size[..] else {
        return Err("a recording begins with the picture's size".into());
    };
    let mut frames = Vec::new();
    for (number, line) in lines.enumerate() {
        let bad = || format!("frame {} cannot be read", number + 1);
        let mut words = line.split_whitespace();
        let time: f64 = words.next().and_then(|word| word.parse().ok()).ok_or_else(bad)?;
        let rest: Vec<&str> = words.collect();
        if rest == ["-"] {
            frames.push((time, None));
            continue;
        }
        let numbers: Vec<f32> = rest.iter().map(|word| word.parse().map_err(|_| bad())).collect::<Result<_, _>>()?;
        if numbers.len() != 2 + 42 {
            return Err(bad());
        }
        let joints = std::array::from_fn(|joint| Point::new(numbers[2 + joint * 2], numbers[3 + joint * 2]));
        frames.push((time, Some(Hand { joints, depth: [0.0; 21], presence: numbers[0], right: numbers[1] })));
    }
    Ok(((width, height), frames))
}

/// What each frame of a recording was marked as, by its time: the note before it.
fn marks(text: &str) -> Vec<(f64, String)> {
    let mut mark = String::new();
    let mut marks = Vec::new();
    for line in text.lines() {
        if let Some(note) = line.strip_prefix('#') {
            mark = note.trim().to_string();
        } else if let Some(time) = line.split_whitespace().next().and_then(|word| word.parse().ok()) {
            marks.push((time, mark.clone()));
        }
    }
    marks
}

/// How often each pose was seen.
pub type Tally = Vec<(Pose, usize)>;

/// The gestures a recording makes, with their times, the pointer's moves left out;
/// and for each part of a marked recording, the poses the hand was seen in.
pub fn replay(path: &str) -> Result<(Vec<(f64, Event)>, Vec<(String, Tally)>), String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let ((width, height), frames) = parse(&text)?;
    let marks = marks(&text);
    let mut gestures = Gestures::default();
    let mut events = Vec::new();
    let mut poses: Vec<(String, Tally)> = Vec::new();
    for (time, hand) in frames {
        for event in gestures.update(hand.as_ref(), width, height, time) {
            if !matches!(event, Event::Motion { .. }) {
                events.push((time, event));
            }
        }
        let Some((_, mark)) = marks.iter().find(|(at, _)| *at == time).filter(|(_, mark)| !mark.is_empty() && hand.is_some()) else {
            continue;
        };
        if poses.last().is_none_or(|(last, _)| last != mark) {
            poses.push((mark.clone(), Vec::new()));
        }
        let tally = &mut poses.last_mut().unwrap().1;
        match tally.iter_mut().find(|(pose, _)| *pose == gestures.pose()) {
            Some((_, count)) => *count += 1,
            None => tally.push((gestures.pose(), 1)),
        }
    }
    Ok((events, poses))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recordings_read_back() {
        let hand = Hand {
            joints: std::array::from_fn(|joint| Point::new(joint as f32, 100.0 + joint as f32 / 2.0)),
            depth: [0.0; 21],
            presence: 0.99,
            right: 0.0,
        };
        let text = format!("picture 640 480\n# open\n{}\n{}\n", line(0.0, Some(&hand)), line(0.033, None));
        let ((width, height), frames) = parse(&text).unwrap();
        assert_eq!((width, height), (640.0, 480.0));
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].1.as_ref().unwrap().joints, hand.joints);
        assert_eq!(frames[1], (0.033, None));
        assert!(parse("0.0 -\n").is_err(), "no size");
        assert!(parse("picture 640 480\n0.0 0.9 1 2 3\n").is_err(), "too few joints");
    }
}
