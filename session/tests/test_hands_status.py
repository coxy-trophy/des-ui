"""Exercise the real status publisher without a compositor, camera or socket.

The enclosing compositor needs a graphics/session environment. Compile its small
publisher method against an in-memory transport with the same queue contract.
"""
import pathlib
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


class HandStatusTests(unittest.TestCase):
    def test_camera_transitions_survive_command_processing_and_reconnect(self):
        source = (ROOT / "compositor/src/hands.rs").read_text()
        start = source.index("    fn tell_hands(")
        body = source.index("{", start)
        depth = 1
        end = body + 1
        while depth:
            depth += (source[end] == "{") - (source[end] == "}")
            end += 1
        method = source[start:end]
        harness = r'''
#[derive(Clone, Copy, PartialEq, Debug)]
enum Hands { Off, Watching, Following, Failed }
struct Event;
impl Event { fn hands(state: Hands) -> String { format!("{state:?}") } }
#[derive(Default)]
struct Transport { sent: Vec<String> }
impl Transport { fn send(&mut self, line: &str) { self.sent.push(line.into()); } }
struct DesComp { hands_state: Hands, ipc: Option<Transport>, unsaid: Vec<String> }
impl DesComp {
    fn tell(&mut self, line: String) {
        match self.ipc.as_mut() {
            Some(ipc) => ipc.send(&line),
            None => self.unsaid.push(line),
        }
    }
METHOD
}
fn main() {
    let mut state = DesComp { hands_state: Hands::Off, ipc: None, unsaid: vec![] };
    // Settings handles commands while the transport is temporarily taken out.
    state.tell_hands(Hands::Watching);
    state.tell_hands(Hands::Watching);
    assert_eq!(state.unsaid, ["Watching"], "camera-on must survive absent transport");
    let mut ipc = Transport::default();
    for line in std::mem::take(&mut state.unsaid) { ipc.send(&line); }
    state.ipc = Some(ipc);
    state.tell_hands(Hands::Following);
    let mut ipc = state.ipc.take().unwrap();
    state.tell_hands(Hands::Off);
    for line in std::mem::take(&mut state.unsaid) { ipc.send(&line); }
    assert_eq!(ipc.sent, ["Watching", "Following", "Off"]);
    state.tell_hands(Hands::Failed);
    assert_eq!(state.unsaid, ["Failed"]);
}
'''.replace("METHOD", method)
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory)
            (path / "status.rs").write_text(harness)
            subprocess.run(["rustc", "--edition=2024", str(path / "status.rs"),
                            "-o", str(path / "status")], check=True, capture_output=True)
            result = subprocess.run([str(path / "status")], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
