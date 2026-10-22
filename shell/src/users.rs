//! The people who can sign in to this machine: seeing them, adding one, changing a
//! name, a password or who is an administrator, and removing one.
//!
//! Reading needs nothing. Changing is the system's own tools (`useradd`, `usermod`,
//! `chpasswd`, `userdel`) run through `sudo`, with the password of whoever is making
//! the change asked for each time. Passwords are only ever written to a tool's input,
//! never put on a command line where other programs could read them.

use crate::theme::Theme;
use crate::ui;
use eframe::egui::{self, Align2, Key, Sense, Stroke, StrokeKind, TextEdit, Ui, vec2};
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The group whose members may act as administrator.
const ADMINS: &str = "sudo";

#[derive(Debug, Clone, PartialEq)]
pub struct User {
    pub name: String,
    /// The name people call them by, which may be empty.
    pub full: String,
    pub admin: bool,
}

/// Reads `/etc/passwd` and `/etc/group`: the accounts people sign in with.
pub fn parse_users(passwd: &str, group: &str) -> Vec<User> {
    let admins: Vec<&str> = group
        .lines()
        .find_map(|line| {
            let mut fields = line.split(':');
            (fields.next() == Some(ADMINS)).then(|| fields.nth(2).unwrap_or(""))
        })
        .map(|members| members.split(',').filter(|member| !member.is_empty()).collect())
        .unwrap_or_default();
    passwd
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split(':').collect();
            let uid: u32 = fields.get(2)?.parse().ok()?;
            let shell = fields.get(6)?;
            let human = (1000..60000).contains(&uid) && !shell.ends_with("nologin") && !shell.ends_with("false");
            human.then(|| User {
                name: fields[0].to_string(),
                // The name comes first of several things kept together, apart by commas.
                full: fields.get(4).and_then(|about| about.split(',').next()).unwrap_or("").to_string(),
                admin: admins.contains(&fields[0]),
            })
        })
        .collect()
}

/// Why a name cannot be an account's, if it cannot.
pub fn name_trouble(name: &str, taken: &[User]) -> Option<&'static str> {
    let mut chars = name.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase() || c == '_');
    if name.is_empty() {
        Some("Give the account a name to sign in with.")
    } else if !first_ok || !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-') {
        Some("A sign-in name is small letters, digits, - and _, starting with a letter.")
    } else if name.len() > 31 {
        Some("A sign-in name is at most 31 characters.")
    } else if taken.iter().any(|user| user.name == name) {
        Some("There is already an account with that name.")
    } else {
        None
    }
}

/// Why a full name cannot be kept, if it cannot: it sits among things set apart by
/// colons and commas.
pub fn full_name_trouble(full: &str) -> Option<&'static str> {
    full.chars().any(|c| c == ':' || c == ',' || c == '=' || c.is_control()).then_some("A name cannot have : , or = in it.")
}

/// Why a new password cannot be set, if it cannot.
pub fn password_trouble(password: &str, again: &str) -> Option<&'static str> {
    if password.is_empty() {
        Some("Give a password.")
    } else if password.contains('\n') {
        Some("A password is one line.")
    } else if password != again {
        Some("The two passwords are not the same.")
    } else {
        None
    }
}

/// One thing to run as administrator: the program and what it is told, and what is
/// written to its input.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub args: Vec<String>,
    pub input: Option<String>,
}

pub(crate) fn step(args: &[&str]) -> Step {
    Step { args: args.iter().map(|arg| arg.to_string()).collect(), input: None }
}

fn set_password(name: &str, password: &str) -> Step {
    Step { args: vec!["/usr/sbin/chpasswd".to_string()], input: Some(format!("{name}:{password}\n")) }
}

pub fn add_steps(name: &str, full: &str, password: &str, admin: bool) -> Vec<Step> {
    let mut steps = vec![step(&["/usr/sbin/useradd", "--create-home", "--shell", "/bin/bash", "--comment", full, "--", name]), set_password(name, password)];
    if admin {
        steps.push(step(&["/usr/sbin/usermod", "--append", "--groups", ADMINS, "--", name]));
    }
    steps
}

