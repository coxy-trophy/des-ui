//! Private, bounded JSON worker bridge. No credentials enter desktop configuration.
use eframe::egui::{self, RichText};
use serde_json::{Value, json};
use std::{io::{BufRead, BufReader, Read, Write}, path::PathBuf, process::{Child, Command, Stdio}, sync::{mpsc::{self, Receiver, SyncSender}, Arc, atomic::{AtomicBool, Ordering}}};
use crate::{theme::Theme, ui};
use base64::Engine as _;
/// Finite thought lifetime with cubic easing; no movement when reduced motion is enabled.
fn thought_motion(age: f32, animate: bool) -> Option<(f32,f32)> {
    if !age.is_finite() || age>=15.0 {return None;}
    if !animate {return Some((1.0,0.0));}
    let enter=crate::powerup::ease(age/0.28);
    let leave=crate::powerup::ease((15.0-age)/0.4);
    Some((enter*leave,(1.0-enter)*6.0))
}

const MAX_EVENT: usize = 2 * 1024 * 1024;
struct Bridge { child: Child, tx: SyncSender<Value>, rx: Receiver<Value>, failed: Arc<AtomicBool> }
impl Bridge {
    fn start(ctx: egui::Context) -> Result<Self, String> {
        let path = worker_path().ok_or("Companion helper missing. Rebuild/install the companion assets.")?;
        let mut child = Command::new("python3").arg(path).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|_| "Could not start Python3 companion helper.")?;
        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, commands) = mpsc::sync_channel::<Value>(16);
        let (events, rx) = mpsc::sync_channel(8);
        let failed=Arc::new(AtomicBool::new(false));let writer_failed=failed.clone();let reader_failed=failed.clone();
        std::thread::spawn(move || { while let Ok(value) = commands.recv() { if serde_json::to_writer(&mut stdin, &value).is_err() || stdin.write_all(b"\n").is_err() || stdin.flush().is_err() { writer_failed.store(true,Ordering::Release);break; } } });
        std::thread::spawn(move || {
            let mut reader=BufReader::new(stdout);
            loop { let mut line=Vec::new(); let read=std::io::Read::take(&mut reader, (MAX_EVENT+1) as u64).read_until(b'\n', &mut line); if !matches!(read, Ok(n) if n>0) || line.len()>MAX_EVENT { break; } if let Ok(value)=serde_json::from_slice(&line) { if events.try_send(value).is_err() { break; } ctx.request_repaint(); } }
            reader_failed.store(true,Ordering::Release);ctx.request_repaint();
        });
        Ok(Self { child, tx, rx, failed })
    }
}
impl Drop for Bridge { fn drop(&mut self) {
    let _=self.tx.try_send(json!({"type":"shutdown","id":0}));
    let deadline=std::time::Instant::now()+std::time::Duration::from_millis(500);
    while std::time::Instant::now()<deadline { if self.child.try_wait().ok().flatten().is_some(){return} std::thread::sleep(std::time::Duration::from_millis(10)); }
    let _=self.child.kill(); let _=self.child.wait();
} }
fn worker_path() -> Option<PathBuf> {
    [if cfg!(debug_assertions) {PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../companion/service.py")}else{PathBuf::from("/usr/local/share/des-ui/companion/service.py")},PathBuf::from("/usr/local/share/des-ui/companion/service.py"),PathBuf::from("/usr/share/des-ui/companion/service.py"),PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../companion/service.py")].into_iter().find(|p|p.is_file())
}
#[derive(Default)]
pub struct Companion {
    pub open: bool, pub chat_selected: bool, pub panel_requested: bool, pub settings_requested: bool, bridge: Option<Bridge>, next_id: u64, snapshot: Value, voice: Value,
    input: String, key: String, show_key: bool, model: String, identity: String, memory: String, memory_edit: Option<(i64, String)>, memory_edit_saving: Option<(i64, String)>,
    worker_ready: bool, pending_voice: bool, keep_requested: bool, voice_requested: bool, voice_notice: Option<(String,std::time::Instant)>, voice_screen: usize, cursor_signal: Option<(bool,Option<(i32,i32,i32,i32)>,std::time::Instant)>, voice_anchor: Option<egui::Pos2>, voice_caption: String, remember_key: Option<bool>,
    capture_requested: bool, capture_pending: Option<u64>, capture_sequence: u64, attachment: Option<ScreenAttachment>, voice_screen_pending: bool,
    error: String, focus: Option<egui::Id>, focus_input: bool, confirm: Option<&'static str>, identity_dirty: bool, social_at: Option<std::time::Instant>, bubble_anchor: Option<egui::Pos2>, bubble: Option<(String,std::time::Instant)>,
    social_probe_started: bool, social_probe: Option<Receiver<bool>>,
}
struct ScreenAttachment { jpeg: String, texture: egui::TextureHandle }

