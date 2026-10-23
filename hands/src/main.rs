//! des-hands: follows a hand in front of the webcam and says what it does: each
//! gesture as it happens, and once a second how fast, the pose and where the pointer
//! would be. Nothing is controlled by it yet.
//!
//!   des-hands [--camera /dev/video0] [--seconds N] [--record file]
//!   des-hands --serve [--speed S]     for the compositor: what the hand does, one
//!                                     line each on standard output
//!   des-hands --replay file           the gestures in a recording
//!   des-hands --learn file            asks for each gesture in turn, records them
//!                                     marked, and says how well each was seen
//!   des-hands --picture photo.jpg     one still picture, every joint printed
//!   des-hands --describe              the models' outputs
//!   des-hands --camera-only           how fast the camera alone sends pictures

mod camera;
mod geometry;
mod gestures;
mod hand;
mod models;
mod palm;
mod replay;

use std::time::{Duration, Instant};

use geometry::{Picture, Point};
use gestures::Gestures;
use hand::Hand;
use models::Models;

/// Follows one hand from picture to picture.
struct Tracker {
    models: Models,
    anchors: Vec<Point>,
    /// Where to look for the hand next; none while no hand is followed.
    square: Option<geometry::Square>,
    /// Pictures since the palm detector last looked, while searching, and when the
    /// last picture came.
    unsearched: u32,
    last_picture: Option<Instant>,
    input: Vec<f32>,
    /// Time spent in each model since the last report, and how often each ran.
    palm_time: Duration,
    palm_runs: u32,
    hand_time: Duration,
    hand_runs: u32,
}

impl Tracker {
    fn new(models: Models) -> Tracker {
        Tracker {
            models,
            anchors: palm::anchors(),
            square: None,
            unsearched: 0,
            last_picture: None,
            input: Vec::new(),
            palm_time: Duration::ZERO,
            palm_runs: 0,
            hand_time: Duration::ZERO,
            hand_runs: 0,
        }
    }

    /// The hand in this picture, if there is one: followed from the last picture, or
    /// else looked for afresh.
    fn track(&mut self, picture: &Picture) -> Option<Hand> {
        let gap = self.last_picture.replace(Instant::now()).map(|last| last.elapsed());
        if self.square.is_none() {
            // Searching costs more than following: when pictures come faster than it
            // can look at them, every other one will do.
            self.unsearched += 1;
            let fast = gap.is_some_and(|gap| gap < Duration::from_millis(70));
            if fast && self.unsearched < 2 {
                return None;
            }
            self.unsearched = 0;
            let began = Instant::now();
            palm::input(picture, &mut self.input);
            let answer = self.models.palm(&self.input).ok()?;
            let (boxes, scores) = models::palm_outputs(&answer)?;
            let palms = palm::decode(picture, &self.anchors, boxes, scores);
            self.palm_time += began.elapsed();
            self.palm_runs += 1;
            self.square = Some(palms.first()?.hand_square());
        }
        let square = self.square?;
        let began = Instant::now();
        hand::input(picture, &square, &mut self.input);
        let answer = self.models.hand(&self.input).ok();
        self.hand_time += began.elapsed();
        self.hand_runs += 1;
        let found = answer.as_ref().and_then(models::hand_outputs).and_then(|(joints, presence, right)| hand::decode(&square, joints, presence, right));
        match found {
            Some(hand) if hand.presence >= hand::PRESENT => {
                self.square = Some(hand.next_square());
                Some(hand)
            }
            // Lost: the palm detector looks again next time.
            _ => {
                self.square = None;
                None
            }
        }
    }
}

fn milliseconds(total: Duration, runs: u32) -> f64 {
    if runs == 0 { 0.0 } else { total.as_secs_f64() * 1000.0 / runs as f64 }
}

