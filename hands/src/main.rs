//! des-hands: follows a hand in front of the webcam. For now it reports what it sees
//! and how fast, once a second, so that tracking can be judged on the machine itself
//! before anything is controlled by it.
//!
//!   des-hands [--camera /dev/video0] [--seconds N]
//!   des-hands --picture photo.jpg     one still picture, every joint printed
//!   des-hands --describe              the models' outputs
//!   des-hands --camera-only           how fast the camera alone sends pictures

mod camera;
mod geometry;
mod hand;
mod models;
mod palm;

use std::time::{Duration, Instant};

use geometry::{Picture, Point};
use hand::Hand;
use models::Models;

/// Follows one hand from picture to picture.
struct Tracker {
    models: Models,
    anchors: Vec<Point>,
    /// Where to look for the hand next; none while no hand is followed.
    square: Option<geometry::Square>,
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
        if self.square.is_none() {
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

fn live(tracker: &mut Tracker, path: &str, seconds: Option<u64>) -> std::io::Result<()> {
    let started = Instant::now();
    let (mut since, mut frames, mut seen) = (Instant::now(), 0u32, 0u32);
    let mut last: Option<Hand> = None;
    camera::run(path, |picture| {
        frames += 1;
        if let Some(hand) = tracker.track(picture) {
            seen += 1;
            last = Some(hand);
        }
        if since.elapsed() >= Duration::from_secs(1) {
            let rate = frames as f64 / since.elapsed().as_secs_f64();
            let what = match &last {
                Some(hand) => {
                    let (wrist, tip) = (hand.joints[hand::WRIST], hand.joints[hand::INDEX_TIP]);
                    let pinch = hand.joints[hand::THUMB_TIP].distance(tip) / hand.joints[hand::WRIST].distance(hand.joints[hand::INDEX_BASE]).max(1.0);
                    format!(
                        "hand in {seen}/{frames}, {} {:.2}, wrist {:.0},{:.0} index tip {:.0},{:.0} pinch {pinch:.2}",
                        if hand.right >= 0.5 { "right" } else { "left" },
                        hand.presence,
                        wrist.x,
                        wrist.y,
                        tip.x,
                        tip.y
                    )
                }
                None => "no hand".to_string(),
            };
            println!(
                "{rate:5.1} fps  palm {:5.1} ms x{:<3} hand {:5.1} ms x{:<3} {what}",
                milliseconds(tracker.palm_time, tracker.palm_runs),
                tracker.palm_runs,
                milliseconds(tracker.hand_time, tracker.hand_runs),
                tracker.hand_runs,
            );
            (since, frames, seen, last) = (Instant::now(), 0, 0, None);
            (tracker.palm_time, tracker.palm_runs, tracker.hand_time, tracker.hand_runs) = (Duration::ZERO, 0, Duration::ZERO, 0);
        }
        seconds.is_none_or(|seconds| started.elapsed() < Duration::from_secs(seconds))
    })
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
    } else if let Some(path) = value("--picture") {
        still(&mut tracker, &path)
    } else {
        let seconds = value("--seconds").and_then(|seconds| seconds.parse().ok());
        live(&mut tracker, &value("--camera").unwrap_or("/dev/video0".into()), seconds).map_err(|e| e.to_string())
    };
    if let Err(e) = result {
        eprintln!("des-hands: {e}");
        std::process::exit(1);
    }
}