pub fn admin_step(name: &str, admin: bool) -> Step {
    if admin { step(&["/usr/sbin/usermod", "--append", "--groups", ADMINS, "--", name]) } else { step(&["/usr/bin/gpasswd", "--delete", name, ADMINS]) }
}

pub fn remove_step(name: &str, with_files: bool) -> Step {
    if with_files { step(&["/usr/sbin/userdel", "--remove", "--", name]) } else { step(&["/usr/sbin/userdel", "--", name]) }
}

/// Runs steps as administrator, giving `sudo` this password. Stops at the first that
/// fails and says why, in words for whoever asked.
pub fn run_as_admin(sudo: &str, password: &str, steps: &[Step]) -> Result<(), String> {
    for step in steps {
        // `-k` asks for the password every time, `-S` reads it from the input, and the
        // empty `-p` keeps the question it would print out of what comes back.
        let mut child = Command::new(sudo)
            .args(["-S", "-k", "-p", "", "--"])
            .args(&step.args)
            .env("LC_ALL", "C")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot run {sudo}: {e}"))?;
        if let Some(mut input) = child.stdin.take() {
            let _ = input.write_all(format!("{password}\n{}", step.input.as_deref().unwrap_or("")).as_bytes());
        }
        let output = child.wait_with_output().map_err(|e| format!("{sudo} did not finish: {e}"))?;
        if !output.status.success() {
            let said = String::from_utf8_lossy(&output.stderr);
            let said = said.trim();
            return Err(if said.contains("incorrect password") || said.contains("try again") || said.contains("no password was provided") {
                "That password was not accepted.".to_string()
            } else if said.contains("not in the sudoers") || said.contains("not allowed") {
                "Your account may not make this change: it is not an administrator.".to_string()
            } else {
                // The tool's own last word, which is usually the reason.
                said.lines().last().map(str::to_string).filter(|line| !line.is_empty()).unwrap_or_else(|| "It did not work, and nothing said why.".to_string())
            });
        }
    }
    Ok(())
}

/// A change waiting for the password of whoever is making it.
struct Pending {
    /// What is about to happen, in words: "add ada".
    what: String,
    steps: Vec<Step>,
}

#[derive(Default)]
struct Outcome {
    working: bool,
    /// How the last change went: what to say, and whether it failed.
    said: Option<(String, bool)>,
    /// Set when something changed, so that the list is read again.
    changed: bool,
}

/// The users part of the settings.
pub struct Users {
    ctx: egui::Context,
    users: Vec<User>,
    read_at: Option<Instant>,
    me: String,
    /// The account whose controls are open.
    open: Option<String>,
    full: String,
    new_password: String,
    new_again: String,
    /// The account being added.
    add_name: String,
    add_full: String,
    add_password: String,
    add_again: String,
    add_admin: bool,
    /// The removal asked for once already, and when: it takes a second click.
    armed: Option<(String, bool, Instant)>,
    pending: Option<Pending>,
    my_password: String,
    outcome: Arc<Mutex<Outcome>>,
    /// What the fields say is wrong, before anything is run.
    trouble: Option<String>,
}

impl Users {
    pub fn new(ctx: egui::Context) -> Users {
        Users {
            ctx,
            users: Vec::new(),
            read_at: None,
            me: std::env::var("USER").unwrap_or_default(),
            open: None,
            full: String::new(),
            new_password: String::new(),
            new_again: String::new(),
            add_name: String::new(),
            add_full: String::new(),
            add_password: String::new(),
            add_again: String::new(),
            add_admin: false,
            armed: None,
            pending: None,
            my_password: String::new(),
            outcome: Arc::default(),
            trouble: None,
        }
    }

    fn read(&mut self) {
        let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
        let group = std::fs::read_to_string("/etc/group").unwrap_or_default();
        self.users = parse_users(&passwd, &group);
        // Whoever is using this comes first.
        self.users.sort_by_key(|user| (user.name != self.me, user.name.clone()));
        self.read_at = Some(Instant::now());
    }