fn live(tracker: &mut Tracker, path: &str, seconds: Option<u64>, mut record: Option<std::fs::File>) -> std::io::Result<()> {
    use std::io::Write;
    let started = Instant::now();
    let (mut since, mut frames, mut seen) = (Instant::now(), 0u32, 0u32);
    let mut gestures = Gestures::default();
    let mut pointer: Option<(f32, f32)> = None;
    let mut header = false;
    let mut edge = false;
    camera::run(path, |picture| {
        frames += 1;
        let time = started.elapsed().as_secs_f64();
        let hand = tracker.track(picture);
        seen += hand.is_some() as u32;
        edge |= hand.as_ref().is_some_and(|hand| gestures::at_edge(hand, picture.width as f32, picture.height as f32));
        if let Some(file) = record.as_mut() {
            if !std::mem::replace(&mut header, true) {
                let _ = writeln!(file, "picture {} {}", picture.width, picture.height);
            }
            let _ = writeln!(file, "{}", replay::line(time, hand.as_ref()));
        }
        for event in gestures.update(hand.as_ref(), picture.width as f32, picture.height as f32, time) {
            match event {
                gestures::Event::Motion { dx, dy } => {
                    let (x, y) = pointer.unwrap_or((0.5, 0.5));
                    pointer = Some(((x + dx).clamp(0.0, 1.0), (y + dy).clamp(0.0, 1.0)));
                }
                other => println!("{time:7.2}  {other:?}"),
            }
        }
        if since.elapsed() >= Duration::from_secs(1) {
            let rate = frames as f64 / since.elapsed().as_secs_f64();
            let at = pointer.map_or("-".to_string(), |(x, y)| format!("{x:.2},{y:.2}"));
            println!(
                "{rate:5.1} fps  palm {:5.1} ms x{:<3} hand {:5.1} ms x{:<3} hand in {seen}/{frames}  {:?}  pointer {at}{}",
                milliseconds(tracker.palm_time, tracker.palm_runs),
                tracker.palm_runs,
                milliseconds(tracker.hand_time, tracker.hand_runs),
                tracker.hand_runs,
                gestures.pose(),
                if edge { "  (hand at the edge of the view: bring it up or back)" } else { "" },
            );
            (since, frames, seen, edge) = (Instant::now(), 0, 0, false);
            (tracker.palm_time, tracker.palm_runs, tracker.hand_time, tracker.hand_runs) = (Duration::ZERO, 0, Duration::ZERO, 0);
        }
        seconds.is_none_or(|seconds| started.elapsed() < Duration::from_secs(seconds))
    })
}

/// What a gesture is, told to the compositor.
fn message(event: gestures::Event, speed: f32) -> des_common::hands::Message {
    use des_common::hands::{Button, Message, Side};
    let button = |which: gestures::Button| if which == gestures::Button::Left { Button::Left } else { Button::Right };
    match event {
        gestures::Event::Motion { dx, dy } => Message::Motion { dx: dx * speed, dy: dy * speed },
        gestures::Event::Press(which) => Message::Press(button(which)),
        gestures::Event::Release(which) => Message::Release(button(which)),
        gestures::Event::Scroll(by) => Message::Scroll(by),
        gestures::Event::Sweep(side) => Message::Sweep(if side == gestures::Side::Left { Side::Left } else { Side::Right }),
        gestures::Event::Rest => Message::Rest,
        gestures::Event::Wake => Message::Wake,
        gestures::Event::Gone => Message::Gone,
    }
}

/// Follows the hand for the compositor, writing what it does to standard output;
/// `speed`, from -1 to 1, makes the pointer go from half as far to twice as far.
/// Stops when nobody is reading any more, or the compositor that started it is gone:
/// the camera is never left on behind its back.
fn serve(tracker: &mut Tracker, path: &str, speed: f32) -> std::io::Result<()> {
    use std::io::Write;
    let parent = std::os::unix::process::parent_id();
    let factor = 2f32.powf(speed.clamp(-1.0, 1.0));
    let started = Instant::now();
    let mut gestures = Gestures::default();
    let mut seen = false;
    let mut out = std::io::stdout().lock();
    let mut broken = false;
    camera::run(path, |picture| {
        let time = started.elapsed().as_secs_f64();
        let hand = tracker.track(picture);
        let mut lines = String::new();
        if hand.is_some() && !std::mem::replace(&mut seen, true) {
            lines.push_str(&des_common::hands::Message::Seen.encode());
        }
        for event in gestures.update(hand.as_ref(), picture.width as f32, picture.height as f32, time) {
            if event == gestures::Event::Gone {
                seen = false;
            }
            lines.push_str(&message(event, factor).encode());
        }
        broken = !lines.is_empty() && out.write_all(lines.as_bytes()).and_then(|()| out.flush()).is_err();
        !broken && std::os::unix::process::parent_id() == parent
    })
}

