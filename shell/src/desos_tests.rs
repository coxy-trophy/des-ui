//! Tests of `desos.rs`. The fixtures are the documented examples: the result
//! object in desos/verify/README.md (schema `desos-verify-v1`) and objects of
//! desos/undo/rootfs/usr/share/desos-undo/schema-v1.json. Nothing here reads this
//! machine's own state or runs its tools: every path points into a temporary
//! directory, and the "tool" is a script written there.

use super::*;
use std::os::unix::fs::PermissionsExt;

// ------------------------------------------------------------ fixtures

/// The example of desos/verify/README.md, with its elided values filled in.
const VERIFY_QUICK_PASS: &str = r#"{
  "schema": "desos-verify-v1", "tool_version": 1, "mode": "quick", "overall": "pass",
  "summary": "DesOS consistent with its signed evidence (14 of 14 quick checks passed)",
  "counts": {"pass": 14, "fail": 0, "unknown": 0},
  "started": "2026-11-12T09:00:00Z", "finished": "2026-11-12T09:00:01Z", "duration_seconds": 0.412,
  "root": "/", "offline": false, "boot_id": "11111111-2222-3333-4444-555555555555",
  "key_fingerprint": "CDBC1835BA4F06D5F0BD4A9000B6AE1830FF9F59", "key_pinned": false,
  "manifest_sha256": "0f0e0d",
  "state": {"kernel": "desos", "kernel_release": "6.18.62-desos", "secure_boot": "disabled", "root_filesystem": "ext4", "session": "installed"},
  "checks": [
    {"id": "signatures.manifest", "group": "signatures", "status": "pass", "reason": "manifest.json is signed by the shipped key and lists 7 packages", "details": {}}
  ],
  "skipped": ["repository.pool", "packages.files"],
  "last_full": {"overall": "pass", "finished": "2026-11-11T21:14:03Z", "counts": {"pass": 16, "fail": 0, "unknown": 0}, "failed": [], "same_manifest": true},
  "assurance": "self-reported by software on the system being checked",
  "a_field_added_later": {"ignored": true}
}"#;

fn verify(change: impl FnOnce(&mut Value)) -> Vec<u8> {
    let mut value: Value = serde_json::from_str(VERIFY_QUICK_PASS).unwrap();
    change(&mut value);
    serde_json::to_vec(&value).unwrap()
}

fn result(bytes: &[u8]) -> Verify {
    match parse_verify(bytes, Some("11111111-2222-3333-4444-555555555555")) {
        Integrity::Result(verify) => verify,
        other => panic!("expected a result, got {other:?}"),
    }
}

fn line(bytes: &[u8]) -> Line { parse_verify(bytes, Some("11111111-2222-3333-4444-555555555555")).line() }

/// The snapshot example of desos/undo/README.md.
const SNAPSHOT_POST: &str = r#"{"schema": 1, "id": 14, "kind": "post", "reason": "after: apt upgraded nano",
 "created": "2026-11-12T10:03:22Z", "kernel": "6.18.62-desos",
 "boot": {"kernels": ["6.18.62-desos", "7.0.0-12-generic"]},
 "pre_id": 13, "post_id": null, "command": null, "exit_code": null,
 "packages": {"installed": [], "removed": [], "upgraded": [{"name": "nano", "from": "7.2-1", "to": "8.0-1"}], "downgraded": []},
 "planned": null, "protected": false}"#;

const SNAPSHOT_PRE: &str = r#"{"schema": 1, "id": 13, "kind": "pre", "reason": "before: apt upgrade nano", "created": "2026-11-12T10:03:01Z",
 "pre_id": null, "post_id": 14, "packages": null,
 "planned": {"installed": [{"name": "libfoo1", "version": "1.0"}], "removed": [], "upgraded": [{"name": "nano", "from": "7.2-1", "to": "8.0-1"}], "downgraded": []}}"#;

fn envelope(command: &str, ok: bool, supported: bool, code: Option<&str>, message: &str, data: &str) -> String {
    let code = code.map_or("null".to_string(), |code| format!("\"{code}\""));
    // One line, as the tool prints it.
    format!(r#"{{"schema": 1, "command": "{command}", "ok": {ok}, "supported": {supported}, "code": {code}, "message": "{message}", "data": {data}}}"#).replace('\n', " ")
}

fn list_answer() -> String {
    let good = r#"{"schema": 1, "id": 12, "kind": "boot-good", "reason": "start reached the sign-in screen", "created": "2026-11-12T08:00:00Z", "protected": true}"#;
    let odd = r#"{"schema": 1, "id": 15, "kind": "a-kind-added-later", "reason": "", "created": null}"#;
    envelope("list", true, true, None, "4 snapshots.", &format!(r#"{{"snapshots": [{good}, {SNAPSHOT_PRE}, {SNAPSHOT_POST}, {odd}, {{"id": "sixteen"}}, 7]}}"#))
}

const STATUS_DATA: &str = r#"{"live": false,
 "root": {"fstype": "btrfs", "uuid": "3f2a8c1e-9b7d-4e55-a1c2-0d9e8f7a6b5c", "subvolume": "@", "booted_snapshot": null, "restart_pending": true},
 "snapshots": {"count": 4, "newest": 15},
 "boot": {"mode": "grub-counted", "threshold": 3, "env": "/boot/efi/desos-undo/env", "tries": 0, "good_id": 12, "healed_id": null, "marked_good": true},
 "rollback": {"target": 13, "undo": 15, "time": "2026-11-12T10:20:00Z", "reason": "Chosen in DEs-UI settings", "automatic": false, "warnings": []},
 "transaction": null, "notices": [], "retention": {"transactions": 10, "manual": 10}}"#;

const DIFF_DATA: &str = r#"{"from": 13, "to": "current", "paths": ["/etc"],
 "packages": {"installed": [{"name": "libfoo1", "version": "1.0"}], "removed": [], "upgraded": [{"name": "nano", "from": "7.2-1", "to": "8.0-1"}], "downgraded": []},
 "files": {"added": ["/etc/foo.conf"], "removed": [], "modified": ["/etc/nanorc", "/etc/ld.so.cache"], "truncated": false}}"#;

const NOTICES_DATA: &str = r#"{"notices": [
 {"id": 3, "time": "2026-11-12T07:59:00Z", "kind": "auto-recovery", "message": "DesOS failed to start 3 times and went back to snapshot 12. The state that failed is kept as snapshot 16.", "acknowledged": false, "snapshot": 12, "undo": 16},
 {"id": 2, "time": "2026-11-10T07:59:00Z", "kind": "rollback", "message": "An older one.", "acknowledged": true, "snapshot": 9, "undo": 10},
 {"id": "x", "message": "no usable id"}, {"id": 5, "kind": "rollback", "message": "", "acknowledged": false}]}"#;