    fn ask(&mut self, what: String, steps: Vec<Step>) {
        self.trouble = None;
        self.my_password.clear();
        self.outcome.lock().unwrap().said = None;
        self.pending = Some(Pending { what, steps });
    }

    /// Runs what is pending with the password given.
    fn go(&mut self) {
        let Some(pending) = self.pending.take() else { return };
        let password = std::mem::take(&mut self.my_password);
        let (outcome, ctx) = (self.outcome.clone(), self.ctx.clone());
        outcome.lock().unwrap().working = true;
        std::thread::spawn(move || {
            let sudo = std::env::var("DES_SUDO").unwrap_or_else(|_| "sudo".to_string());
            let result = run_as_admin(&sudo, &password, &pending.steps);
            let mut outcome = outcome.lock().unwrap();
            outcome.working = false;
            outcome.changed = true;
            outcome.said = Some(match result {
                Ok(()) => (format!("Done: {}.", pending.what), false),
                Err(why) => (format!("Could not {}. {why}", pending.what), true),
            });
            drop(outcome);
            ctx.request_repaint();
        });
    }

    pub fn show(&mut self, ui: &mut Ui, theme: &Theme) {
        let (working, said) = {
            let mut outcome = self.outcome.lock().unwrap();
            if std::mem::take(&mut outcome.changed) {
                self.read_at = None;
                // What was typed has been used, one way or the other.
                for field in [&mut self.new_password, &mut self.new_again, &mut self.add_password, &mut self.add_again] {
                    field.clear();
                }
                if outcome.said.as_ref().is_some_and(|(_, failed)| !failed) {
                    self.add_name.clear();
                    self.add_full.clear();
                    self.add_admin = false;
                }
            }
            (outcome.working, outcome.said.clone())
        };
        if self.read_at.is_none_or(|at| at.elapsed() > Duration::from_secs(10)) {
            self.read();
        }
        let users = self.users.clone();
        let am_admin = users.iter().any(|user| user.name == self.me && user.admin);

        ui::section(ui, "USERS", |ui| {
            if working {
                ui::meta(ui, "WORKING", theme.main);
            }
        });
        ui::text(ui, "The people who can sign in to this machine.", ui::mono(12.0), ui::SECONDARY);
        ui.add_space(10.0);

        // A change waiting for a password comes first: nothing else can be done meanwhile.
        if let Some(what) = self.pending.as_ref().map(|pending| pending.what.clone()) {
            let mut decided = None;
            ui::tile(ui, theme, true, |ui| {
                ui::tracked(ui, &format!("TO {}, TYPE YOUR OWN PASSWORD", what.to_uppercase()), ui::display_bold(12.0), ui::TEXT, 1.5);
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let entered = field(ui, theme, &mut self.my_password, "your password", true, 260.0);
                    let ready = !self.my_password.is_empty();
                    if (ui::button(ui, theme, "CONFIRM", None, ready).clicked() || entered) && ready {
                        decided = Some(true);
                    }
                    if ui::button(ui, theme, "CANCEL", None, false).clicked() {
                        decided = Some(false);
                    }
                });
            });
            match decided {
                Some(true) => self.go(),
                Some(false) => {
                    self.pending = None;
                    self.my_password.clear();
                }
                None => {}
            }
            ui.add_space(14.0);
            return;
        }
        if let Some((text, failed)) = &said {
            ui::text(ui, text, ui::mono(12.0), if *failed { ui::CAUTION } else { theme.main });
            ui.add_space(10.0);
        }
        if let Some(trouble) = &self.trouble {
            ui::text(ui, trouble, ui::mono(12.0), ui::CAUTION);
            ui.add_space(10.0);
        }
        if !am_admin {
            ui::text(ui, "Your account is not an administrator, so it can look but not change.", ui::mono(12.0), ui::LABEL);
            ui.add_space(10.0);
        }

        for user in &users {
            let open = self.open.as_deref() == Some(user.name.as_str());
            let mine = user.name == self.me;
            let state = match (mine, user.admin) {
                (true, true) => "YOU · ADMINISTRATOR",
                (true, false) => "YOU",
                (false, true) => "ADMINISTRATOR",
                (false, false) => "",
            };
            let shown = if user.full.is_empty() { user.name.clone() } else { format!("{}  ·  {}", user.full, user.name) };
            if row(ui, theme, &shown, state, mine, open) {
                self.open = if open { None } else { Some(user.name.clone()) };
                self.full = user.full.clone();
                self.new_password.clear();
                self.new_again.clear();
                self.trouble = None;
                self.armed = None;
            }
            if !open || !am_admin {
                continue;
            }
            ui.add_space(10.0);
            ui.indent(("user", &user.name), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    field(ui, theme, &mut self.full, "full name", false, 260.0);
                    let differs = self.full.trim() != user.full;
                    if ui::button(ui, theme, "SAVE NAME", None, differs).clicked() && differs {
                        match full_name_trouble(self.full.trim()) {
                            Some(trouble) => self.trouble = Some(trouble.to_string()),
                            None => {
                                let full = self.full.trim().to_string();
                                self.ask(format!("rename {}", user.name), vec![step(&["/usr/sbin/usermod", "--comment", &full, "--", &user.name])]);
                            }
                        }
                    }
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    field(ui, theme, &mut self.new_password, "new password", true, 180.0);
                    field(ui, theme, &mut self.new_again, "again", true, 180.0);
                    let ready = !self.new_password.is_empty();
                    if ui::button(ui, theme, "SET PASSWORD", None, ready).clicked() && ready {
                        match password_trouble(&self.new_password, &self.new_again) {
                            Some(trouble) => self.trouble = Some(trouble.to_string()),
                            None => {
                                let steps = vec![set_password(&user.name, &self.new_password)];
                                self.ask(format!("change the password of {}", user.name), steps);
                            }
                        }
                    }
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    // Whoever is doing this stays an administrator: otherwise nobody
                    // might be left who could undo it.
                    if !mine {
                        let label = if user.admin { "ADMINISTRATOR" } else { "MAKE ADMINISTRATOR" };
                        if ui::chip(ui, theme, label, None, user.admin).on_hover_text("An administrator may change the system and its users").clicked() {
                            let verb = if user.admin { "stop" } else { "make" };
                            self.ask(format!("{verb} {} being an administrator", user.name), vec![admin_step(&user.name, !user.admin)]);
                        }
                        for (with_files, label) in [(false, "REMOVE, KEEP FILES"), (true, "REMOVE WITH FILES")] {
                            let armed = self.armed.as_ref().is_some_and(|(name, files, at)| name == &user.name && *files == with_files && at.elapsed() < Duration::from_secs(4));
                            if ui::button(ui, theme, if armed { "CLICK AGAIN TO REMOVE" } else { label }, None, armed).clicked() {
                                if armed {
                                    self.armed = None;
                                    let what = if with_files { format!("remove {} and everything in their home folder", user.name) } else { format!("remove {}", user.name) };
                                    self.ask(what, vec![remove_step(&user.name, with_files)]);
                                } else {
                                    self.armed = Some((user.name.clone(), with_files, Instant::now()));
                                    ui.ctx().request_repaint_after(Duration::from_secs(4));
                                }
                            }
                        }
                    } else {
                        ui::text(ui, "Your picture is under Session.", ui::mono(12.0), ui::LABEL);
                    }
                });
            });
            ui.add_space(14.0);
        }

        if !am_admin {
            return;
        }
        ui.add_space(22.0);
        ui::section(ui, "ADD A USER", |_| {});
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            field(ui, theme, &mut self.add_name, "sign-in name", false, 180.0);
            field(ui, theme, &mut self.add_full, "full name", false, 240.0);
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            field(ui, theme, &mut self.add_password, "password", true, 180.0);
            field(ui, theme, &mut self.add_again, "again", true, 180.0);
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if ui::chip(ui, theme, "ADMINISTRATOR", None, self.add_admin).clicked() {
                self.add_admin = !self.add_admin;
            }
            let ready = !self.add_name.is_empty() && !self.add_password.is_empty();
            if ui::button(ui, theme, "ADD", None, ready).clicked() && ready {
                let name = self.add_name.trim().to_string();
                let full = self.add_full.trim().to_string();
                let trouble = name_trouble(&name, &users).or(full_name_trouble(&full)).or(password_trouble(&self.add_password, &self.add_again));
                match trouble {
                    Some(trouble) => self.trouble = Some(trouble.to_string()),
                    None => {
                        let steps = add_steps(&name, &full, &self.add_password, self.add_admin);
                        self.ask(format!("add {name}"), steps);
                    }
                }
            }
        });
        ui.add_space(10.0);
        ui::text(ui, "To sign in as someone else: log out, then Tab on the login screen.", ui::mono(12.0), ui::LABEL);
    }
}