/// The gestures `--learn` asks for, each by its name in the recording and what to do.
const LESSONS: [(&str, &str); 6] = [
    ("open", "Open hand, palm to the camera, fingers apart: move it slowly about"),
    ("pinch", "Pinch your thumb and index fingertip together, and hold"),
    ("drag", "Pinch again, and keep pinching while you move your hand slowly"),
    ("two", "Index and middle fingers up, the other two folded down"),
    ("fist", "Make a fist"),
    ("away", "Take your hand out of view"),
];
/// How long to get ready for each, and to hold it, in seconds.
const READY: f64 = 2.5;
const HOLD: f64 = 4.0;

/// Asks for each gesture in turn and records the hand meanwhile, each part of the
/// recording marked with what was asked for; then says how each went.
fn learn(tracker: &mut Tracker, camera_path: &str, path: &str) -> Result<(), String> {
    use std::io::Write;
    let mut file = std::fs::File::create(path).map_err(|e| format!("cannot record to {path}: {e}"))?;
    let started = Instant::now();
    let mut lesson = usize::MAX;
    let mut header = false;
    // For each lesson: frames held, frames with a hand, and the pose each was seen in.
    let mut tally: Vec<(u32, u32, Vec<gestures::Pose>)> = vec![(0, 0, Vec::new()); LESSONS.len()];
    let mut before = gestures::Pose::Other;
    println!("Hold your hand up, about an arm's length from the camera, so that it sees your wrist too.");
    camera::run(camera_path, |picture| {
        let time = started.elapsed().as_secs_f64();
        let now = (time / (READY + HOLD)) as usize;
        if now >= LESSONS.len() {
            return false;
        }
        let holding = time - now as f64 * (READY + HOLD) >= READY;
        if now != lesson {
            lesson = now;
            println!("\n{}/{}  {}  (in {READY} seconds)", now + 1, LESSONS.len(), LESSONS[now].1);
        }
        let hand = tracker.track(picture);
        if !std::mem::replace(&mut header, true) {
            let _ = writeln!(file, "picture {} {}", picture.width, picture.height);
        }
        if holding {
            if tally[now].0 == 0 {
                println!("    now");
                let _ = writeln!(file, "# {}", LESSONS[now].0);
            }
            let _ = writeln!(file, "{}", replay::line(time, hand.as_ref()));
            tally[now].0 += 1;
            if let Some(hand) = &hand {
                tally[now].1 += 1;
                before = gestures::pose(hand, before);
                tally[now].2.push(before);
            }
        }
        true
    })
    .map_err(|e| e.to_string())?;
    println!("\nrecorded to {path}\n");
    for ((name, _), (frames, found, poses)) in LESSONS.iter().zip(&tally) {
        let mut counts: Vec<(gestures::Pose, usize)> = Vec::new();
        for pose in poses {
            match counts.iter_mut().find(|(seen, _)| seen == pose) {
                Some((_, count)) => *count += 1,
                None => counts.push((*pose, 1)),
            }
        }
        counts.sort_by(|a, b| b.1.cmp(&a.1));
        let poses: Vec<String> = counts.iter().map(|(pose, count)| format!("{pose:?} {count}")).collect();
        println!("{name:7} hand in {found}/{frames}  seen as: {}", if poses.is_empty() { "-".into() } else { poses.join(", ") });
    }
    Ok(())
}

