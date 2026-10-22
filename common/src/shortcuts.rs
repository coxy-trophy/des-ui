//! Shared, validated keyboard shortcuts. Only overrides are persisted.
use std::{collections::BTreeMap, fmt};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope { Desktop, Shell, Terminal }
#[derive(Clone, Copy, Debug)]
pub struct Definition { pub id: &'static str, pub label: &'static str, pub group: &'static str, pub scope: Scope, pub default: &'static str }
pub const DEFINITIONS: &[Definition] = &[
    Definition { id: "desktop.cycle", label: "Next application", group: "Windows", scope: Scope::Desktop, default: "Super+Tab" },
    Definition { id: "desktop.close", label: "Close application", group: "Windows", scope: Scope::Desktop, default: "Super+Q" },
    Definition { id: "desktop.move_display", label: "Move to next display", group: "Windows", scope: Scope::Desktop, default: "Super+O" },
    Definition { id: "desktop.launcher", label: "Open launcher", group: "Desktop", scope: Scope::Desktop, default: "Super+Space" },
    Definition { id: "desktop.dock", label: "Open dock", group: "Desktop", scope: Scope::Desktop, default: "Super+D" },
    Definition { id: "desktop.settings", label: "Open settings", group: "Desktop", scope: Scope::Desktop, default: "Super+Comma" },
    Definition { id: "desktop.music", label: "Open music", group: "Desktop", scope: Scope::Desktop, default: "Super+M" },
    Definition { id: "desktop.notifications", label: "Open notifications", group: "Desktop", scope: Scope::Desktop, default: "Super+N" },
    Definition { id: "desktop.quick", label: "Quick settings", group: "Desktop", scope: Scope::Desktop, default: "Super+A" },
    Definition { id: "desktop.power", label: "Power menu", group: "Desktop", scope: Scope::Desktop, default: "Super+Escape" },
    Definition { id: "desktop.split", label: "Toggle split", group: "Windows", scope: Scope::Desktop, default: "Super+S" },
    Definition { id: "desktop.cycle_layout", label: "Next layout", group: "Windows", scope: Scope::Desktop, default: "Super+Shift+S" },
    Definition { id: "desktop.nudge_left", label: "Move divider left", group: "Windows", scope: Scope::Desktop, default: "Super+Alt+Left" },
    Definition { id: "desktop.nudge_right", label: "Move divider right", group: "Windows", scope: Scope::Desktop, default: "Super+Alt+Right" },
    Definition { id: "desktop.swap_master", label: "Make master", group: "Windows", scope: Scope::Desktop, default: "Super+Enter" },
    Definition { id: "desktop.fullscreen", label: "Toggle fullscreen", group: "Windows", scope: Scope::Desktop, default: "Super+F" },
    Definition { id: "desktop.pin", label: "Pin application", group: "Windows", scope: Scope::Desktop, default: "Super+P" },
    Definition { id: "desktop.focus_left", label: "Focus left half", group: "Windows", scope: Scope::Desktop, default: "Super+Left" },
    Definition { id: "desktop.focus_right", label: "Focus right half", group: "Windows", scope: Scope::Desktop, default: "Super+Right" },
    Definition { id: "desktop.panel_left", label: "Toggle left panel", group: "Panels", scope: Scope::Desktop, default: "Super+BracketLeft" },
    Definition { id: "desktop.panel_right", label: "Toggle right panel", group: "Panels", scope: Scope::Desktop, default: "Super+BracketRight" },
    Definition { id: "desktop.panel_bottom", label: "Toggle bottom panel", group: "Panels", scope: Scope::Desktop, default: "Super+Backslash" },
    Definition { id: "desktop.restart_shell", label: "Restart shell", group: "Desktop", scope: Scope::Desktop, default: "Super+Shift+R" },
    Definition { id: "desktop.overview", label: "Workspace overview", group: "Workspaces", scope: Scope::Desktop, default: "Super+W" },
    Definition { id: "desktop.help", label: "Shortcut reference", group: "Desktop", scope: Scope::Desktop, default: "Super+Slash" },
    Definition { id: "desktop.workspace_1", label: "Workspace 1", group: "Workspaces", scope: Scope::Desktop, default: "Super+1" },
    Definition { id: "desktop.send_1", label: "Send to workspace 1", group: "Workspaces", scope: Scope::Desktop, default: "Super+Shift+1" },
    Definition { id: "desktop.workspace_2", label: "Workspace 2", group: "Workspaces", scope: Scope::Desktop, default: "Super+2" },
    Definition { id: "desktop.send_2", label: "Send to workspace 2", group: "Workspaces", scope: Scope::Desktop, default: "Super+Shift+2" },
    Definition { id: "desktop.workspace_3", label: "Workspace 3", group: "Workspaces", scope: Scope::Desktop, default: "Super+3" },
    Definition { id: "desktop.send_3", label: "Send to workspace 3", group: "Workspaces", scope: Scope::Desktop, default: "Super+Shift+3" },
    Definition { id: "desktop.workspace_4", label: "Workspace 4", group: "Workspaces", scope: Scope::Desktop, default: "Super+4" },
    Definition { id: "desktop.send_4", label: "Send to workspace 4", group: "Workspaces", scope: Scope::Desktop, default: "Super+Shift+4" },
    Definition { id: "shell.scratchpad", label: "Scratchpad", group: "Shell", scope: Scope::Shell, default: "Ctrl+Shift+Backtick" },
    Definition { id: "shell.keyboard", label: "On-screen keyboard", group: "Shell", scope: Scope::Shell, default: "Ctrl+Shift+K" },
    Definition { id: "shell.dock", label: "Dock", group: "Shell", scope: Scope::Shell, default: "Ctrl+Shift+D" },
    Definition { id: "shell.new_tab", label: "New terminal tab", group: "Shell", scope: Scope::Shell, default: "Ctrl+Shift+T" },
    Definition { id: "shell.launcher", label: "Launcher", group: "Shell", scope: Scope::Shell, default: "Ctrl+Shift+A" },
    Definition { id: "shell.settings", label: "Settings", group: "Shell", scope: Scope::Shell, default: "Ctrl+Shift+S" },
    Definition { id: "shell.music", label: "Music", group: "Shell", scope: Scope::Shell, default: "Ctrl+Shift+M" },
    Definition { id: "shell.notifications", label: "Notifications", group: "Shell", scope: Scope::Shell, default: "Ctrl+Shift+N" },
    Definition { id: "shell.power", label: "Power menu", group: "Shell", scope: Scope::Shell, default: "Ctrl+Shift+P" },
    Definition { id: "shell.quit", label: "Close tab or pane", group: "Shell", scope: Scope::Shell, default: "Ctrl+Shift+Q" },
    Definition { id: "shell.split_right", label: "Split right", group: "Shell", scope: Scope::Shell, default: "Ctrl+Shift+E" },
    Definition { id: "shell.split_down", label: "Split down", group: "Shell", scope: Scope::Shell, default: "Ctrl+Shift+O" },
    Definition { id: "shell.focus_left", label: "Focus pane left", group: "Shell", scope: Scope::Shell, default: "Ctrl+Shift+Left" },
    Definition { id: "shell.focus_right", label: "Focus pane right", group: "Shell", scope: Scope::Shell, default: "Ctrl+Shift+Right" },
    Definition { id: "shell.focus_up", label: "Focus pane up", group: "Shell", scope: Scope::Shell, default: "Ctrl+Shift+Up" },
    Definition { id: "shell.focus_down", label: "Focus pane down", group: "Shell", scope: Scope::Shell, default: "Ctrl+Shift+Down" },
    Definition { id: "shell.help", label: "Shortcut reference", group: "Shell", scope: Scope::Shell, default: "Ctrl+Shift+H" },
    Definition { id: "shell.next_tab", label: "Next tab", group: "Shell", scope: Scope::Shell, default: "Ctrl+Tab" },
    Definition { id: "shell.previous_tab", label: "Previous tab", group: "Shell", scope: Scope::Shell, default: "Ctrl+Shift+Tab" },
    Definition { id: "terminal.find", label: "Find in terminal", group: "Terminal", scope: Scope::Terminal, default: "Ctrl+Shift+F" },
    Definition { id: "terminal.prompt_previous", label: "Previous prompt", group: "Terminal", scope: Scope::Terminal, default: "Ctrl+Shift+Up" },
    Definition { id: "terminal.prompt_next", label: "Next prompt", group: "Terminal", scope: Scope::Terminal, default: "Ctrl+Shift+Down" },
];
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chord { pub ctrl: bool, pub alt: bool, pub shift: bool, pub super_key: bool, pub key: String }
impl Chord {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut chord=Self { ctrl:false, alt:false, shift:false, super_key:false, key:String::new() };
        for token in text.trim().split('+') {
            let lower=token.trim().to_ascii_lowercase();
            let modifier=match lower.as_str() { "ctrl"|"control"=>Some(&mut chord.ctrl), "alt"=>Some(&mut chord.alt), "shift"=>Some(&mut chord.shift), "super"|"logo"|"meta"=>Some(&mut chord.super_key), _=>None };
            if let Some(flag)=modifier { if *flag { return Err("Duplicate modifier".into()) } *flag=true; continue }
            if !chord.key.is_empty() { return Err("Use one key per shortcut".into()) }
            chord.key=match lower.as_str() { "space"|" "=>"Space".into(), "return"|"enter"|"kp_enter"=>"Enter".into(), "esc"|"escape"=>"Escape".into(), "tab"|"iso_left_tab"=>"Tab".into(), "backspace"=>"Backspace".into(), "home"=>"Home".into(), "end"=>"End".into(), "insert"=>"Insert".into(), "delete"=>"Delete".into(), "pageup"|"page_up"=>"PageUp".into(), "pagedown"|"page_down"=>"PageDown".into(), "left"=>"Left".into(), "right"=>"Right".into(), "up"=>"Up".into(), "down"=>"Down".into(), "`"|"grave"|"backtick"=>"Backtick".into(), "/"|"slash"=>"Slash".into(), ","|"comma"=>"Comma".into(), "."|"period"=>"Period".into(), "["|"bracketleft"=>"BracketLeft".into(), "]"|"bracketright"=>"BracketRight".into(), "\\"|"backslash"=>"Backslash".into(), "-"|"minus"=>"Minus".into(), "="|"equal"=>"Equal".into(), ";"|"semicolon"=>"Semicolon".into(), "'"|"apostrophe"=>"Apostrophe".into(), _ if lower.len()==1 && lower.as_bytes()[0].is_ascii_alphanumeric()=>lower.to_ascii_uppercase(), _ if lower.starts_with('f') && lower[1..].parse::<u8>().is_ok_and(|n|(1..=24).contains(&n))=>lower.to_ascii_uppercase(), _=>return Err("Unsupported key".into()) };
        }
        if chord.key.is_empty() { return Err("Choose a key with modifiers".into()) }
        Ok(chord)
    }
}
impl fmt::Display for Chord {
    fn fmt(&self,f:&mut fmt::Formatter<'_>)->fmt::Result { for (set,name) in [(self.ctrl,"Ctrl"),(self.alt,"Alt"),(self.shift,"Shift"),(self.super_key,"Super")] { if set { write!(f,"{name}+")?; } } write!(f,"{}",self.key) }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bindings { overrides: BTreeMap<String,String> }
impl Bindings {
    pub fn binding(&self,id:&str)->String { self.overrides.get(id).cloned().unwrap_or_else(||DEFINITIONS.iter().find(|d|d.id==id).map_or("",|d|d.default).into()) }
    pub fn matches(&self,id:&str,chord:&Chord)->bool { Chord::parse(&self.binding(id)).is_ok_and(|c|c==*chord) }
    fn validate(&self)->Result<(),String> {
        for (i,d) in DEFINITIONS.iter().enumerate() {
            let text=self.binding(d.id); if text.is_empty(){continue} let c=Chord::parse(&text)?;
            if d.scope==Scope::Desktop && !(c.super_key || c.ctrl&&c.alt) { return Err("Desktop shortcuts require Super or Ctrl+Alt".into()) }
            if d.scope!=Scope::Desktop && (!c.ctrl || c.super_key || c.alt) { return Err("Shell and terminal shortcuts require Ctrl and cannot use Alt or Super".into()) }
            if (c.super_key && c.key=="L") || (c.ctrl&&c.alt&&(c.key=="Backspace"||c.key=="L"||c.key.starts_with('F'))) || (!c.ctrl&&!c.super_key&&c.alt&&(c.key=="Tab"||c.key=="Backtick")) { return Err("Reserved for locking, recovery or window switching".into()) }
            if c.ctrl && !c.alt && !c.super_key && ((!c.shift && matches!(c.key.as_str(),"C"|"D"|"X"|"V")) || (c.shift && matches!(c.key.as_str(),"C"|"V"))) {
                return Err("Reserved for terminal editing, copy and paste".into());
            }
            for other in &DEFINITIONS[..i] {
                let context_pair=matches!((d.id,other.id),
                    ("terminal.prompt_previous","shell.focus_up") | ("terminal.prompt_next","shell.focus_down") |
                    ("shell.focus_up","terminal.prompt_previous") | ("shell.focus_down","terminal.prompt_next"));
                if context_pair {continue}
                let other_text=self.binding(other.id);
                let collision=Chord::parse(&other_text).is_ok_and(|o| effective_chords(&c).iter().any(|a|effective_chords(&o).contains(a)));
                if collision {return Err(format!("Already used by {}",other.label))} }
        } Ok(())
    }
    pub fn set(&mut self,id:&str,chord:&str)->Result<(),String> { if !DEFINITIONS.iter().any(|d|d.id==id){return Err("Unknown shortcut".into())} let value=if chord.trim().is_empty(){String::new()}else{Chord::parse(chord)?.to_string()}; let mut next=self.clone(); next.overrides.insert(id.into(),value); next.validate()?; *self=next; Ok(()) }
    pub fn reset(&mut self,id:&str)->Result<(),String> { if !DEFINITIONS.iter().any(|d|d.id==id){return Err("Unknown shortcut".into())} let mut next=self.clone(); next.overrides.remove(id); next.validate()?; *self=next; Ok(()) }
    pub fn reset_all(&mut self){self.overrides.clear()}
    pub fn encode(&self)->String { let mut text="v1".to_string(); for (id,chord) in &self.overrides {text.push(';');text.push_str(id);text.push(':');text.push_str(chord)} text }
    pub fn parse(text:&str)->Option<Self> {
        let value=if text.trim().starts_with("v1"){text.trim()}else{text.lines().filter_map(|line|line.split('#').next()?.split_once('=')).filter(|(key,_)|key.trim()=="shortcuts").last()?.1.trim()};
        let mut words=value.split(';'); if words.next()? != "v1" {return None} let mut result=Self::default();
        for word in words {let(id,chord)=word.split_once(':')?; if !DEFINITIONS.iter().any(|d|d.id==id)||result.overrides.contains_key(id){return None} let chord=if chord.is_empty(){String::new()}else{Chord::parse(chord).ok()?.to_string()}; result.overrides.insert(id.into(),chord);}
        result.validate().ok()?; Some(result)
    }
    pub fn load()->Self { super::input::config_path().and_then(|path|std::fs::read_to_string(path).ok()).and_then(|text|Self::parse(&text)).unwrap_or_default() }
}

/// Physical alternatives available in nested compositor sessions.
fn effective_chords(chord:&Chord)->Vec<Chord> {
    let mut result=vec![chord.clone()];
    if chord.super_key && !chord.ctrl {
        let mut alias=chord.clone(); alias.super_key=false;alias.ctrl=true;alias.alt=true;
        if chord.alt {alias.shift=true} result.push(alias);
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn remap_and_disable_survive_roundtrip() {
        let mut b=Bindings::default();b.set("desktop.help","Super+J").unwrap();
        assert!(!b.matches("desktop.help",&Chord::parse("Super+Slash").unwrap()));
        b.set("desktop.help","").unwrap(); assert_eq!(Bindings::parse(&b.encode()),Some(b));
    }
    #[test] fn conflicts_are_atomic_and_resets_validate() {
        let mut b=Bindings::default(); let old=b.clone(); assert!(b.set("desktop.help","Super+D").is_err());assert_eq!(b,old);
        b.set("desktop.dock","").unwrap();b.set("desktop.help","Super+D").unwrap();assert!(b.reset("desktop.dock").is_err());
        b.reset_all();assert_eq!(b,Bindings::default());
    }
    #[test] fn safe_keys_and_terminal_context() {
        for key in ["A","Alt+A","Super+L","Ctrl+Alt+Backspace","Alt+Tab"] {assert!(Bindings::default().set("desktop.help",key).is_err(),"{key}")}
        assert!(Bindings::default().validate().is_ok());
        assert!(Bindings::default().set("shell.help","Super+J").is_err());
        assert_eq!(Chord::parse("shift+ctrl+`").unwrap().to_string(),"Ctrl+Shift+Backtick");
    }
    #[test] fn protected_terminal_keys_and_cross_scope_conflicts() {
        for id in ["shell.new_tab", "terminal.find"] {
            for chord in ["Ctrl+C", "Ctrl+D", "Ctrl+X", "Ctrl+V", "Ctrl+Shift+C", "Ctrl+Shift+V"] {
                let mut bindings=Bindings::default(); let before=bindings.clone();
                assert!(bindings.set(id,chord).is_err(), "{id} {chord}");assert_eq!(bindings,before);
            }
        }
        assert!(Bindings::default().set("terminal.find","Ctrl+Shift+T").is_err());
        assert!(Bindings::default().validate().is_ok());
        assert!(Bindings::default().set("desktop.help","Ctrl+Alt+D").is_err());
    }
    #[test] fn last_config_line_controls_shortcuts() {
        let custom="v1;desktop.help:Super+J";
        assert_eq!(Bindings::parse(&format!("shortcuts = v1\nshortcuts = {custom}")),Bindings::parse(custom));
        assert_eq!(Bindings::parse(&format!("shortcuts = {custom}\nshortcuts = broken")),None);
    }
    #[test] fn missing_and_malformed_config_keep_migration_explicit() {
        assert_eq!(Bindings::parse("theme = matrix"),None);
        assert_eq!(Bindings::parse("shortcuts = v1"),Some(Bindings::default()));
        assert_eq!(Bindings::parse("v2"),None);assert_eq!(Bindings::parse("v1;desktop.help:Super+D"),None);
    }
}