const EXT4: &str = "Undo is not available: the system is on ext4, and snapshots need btrfs. Reinstall on btrfs to use it.";

// ------------------------------------------------------------ which system

#[test]
fn only_an_os_release_with_id_desos_is_desos() {
    assert!(is_desos("NAME=\"DesOS\"\nID=desos\nID_LIKE=ubuntu debian\n"));
    assert!(is_desos("ID=\"desos\"\n"));
    assert!(!is_desos("NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\n"));
    assert!(!is_desos("ID=ubuntu\nID_LIKE=desos\nVERSION_ID=desos\n"), "only ID decides");
    assert!(!is_desos("ID=desos-like\n") && !is_desos("") && !is_desos("# ID=desos"));
}

// ------------------------------------------------------------ verify: parsing

#[test]
fn the_documented_result_is_read_and_unknown_fields_are_ignored() {
    let verify = result(VERIFY_QUICK_PASS.as_bytes());
    assert_eq!(verify.overall, Verdict::Pass);
    assert!(!verify.full && !verify.stale);
    assert_eq!(verify.summary, "DesOS consistent with its signed evidence (14 of 14 quick checks passed)");
    assert_eq!(verify.key_fingerprint.as_deref(), Some("CDBC1835BA4F06D5F0BD4A9000B6AE1830FF9F59"));
    assert_eq!(verify.checks, vec![Check { id: "signatures.manifest".into(), status: Verdict::Pass, reason: "manifest.json is signed by the shipped key and lists 7 packages".into() }]);
    assert_eq!(verify.last_full, Some(LastFull { overall: Verdict::Pass, finished: Some("2026-11-11T21:14:03Z".into()), failed: vec![], same_manifest: true }));
}