fn still(tracker: &mut Tracker, path: &str) -> Result<(), String> {
    let decoded = image::open(path).map_err(|e| format!("cannot read {path}: {e}"))?.into_rgb8();
    let picture = Picture { width: decoded.width() as usize, height: decoded.height() as usize, rgb: decoded.into_raw() };
    palm::input(&picture, &mut tracker.input);
    let answer = tracker.models.palm(&tracker.input).map_err(|e| e.to_string())?;
    let (boxes, scores) = models::palm_outputs(&answer).ok_or("the palm detector answered in an unexpected shape")?;
    let palms = palm::decode(&picture, &tracker.anchors, boxes, scores);
    println!("{} palm(s)", palms.len());
    for palm in &palms {
        println!("palm {:.2} from {:.0},{:.0} to {:.0},{:.0}", palm.score, palm.min.x, palm.min.y, palm.max.x, palm.max.y);
    }
    // Twice over: the second time from the first answer's joints, as when following.
    for _ in 0..2 {
        match tracker.track(&picture) {
            Some(hand) => {
                println!("hand {:.2}, {}", hand.presence, if hand.right >= 0.5 { "right" } else { "left" });
                for (joint, point) in hand.joints.iter().enumerate() {
                    println!("  {joint:2} {:6.1} {:6.1} {:6.1}", point.x, point.y, hand.depth[joint]);
                }
            }
            None => println!("no hand"),
        }
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str| args.iter().position(|arg| arg == flag).and_then(|at| args.get(at + 1)).cloned();
    if args.iter().any(|arg| arg == "--camera-only") {
        let (mut since, mut frames) = (Instant::now(), 0);
        let result = camera::run(&value("--camera").unwrap_or("/dev/video0".into()), |_| {
            frames += 1;
            if since.elapsed() >= Duration::from_secs(1) {
                println!("{:5.1} fps", frames as f64 / since.elapsed().as_secs_f64());
                (since, frames) = (Instant::now(), 0);
            }
            true
        });
        if let Err(e) = result {
            eprintln!("des-hands: {e}");
        }
        return;
    }
    if let Some(path) = value("--replay") {
        match replay::replay(&path) {
            Ok((events, poses)) => {
                events.iter().for_each(|(time, event)| println!("{time:7.2}  {event:?}"));
                for (label, counts) in poses {
                    let counts: Vec<String> = counts.iter().map(|(pose, count)| format!("{pose:?} {count}")).collect();
                    println!("{label:7} seen as: {}", counts.join(", "));
                }
            }
            Err(e) => {
                eprintln!("des-hands: cannot replay {path}: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    let began = Instant::now();
    let models = match Models::load() {
        Ok(models) => models,
        Err(e) => {
            eprintln!("des-hands: cannot load the models: {e:?}");
            std::process::exit(1);
        }
    };
    eprintln!("models ready in {} ms", began.elapsed().as_millis());
    let mut tracker = Tracker::new(models);
    let result = if args.iter().any(|arg| arg == "--describe") {
        let palm = tracker.models.palm(&vec![0.0; palm::SIZE * palm::SIZE * 3]).map_err(|e| e.to_string());
        let hand = tracker.models.hand(&vec![0.0; hand::SIZE * hand::SIZE * 3]).map_err(|e| e.to_string());
        palm.and_then(|palm| hand.map(|hand| println!("palm: {}\nhand: {}", palm.describe(), hand.describe())))
    } else if args.iter().any(|arg| arg == "--serve") {
        let speed = value("--speed").and_then(|speed| speed.parse().ok()).unwrap_or(0.0);
        serve(&mut tracker, &value("--camera").unwrap_or("/dev/video0".into()), speed).map_err(|e| e.to_string())
    } else if let Some(path) = value("--learn") {
        learn(&mut tracker, &value("--camera").unwrap_or("/dev/video0".into()), &path)
    } else if let Some(path) = value("--picture") {
        still(&mut tracker, &path)
    } else {
        let seconds = value("--seconds").and_then(|seconds| seconds.parse().ok());
        let record = match value("--record").map(std::fs::File::create).transpose() {
            Ok(record) => record,
            Err(e) => {
                eprintln!("des-hands: cannot record: {e}");
                std::process::exit(1);
            }
        };
        live(&mut tracker, &value("--camera").unwrap_or("/dev/video0".into()), seconds, record).map_err(|e| e.to_string())
    };
    if let Err(e) = result {
        eprintln!("des-hands: {e}");
        std::process::exit(1);
    }
}
