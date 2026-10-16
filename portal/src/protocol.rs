//! What the portal asks the shell, and how the shell's answer becomes a portal
//! result. Pure, so that it can be tested without a bus or a shell.

use std::collections::HashMap;
use zbus::zvariant::{OwnedValue, Value};

/// The portal's response codes: done, the user said no, something else went wrong.
pub const SUCCESS: u32 = 0;
pub const CANCELLED: u32 = 1;
pub const FAILED: u32 = 2;

/// How long the shell may take: the user is choosing an area or a pixel meanwhile.
pub const INTERACTION: std::time::Duration = std::time::Duration::from_secs(120);

/// The shell's word for what a screenshot request wants. An interactive request
/// lets the user mark an area; otherwise it is the whole display.
pub fn screenshot_request(options: &HashMap<String, OwnedValue>) -> String {
    let interactive = options.get("interactive").and_then(|v| bool::try_from(v).ok()).unwrap_or(false);
    format!("screenshot-file {}", if interactive { "region" } else { "screen" })
}

/// The shell's word for picking a colour.
pub fn pick_color_request() -> &'static str {
    "pick-color"
}

/// One line of the shell's answer: `ok`, `ok <json>`, or `error <why>`.
pub enum Answer {
    Ok(Option<serde_json::Value>),
    Error(String),
}

pub fn parse_answer(line: &str) -> Answer {
    let line = line.trim_end_matches(['\n', '\r']);
    if let Some(rest) = line.strip_prefix("ok") {
        let rest = rest.trim();
        if rest.is_empty() {
            return Answer::Ok(None);
        }
        return match serde_json::from_str(rest) {
            Ok(json) => Answer::Ok(Some(json)),
            Err(error) => Answer::Error(format!("the shell's answer was not JSON: {error}")),
        };
    }
    Answer::Error(line.strip_prefix("error").map_or(line, str::trim).to_string())
}

/// A response and its results, as the portal returns them.
pub type Outcome = (u32, HashMap<String, OwnedValue>);

fn failure(answer: Answer) -> Outcome {
    match answer {
        Answer::Error(why) if why == "cancelled" => (CANCELLED, HashMap::new()),
        _ => (FAILED, HashMap::new()),
    }
}

/// `{"uri": "file:///…"}` from the shell becomes the `uri` result.
pub fn screenshot_outcome(answer: Answer) -> Outcome {
    if let Answer::Ok(Some(json)) = &answer {
        if let Some(uri) = json.get("uri").and_then(serde_json::Value::as_str).filter(|uri| uri.starts_with("file:///")) {
            let mut results = HashMap::new();
            results.insert("uri".to_string(), Value::from(uri.to_string()).try_into().expect("a string is a value"));
            return (SUCCESS, results);
        }
    }
    failure(answer)
}

/// `{"r": 0-255, "g": 0-255, "b": 0-255}` from the shell becomes the `color`
/// result: three doubles from 0 to 1, in sRGB.
pub fn color_outcome(answer: Answer) -> Outcome {
    if let Answer::Ok(Some(json)) = &answer {
        let channel = |name: &str| json.get(name).and_then(serde_json::Value::as_u64).filter(|v| *v <= 255).map(|v| v as f64 / 255.0);
        if let (Some(r), Some(g), Some(b)) = (channel("r"), channel("g"), channel("b")) {
            let mut results = HashMap::new();
            results.insert("color".to_string(), Value::from((r, g, b)).try_into().expect("a tuple of doubles is a value"));
            return (SUCCESS, results);
        }
    }
    failure(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(pairs: &[(&str, bool)]) -> HashMap<String, OwnedValue> {
        pairs.iter().map(|(key, flag)| (key.to_string(), Value::from(*flag).try_into().unwrap())).collect()
    }

    #[test]
    fn interactive_requests_let_the_user_mark_an_area() {
        assert_eq!(screenshot_request(&options(&[])), "screenshot-file screen");
        assert_eq!(screenshot_request(&options(&[("modal", true)])), "screenshot-file screen");
        assert_eq!(screenshot_request(&options(&[("interactive", true)])), "screenshot-file region");
        assert_eq!(screenshot_request(&options(&[("interactive", false)])), "screenshot-file screen");
    }

    #[test]
    fn the_shell_s_answers_are_read() {
        assert!(matches!(parse_answer("ok\n"), Answer::Ok(None)));
        assert!(matches!(parse_answer("ok {\"uri\":\"file:///a.png\"}\n"), Answer::Ok(Some(_))));
        assert!(matches!(parse_answer("error locked\n"), Answer::Error(why) if why == "locked"));
        assert!(matches!(parse_answer("ok {not json"), Answer::Error(_)));
        assert!(matches!(parse_answer("gibberish"), Answer::Error(_)));
    }

    #[test]
    fn a_saved_picture_is_a_uri_and_a_refusal_is_a_code() {
        let (response, results) = screenshot_outcome(parse_answer("ok {\"uri\":\"file:///home/me/Pictures/Screenshots/a.png\"}"));
        assert_eq!(response, SUCCESS);
        assert_eq!(String::try_from(results["uri"].clone()).unwrap(), "file:///home/me/Pictures/Screenshots/a.png");
        assert_eq!(screenshot_outcome(parse_answer("error cancelled")).0, CANCELLED);
        assert_eq!(screenshot_outcome(parse_answer("error locked")).0, FAILED);
        assert_eq!(screenshot_outcome(parse_answer("ok {\"uri\":\"http://evil\"}")).0, FAILED);
        assert_eq!(screenshot_outcome(parse_answer("ok")).0, FAILED);
    }

    #[test]
    fn a_picked_colour_is_three_doubles() {
        let (response, results) = color_outcome(parse_answer("ok {\"r\":255,\"g\":0,\"b\":51}"));
        assert_eq!(response, SUCCESS);
        let (r, g, b) = <(f64, f64, f64)>::try_from(results["color"].clone()).unwrap();
        assert!((r - 1.0).abs() < 1e-9 && g == 0.0 && (b - 0.2).abs() < 1e-9);
        assert_eq!(color_outcome(parse_answer("ok {\"r\":256,\"g\":0,\"b\":0}")).0, FAILED);
        assert_eq!(color_outcome(parse_answer("error cancelled")).0, CANCELLED);
    }
}