/// A one-line text field. Returns true when Enter was pressed in it.
pub(crate) fn field(ui: &mut Ui, theme: &Theme, text: &mut String, hint: &str, secret: bool, width: f32) -> bool {
    let (rect, _) = ui.allocate_exact_size(vec2(width.min(ui.available_width()), 28.0), Sense::hover());
    ui.painter().rect_filled(rect, ui::CONTROL_RADIUS, ui::PANEL);
    ui.painter().rect_stroke(rect, ui::CONTROL_RADIUS, Stroke::new(1.0, theme.main), StrokeKind::Inside);
    let edit = ui.put(
        rect.shrink2(vec2(8.0, 4.0)),
        TextEdit::singleline(text).password(secret).frame(false).font(ui::mono(13.0)).text_color(ui::TEXT).hint_text(hint),
    );
    let entered = edit.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
    // What is put into a place of its own does not move the layout on past the gap.
    ui.add_space(8.0);
    entered
}

/// One account in the list. Returns true when it was clicked.
fn row(ui: &mut Ui, theme: &Theme, name: &str, state: &str, live: bool, open: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
    let painter = ui.painter_at(rect);
    if open {
        painter.rect_filled(rect, 0.0, theme.tint());
        painter.rect_stroke(rect, ui::CONTROL_RADIUS, Stroke::new(1.0, theme.main), StrokeKind::Inside);
    } else {
        if response.hovered() {
            painter.rect_filled(rect, ui::CONTROL_RADIUS, ui::PANEL);
        }
        painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, ui::TRACK));
    }
    let mark = egui::Rect::from_center_size(rect.left_center() + vec2(14.0, 0.0), vec2(6.0, 6.0));
    painter.rect_filled(mark, 0.0, if live { theme.main } else { ui::LINE_STRONG });
    painter.text(rect.left_center() + vec2(30.0, 0.0), Align2::LEFT_CENTER, crate::panels::truncate(name, 44), ui::mono(13.0), ui::TEXT);
    painter.text(rect.right_center() - vec2(14.0, 0.0), Align2::RIGHT_CENTER, state, ui::mono(11.0), if live { theme.main } else { ui::LABEL });
    response.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWD: &str = "root:x:0:0:root:/root:/bin/bash\ndaemon:x:1:1:daemon:/usr/sbin:/usr/sbin/nologin\nada:x:1000:1000:Ada Lovelace,,,:/home/ada:/bin/zsh\ngrace:x:1001:1001::/home/grace:/bin/bash\nsvc:x:1002:1002:Service:/home/svc:/usr/sbin/nologin\nnobody:x:65534:65534:nobody:/nonexistent:/usr/sbin/nologin\n";
    const GROUP: &str = "root:x:0:\nsudo:x:27:ada,someone\nusers:x:100:ada,grace\n";

    #[test]
    fn reads_the_people_who_sign_in() {
        let users = parse_users(PASSWD, GROUP);
        assert_eq!(
            users,
            [
                User { name: "ada".into(), full: "Ada Lovelace".into(), admin: true },
                User { name: "grace".into(), full: String::new(), admin: false },
            ]
        );
        assert!(parse_users("", "").is_empty());
    }

    #[test]
    fn checks_what_is_typed_before_running_anything() {
        let users = parse_users(PASSWD, GROUP);
        assert_eq!(name_trouble("linus", &users), None);
        assert_eq!(name_trouble("le_guin-2", &users), None);
        assert!(name_trouble("", &users).is_some());
        assert!(name_trouble("Ada", &users).is_some(), "capitals");
        assert!(name_trouble("9lives", &users).is_some(), "starts with a digit");
        assert!(name_trouble("a b", &users).is_some());
        assert!(name_trouble("ada", &users).is_some(), "taken");
        assert!(name_trouble(&"a".repeat(32), &users).is_some());
        assert_eq!(full_name_trouble("Ursula K. Le Guin"), None);
        assert!(full_name_trouble("a:b").is_some() && full_name_trouble("a,b").is_some());
        assert_eq!(password_trouble("s3cret", "s3cret"), None);
        assert!(password_trouble("", "").is_some());
        assert!(password_trouble("one", "two").is_some());
    }

    #[test]
    fn passwords_go_to_a_tool_s_input_and_never_its_arguments() {
        let steps = add_steps("linus", "Linus T", "p4ss word", true);
        assert_eq!(steps.len(), 3);
        assert!(steps.iter().all(|step| step.args.iter().all(|arg| !arg.contains("p4ss"))));
        assert_eq!(steps[1].input.as_deref(), Some("linus:p4ss word\n"));
        assert_eq!(steps[0].args.last().map(String::as_str), Some("linus"));
        assert_eq!(add_steps("linus", "", "x", false).len(), 2);
        assert_eq!(remove_step("grace", true).args, ["/usr/sbin/userdel", "--remove", "--", "grace"]);
        assert_eq!(remove_step("grace", false).args, ["/usr/sbin/userdel", "--", "grace"]);
        assert_eq!(admin_step("grace", false).args, ["/usr/bin/gpasswd", "--delete", "grace", "sudo"]);
    }

    /// Stands in for `sudo`: takes one password, and writes down what it was asked to
    /// run and what else was on its input.
    const STAND_IN: &str = r#"#!/bin/sh
