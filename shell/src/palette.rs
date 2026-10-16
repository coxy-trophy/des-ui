//! Typed palette results; queries are never interpreted as executable commands.
use crate::apps::AppEntry;
use des_common::ipc::{Command, DesktopState};

#[derive(Debug,Clone,PartialEq)]
pub enum Action {
    Launch(AppEntry), Command(Command), Settings(&'static str), File(std::path::PathBuf),
    Music, Notifications, Power, Lock, SaveSession, RestoreSession, FindTerminal,
}
#[derive(Clone)]
pub struct Item { pub label:String, pub detail:String, pub kind:&'static str, pub action:Action, pub pin:Option<String>, pub icon:Option<String> }

pub fn results(query:&str, entries:&[AppEntry], state:Option<&DesktopState>) -> Vec<Item> {
    let query=query.trim().to_lowercase();
    let mut items=Vec::new();
    for entry in entries {
        let running=state.and_then(|state|state.apps.iter().position(|app| crate::dock::entry_for(&app.app_id,entries).is_some_and(|i|entries[i].desktop_id==entry.desktop_id)));
        items.push(Item { label:entry.name.clone(),detail:if running.is_some(){"Focus running application".into()}else{entry.exec.first().cloned().unwrap_or_default()},kind:"APP",
            action:running.map_or_else(||Action::Launch(entry.clone()),|index|Action::Command(Command::Activate(index))),pin:Some(entry.desktop_id.clone()),icon:entry.icon.clone() });
    }
    if let Some(state)=state {
        for (index,app) in state.apps.iter().enumerate() {
            items.push(Item { label:app.title.clone(), detail:format!("{} · display {} · workspace {}",app.app_id,app.screen+1,app.workspace+1),kind:"WINDOW",action:Action::Command(Command::Activate(index)),pin:None,icon:None });
        }
    }
    for (id,title) in crate::settings::destinations() {
        items.push(Item { label:format!("Settings: {title}"),detail:"Open settings section".into(),kind:"SETTING",action:Action::Settings(id),pin:None,icon:None });
    }
    let mut commands=vec![
        ("Show terminal",Action::Command(Command::Terminal)),
        ("Find in terminal",Action::FindTerminal),
        ("Split workspace",Action::Command(Command::Split)),
        ("Toggle fullscreen",Action::Command(Command::Fullscreen)),
        ("Take screenshot",Action::Command(Command::Shot(des_common::ipc::ShotMode::Pick))),
        ("Lock screen",Action::Lock),("Music player",Action::Music),("Notifications",Action::Notifications),("Power menu",Action::Power),
        ("Save session",Action::SaveSession),("Restore session",Action::RestoreSession),
    ];
    for (label,action) in commands.drain(..) { items.push(Item {label:label.into(),detail:"Desktop action".into(),kind:"ACTION",action,pin:None,icon:None}); }
    for workspace in 0..des_common::ipc::WORKSPACES {
        items.push(Item {label:format!("Switch to workspace {}",workspace+1),detail:"Desktop action".into(),kind:"ACTION",action:Action::Command(Command::Workspace(workspace)),pin:None,icon:None});
    }
    items.retain(|item|query.is_empty() || item.label.to_lowercase().contains(&query) || item.detail.to_lowercase().contains(&query));
    items.sort_by_key(|item| {
        let label=item.label.to_lowercase();
        let rank=if item.kind=="APP" && label.starts_with(&query){0}else if label.starts_with(&query){1}else if item.kind=="APP"{2}else{3};
        (rank,label)
    });
    items.truncate(128);
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prefix_apps_rank_before_substrings_and_queries_are_not_executed() {
        let entries=["Campfire","Firefox","Fire Alarm"].map(|name|AppEntry { name:name.into(),desktop_id:format!("{name}.desktop"),exec:vec!["x".into()],icon:None,startup_wm_class:None,terminal:false });
        let items=results("fire",&entries,None);
        let apps:Vec<_>=items.iter().filter(|i|i.kind=="APP").map(|i|i.label.as_str()).collect();
        assert_eq!(apps,["Fire Alarm","Firefox","Campfire"]);
        assert!(results("$(touch /tmp/no)",&entries,None).is_empty());
    }
    #[test]
    fn running_application_focuses_existing_window_and_settings_are_searchable() {
        let entry=AppEntry { name:"Editor".into(),desktop_id:"editor.desktop".into(),exec:vec!["editor".into()],icon:None,startup_wm_class:None,terminal:false };
        let state=DesktopState { apps:vec![des_common::ipc::AppInfo{app_id:"editor".into(),title:"Notes".into(),screen:0,workspace:0,place:des_common::ipc::Place::Main}],..Default::default() };
        let items=results("editor",&[entry],Some(&state));
        assert!(matches!(items[0].action,Action::Command(Command::Activate(0))));
        assert!(results("health",&[],None).iter().any(|i|matches!(i.action,Action::Settings(_))));
        assert!(results("save session",&[],None).iter().any(|i|i.action==Action::SaveSession));
    }
}