#[test]
fn what_cannot_be_read_is_unreadable_and_never_a_pass() {
    let cases: [&[u8]; 9] = [b"", b"{", b"not json", b"[1, 2]", b"\"pass\"", b"null", b"{}",
        br#"{"schema": "desos-verify-v2", "overall": "pass"}"#, br#"{"schema": 1, "overall": "pass"}"#];
    for bytes in cases {
        let integrity = parse_verify(bytes, None);
        assert!(matches!(integrity, Integrity::Unreadable(_)), "{:?} gave {integrity:?}", String::from_utf8_lossy(bytes));
        assert_eq!(integrity.line().value, "UNREADABLE");
    }
    let Integrity::Unreadable(why) = parse_verify(br#"{"schema": "desos-verify-v2"}"#, None) else { panic!() };
    assert!(why.contains("desos-verify-v2"), "{why}");
    let huge = vec![b' '; MAX_VERIFY_BYTES as usize + 1];
    assert!(matches!(parse_verify(&huge, None), Integrity::Unreadable(_)));
}

#[test]
fn a_missing_or_unrecognised_verdict_is_unknown() {
    for overall in [Value::Null, Value::from("PASS"), Value::from("ok"), Value::from(true), Value::from(1)] {
        assert_eq!(result(&verify(|value| value["overall"] = overall.clone())).overall, Verdict::Unknown, "{overall}");
    }
    assert_eq!(result(&verify(|value| { value.as_object_mut().unwrap().remove("overall"); })).overall, Verdict::Unknown);
    // Checks of the wrong shape are passed over; one with an unknown status counts as unknown.
    let odd = verify(|value| value["checks"] = serde_json::json!([7, "x", {"id": "kernel.image", "status": "skipped-by-a-later-version"}]));
    let read = result(&odd);
    assert_eq!(read.checks.len(), 1);
    assert_eq!(read.overall, Verdict::Unknown);
    assert_eq!(result(&verify(|value| value["checks"] = Value::from("none"))).checks, vec![]);
}

#[test]
fn a_failing_check_outweighs_an_overall_pass() {
    let bytes = verify(|value| value["checks"].as_array_mut().unwrap().push(serde_json::json!({"id": "packages.installed", "status": "fail", "reason": "des-ui is not installed"})));
    assert_eq!(result(&bytes).overall, Verdict::Fail);
    assert_eq!(line(&bytes).value, "DIFFERS");
}

#[test]
fn text_from_the_system_is_made_safe_to_draw() {
    let bytes = verify(|value| {
        value["summary"] = Value::from(format!("line one\nline two\u{1b}[31m {}", "x".repeat(500)));
        value["checks"][0]["reason"] = Value::from("tab\there\u{7}");
    });
    let read = result(&bytes);
    assert!(!read.summary.chars().any(char::is_control) && read.summary.chars().count() <= 241, "{:?}", read.summary);
    assert_eq!(read.checks[0].reason, "tab here");
}

#[test]
fn the_result_file_is_read_with_every_failure_tolerated() {
    let dir = tempfile::tempdir().unwrap();
    let mut paths = paths_in(dir.path());
    assert_eq!(read_verify(&paths), Integrity::NotRun, "no file: the check has not run yet this start");
    std::fs::write(&paths.verify, VERIFY_QUICK_PASS).unwrap();
    assert!(matches!(read_verify(&paths), Integrity::Result(ref verify) if verify.overall == Verdict::Pass && !verify.stale), "no boot id to compare with");
    std::fs::write(&paths.boot_id, "11111111-2222-3333-4444-555555555555\n").unwrap();
    assert!(matches!(read_verify(&paths), Integrity::Result(ref verify) if !verify.stale));
    std::fs::write(&paths.boot_id, "99999999-2222-3333-4444-555555555555\n").unwrap();
    assert!(matches!(read_verify(&paths), Integrity::Result(ref verify) if verify.stale));
    std::fs::write(&paths.verify, vec![b'{'; MAX_VERIFY_BYTES as usize + 10]).unwrap();
    assert!(matches!(read_verify(&paths), Integrity::Unreadable(_)), "oversized");
    std::fs::write(&paths.verify, [0xff, 0xfe, 0x00]).unwrap();
    assert!(matches!(read_verify(&paths), Integrity::Unreadable(_)), "not text");
    paths.verify = dir.path().to_path_buf();
    assert!(matches!(read_verify(&paths), Integrity::Unreadable(_)), "a directory");
}

// ------------------------------------------------------------ verify: the line

#[test]
fn the_line_follows_the_result() {
    // Quick pass, last full check passed against the same record.
    let both = line(VERIFY_QUICK_PASS.as_bytes());
    assert_eq!((both.value, both.tone), ("MATCHES", Tone::Plain));
    // Quick pass alone says less, and says so.
    let quick = line(&verify(|value| value["last_full"] = Value::Null));
    assert_eq!((quick.value, quick.tone), ("QUICK MATCH", Tone::Plain));
    assert!(quick.detail.contains("as far as the quick check looks"));
    let other_record = line(&verify(|value| value["last_full"]["same_manifest"] = Value::from(false)));
    assert_eq!(other_record.value, "QUICK MATCH", "a full check of another record is no support");
    let full = line(&verify(|value| { value["mode"] = Value::from("full"); value["last_full"] = Value::Null; }));
    assert_eq!(full.value, "MATCHES");
    assert!(full.detail.starts_with("Matches the signed build record"));
    let later_mode = line(&verify(|value| { value["mode"] = Value::from("deep"); value["last_full"] = Value::Null; }));
    assert_eq!(later_mode.value, "QUICK MATCH", "an unknown mode gets the weaker wording");

    // The Ubuntu compatibility kernel: unknown, not a failure.
    let unknown = line(&verify(|value| {
        value["overall"] = Value::from("unknown");
        value["checks"].as_array_mut().unwrap().push(serde_json::json!({"id": "kernel.release", "status": "unknown", "reason": "the Ubuntu compatibility kernel is running; DesOS evidence does not cover it"}));
    }));
    assert_eq!((unknown.value, unknown.tone), ("UNCONFIRMED", Tone::Quiet));
    assert!(unknown.detail.contains("kernel.release: the Ubuntu compatibility kernel is running"));
    assert!(!unknown.detail.contains("signatures.manifest"), "only what did not pass is listed");

    let failed = line(&verify(|value| {
        value["overall"] = Value::from("fail");
        value["checks"].as_array_mut().unwrap().push(serde_json::json!({"id": "identity.os-release", "status": "fail", "reason": "/etc/os-release differs from the signed desos-release package"}));
    }));
    assert_eq!((failed.value, failed.tone), ("DIFFERS", Tone::Caution));
    assert!(failed.detail.contains("identity.os-release: /etc/os-release differs from the signed desos-release package"));

    // A quick pass does not clear what the last full check of the same record found.
    let full_failed = line(&verify(|value| { value["last_full"]["overall"] = Value::from("fail"); value["last_full"]["failed"] = serde_json::json!(["packages.files"]); }));
    assert_eq!((full_failed.value, full_failed.tone), ("DIFFERS", Tone::Caution));
    assert!(full_failed.detail.contains("packages.files"));

    let stale = parse_verify(VERIFY_QUICK_PASS.as_bytes(), Some("another-start")).line();
    assert_eq!((stale.value, stale.tone), ("UNCONFIRMED", Tone::Quiet));
    assert_eq!(Integrity::NotRun.line().value, "NOT CHECKED");
}

#[test]
fn no_wording_claims_more_than_a_match_with_the_signed_record() {
    let lines = [
        line(VERIFY_QUICK_PASS.as_bytes()),
        line(&verify(|value| value["last_full"] = Value::Null)),
        line(&verify(|value| value["mode"] = Value::from("full"))),
        line(&verify(|value| value["overall"] = Value::from("unknown"))),
        line(&verify(|value| value["overall"] = Value::from("fail"))),
        Integrity::NotRun.line(),
        Integrity::Unreadable("x".into()).line(),
    ];
    for line in &lines {
        for word in ["SECURE", "SAFE", "TRUSTED", "VERIFIED", "PROTECTED", "GENUINE", "OK"] {
            assert!(!line.value.contains(word), "{:?} overclaims", line.value);
        }
        assert!(line.value.chars().count() <= 11, "{:?} must fit beside its label in the narrowest column", line.value);
        // The limit is said every time, and is the only place the word "secure" appears.
        assert!(line.detail.ends_with(CAVEAT), "{}", line.detail);
        let before = line.detail.strip_suffix(CAVEAT).unwrap().to_lowercase();
        for word in ["secure", "safe", "trusted", "tamper", "genuine", "untouched"] {
            assert!(!before.contains(word), "{before:?} overclaims with {word:?}");
        }
    }
    assert!(CAVEAT.contains("does not show that the system is secure") && CAVEAT.contains("administrator access"));
}

// ------------------------------------------------------------ undo: parsing

#[test]
fn the_answer_is_the_last_line_of_output() {
    let answer = envelope("snapshot", true, true, None, "Created snapshot 14.", &format!(r#"{{"snapshot": {}}}"#, SNAPSHOT_POST.replace('\n', "")));
    let read = parse_envelope(format!("apt said something\n{{\"schema\": 1}}\n{answer}\n\n").as_bytes()).unwrap();
    assert!(read.ok && read.supported);
    assert_eq!((read.code.as_deref(), read.message.as_str()), (None, "Created snapshot 14."));
    assert_eq!(read.data["snapshot"]["id"], 14);
}

#[test]
fn answers_that_cannot_be_used_say_why() {
    assert_eq!(parse_envelope(b""), Err(Problem::Malformed));
    assert_eq!(parse_envelope(b"\n  \n"), Err(Problem::Malformed));
    assert_eq!(parse_envelope(b"Traceback (most recent call last):\n  boom"), Err(Problem::Malformed));
    assert_eq!(parse_envelope(b"[1]"), Err(Problem::Malformed));
    assert_eq!(parse_envelope(br#"{"ok": true, "supported": true}"#), Err(Problem::Malformed), "no schema");
    assert_eq!(parse_envelope(br#"{"schema": 1, "ok": "yes", "supported": true}"#), Err(Problem::Malformed));
    assert_eq!(parse_envelope(br#"{"schema": 1, "ok": true}"#), Err(Problem::Malformed));
    assert_eq!(parse_envelope(br#"{"schema": 2, "ok": true, "supported": true}"#), Err(Problem::UnknownSchema("2".into())));
    assert_eq!(parse_envelope(br#"{"schema": "1", "ok": true, "supported": true}"#), Err(Problem::UnknownSchema("\"1\"".into())));
    assert_eq!(parse_envelope(&vec![b'x'; MAX_OUTPUT_BYTES + 1]), Err(Problem::Oversized));
    // Missing optional parts are tolerated; an unknown code is carried, not rejected.
    let bare = parse_envelope(br#"{"schema": 1, "ok": false, "supported": true, "code": "a-code-added-later", "data": [1]}"#).unwrap();
    assert_eq!((bare.message.as_str(), bare.code.as_deref()), ("", Some("a-code-added-later")));
    assert!(bare.data.is_object());
    for problem in [Problem::Absent, Problem::Timeout, Problem::Oversized, Problem::Malformed, Problem::UnknownSchema("2".into()), Problem::Denied, Problem::Failed("x".into())] {
        assert!(problem.to_string().ends_with('.') || matches!(problem, Problem::Failed(_)), "{problem}");
    }
}

#[test]
fn the_snapshot_list_is_newest_first_and_says_what_changed_and_why() {
    let Fetch::Ready(snapshots) = snapshots_from(parse_envelope(list_answer().as_bytes())) else { panic!("expected a list") };
    assert_eq!(snapshots.iter().map(|snapshot| snapshot.id).collect::<Vec<_>>(), vec![15, 14, 13, 12], "entries without a usable id are left out");
    let post = &snapshots[1];
    assert_eq!((post.kind, post.reason.as_str()), (Kind::Post, "after: apt upgraded nano"));
    assert_eq!(post.what_changed(), "upgraded nano 7.2-1 → 8.0-1", "the package database's own difference");
    assert_eq!(post.kernel.as_deref(), Some("6.18.62-desos"));
    let pre = &snapshots[2];
    assert_eq!(pre.what_changed(), "planned: upgraded nano 7.2-1 → 8.0-1 · installed libfoo1 1.0", "what apt announced, marked as such");
    assert!(pre.crumb().starts_with("#13 · BEFORE A CHANGE · "), "{}", pre.crumb());
    assert!(pre.crumb().contains("2026"));
    let good = &snapshots[3];
    assert_eq!((good.kind, good.what_changed().as_str()), (Kind::BootGood, ""));
    assert!(good.protected && good.crumb().ends_with("· KEPT"));
    assert_eq!((snapshots[0].kind, snapshots[0].created.clone()), (Kind::Unknown, None), "a kind added later is still listed");
    assert_eq!(snapshots[0].crumb(), "#15 · SNAPSHOT");
}

#[test]
fn many_package_changes_are_counted_not_listed() {
    let change = |name: &str| PackageChange { name: name.into(), version: Some("1".into()), from: None, to: None };
    let packages = Packages { installed: vec![change("a"), change("b"), change("c")], removed: vec![change("x"), change("y")], ..Default::default() };
    assert_eq!(packages.summary(), "installed 3 packages · removed x and y");
    assert_eq!(packages.lines(), vec!["Installed a 1", "Installed b 1", "Installed c 1", "Removed x 1", "Removed y 1"]);
    assert!(Packages::default().is_empty() && Packages::default().summary().is_empty());
}

#[test]
fn where_undo_is_unsupported_the_tools_reason_is_kept() {
    // status still succeeds; other commands report ok false. Both are "unsupported", not failures.
    let status = envelope("status", true, false, Some("not-btrfs"), EXT4, "{}");
    assert_eq!(status_from(parse_envelope(status.as_bytes())), Fetch::Unsupported(EXT4.into()));
    let list = envelope("list", false, false, Some("not-btrfs"), EXT4, "{}");
    assert_eq!(snapshots_from(parse_envelope(list.as_bytes())), Fetch::Unsupported(EXT4.into()));
    let live = envelope("list", false, false, Some("live-session"), "This is the live session; nothing is installed yet, so there is nothing to undo.", "{}");
    assert!(matches!(snapshots_from(parse_envelope(live.as_bytes())), Fetch::Unsupported(why) if why.starts_with("This is the live session")));
    let silent = envelope("list", false, false, Some("a-code-added-later"), "", "{}");
    assert!(matches!(snapshots_from(parse_envelope(silent.as_bytes())), Fetch::Unsupported(why) if !why.is_empty()));
    // A real failure, and every problem running the tool, is a failure with a sentence.
    let failed = envelope("list", false, true, Some("error"), "The snapshot store could not be read.", "{}");
    assert_eq!(snapshots_from(parse_envelope(failed.as_bytes())), Fetch::Failed("The snapshot store could not be read.".into()));
    assert_eq!(snapshots_from(Err(Problem::Timeout)), Fetch::Failed(Problem::Timeout.to_string()));
    let shapeless = envelope("list", true, true, None, "", r#"{"snapshots": "none"}"#);
    assert!(matches!(snapshots_from(parse_envelope(shapeless.as_bytes())), Fetch::Failed(_)));
}

#[test]
fn status_reports_a_pending_restart_and_the_known_good_snapshot() {
    let answer = envelope("status", true, true, None, "Undo is active.", STATUS_DATA);
    let Fetch::Ready(status) = status_from(parse_envelope(answer.as_bytes())) else { panic!() };
    assert!(status.restart_pending);
    assert_eq!(status.booted_snapshot, None);
    assert_eq!(status.good_id, Some(12));
    assert_eq!(status.rollback, Some(Rollback { target: 13, undo: Some(15), time: Some("2026-11-12T10:20:00Z".into()), automatic: false }));
    // A status with parts missing is still a status.
    let thin = envelope("status", true, true, None, "", r#"{"root": {"booted_snapshot": 9}, "rollback": null}"#);
    assert_eq!(status_from(parse_envelope(thin.as_bytes())), Fetch::Ready(Status { booted_snapshot: Some(9), ..Default::default() }));
}

#[test]
fn a_diff_lists_packages_and_files() {
    let answer = envelope("diff", true, true, None, "Snapshot 13 compared with current.", DIFF_DATA);
    let Fetch::Ready(diff) = diff_from(parse_envelope(answer.as_bytes())) else { panic!() };
    assert_eq!((diff.from, diff.to.as_str(), diff.paths.clone()), (13, "current", vec!["/etc".to_string()]));
    assert_eq!(diff.packages.lines(), vec!["Upgraded nano 7.2-1 → 8.0-1", "Installed libfoo1 1.0"]);
    assert_eq!((diff.added.clone(), diff.removed.len(), diff.modified.len(), diff.truncated), (vec!["/etc/foo.conf".to_string()], 0, 2, false));
    assert!(!diff.is_empty());
    let between = envelope("diff", true, true, None, "", r#"{"from": 13, "to": 14, "paths": [], "packages": {}, "files": {"added": [], "removed": [], "modified": [], "truncated": true}}"#);
    let Fetch::Ready(diff) = diff_from(parse_envelope(between.as_bytes())) else { panic!() };
    assert!(diff.to == "14" && diff.truncated && diff.is_empty());
    let gone = envelope("diff", false, true, Some("no-such-snapshot"), "There is no snapshot 99.", "{}");
    assert_eq!(diff_from(parse_envelope(gone.as_bytes())), Fetch::Failed("There is no snapshot 99.".into()));
    assert!(matches!(diff_from(parse_envelope(envelope("diff", true, true, None, "", "{}").as_bytes())), Fetch::Failed(_)), "a diff without its snapshot number");
}

#[test]
fn only_notices_not_yet_acknowledged_are_shown() {
    let answer = parse_envelope(envelope("notices", true, true, None, "…", &NOTICES_DATA.replace('\n', "")).as_bytes()).unwrap();
    let notices = unread_notices(&answer);
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert_eq!((notices[0].id, notices[0].kind.as_str()), (3, "auto-recovery"));
    assert!(notices[0].message.starts_with("DesOS failed to start 3 times"));
    let unsupported = parse_envelope(envelope("notices", true, false, Some("live-session"), "live", r#"{"notices": []}"#).as_bytes()).unwrap();
    assert!(unread_notices(&unsupported).is_empty());
}

#[test]
fn a_privileged_action_reports_what_the_tool_said() {
    let done = envelope("rollback", true, true, None, "Snapshot 13 becomes the system at the next restart. Home directories are untouched.", r#"{"restart_required": true}"#);
    assert_eq!(outcome_from(parse_envelope(done.as_bytes())), (true, "Snapshot 13 becomes the system at the next restart. Home directories are untouched.".into()));
    let pending = envelope("rollback", false, true, Some("restart-pending"), "A rollback is already waiting for a restart.", "{}");
    assert_eq!(outcome_from(parse_envelope(pending.as_bytes())), (false, "A rollback is already waiting for a restart.".into()));
    assert_eq!(outcome_from(Err(Problem::Denied)), (false, "Not authorised. Nothing was changed.".into()));
    assert_eq!(outcome_from(parse_envelope(envelope("snapshot", false, false, Some("not-btrfs"), EXT4, "{}").as_bytes())), (false, EXT4.into()));
}

// ------------------------------------------------------------ commands

#[test]
fn every_command_is_a_fixed_argument_list() {
    let paths = Paths::system();
    let words = |action| { let (program, arguments) = command(&paths, action); (program.to_string_lossy().into_owned(), arguments) };
    let list = |words: &[&str]| words.iter().map(|word| word.to_string()).collect::<Vec<_>>();
    assert_eq!(words(Action::Status), ("/usr/sbin/desos-undo".into(), list(&["--json", "status"])));
    assert_eq!(words(Action::List), ("/usr/sbin/desos-undo".into(), list(&["--json", "list"])));
    assert_eq!(words(Action::Diff(13)), ("/usr/sbin/desos-undo".into(), list(&["--json", "diff", "13"])));
    assert_eq!(words(Action::Notices), ("/usr/sbin/desos-undo".into(), list(&["--json", "notices"])));
    assert_eq!(words(Action::AckNotices), ("/usr/sbin/desos-undo".into(), list(&["--json", "notices", "--ack"])));
    // Root only through pkexec, naming the tool by its full path, with nothing the user typed.
    assert_eq!(words(Action::Snapshot), ("/usr/bin/pkexec".into(), list(&["/usr/sbin/desos-undo", "--json", "snapshot", "--reason", "Taken from DEs-UI settings"])));
    assert_eq!(words(Action::Rollback(13)), ("/usr/bin/pkexec".into(), list(&["/usr/sbin/desos-undo", "--json", "rollback", "13", "--reason", "Chosen in DEs-UI settings"])));
    for action in [Action::Status, Action::List, Action::Diff(u64::MAX), Action::Notices, Action::AckNotices, Action::Snapshot, Action::Rollback(u64::MAX)] {
        let (program, arguments) = command(&paths, action);
        assert_eq!(action.privileged(), program == paths.pkexec, "{action:?}");
        assert!(program.is_absolute());
        assert!(!arguments.iter().any(|word| word == "sh" || word == "-c" || word == "run" || word == "--"), "{arguments:?}");
        // The only variable word is a number.
        let fixed = ["/usr/sbin/desos-undo", "--json", "status", "list", "diff", "notices", "--ack", "snapshot", "rollback", "--reason", SNAPSHOT_REASON, ROLLBACK_REASON];
        for word in arguments.iter().filter(|word| !fixed.contains(&word.as_str())) { assert!(word.parse::<u64>().is_ok(), "{word:?}"); }
    }
    assert!(Action::Rollback(1).timeout() > Action::Diff(1).timeout() && Action::Diff(1).timeout() > Action::List.timeout());
}

// ------------------------------------------------------------ running

fn paths_in(dir: &Path) -> Paths {
    Paths { os_release: dir.join("os-release"), verify: dir.join("verify.json"), boot_id: dir.join("boot_id"), undo: dir.join("desos-undo"), pkexec: dir.join("pkexec") }
}

/// Writes an executable script, and waits until it can be started: another test
/// thread forking at the wrong moment holds it open for writing for an instant.
fn script(path: &Path, body: &str) {
    std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    for _ in 0..200 {
        match Command::new(path).arg("probe").stdout(Stdio::null()).stderr(Stdio::null()).status() {
            Err(error) if error.raw_os_error() == Some(26) => std::thread::sleep(Duration::from_millis(5)),
            _ => return,
        }
    }
}

/// A stand-in tool: answers each command from `<command>.json` beside it, after a
/// line of noise, and notes how it was called.
fn fake_tool(dir: &Path) -> Paths {
    let paths = paths_in(dir);
    script(&paths.undo, r#"here=$(dirname "$0")
[ "$1" = probe ] && exit 0
echo "$@" >> "$here/calls"
echo "a line that is not the answer"
cat "$here/$2.json""#);
    script(&paths.pkexec, r#"[ "$1" = probe ] && exit 0
echo "pkexec $@" >> "$(dirname "$0")/calls"
exec "$@""#);
    std::fs::write(&paths.os_release, "NAME=DesOS\nID=desos\n").unwrap();
    paths
}

fn calls(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("calls")).unwrap_or_default().lines().map(str::to_string).collect()
}

fn sh(body: &str, timeout: Duration, kill: bool) -> Result<Ran, Problem> {
    run(Path::new("/bin/sh"), &["-c".to_string(), body.to_string()], timeout, kill)
}

#[test]
fn running_tolerates_absence_silence_and_floods() {
    assert!(matches!(run(Path::new("/nonexistent/desos-undo"), &[], READ_TIMEOUT, true), Err(Problem::Absent)));
    let ran = sh("echo hello; exit 3", READ_TIMEOUT, true).ok().unwrap();
    assert_eq!((ran.code, ran.stdout.as_slice()), (Some(3), b"hello\n".as_slice()));
    let started = Instant::now();
    assert!(matches!(sh("sleep 30", Duration::from_millis(150), true), Err(Problem::Timeout)));
    assert!(matches!(sh("exec >&-; sleep 30", Duration::from_millis(150), true), Err(Problem::Timeout)), "output closed, still running");
    assert!(started.elapsed() < Duration::from_secs(10), "a silent tool is given up on, not waited for");
    assert!(matches!(sh("head -c 6000000 /dev/zero; sleep 30", READ_TIMEOUT, true), Err(Problem::Oversized)));
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn asking_runs_the_tool_and_reads_its_answer() {
    let dir = tempfile::tempdir().unwrap();
    let paths = fake_tool(dir.path());
    std::fs::write(dir.path().join("list.json"), list_answer()).unwrap();
    std::fs::write(dir.path().join("status.json"), envelope("status", true, false, Some("not-btrfs"), EXT4, "{}")).unwrap();
    std::fs::write(dir.path().join("diff.json"), "{\"schema\": 3}").unwrap();
    assert!(matches!(snapshots_from(ask(&paths, Action::List)), Fetch::Ready(list) if list.len() == 4));
    assert_eq!(status_from(ask(&paths, Action::Status)), Fetch::Unsupported(EXT4.into()));
    assert_eq!(ask(&paths, Action::Diff(13)), Err(Problem::UnknownSchema("3".into())));
    assert_eq!(ask(&paths, Action::Notices), Err(Problem::Malformed), "no answer file: only the noise line came back");
    assert_eq!(calls(dir.path()), vec!["--json list", "--json status", "--json diff 13", "--json notices"]);
    let absent = Paths { undo: dir.path().join("not-installed"), ..paths };
    assert_eq!(ask(&absent, Action::List), Err(Problem::Absent));
}

#[test]
fn privileged_actions_go_through_pkexec_and_a_refusal_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let paths = fake_tool(dir.path());
    std::fs::write(dir.path().join("rollback.json"), envelope("rollback", true, true, None, "Snapshot 13 becomes the system at the next restart.", r#"{"restart_required": true}"#)).unwrap();
    assert_eq!(outcome_from(ask(&paths, Action::Rollback(13))), (true, "Snapshot 13 becomes the system at the next restart.".into()));
    let undo = paths.undo.to_string_lossy();
    assert_eq!(calls(dir.path()), vec![format!("pkexec {undo} --json rollback 13 --reason Chosen in DEs-UI settings"), "--json rollback 13 --reason Chosen in DEs-UI settings".to_string()]);

    // The user closes the password question: pkexec exits 126 and the tool never runs.
    std::fs::remove_file(dir.path().join("calls")).unwrap();
    script(&paths.pkexec, r#"[ "$1" = probe ] && exit 0
exit 126"#);
    assert_eq!(ask(&paths, Action::Snapshot), Err(Problem::Denied));
    assert!(calls(dir.path()).is_empty());
    script(&paths.pkexec, r#"[ "$1" = probe ] && exit 0
exit 127"#);
    assert!(matches!(ask(&paths, Action::Snapshot), Err(Problem::Failed(_))));
    let no_pkexec = Paths { pkexec: dir.path().join("no-pkexec"), ..paths };
    assert_eq!(ask(&no_pkexec, Action::Snapshot), Err(Problem::Absent));
}

// ------------------------------------------------------------ the worker and the desktop's side

#[test]
fn on_a_system_that_is_not_desos_nothing_is_read_run_or_shown() {
    let dir = tempfile::tempdir().unwrap();
    let paths = fake_tool(dir.path());
    std::fs::write(&paths.verify, VERIFY_QUICK_PASS).unwrap();
    for os_release in [Some("NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\n"), None] {
        match os_release { Some(text) => std::fs::write(&paths.os_release, text).unwrap(), None => std::fs::remove_file(&paths.os_release).unwrap() }
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (sender, receiver) = mpsc::channel();
        sender.send(Action::List).unwrap();
        sender.send(Action::Notices).unwrap();
        // Returns although the channel is still open: there is nothing to serve.
        serve(paths.clone(), shared.clone(), receiver, || panic!("nothing changed, nothing to repaint"));
        let shared = shared.lock().unwrap();
        assert!(!shared.present && shared.integrity.is_none() && shared.snapshots == Fetch::Idle);
        assert!(calls(dir.path()).is_empty(), "the tool is never run");
    }
    let mut desos = Desos::default();
    assert!(!desos.present() && desos.integrity_line().is_none() && desos.session_notices().is_empty());
    desos.request(Action::Snapshot);
    assert!(desos.view().working.is_none());
    assert!(!present());
}

#[test]
fn on_desos_the_worker_reads_the_result_and_does_what_is_asked() {
    let dir = tempfile::tempdir().unwrap();
    let paths = fake_tool(dir.path());
    std::fs::write(&paths.verify, VERIFY_QUICK_PASS).unwrap();
    std::fs::write(dir.path().join("list.json"), list_answer()).unwrap();
    std::fs::write(dir.path().join("status.json"), envelope("status", true, true, None, "Undo is active.", STATUS_DATA)).unwrap();
    std::fs::write(dir.path().join("diff.json"), envelope("diff", true, true, None, "", DIFF_DATA)).unwrap();
    std::fs::write(dir.path().join("notices.json"), envelope("notices", true, true, None, "", &NOTICES_DATA.replace('\n', ""))).unwrap();
    std::fs::write(dir.path().join("snapshot.json"), envelope("snapshot", true, true, None, "Created snapshot 17.", "{}")).unwrap();
    let shared = Arc::new(Mutex::new(Shared::default()));
    let (sender, receiver) = mpsc::channel();
    for action in [Action::Diff(13), Action::Notices, Action::Snapshot, Action::AckNotices] { sender.send(action).unwrap(); }
    drop(sender);
    let repaints = std::sync::atomic::AtomicUsize::new(0);
    serve(paths.clone(), shared.clone(), receiver, || { repaints.fetch_add(1, Ordering::Relaxed); });
    let shared = shared.lock().unwrap();
    assert!(shared.present);
    assert_eq!(shared.integrity.as_ref().map(|integrity| integrity.line().value), Some("MATCHES"));
    assert!(matches!(&shared.diff, Some((13, Fetch::Ready(diff))) if diff.added == ["/etc/foo.conf"]));
    assert_eq!(shared.notices.iter().map(|notice| notice.id).collect::<Vec<_>>(), vec![3]);
    assert_eq!(shared.outcome, Some((true, "Created snapshot 17.".into())));
    assert!(shared.working.is_none());
    // After a change the list and the status are read again without being asked.
    assert!(matches!(&shared.snapshots, Fetch::Ready(list) if list.len() == 4) && matches!(&shared.status, Fetch::Ready(status) if status.restart_pending));
    let undo = paths.undo.to_string_lossy();
    assert_eq!(calls(dir.path()), vec![
        "--json diff 13".to_string(), "--json notices".to_string(),
        format!("pkexec {undo} --json snapshot --reason Taken from DEs-UI settings"), "--json snapshot --reason Taken from DEs-UI settings".to_string(),
        "--json status".to_string(), "--json list".to_string(), "--json notices --ack".to_string(),
    ]);
    assert_eq!(repaints.load(Ordering::Relaxed), 5, "once when found to be DesOS, once for each thing done");
}

#[test]
fn notices_are_asked_for_once_handed_over_once_and_then_acknowledged() {
    let mut desos = Desos::for_test(Some(Integrity::NotRun), Fetch::Idle, Fetch::Idle);
    let (sender, receiver) = mpsc::channel();
    desos.jobs = Some(sender);
    assert!(desos.session_notices().is_empty());
    assert_eq!(receiver.try_recv(), Ok(Action::Notices));
    assert!(desos.session_notices().is_empty());
    assert!(receiver.try_recv().is_err(), "asked once; nothing to acknowledge yet");
    desos.shared.lock().unwrap().notices = vec![Notice { id: 3, kind: "auto-recovery".into(), message: "Recovered.".into(), acknowledged: false }];
    assert_eq!(desos.session_notices().iter().map(|notice| notice.message.as_str()).collect::<Vec<_>>(), vec!["Recovered."]);
    assert_eq!(receiver.try_recv(), Ok(Action::AckNotices), "acknowledged after being handed over to be shown");
    assert!(desos.session_notices().is_empty() && receiver.try_recv().is_err());
}

#[test]
fn requests_mark_what_is_under_way_and_allow_one_privileged_action_at_a_time() {
    let mut desos = Desos::for_test(None, Fetch::Idle, Fetch::Ready(vec![]));
    let (sender, receiver) = mpsc::channel();
    desos.jobs = Some(sender);
    assert!(desos.integrity_line().is_none(), "nothing to say until the result has been read");
    desos.request(Action::Status);
    desos.request(Action::List);
    desos.request(Action::Diff(4));
    let view = desos.view();
    assert_eq!(view.status, Fetch::Loading);
    assert_eq!(view.snapshots, Fetch::Ready(vec![]), "a list already shown stays while it is read again");
    assert_eq!(view.diff, Some((4, Fetch::Loading)));
    desos.request(Action::Rollback(4));
    desos.request(Action::Snapshot);
    assert_eq!(desos.view().working, Some(Action::Rollback(4)));
    assert_eq!(receiver.try_iter().collect::<Vec<_>>(), vec![Action::Status, Action::List, Action::Diff(4), Action::Rollback(4)]);
}

#[test]
fn the_details_request_is_taken_once() {
    let ctx = egui::Context::default();
    assert!(!take_details_request(&ctx));
    request_details(&ctx);
    assert!(take_details_request(&ctx) && !take_details_request(&ctx));
}
