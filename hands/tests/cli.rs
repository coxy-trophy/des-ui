use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_des-hands")).args(args).output().unwrap()
}

#[test]
fn help_exits_before_loading_models_or_opening_a_camera() {
    let output = run(&["--help", "--camera", "/definitely-missing-desui-camera"]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("Usage:"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("models"));
}

#[test]
fn unknown_options_are_rejected_before_camera_startup() {
    let output = run(&["--unknown", "--camera", "/definitely-missing-desui-camera"]);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("unknown option"), "{error}");
    assert!(!error.contains("models"));
}

#[test]
fn camera_only_failure_returns_nonzero() {
    let output = run(&["--camera-only", "--camera", "/definitely-missing-desui-camera"]);
    assert!(!output.status.success(), "camera failure must not report success");
}