impl Companion {
    pub fn take_capture_request(&mut self) -> bool { std::mem::take(&mut self.capture_requested) }
    pub fn begin_capture(&mut self) -> u64 {
        self.capture_sequence = self.capture_sequence.wrapping_add(1) & (u64::MAX >> 1);
        let id = (1u64 << 63) | self.capture_sequence;
        self.capture_pending = Some(id);
        id
    }
    pub fn capture_failed(&mut self, id: u64, message: &str) {
        if self.capture_pending == Some(id) { self.capture_pending = None; self.error = message.into(); }
    }
    pub fn accepts_capture(id: Option<u64>) -> bool { id.is_some_and(|id| id & (1u64 << 63) != 0) }
    pub fn receive_capture(&mut self, ctx: &egui::Context, shot: crate::capture::Shot) {
        let pending = shot.request == self.capture_pending && self.capture_pending.is_some();
        if pending { self.capture_pending = None; }
        let path = std::path::Path::new(&shot.path);
        let result = if pending { (|| -> Result<ScreenAttachment, String> {
            let size = (shot.width as usize).checked_mul(shot.height as usize).and_then(|v| v.checked_mul(4)).ok_or("Screenshot is too large")?;
            if size == 0 || size > 64 * 1024 * 1024 { return Err("Screenshot is too large".into()); }
            let file = std::fs::File::open(path).map_err(|_| "Could not read screenshot")?;
            let mut data = Vec::with_capacity(size);
            std::io::Read::take(file, size as u64 + 1).read_to_end(&mut data).map_err(|_| "Could not read screenshot")?;
            if data.len() != size { return Err("Screenshot is incomplete".into()); }
            let image = image::RgbaImage::from_raw(shot.width, shot.height, data).ok_or("Screenshot is invalid")?;
            let resized = image::DynamicImage::ImageRgba8(image).resize(1280, 1280, image::imageops::FilterType::Triangle).to_rgb8();
            let mut bytes = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 75).encode_image(&resized).map_err(|_| "Could not prepare screenshot")?;
            if bytes.len() > 1_500_000 { return Err("Screenshot is too large to attach".into()); }
            let preview = image::imageops::thumbnail(&resized, 160, 90);
            let texture = ctx.load_texture("companion-screen-preview", egui::ColorImage::from_rgb([preview.width() as usize, preview.height() as usize], preview.as_raw()), egui::TextureOptions::LINEAR);
            Ok(ScreenAttachment { jpeg: base64::engine::general_purpose::STANDARD.encode(bytes), texture })
        })() } else { Err(String::new()) };
        let _ = std::fs::remove_file(path);
        if pending { match result { Ok(image) => { self.attachment = Some(image); self.error.clear(); }, Err(message) => self.error = message } }
    }
    pub fn open(&mut self, ctx: &egui::Context) {
        self.open=true; self.focus=ctx.memory(|m|m.focused()); self.focus_input=true;
        self.chat_selected=true;self.panel_requested=true;
        self.connect(ctx);
        self.send(json!({"type":"status"}));
    }
    /// Prepare an explicit draft; never send terminal content automatically.
    pub fn prepare_terminal_explanation(&mut self, ctx: &egui::Context, output: &str) {
        let remaining = 7000usize.saturating_sub(self.input.chars().count());
        if remaining < 64 { self.open(ctx); self.error = "Draft is full; clear space before adding terminal output.".into(); return; }
        if !self.input.is_empty() { self.input.push_str("\n\n"); }
        self.input.push_str("Explain this terminal output:\n\n");
        self.input.extend(output.chars().take(remaining.saturating_sub(40)));
        self.open(ctx);
    }
    /// Companion configuration is owned by Settings, never the conversation overlay.
    pub fn settings_ui(&mut self, ui: &mut egui::Ui, _theme: &Theme) {
        if self.bridge.is_none() && self.error.is_empty() { self.connect(ui.ctx()); self.send(json!({"type":"status"})); }
        ui.style_mut().wrap_mode=Some(egui::TextWrapMode::Wrap);
        if self.model.is_empty() { self.model=self.snapshot["model"].as_str().unwrap_or("gpt-5.6-luna").into(); }
        ui.label(RichText::new("CONNECTION").font(ui::display_bold(13.0)));
        ui.label(if self.snapshot["configured"].as_bool()==Some(true) { "Key available · access checked when you send" } else { "No API key configured" });
        if !self.error.is_empty() {
            ui.label(RichText::new(&self.error).color(ui::CAUTION));
            if ui.button("Reconnect companion worker").clicked() {
                self.bridge=None;
                self.error.clear();
                self.connect(ui.ctx());
            }
        }
        if let Some(message)=self.voice["message"].as_str().filter(|text|!text.is_empty()) {
            ui.label(RichText::new(message).color(ui::CAUTION));
        }
        ui.label("Save the key in your system keyring to restore it automatically after restarting the shell.");
        let credential=self.snapshot["credential_status"].as_str().unwrap_or("loading");
        ui.label(format!("Secure storage: {}",credential.replace('_'," ")));
        let mut remember=self.remember_key.unwrap_or_else(||self.snapshot["remember_key"].as_bool().unwrap_or(true));
        if ui.checkbox(&mut remember,"Remember API key securely").changed(){self.remember_key=Some(remember);}
                let mut enabled=self.snapshot["enabled"].as_bool().unwrap_or(false);let mut social=self.snapshot["social"].as_bool().unwrap_or(false);
                if ui.checkbox(&mut enabled,"Enable companion").changed(){self.send(json!({"type":"configure","enabled":enabled}));}
                if ui.checkbox(&mut social,"Social text approaches").changed(){self.send(json!({"type":"configure","social":social}));}
                if ui.button("Quiet now").clicked(){self.send(json!({"type":"configure","social":false}));}
                ui.separator();
                    ui.label("OpenAI API key");ui.add(egui::TextEdit::singleline(&mut self.key).password(!self.show_key).desired_width(ui.available_width()));ui.checkbox(&mut self.show_key,"Show API key");
                    if ui.button("Remove saved key").clicked(){self.send(json!({"type":"remove_saved_key"}));self.key.clear();self.remember_key=Some(false);}
                    ui.label("Responses model");ui.add(egui::TextEdit::singleline(&mut self.model).desired_width(ui.available_width()));
                    if ui.button("Apply setup").clicked(){let mut setup=json!({"type":"configure","model":self.model,"persist_key":remember});if !self.key.is_empty(){setup["api_key"]=json!(self.key);}self.error.clear();self.send(setup);if self.bridge.is_some(){self.key.clear();}}
                    let mut requests=self.snapshot["daily_requests"].as_u64().unwrap_or(100) as u32;
                    let mut minutes=self.snapshot["daily_voice_minutes"].as_u64().unwrap_or(15) as u32;
                    ui.label("Daily request limit");if ui.add(egui::DragValue::new(&mut requests).range(1..=10000)).changed(){self.send(json!({"type":"configure","daily_requests":requests}));}
                    ui.label("Daily voice minutes");if ui.add(egui::DragValue::new(&mut minutes).range(5..=1440)).changed(){self.send(json!({"type":"configure","daily_voice_minutes":minutes}));}
                    ui.label(format!("Today: {} requests · {} voice seconds used/reserved",self.snapshot["used_requests"],self.snapshot["voice_seconds"]));
                    ui.label("Identity journal");if ui.add(egui::TextEdit::multiline(&mut self.identity).desired_rows(3).desired_width(ui.available_width())).changed(){self.identity_dirty=true;}if ui.button("Save identity").clicked(){self.send(json!({"type":"identity","text":self.identity}));self.identity_dirty=false;}
                    let identities=self.snapshot["identities"].as_array().cloned().unwrap_or_default();
                    ui.collapsing("Previous identity versions",|ui|{for identity in identities {ui.horizontal_wrapped(|ui|{ui.label(identity["text"].as_str().unwrap_or(""));if ui.button("Restore version").clicked(){self.send(json!({"type":"restore_identity","version":identity["id"]}));self.identity=identity["text"].as_str().unwrap_or("").into();}});}});
                    ui.label("Saved memories can be included in AI requests.");
                    ui.label("Remember an approved fact or preference");ui.add(egui::TextEdit::singleline(&mut self.memory).desired_width(ui.available_width()));if ui.button("Remember").clicked()&&!self.memory.trim().is_empty(){self.send(json!({"type":"remember","text":self.memory}));self.memory.clear();}
                    let memories=self.snapshot["memories"].as_array().cloned().unwrap_or_default();
                    if memories.is_empty() { ui.label(RichText::new("No saved memories.").color(ui::SECONDARY)); }
                    for memory in memories {
                        let Some(id)=memory["id"].as_i64() else { continue };
                        if self.memory_edit.as_ref().is_some_and(|(editing,_)|*editing==id) {
                            let mut save=false;let mut cancel=false;
                            let saving=self.memory_edit_saving.is_some();
                            if let Some((_,draft))=&mut self.memory_edit {
                                let width=ui.available_width();
                                ui.add_enabled_ui(!saving, |ui| ui.add_sized(egui::vec2(width, ui::CONTROL_HEIGHT), egui::TextEdit::singleline(draft).char_limit(1000).hint_text("Memory")));
                                ui.horizontal_wrapped(|ui| { save=ui.add_enabled(!saving && !draft.trim().is_empty(),egui::Button::new(if saving {"Saving…"} else {"Save memory"})).clicked(); cancel=ui.add_enabled(!saving,egui::Button::new("Cancel")).clicked(); });
                            }
                            if save { if let Some((id,draft))=self.memory_edit.clone() { self.send(json!({"type":"edit_memory","memory_id":id,"text":draft})); if self.bridge.is_some() {self.memory_edit_saving=Some((id,draft.trim().to_owned()));} } }
                            else if cancel { self.memory_edit=None; }
                        } else {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(memory["text"].as_str().unwrap_or(""));
                                if ui.button("Edit").clicked() { self.memory_edit=Some((id,memory["text"].as_str().unwrap_or("").to_owned())); }
                                if ui.button("Forget").clicked() { self.send(json!({"type":"forget","memory_id":id})); }
                            });
                        }
                    }
                    for (label,command) in [("Clear conversation","clear"),("Reset identity","reset_identity")] {if ui.button(label).clicked(){self.confirm=Some(command)}}
                    if let Some(command)=self.confirm {ui.label("This removes the selected local history. Continue?");ui.horizontal(|ui|{if ui.button("Confirm removal").clicked(){self.send(json!({"type":command}));self.confirm=None;}if ui.button("Cancel").clicked(){self.confirm=None;}});}


        ui.separator();
        ui.label(RichText::new("SHORTCUTS").font(ui::display_bold(13.0)));
        for (label,id) in [("Text conversation","desktop.companion"),("Summon voice","desktop.voice_companion"),("Keep talking","desktop.companion_keep"),("Mute microphone","desktop.companion_mute"),("Stop voice","desktop.companion_stop"),("Quiet mode","desktop.companion_quiet")] {
            ui.label(crate::shortcuts::hint(ui.ctx(),label,id));
        }
        ui.label("Change these mappings in Keyboard → Shortcuts. Voice always closes at five minutes.");
    }

    fn reset_connection_state(&mut self) {
        self.memory_edit_saving=None;self.voice_screen_pending=false;
        self.voice=json!({"state":"off","muted":false});
        self.worker_ready=false;self.keep_requested=false;self.voice_requested=false;self.pending_voice=false;self.voice_caption.clear();
        if !self.snapshot.is_object() {self.snapshot=json!({});}
        self.snapshot["configured"]=json!(false);
        self.snapshot["voice_active"]=json!(false);
        self.snapshot["busy"]=json!(false);
    }

    fn connect(&mut self, ctx: &egui::Context) {
        if self.bridge.is_none() {
            self.reset_connection_state();
            match Bridge::start(ctx.clone()) {
                Ok(bridge)=>{self.bridge=Some(bridge);self.error.clear();},
                Err(error)=>self.error=error,
            }
        }
    }

    pub fn control(&mut self, action: des_common::ipc::CompanionAction) {
        use des_common::ipc::CompanionAction;
        match action {
            CompanionAction::Keep=>{if self.pending_voice || self.voice_requested {self.keep_requested=true;}else if self.bridge.is_some(){self.send(json!({"type":"voice_keep"}));}},
            CompanionAction::Mute=>{if self.bridge.is_some(){let muted=self.voice["muted"].as_bool().unwrap_or(false);self.send(json!({"type":"voice_mute","muted":!muted}));}},
            CompanionAction::Stop=>{self.pending_voice=false;self.voice_requested=false;self.keep_requested=false;if self.bridge.is_some(){self.send(json!({"type":"voice_stop"}));}},
            CompanionAction::Quiet=>{self.bubble=None;if self.bridge.is_some(){self.send(json!({"type":"configure","social":false}));}},
        }
    }

    pub fn control_from_desktop(&mut self, ctx: &egui::Context, action: des_common::ipc::CompanionAction) {
        if matches!(action, des_common::ipc::CompanionAction::Quiet) && self.bridge.is_none() {
            self.connect(ctx);
        }
        self.control(action);
    }

    pub fn start_voice(&mut self, ctx: &egui::Context) {
        if self.voice_active(){return;}
        self.connect(ctx);
        self.voice_screen=0;self.voice_anchor=ctx.input(|i|i.pointer.hover_pos());
        self.voice_caption.clear();
        self.pending_voice=true;self.keep_requested=false;self.voice_requested=true;self.voice_notice=None;
        self.send(json!({"type":"status"}));
    }
    pub fn close(&mut self) {
        let active=self.open || self.voice_active();
        self.pending_voice=false;self.voice_requested=false;self.keep_requested=false;
        self.capture_requested=false;self.capture_pending=None;self.attachment=None;self.voice_screen_pending=false;
        if active && self.bridge.is_some(){self.send(json!({"type":"voice_stop"}));self.send(json!({"type":"cancel"}));}
        self.blur_text();
    }
    fn send(&mut self, mut value: Value) {
        self.next_id=self.next_id.wrapping_add(1); value["id"]=json!(self.next_id);
        if self.bridge.as_ref().is_none_or(|b|b.tx.try_send(value).is_err()) { self.bridge=None;self.reset_connection_state();self.error="Companion is busy or disconnected. Close and reopen to retry.".into(); }
    }
    pub fn pump(&mut self, ctx: &egui::Context) {
        let events:Vec<_>=self.bridge.as_ref().map(|b|b.rx.try_iter().collect()).unwrap_or_default();
        for event in events { match event["type"].as_str().or(event["event"].as_str()) {
            Some("state")=>{ self.worker_ready=true;let state=event.get("snapshot").cloned().unwrap_or(event); if self.memory_edit_saving.as_ref().is_some_and(|(id,draft)|state["memories"].as_array().is_some_and(|memories|memories.iter().any(|memory|memory["id"].as_i64()==Some(*id) && memory["text"].as_str()==Some(draft.as_str())))) {self.memory_edit_saving=None;self.memory_edit=None;} if self.model.is_empty() {self.model=state["model"].as_str().unwrap_or("gpt-5.6-luna").into();} if !self.identity_dirty{self.identity=state["identity"].as_str().unwrap_or("").into();} self.snapshot=state; },
            Some("voice")=>{
                self.pending_voice=false;self.voice_requested=false;
                if event["state"].as_str()==Some("stopped") {
                    self.voice_screen_pending=false;
                    if let Some(message)=event["message"].as_str().filter(|m|!m.is_empty()) {self.voice_notice=Some((message.into(),std::time::Instant::now()));}
                    else if event["reason"].as_str()==Some("deadline") {self.voice_notice=Some(("Voice session or daily limit reached. Check your voice allowance to continue.".into(),std::time::Instant::now()));}
                }
                let ready=matches!(event["state"].as_str(),Some("listening"|"speaking"));
                self.voice=event;
                if ready && self.keep_requested {self.keep_requested=false;self.send(json!({"type":"voice_keep"}));}
            },
            Some("voice_screen_queued")=>{ self.voice_screen_pending=false;self.attachment=None;self.voice_notice=Some(("Screen queued for the voice backend.".into(),std::time::Instant::now())); },
            Some("transcript")=>{if event["role"].as_str()==Some("assistant"){if event["is_new"].as_bool()==Some(true){self.voice_caption.clear();}self.voice_caption.push_str(event["text"].as_str().unwrap_or(""));self.voice_caption=self.voice_caption.chars().rev().take(1000).collect::<String>().chars().rev().collect();}let role=event["role"].as_str().unwrap_or("assistant");let delta=event["text"].as_str().unwrap_or("");if let Some(messages)=self.snapshot["messages"].as_array_mut(){if event["is_new"].as_bool()==Some(false) && messages.last().is_some_and(|m|m["role"]==role){let last=messages.last_mut().unwrap();let text=last["text"].as_str().unwrap_or("").to_owned()+delta;last["text"]=json!(text.chars().take(16000).collect::<String>());}else if messages.len()<200{messages.push(json!({"role":role,"text":delta}));}}},
            Some("reply") if event["proactive"].as_bool()==Some(true) && !self.open => {self.bubble_anchor=Some(ctx.input(|i|i.pointer.hover_pos()).unwrap_or(ctx.screen_rect().center()));self.bubble=Some((event["text"].as_str().unwrap_or("").chars().take(240).collect(),std::time::Instant::now()));},
            Some("error")=>{
                self.memory_edit_saving=None;self.voice_screen_pending=false;
                let voice_error=self.voice_requested || self.pending_voice;
                self.pending_voice=false;self.voice_requested=false;self.snapshot["busy"]=json!(false);
                self.error=event["message"].as_str().unwrap_or("Companion request failed.").into();
                if voice_error {self.voice_notice=Some((self.error.clone(),std::time::Instant::now()));}
            }, _=>{}
        } }
        if self.bridge.as_mut().is_some_and(|b|b.failed.load(Ordering::Acquire) || b.child.try_wait().ok().flatten().is_some()) { self.bridge=None;self.reset_connection_state();self.error="Companion stopped. Close and reopen to reconnect.".into(); }
        if self.pending_voice && self.worker_ready && self.snapshot.is_object() && !self.snapshot["credential_loading"].as_bool().unwrap_or(false) {
            self.pending_voice=false;
            self.send(json!({"type":"voice_start"}));
        }
        if !self.open { if let Some(id)=self.focus.take(){ctx.memory_mut(|m|m.request_focus(id));} }
    }
    pub fn social_tick(&mut self, ctx: &egui::Context, suppressed: bool, theme: &Theme, animate: bool) -> Option<egui::Rect> {
        if !self.social_probe_started && self.bridge.is_none() && self.error.is_empty() {
            self.social_probe_started = true;
            let (sender, receiver) = mpsc::channel();
            self.social_probe = Some(receiver);
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let enabled = worker_path().and_then(|path| Command::new("python3").arg(path).arg("--social-probe")
                    .env_remove("OPENAI_API_KEY").stdin(Stdio::null()).stderr(Stdio::null()).output().ok())
                    .filter(|output| output.status.success())
                    .map_or(true, |output| match output.stdout.as_slice() { b"no\n" => false, _ => true });
                let _ = sender.send(enabled);
                ctx.request_repaint();
            });
        }
        if let Some(enabled) = self.social_probe.as_ref().and_then(|probe| probe.try_recv().ok()) {
            self.social_probe = None;
            if enabled && self.bridge.is_none() && self.error.is_empty() {
                self.connect(ctx);
                self.send(json!({"type":"status"}));
            }
        }
        if suppressed || self.open { self.bubble=None; return None; }
        let now=std::time::Instant::now();
        if !ctx.input(|i|i.pointer.any_down()) && self.snapshot["enabled"].as_bool()==Some(true)
            && self.snapshot["social"].as_bool()==Some(true)
            && self.social_at.is_none_or(|last|now.duration_since(last).as_secs()>=1800) {
            self.social_at=Some(now);
            self.send(json!({"type":"social","suppressed":false}));
        }
        let (text,at)=self.bubble.as_ref()?;
        let age=now.duration_since(*at).as_secs_f32();
        let Some((alpha,rise))=thought_motion(age,animate) else {self.bubble=None;return None};
        let text=text.clone();
        let screen=ctx.screen_rect();
        let width=(screen.width()-72.0).clamp(1.0,300.0);
        let anchor=*self.bubble_anchor.get_or_insert_with(||ctx.input(|i|i.pointer.hover_pos()).unwrap_or(screen.center()));
        let pos=egui::pos2((anchor.x+28.0).clamp(screen.left()+24.0,(screen.right()-width-48.0).max(screen.left()+24.0)),
            (anchor.y+24.0).clamp(screen.top()+20.0,(screen.bottom()-190.0).max(screen.top()+20.0)));
        // Keep the hit rectangle fixed; only the thought trail rises. There is no moving target to chase.
        let mut talk=false;
        let mut quiet=false;
        let response=egui::Area::new(egui::Id::new("companion-social"))
            .order(egui::Order::Foreground).fixed_pos(pos).show(ctx,|ui|{
                ui.set_opacity(alpha);
                let surface=crate::material::surface(ui,
                    egui::Frame::NONE.inner_margin(16).corner_radius(ui::SURFACE_RADIUS)
                        .stroke(egui::Stroke::new(1.0,ui::LINE_STRONG)),
                    crate::material::Role::Floating,|ui|{
                        ui.set_width(width);
                        ui.style_mut().wrap_mode=Some(egui::TextWrapMode::Wrap);
                        ui.horizontal(|ui|{
                            ui.painter().circle_filled(ui.cursor().min+egui::vec2(3.0,7.0),2.5,theme.main);
                            ui.add_space(12.0);
                            ui.label(RichText::new("COMPANION").font(ui::display_bold(11.0)).color(ui::SECONDARY));
                        });
                        ui.add_space(6.0);
                        ui.label(RichText::new(text).font(ui::display(15.0)).color(ui::TEXT));
                        ui.add_space(10.0);
                        ui.horizontal(|ui|{
                            talk=ui::button(ui,theme,"TALK",None,false).clicked();
                            quiet=ui::button(ui,theme,"QUIET",None,false).clicked();
                        });
                    });
                let rect=surface.response.rect;
                ui.painter().rect_stroke(rect,ui::SURFACE_RADIUS,egui::Stroke::new(1.0,theme.main.gamma_multiply(0.18)),egui::StrokeKind::Inside);
                for (distance,radius) in [(7.0,4.0),(16.0,2.5)] {
                    let center=egui::pos2(rect.left()-distance,rect.top()+12.0-distance*0.5+rise);
                    ui.painter().circle_filled(center,radius,ui::PANEL);
                    ui.painter().circle_stroke(center,radius,egui::Stroke::new(0.8,theme.main.gamma_multiply(0.25)));
                }
            });
        if animate { ctx.request_repaint_after(std::time::Duration::from_millis(if age<0.28 || age>14.6 {16}else{100})); }
        else { ctx.request_repaint_after(std::time::Duration::from_secs_f32((15.0-age).max(0.01))); }
        if quiet {self.bubble=None;self.send(json!({"type":"configure","social":false}));}
        if talk {self.bubble=None;self.open(ctx);}
        Some(response.response.rect)
    }
    pub fn voice_active(&self) -> bool {
        self.pending_voice || self.voice_requested || self.snapshot["voice_active"].as_bool().unwrap_or(false)
            || matches!(self.voice["state"].as_str(),Some("connecting"|"listening"|"speaking"|"stopping"))
    }

    pub fn speaking(&self) -> bool { self.voice["state"].as_str()==Some("speaking") }

    /// Text chat occupies the shared bottom panel; it never opens a floating modal.
    pub fn panel_ui(&mut self, ui: &mut egui::Ui, theme: &Theme) {
        let rect=ui.max_rect();
        let escaped=self.open && ui.input_mut(|i|i.consume_key(egui::Modifiers::NONE,egui::Key::Escape));
        if escaped {
            self.blur_text();
        }
        let width=ui.available_width();
        let height=ui.available_height();
        ui.style_mut().wrap_mode=Some(egui::TextWrapMode::Wrap);
        if ui.input(|i|i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p|rect.contains(p))) {
            self.open=true;
        }
        let busy=self.snapshot["busy"].as_bool().unwrap_or(false);
        let active=self.voice_active();
        let ready=self.snapshot["enabled"].as_bool().unwrap_or(false)
            && self.snapshot["configured"].as_bool().unwrap_or(false) && !busy && !active;
        let message_height=(height-104.0).max(16.0);
        egui::ScrollArea::vertical().id_salt("companion-chat-history")
            .max_height(message_height).min_scrolled_height(message_height)
            .auto_shrink([false,false]).stick_to_bottom(true).show(ui,|ui|{
                if !self.error.is_empty() { ui.label(RichText::new(&self.error).color(ui::CAUTION)); }
                let messages=self.snapshot["messages"].as_array();
                if messages.is_none_or(|m|m.is_empty()) {
                    ui.label(RichText::new("Your conversation lives here.").color(ui::SECONDARY));
                }
                for message in messages.into_iter().flatten().rev().take(40).collect::<Vec<_>>().into_iter().rev() {
                    let user=message["role"].as_str()==Some("user");
                    ui.horizontal_top(|ui|{
                        ui.add_sized(egui::vec2(64.0,18.0),egui::Label::new(RichText::new(if user{"YOU"}else{"COMPANION"})
                            .font(ui::display_bold(10.0)).color(if user{ui::SECONDARY}else{theme.main})));
                        let text = message["text"].as_str().unwrap_or("");
                        ui.vertical(|ui| {
                            ui.add(egui::Label::new(text).wrap());
                            if !user && !text.is_empty() && ui.small_button("Copy reply").clicked() { ui.ctx().copy_text(text.to_owned()); }
                        });
                    });
                }
                if busy { ui.horizontal(|ui|{ui.spinner();ui.label("Thinking…");}); }
                if self.snapshot["credential_loading"].as_bool().unwrap_or(false) {
                    ui.label(RichText::new("Restoring saved key…").color(ui::SECONDARY));
                } else if !self.snapshot["enabled"].as_bool().unwrap_or(false) || !self.snapshot["configured"].as_bool().unwrap_or(false) {
                    let label=if !self.snapshot["enabled"].as_bool().unwrap_or(false){"Enable companion in Settings"}else{"Add an API key in Settings"};
                    if ui.small_button(label).clicked() {self.settings_requested=true;self.blur_text();}
                }
            });
        ui.add_space(4.0);
        let mut remove_attachment = false;
        let mut share_screen = false;
        ui.horizontal(|ui| {
            if self.attachment.is_none() && ui.add_enabled(self.capture_pending.is_none() && !self.voice_screen_pending, egui::Button::new("Attach screen").min_size(egui::vec2(104.0, 30.0))).on_hover_text("Capture the screen for review; it is only sent after you choose to share it").clicked() {
                self.capture_requested = true;
            }
            if self.capture_pending.is_some() { ui.label("Capturing…"); }
            if let Some(attachment) = &self.attachment {
                ui.add(egui::Image::new(&attachment.texture).max_size(egui::vec2(48.0, 28.0)))
                    .on_hover_ui(|ui| { ui.label("Will send to OpenAI with your message"); ui.add(egui::Image::new(&attachment.texture).max_size(egui::vec2(320.0, 180.0))); });
                if active {
                    let label=if self.voice_screen_pending {"Queuing…"} else {"Share with voice"};
                    if ui.add_enabled(!self.voice_screen_pending,egui::Button::new(label)).on_hover_text("Send this preview to OpenAI for the current voice conversation").clicked() {
                        share_screen = true;
                    }
                }
                if ui.add_enabled(!self.voice_screen_pending,egui::Button::new("Remove")).on_hover_text("Discard screen attachment").clicked() { remove_attachment = true; }
                ui.add(egui::Label::new(if active {"Share with voice sends to OpenAI"} else {"Will send to OpenAI with your message"}).truncate());
            }
            if busy && ui.button("Stop reply").clicked() { self.send(json!({"type":"cancel"})); }
        });
        if remove_attachment { self.attachment = None; }
        if share_screen {
            if let Some(image)=self.attachment.as_ref().map(|attachment|attachment.jpeg.clone()) {
                self.send(json!({"type":"voice_share_screen","image":image}));
                if self.bridge.is_some() {self.voice_screen_pending=true;}
            }
        }
        let mut send_clicked=false;
        let input_id=egui::Id::new("companion-chat-input");
        let enter=ui.memory(|memory| memory.has_focus(input_id)) && !escaped
            && ui.input_mut(|input| input.modifiers.matches_exact(egui::Modifiers::NONE) && input.consume_key(egui::Modifiers::NONE,egui::Key::Enter));
        ui.horizontal(|ui|{
            let edit=ui.add_sized(egui::vec2((width-70.0).max(1.0),56.0),
                egui::TextEdit::multiline(&mut self.input).id(input_id).desired_rows(2)
                    .char_limit(8000).hint_text(if active{"Voice conversation active"}else{"Message…  Shift+Enter for a new line"}));
            if self.focus_input {edit.request_focus();self.focus_input=false;}
            if edit.has_focus() && !escaped {self.open=true;}
            send_clicked=ui.add_enabled(ready&&!self.input.trim().is_empty(),egui::Button::new("Send").min_size(egui::vec2(56.0,56.0))).clicked();
        });
        if enter {self.focus_input=true;}
        if ready && (send_clicked || enter) && !self.input.trim().is_empty() {
            let text=self.input.clone();
            let mut command=json!({"type":"send","text":text});
            if let Some(attachment) = &self.attachment { command["image"] = json!(attachment.jpeg); }
            self.send(command);
            if self.bridge.is_some(){self.input.clear();self.attachment=None;self.snapshot["busy"]=json!(true);}
        }
        if !self.open { ui.memory_mut(|m|{if m.focused()==Some(egui::Id::new("companion-chat-input")){m.surrender_focus(egui::Id::new("companion-chat-input"));}}); }
    }

    pub fn select_files(&mut self) {self.chat_selected=false;self.blur_text();}

    pub fn set_voice_pointer(&mut self, pointer:(i32,i32,usize)) {
        self.voice_anchor=Some(egui::pos2(pointer.0 as f32,pointer.1 as f32));self.voice_screen=pointer.2;
    }

    pub fn cursor_commands(&mut self, caption:Option<egui::Rect>, allowed:bool, now:std::time::Instant) -> Vec<des_common::ipc::Command> {
        let speaking=allowed && self.speaking();
        let area=if allowed {caption.map(crate::scratchpad::area)}else{None};
        let changed=self.cursor_signal.is_none_or(|(old,rect,at)|old!=speaking || rect!=area || ((speaking || area.is_some()) && now.duration_since(at)>=std::time::Duration::from_millis(500)));
        if !changed {return Vec::new();}
        self.cursor_signal=Some((speaking,area,now));
        vec![des_common::ipc::Command::CompanionSpeaking(speaking),des_common::ipc::Command::CompanionBubble(area)]
    }

    pub fn blur_text(&mut self) {self.open=false;self.focus_input=false;}

    /// Voice status/caption is pointer-transparent and never captures app keyboard input.
    pub fn voice_bubble(&mut self, ctx:&egui::Context, theme:&Theme, animate:bool) -> Option<egui::Rect> {
        let active=self.voice_active();
        if self.voice_notice.as_ref().is_some_and(|(_,at)|at.elapsed().as_secs_f32()>=6.0){self.voice_notice=None;}
        if (!active && self.voice_notice.is_none()) || self.voice_screen!=0{return None;}
        let screen=ctx.screen_rect();
        let width=(screen.width()-64.0).clamp(1.0,310.0);
        let anchor=self.voice_anchor.get_or_insert_with(||ctx.input(|i|i.pointer.hover_pos()).unwrap_or(screen.center()));
        let pos=egui::pos2((anchor.x+24.0).clamp(screen.left()+16.0,(screen.right()-width-48.0).max(screen.left()+16.0)),
            (anchor.y+24.0).clamp(screen.top()+16.0,(screen.bottom()-190.0).max(screen.top()+16.0)));
        let speaking=self.speaking();
        let muted=self.voice["muted"].as_bool()==Some(true);
        let phase=ctx.input(|i|i.time) as f32;
        let pulse=if speaking && animate {0.5+0.5*(phase*std::f32::consts::TAU*0.9).sin()}else{0.0};
        let caption=self.voice_caption.chars().rev().take(180).collect::<String>().chars().rev().collect::<String>();
        let status=if !active {"VOICE ENDED"} else if speaking {"SPEAKING"} else if self.pending_voice || self.voice_requested {"CONNECTING"} else if muted {"MICROPHONE MUTED"} else {"LISTENING"};
        let response=egui::Area::new(egui::Id::new("companion-voice-thought"))
            .order(egui::Order::Tooltip).fixed_pos(pos).interactable(false).show(ctx,|ui|{
                crate::material::surface(ui,egui::Frame::NONE.inner_margin(16).corner_radius(ui::SURFACE_RADIUS)
                    .stroke(egui::Stroke::new(1.0,ui::LINE_STRONG)),crate::material::Role::Floating,|ui|{
                    ui.set_width(width);ui.style_mut().wrap_mode=Some(egui::TextWrapMode::Wrap);
                    ui.horizontal(|ui|{
                        let (rect,_)=ui.allocate_exact_size(egui::vec2(12.0,12.0),egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(),3.0+1.4*pulse,if muted {ui::CAUTION}else{theme.main});
                        ui.label(RichText::new(status).font(ui::display_bold(11.0)).color(if muted {ui::CAUTION}else{theme.main}));
                        if active {
                            let remaining=self.voice["remaining"].as_u64().unwrap_or(300);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{
                                ui.label(RichText::new(format!("{}:{:02}",remaining/60,remaining%60)).font(ui::mono(11.0)).color(ui::SECONDARY));
                            });
                        }
                    });
                    ui.add_space(6.0);
                    if speaking && !caption.is_empty() {
                        ui.label(RichText::new(caption).font(ui::display(14.0)).color(ui::TEXT));
                    } else if !active {
                        if let Some((message,_))=&self.voice_notice { ui.label(RichText::new(message).font(ui::display(13.0)).color(ui::TEXT)); }
                    } else {
                        let hint=if muted {"Resume the microphone from Chat."} else if self.pending_voice || self.voice_requested {"Starting your voice conversation…"} else {"Speak naturally. Voice controls are in Chat."};
                        ui.label(RichText::new(hint).font(ui::display(12.0)).color(ui::SECONDARY));
                    }
                    if active { if let Some((message,_))=&self.voice_notice {
                        ui.label(RichText::new(message).font(ui::display(11.0)).color(ui::SECONDARY));
                    } }
                });
            });
        if speaking && animate {ctx.request_repaint_after(std::time::Duration::from_millis(33));}
        if self.voice_notice.is_some(){ctx.request_repaint_after(std::time::Duration::from_millis(100));}
        Some(response.response.rect)
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_social_probe_does_not_start_companion_worker() {
        let (send, receive) = mpsc::channel();
        send.send(false).unwrap();
        let mut companion = Companion { social_probe_started:true, social_probe:Some(receive), ..Default::default() };
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| { companion.social_tick(ctx, false, &Theme::load("signal"), false); });
        assert!(companion.bridge.is_none());
        assert!(companion.social_probe.is_none());
    }
    #[test]
    fn attachment_keeps_the_composer_visible_in_a_short_panel() {
        let ctx=egui::Context::default(); ui::install_fonts(&ctx,None,None);
        let texture=ctx.load_texture("fixture",egui::ColorImage::new([2,2],egui::Color32::WHITE),egui::TextureOptions::LINEAR);
        let mut companion=Companion {attachment:Some(ScreenAttachment{jpeg:String::new(),texture}),..Default::default()};
        let screen=egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(800.0,140.0));
        let mut input_rect=None;
        for _ in 0..2 { let _=ctx.run(egui::RawInput {screen_rect:Some(screen),..Default::default()},|ctx| {
            egui::CentralPanel::default().show(ctx,|ui|companion.panel_ui(ui,&Theme::load("signal")));
            input_rect=ctx.read_response(egui::Id::new("companion-chat-input")).map(|response|response.rect);
        }); }
        let rect=input_rect.expect("chat input");
        assert!(rect.bottom() <= screen.bottom(), "composer below short panel: {rect:?}");
    }

    #[test]
    fn voice_card_stays_on_screen_and_does_not_take_keyboard_focus() {
        let ctx=egui::Context::default(); ui::install_fonts(&ctx,None,None);
        let focus=egui::Id::new("unrelated-input");
        let mut companion=Companion::default();
        companion.voice=json!({"state":"speaking","remaining":125,"muted":false});
        companion.voice_caption="A short spoken explanation".into();
        companion.voice_anchor=Some(egui::pos2(1270.0,790.0));
        let screen=egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(1280.0,800.0));
        let mut rect=None; let mut text=String::new();
        for frame in 0..2 {
            if frame == 1 { ctx.memory_mut(|memory|memory.request_focus(focus)); }
            let _=ctx.run(egui::RawInput {screen_rect:Some(screen),..Default::default()},|ctx|{
                egui::CentralPanel::default().show(ctx,|ui|{ui.add(egui::TextEdit::singleline(&mut text).id(focus));});
                rect=companion.voice_bubble(ctx,&Theme::load("signal"),false);
            });
        }
        let rect=rect.expect("active voice card");
        assert!(screen.contains_rect(rect), "card must fit the screen: {rect:?}");
        assert_eq!(ctx.memory(|memory|memory.focused()),Some(focus));
    }

    #[test]
    fn screen_attachment_is_previewed_and_stale_captures_are_discarded() {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.rgba");
        let mut companion = Companion::default();
        let id = companion.begin_capture();
        std::fs::write(&path, [255u8, 0, 0, 255, 0, 255, 0, 255]).unwrap();
        companion.receive_capture(&ctx, crate::capture::Shot { request: Some(id), mode: des_common::ipc::ShotMode::Screen, width: 2, height: 1, window: None, path: path.display().to_string() });
        assert!(!path.exists());
        let encoded = &companion.attachment.as_ref().unwrap().jpeg;
        assert!(base64::engine::general_purpose::STANDARD.decode(encoded).unwrap().starts_with(&[0xff, 0xd8, 0xff]));
        companion.close();
        assert!(companion.attachment.is_none());
        std::fs::write(&path, [0u8; 8]).unwrap();
        companion.receive_capture(&ctx, crate::capture::Shot { request: Some(id), mode: des_common::ipc::ShotMode::Screen, width: 2, height: 1, window: None, path: path.display().to_string() });
        assert!(companion.attachment.is_none());
        assert!(!path.exists());
    }

    #[test]
    fn invalid_screen_capture_is_removed_and_never_attached() {
        let ctx = egui::Context::default(); let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken.rgba"); std::fs::write(&path, [0u8; 4]).unwrap();
        let mut companion = Companion::default(); let id = companion.begin_capture();
        companion.receive_capture(&ctx, crate::capture::Shot { request: Some(id), mode: des_common::ipc::ShotMode::Screen, width: 2, height: 2, window: None, path: path.display().to_string() });
        assert!(companion.attachment.is_none()); assert!(!companion.error.is_empty()); assert!(!path.exists());
    }

    #[test]
    fn companion_starts_disabled_and_closing_does_not_start_worker() {
        let mut companion=Companion::default();
        assert!(!companion.open);
        assert!(companion.bridge.is_none());
        assert!(!companion.snapshot["enabled"].as_bool().unwrap_or(false));
        companion.close();
        assert!(companion.bridge.is_none());
    }
    #[test]
    fn explicit_voice_summon_does_not_queue_a_social_request() {
        let child=Command::new("python3").args(["-c","import time; time.sleep(30)"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        let (tx,commands)=mpsc::sync_channel(16);let (_events,rx)=mpsc::sync_channel(1);
        let bridge=Bridge {child,tx,rx,failed:Arc::new(AtomicBool::new(false))};
        let mut companion=Companion {bridge:Some(bridge),..Default::default()};
        companion.open(&egui::Context::default());companion.snapshot=json!({"credential_loading":false});companion.worker_ready=true;companion.start_voice(&egui::Context::default());companion.pump(&egui::Context::default());
        let commands:Vec<_>=commands.try_iter().map(|c|c["type"].as_str().unwrap().to_owned()).collect();
        assert_eq!(commands,vec!["status","status","voice_start"]);
    }
    #[test]
    fn saturated_control_queue_destroys_worker_instead_of_leaving_voice_active() {
        let child=Command::new("python3").args(["-c","import time; time.sleep(30)"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        let pid=child.id();let (tx,_commands)=mpsc::sync_channel(1);tx.try_send(json!({"type":"status"})).unwrap();let (_events,rx)=mpsc::sync_channel(1);
        let bridge=Bridge {child,tx,rx,failed:Arc::new(AtomicBool::new(false))};
        let mut companion=Companion {open:true,bridge:Some(bridge),..Default::default()};
        companion.close();
        assert!(companion.bridge.is_none());assert!(!companion.open);
        assert!(!PathBuf::from(format!("/proc/{pid}")).exists(),"worker survived failed stop delivery");
    }
    #[test]
    fn social_bubble_survives_pointer_press_and_release_without_focus() {
        let ctx=egui::Context::default();
        ui::install_fonts(&ctx,None,None);
        let mut companion=Companion::default();
        companion.error="No worker needed in UI test".into();
        companion.bubble=Some(("Would you like to talk?".into(),std::time::Instant::now()));
        let input=egui::RawInput {screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(800.0,600.0))),..Default::default()};
        let mut rect=None;
        let _=ctx.run(input.clone(),|ctx|{rect=companion.social_tick(ctx,false,&Theme::load("signal"),true);});
        let rect=rect.unwrap();
        for pressed in [true,false] {
            let mut frame=input.clone();
            frame.events=vec![egui::Event::PointerMoved(rect.center()),egui::Event::PointerButton {pos:rect.center(),button:egui::PointerButton::Primary,pressed,modifiers:egui::Modifiers::NONE}];
            let _=ctx.run(frame,|ctx|{companion.social_tick(ctx,false,&Theme::load("signal"),true);});
            assert!(companion.bubble.is_some());
            assert!(!companion.open);
            assert!(ctx.memory(|m|m.focused()).is_none());
        }
    }
}

#[cfg(test)]
mod revision_tests {
    use super::*;
    #[test]
    fn thought_motion_has_bounded_entrance_exit_and_reduced_motion() {
        let start=thought_motion(0.0,true).unwrap();
        let shown=thought_motion(1.0,true).unwrap();
        let leaving=thought_motion(14.9,true).unwrap();
        assert!(start.0<shown.0 && start.1>shown.1);
        assert!(leaving.0<shown.0);
        assert_eq!(thought_motion(0.0,false),Some((1.0,0.0)));
        assert_eq!(thought_motion(15.0,true),None);
    }
    #[test]
    fn disconnected_worker_clears_stale_voice_and_key_availability() {
        let child=Command::new("python3").args(["-c","pass"]).spawn().unwrap();
        let (tx,_commands)=mpsc::sync_channel(1);let (_events,rx)=mpsc::sync_channel(1);
        let bridge=Bridge {child,tx,rx,failed:Arc::new(AtomicBool::new(true))};
        let mut companion=Companion {bridge:Some(bridge),voice:json!({"state":"listening","muted":true}),snapshot:json!({"voice_active":true,"configured":true,"busy":true}),..Default::default()};
        companion.pump(&egui::Context::default());
        assert!(companion.bridge.is_none());
        assert!(!companion.snapshot["voice_active"].as_bool().unwrap_or(false));
        assert!(!companion.snapshot["configured"].as_bool().unwrap_or(false));
        assert!(!matches!(companion.voice["state"].as_str(),Some("listening"|"connecting"|"stopping")));
    }
    #[test]
    fn setup_can_render_in_settings_without_opening_a_conversation() {
        let ctx=egui::Context::default();
        ui::install_fonts(&ctx,None,None);
        let mut companion=Companion {error:"No worker in this test".into(),..Default::default()};
        let _=ctx.run(Default::default(),|ctx|{egui::CentralPanel::default().show(ctx,|ui|companion.settings_ui(ui,&Theme::load("signal")));});
        assert!(!companion.open);
        assert!(companion.bridge.is_none());
    }
}

#[cfg(test)]
mod panel_revision_tests {
    use super::*;
    #[test]
    fn chat_panel_fits_existing_bottom_panel_and_does_not_create_a_modal() {
        for width in [320.0,900.0] {
            let ctx=egui::Context::default();ui::install_fonts(&ctx,None,None);
            let theme=Theme::load("signal");crate::apply_style(&ctx,&theme);
            let mut companion=Companion {open:true,chat_selected:true,error:"No worker in test".into(),..Default::default()};
            let rect=egui::Rect::from_min_size(egui::pos2(16.0,16.0),egui::vec2(width-32.0,170.0));
            for _ in 0..3 {let _=ctx.run(egui::RawInput {screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(width,240.0))),..Default::default()},|ctx|{
                egui::CentralPanel::default().show(ctx,|ui|{ui.scope_builder(egui::UiBuilder::new().max_rect(rect),|ui|companion.panel_ui(ui,&theme));});
            });}
            assert!(ctx.memory(|m|m.area_rect(egui::Id::new("cursor-companion"))).is_none());
            assert!(companion.open);
        }
    }
    #[test]
    fn escape_releases_chat_keyboard_but_preserves_selected_tab_and_draft() {
        let ctx=egui::Context::default();ui::install_fonts(&ctx,None,None);
        let mut companion=Companion {open:true,chat_selected:true,input:"Unsent draft".into(),..Default::default()};
        let event=egui::Event::Key {key:egui::Key::Escape,physical_key:None,pressed:true,repeat:false,modifiers:egui::Modifiers::NONE};
        let _=ctx.run(egui::RawInput {events:vec![event],..Default::default()},|ctx|{egui::CentralPanel::default().show(ctx,|ui|companion.panel_ui(ui,&Theme::load("signal")));});
        assert!(!companion.open);assert!(companion.chat_selected);assert_eq!(companion.input,"Unsent draft");
        assert!(!ctx.input(|i|i.key_pressed(egui::Key::Escape)));
    }
}