read password
if [ "$password" != "right" ]; then echo "Sorry, try again." >&2; echo "sudo: 1 incorrect password attempt" >&2; exit 1; fi
while [ "$1" != "--" ]; do shift; done; shift
rest=$(cat)
echo "ran: $* | input: $rest" >> "$0.log"
for last; do :; done
case "$1" in */useradd) [ "$last" = "taken" ] && { echo "useradd: user 'taken' already exists" >&2; exit 9; };; esac
exit 0
"#;

    fn stand_in(name: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let script = std::env::temp_dir().join(format!("des-sudo-{name}-{}.sh", std::process::id()));
        std::fs::write(&script, STAND_IN).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    #[test]
    fn runs_each_step_with_the_password_and_stops_at_a_failure() {
        let script = stand_in("steps");
        let sudo = script.to_str().unwrap();
        let log = script.with_extension("sh.log");

        assert_eq!(run_as_admin(sudo, "right", &add_steps("linus", "Linus T", "s3cret", true)), Ok(()));
        let ran = std::fs::read_to_string(&log).unwrap();
        let lines: Vec<&str> = ran.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("ran: /usr/sbin/useradd --create-home") && lines[0].ends_with("linus | input: "));
        assert_eq!(lines[1], "ran: /usr/sbin/chpasswd | input: linus:s3cret");
        assert!(lines[2].contains("usermod --append --groups sudo"));

        assert_eq!(run_as_admin(sudo, "wrong", &[remove_step("grace", false)]), Err("That password was not accepted.".to_string()));
        // The tool's own reason is passed on, and what follows is not run.
        let _ = std::fs::remove_file(&log);
        assert_eq!(run_as_admin(sudo, "right", &add_steps("taken", "", "x", false)), Err("useradd: user 'taken' already exists".to_string()));
        assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(), 1);
        let _ = std::fs::remove_file(&script);
        let _ = std::fs::remove_file(&log);
    }
}
