//! Typed palette results; queries are never interpreted as executable commands.
use crate::apps::AppEntry;
use crate::clipboard;
use crate::commands::{self, Catalog, UnitOp};
use des_common::ipc::{Command, DesktopState};

#[derive(Debug,Clone,PartialEq)]
pub enum Action {
    Launch(AppEntry), Command(Command), Settings(&'static str), File(std::path::PathBuf),
    Music, Notifications, Power, Lock, SaveSession, RestoreSession, FindTerminal, CopyLastOutput, Scratchpad,
    /// Run this program with these arguments in a new terminal tab; no shell between.
    RunInTerminal(Vec<String>),
    /// Put this text on the clipboard, which the palette does itself before reporting it.
    Copy(String),
    /// Ask the process with this id to end.
    Terminate(u32),
    Unit { name:String, user:bool, op:UnitOp },
    /// Pin or unpin a clipboard entry; the palette stays open.
    ClipPin(u64,bool),
    /// Forget one clipboard entry.
    ClipRemove(u64),
    /// Forget the whole clipboard history.
    ClipClear,
}

impl Action {
    /// Actions that tidy the clipboard history and leave the palette where it is.
    pub fn keeps_palette_open(&self)->bool { matches!(self,Action::ClipPin(..)|Action::ClipRemove(_)|Action::ClipClear) }
}
#[derive(Clone)]
pub struct Item { pub label:String, pub detail:String, pub kind:&'static str, pub action:Action, pub pin:Option<String>, pub icon:Option<String>,
    /// A clipboard entry's id and whether it is pinned; the row's menu acts on it.
    pub clip:Option<(u64,bool)> }

fn item(label:String,detail:String,kind:&'static str,action:Action)->Item { Item {label,detail,kind,action,pin:None,icon:None,clip:None} }

impl Item {
    /// Clipboard rows keep the history's order; a label would sort them alphabetically.
    fn ordered(mut self,order:usize)->Item { self.detail=format!("{}\u{200b}{order:06}",self.detail); self }
    fn sort_key(&self)->String {
        if self.kind=="CLIP" { self.detail.rsplit('\u{200b}').next().unwrap_or("").to_string() } else { self.label.to_lowercase() }
    }
}

/// `clip` or `clipboard` asks for the history; words after it narrow it down.
pub fn clip_query(query:&str)->Option<String> {
    let query=query.trim();
    for word in ["clipboard","clip"] {
        if query.eq_ignore_ascii_case(word) { return Some(String::new()); }
        if let Some(rest)=query.get(..word.len()).filter(|head|head.eq_ignore_ascii_case(word)).and_then(|_|query[word.len()..].strip_prefix(' ')) { return Some(rest.trim().to_lowercase()); }
    }
    None
}

pub fn results(query:&str, entries:&[AppEntry], state:Option<&DesktopState>, catalog:&Catalog, clips:&[clipboard::Entry]) -> Vec<Item> {
    let query=query.trim().to_lowercase();
    let mut items=Vec::new();
    for entry in entries {
        let running=state.and_then(|state|state.apps.iter().position(|app| crate::dock::entry_for(&app.app_id,entries).is_some_and(|i|entries[i].desktop_id==entry.desktop_id)));
        items.push(Item { label:entry.name.clone(),detail:if running.is_some(){"Focus running application".into()}else{entry.exec.first().cloned().unwrap_or_default()},kind:"APP",
            action:running.map_or_else(||Action::Launch(entry.clone()),|index|Action::Command(Command::Activate(index))),pin:Some(entry.desktop_id.clone()),icon:entry.icon.clone(),clip:None });
    }
    if let Some(state)=state {
        for (index,app) in state.apps.iter().enumerate() {
            items.push(item(app.title.clone(),format!("{} · display {} · workspace {}",app.app_id,app.screen+1,app.workspace+1),"WINDOW",Action::Command(Command::Activate(index))));
        }
    }
    for (id,title) in crate::settings::destinations() {
        items.push(item(format!("Settings: {title}"),"Open settings section".into(),"SETTING",Action::Settings(id)));
    }
    let mut commands=vec![
        ("Show terminal",Action::Command(Command::Terminal)),
        ("Find in terminal",Action::FindTerminal),
        ("Scratchpad terminal",Action::Scratchpad),
        ("Copy last command output",Action::CopyLastOutput),
        ("Split workspace",Action::Command(Command::Split)),
        ("Toggle fullscreen",Action::Command(Command::Fullscreen)),
        ("Take screenshot",Action::Command(Command::Shot(des_common::ipc::ShotMode::Pick))),
        ("Lock screen",Action::Lock),("Music player",Action::Music),("Notifications",Action::Notifications),("Power menu",Action::Power),
        ("Save session",Action::SaveSession),("Restore session",Action::RestoreSession),
    ];
    for (label,action) in commands.drain(..) { items.push(item(label.into(),"Desktop action".into(),"ACTION",action)); }
    for workspace in 0..des_common::ipc::WORKSPACES {
        items.push(item(format!("Switch to workspace {}",workspace+1),"Desktop action".into(),"ACTION",Action::Command(Command::Workspace(workspace))));
    }
    for host in &catalog.hosts {
        let detail=match &host.address { Some(address)=>format!("ssh · {address}"), None=>"ssh".into() };
        items.push(item(host.name.clone(),detail,"HOST",Action::RunInTerminal(vec!["ssh".into(),host.name.clone()])));
    }
    for command in &catalog.commands {
        let mut detail=command.exec.join(" ");
        if !command.keywords.is_empty(){detail=format!("{detail} · {}",command.keywords.join(", "));}
        let action=if command.terminal {Action::RunInTerminal(command.exec.clone())} else {
            Action::Launch(AppEntry {name:command.name.clone(),desktop_id:format!("command:{}",command.id),exec:command.exec.clone(),icon:None,startup_wm_class:None,terminal:false})
        };
        items.push(item(command.name.clone(),detail,"COMMAND",action));
    }
    let (unit_op,unit_needle)=commands::unit_query(&query);
    if !unit_needle.is_empty() {
        for unit in catalog.units.iter().filter(|unit|unit.name.to_lowercase().contains(&unit_needle)) {
            let running=unit.active=="active";
            let op=unit_op.unwrap_or(if running {UnitOp::Restart} else {UnitOp::Start});
            let scope=if unit.user {"user"} else {"system"};
            let label=format!("{} {}",match op {UnitOp::Start=>"Start",UnitOp::Stop=>"Stop",UnitOp::Restart=>"Restart"},unit.name);
            items.push(item(label,format!("{scope} · {} ({}) · {}",unit.active,unit.sub,unit.description),"UNIT",Action::Unit {name:unit.name.clone(),user:unit.user,op}));
        }
    }
    // Everything above answers to the words typed; what follows answers to the query as a whole.
    items.retain(|item| item.kind=="UNIT" || query.is_empty() || item.label.to_lowercase().contains(&query) || item.detail.to_lowercase().contains(&query));
    if let Some(needle)=clip_query(&query) {
        // Newest first, as the history keeps them; the sort below keeps that order.
        for (order,entry) in clips.iter().filter(|entry|needle.is_empty() || entry.text.to_lowercase().contains(&needle)).enumerate() {
            items.push(Item {label:entry.first_line(80),detail:format!("{} · Enter copies it back",entry.summary()),kind:"CLIP",action:Action::Copy(entry.text.clone()),pin:None,icon:None,clip:Some((entry.id,entry.pinned))}.ordered(order));
        }
    }
    if let Some(calc)=commands::calculate(&query) {
        items.insert(0,item(format!("{} = {}",calc.expression,calc.result),"Calculator · Enter copies the result".into(),"CALC",Action::Copy(calc.result)));
    }
    if commands::process_query(&query).is_some() {
        for proc in &catalog.processes {
            items.push(item(format!("End {} ({})",proc.name,proc.pid),format!("SIGTERM · {}",proc.command_line),"PROCESS",Action::Terminate(proc.pid)));
        }
    }
    items.sort_by_key(|item| {
        let label=item.sort_key();
        let rank=match item.kind {
            "CALC"|"CLIP"=>0,
            "APP" if label.starts_with(&query)=>1,
            "HOST"|"COMMAND" if label==query=>2,
            _ if label.starts_with(&query)=>3,
            "APP"=>4,
            _=>5,
        };
        (rank,label)
    });
    items.truncate(128);
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{Host,Proc,Unit,UserCommand};
    fn entry(name:&str)->AppEntry { AppEntry { name:name.into(),desktop_id:format!("{name}.desktop"),exec:vec!["x".into()],icon:None,startup_wm_class:None,terminal:false } }
    #[test]
    fn prefix_apps_rank_before_substrings_and_queries_are_not_executed() {
        let entries=["Campfire","Firefox","Fire Alarm"].map(entry);
        let items=results("fire",&entries,None,&Catalog::default(),&[]);
        let apps:Vec<_>=items.iter().filter(|i|i.kind=="APP").map(|i|i.label.as_str()).collect();
        assert_eq!(apps,["Fire Alarm","Firefox","Campfire"]);
        assert!(results("$(touch /tmp/no)",&entries,None,&Catalog::default(),&[]).is_empty());
    }
    #[test]
    fn running_application_focuses_existing_window_and_settings_are_searchable() {
        let entry=entry("Editor");
        let state=DesktopState { apps:vec![des_common::ipc::AppInfo{app_id:"editor".into(),title:"Notes".into(),screen:0,workspace:0,place:des_common::ipc::Place::Main,pinned:false,urgent:false}],..Default::default() };
        let items=results("editor",&[entry],Some(&state),&Catalog::default(),&[]);
        assert!(matches!(items[0].action,Action::Command(Command::Activate(0))));
        assert!(results("health",&[],None,&Catalog::default(),&[]).iter().any(|i|matches!(i.action,Action::Settings(_))));
        assert!(results("save session",&[],None,&Catalog::default(),&[]).iter().any(|i|i.action==Action::SaveSession));
    }
    #[test]
    fn calculations_come_first_and_exact_hosts_and_commands_outrank_substrings() {
        let catalog=Catalog {
            hosts:vec![Host {name:"web".into(),address:Some("web.example.org".into())},Host {name:"webcam".into(),address:None}],
            commands:vec![UserCommand {id:"deploy".into(),name:"Deploy".into(),exec:vec!["deploy".into(),"--all".into()],terminal:true,keywords:vec!["ship".into()]}],
            ..Default::default()
        };
        let items=results("2*21",&[entry("Calculator")],None,&catalog,&[]);
        assert_eq!(items[0].kind,"CALC");
        assert_eq!(items[0].action,Action::Copy("42".into()));
        let items=results("web",&[entry("Web Browser")],None,&catalog,&[]);
        let kinds:Vec<_>=items.iter().map(|i|(i.kind,i.label.as_str())).collect();
        assert_eq!(kinds[0],("APP","Web Browser"));
        assert_eq!(kinds[1],("HOST","web"));
        assert_eq!(items[1].action,Action::RunInTerminal(vec!["ssh".into(),"web".into()]));
        assert!(results("ship",&[],None,&catalog,&[]).iter().any(|i|i.kind=="COMMAND" && i.action==Action::RunInTerminal(vec!["deploy".into(),"--all".into()])));
    }
    #[test]
    fn processes_and_units_answer_their_own_queries() {
        let catalog=Catalog {
            processes:vec![Proc {pid:7,uid:1000,name:"node".into(),command_line:"node app".into()}],
            units:vec![Unit {name:"ssh.service".into(),user:false,active:"active".into(),sub:"running".into(),description:"Shell".into()},
                       Unit {name:"sync.service".into(),user:true,active:"failed".into(),sub:"failed".into(),description:"Sync".into()}],
            ..Default::default()
        };
        let items=results("kill node",&[],None,&catalog,&[]);
        assert_eq!(items.len(),1);
        assert_eq!(items[0].action,Action::Terminate(7));
        assert!(results("node",&[],None,&catalog,&[]).iter().all(|i|i.kind!="PROCESS"));
        let items=results("s",&[],None,&catalog,&[]);
        let units:Vec<_>=items.iter().filter(|i|i.kind=="UNIT").map(|i|(i.label.clone(),i.action.clone())).collect();
        assert!(units.contains(&("Restart ssh.service".into(),Action::Unit {name:"ssh.service".into(),user:false,op:UnitOp::Restart})));
        assert!(units.contains(&("Start sync.service".into(),Action::Unit {name:"sync.service".into(),user:true,op:UnitOp::Start})));
        let items=results("stop ssh",&[],None,&catalog,&[]);
        assert_eq!(items[0].action,Action::Unit {name:"ssh.service".into(),user:false,op:UnitOp::Stop});
    }
    #[test]
    fn clipboard_history_lists_newest_first_and_filters() {
        let mut history=clipboard::History::default();
        let text=vec!["text/plain".to_string()];
        history.push("first line\nmore",&text);
        history.push("zebra",&text);
        history.push("apple pie",&text);
        let clips:Vec<_>=history.entries().cloned().collect();
        assert!(results("apple",&[],None,&Catalog::default(),&clips).iter().all(|i|i.kind!="CLIP"));
        let items=results("clip",&[],None,&Catalog::default(),&clips);
        let labels:Vec<_>=items.iter().filter(|i|i.kind=="CLIP").map(|i|i.label.as_str()).collect();
        assert_eq!(labels,["apple pie","zebra","first line"]);
        assert_eq!(items[0].action,Action::Copy("apple pie".into()));
        assert!(items[2].detail.starts_with("2 lines · "));
        let items=results("Clipboard ZEB",&[],None,&Catalog::default(),&clips);
        assert_eq!(items.iter().filter(|i|i.kind=="CLIP").count(),1);
        assert_eq!(clip_query("clipper"),None);
        assert_eq!(clip_query("clip "),Some(String::new()));
        assert!(Action::ClipClear.keeps_palette_open() && !Action::Copy(String::new()).keeps_palette_open());
    }
}