#[cfg(test)]
mod panel_flow_tests {
    use super::*;
    fn fake_bridge() -> (Bridge,Receiver<Value>,SyncSender<Value>) {
        let child=Command::new("python3").args(["-c","import time; time.sleep(30)"]).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        let (tx,commands)=mpsc::sync_channel(16);let (events,rx)=mpsc::sync_channel(8);
        (Bridge {child,tx,rx,failed:Arc::new(AtomicBool::new(false))},commands,events)
    }
    #[test]
    fn explicit_voice_waits_for_saved_key_load_then_starts_once_without_opening_chat() {
        let (bridge,commands,events)=fake_bridge();
        let mut companion=Companion {bridge:Some(bridge),worker_ready:true,snapshot:json!({"credential_loading":true}),..Default::default()};
        let ctx=egui::Context::default();
        companion.start_voice(&ctx);companion.pump(&ctx);
        assert_eq!(commands.try_iter().map(|c|c["type"].as_str().unwrap().to_owned()).collect::<Vec<_>>(),vec!["status"]);
        events.send(json!({"type":"state","credential_loading":false,"enabled":true,"configured":true})).unwrap();
        companion.pump(&ctx);companion.pump(&ctx);
        assert_eq!(commands.try_iter().map(|c|c["type"].as_str().unwrap().to_owned()).collect::<Vec<_>>(),vec!["voice_start"]);
        assert!(!companion.open);assert!(!companion.chat_selected);
    }
    #[test]
    fn stopping_during_saved_key_restore_revokes_the_pending_voice_start() {
        let (bridge,commands,events)=fake_bridge();
        let mut companion=Companion {bridge:Some(bridge),worker_ready:true,snapshot:json!({"credential_loading":true}),..Default::default()};
        let ctx=egui::Context::default();
        companion.start_voice(&ctx);
        companion.control(des_common::ipc::CompanionAction::Stop);
        events.send(json!({"type":"state","credential_loading":false,"enabled":true,"configured":true})).unwrap();
        companion.pump(&ctx);
        assert!(!commands.try_iter().any(|c|c["type"]=="voice_start"),"Stop must revoke the earlier voice request");
        assert!(!companion.voice_active());
    }
    #[test]
    fn shift_enter_inserts_a_newline_without_sending() {
        let (bridge,commands,_events)=fake_bridge();
        let ctx=egui::Context::default();ui::install_fonts(&ctx,None,None);
        let mut companion=Companion {bridge:Some(bridge),open:true,chat_selected:true,focus_input:true,snapshot:json!({"enabled":true,"configured":true}),..Default::default()};
        let screen=egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(800.0,240.0));
        let run=|input, companion:&mut Companion| { let _=ctx.run(input,|ctx| {egui::CentralPanel::default().show(ctx,|ui|companion.panel_ui(ui,&Theme::load("signal")));}); };
        run(egui::RawInput {screen_rect:Some(screen),..Default::default()},&mut companion);
        run(egui::RawInput {screen_rect:Some(screen),events:vec![egui::Event::Text("hello".into())],..Default::default()},&mut companion);
        run(egui::RawInput {screen_rect:Some(screen),modifiers:egui::Modifiers::SHIFT,events:vec![egui::Event::Key {key:egui::Key::Enter,physical_key:None,pressed:true,repeat:false,modifiers:egui::Modifiers::SHIFT}],..Default::default()},&mut companion);
        assert_eq!(companion.input,"hello\n");
        assert!(!commands.try_iter().any(|item|item["type"]=="send"));
    }

    #[test]
    fn enter_sends_the_chat_draft_and_consumes_the_key() {
        let (bridge,commands,_events)=fake_bridge();
        let ctx=egui::Context::default();ui::install_fonts(&ctx,None,None);
        let mut companion=Companion {bridge:Some(bridge),open:true,chat_selected:true,focus_input:true,snapshot:json!({"enabled":true,"configured":true}),..Default::default()};
        let frame=|ctx:&egui::Context,companion:&mut Companion,events|{
            let _=ctx.run(egui::RawInput {events,screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(800.0,240.0))),..Default::default()},|ctx|{
                egui::CentralPanel::default().show(ctx,|ui|companion.panel_ui(ui,&Theme::load("signal")));
            });
        };
        frame(&ctx,&mut companion,Vec::new());
        frame(&ctx,&mut companion,vec![egui::Event::Text("hello".into())]);
        assert_eq!(companion.input,"hello");
        frame(&ctx,&mut companion,vec![egui::Event::Key {key:egui::Key::Enter,physical_key:None,pressed:true,repeat:false,modifiers:egui::Modifiers::NONE}]);
        let sent=commands.try_iter().find(|c|c["type"]=="send").expect("Enter sends chat");
        assert_eq!(sent["text"],"hello");assert!(companion.input.is_empty());
        assert!(!ctx.input(|i|i.key_pressed(egui::Key::Enter)));
        frame(&ctx,&mut companion,Vec::new());
        frame(&ctx,&mut companion,vec![egui::Event::Text("next draft".into())]);
        assert_eq!(companion.input,"next draft");
    }
}
