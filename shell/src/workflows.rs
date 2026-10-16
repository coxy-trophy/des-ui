//! Palette dispatch and explicit saved-session workflows.
use super::*;
impl App {
    pub(super) fn palette_action(&mut self,action:palette::Action){
        match action {
            palette::Action::Launch(entry)=>self.apply_dock_action(dock::Action::Launch(entry)),
            palette::Action::Command(command)=>self.apply_dock_action(dock::Action::Command(command)),
            palette::Action::Settings(section)=>{
                self.launcher.open=false;self.music.open=false;self.notices.open=false;self.power_menu.open=false;
                self.settings.open_at(&self.config,section);self.send(Command::Terminal);
            }
            palette::Action::File(path)=>{
                if self.document_openers.len()<32{self.document_openers.push(apps::open_document(path,self.ctx.clone()));}
                else{self.launcher.fail("Wait for existing document viewers to respond before opening another file.".into());}
            }
            palette::Action::Music=>self.toggle_music(),
            palette::Action::Notifications=>self.toggle_notices(),
            palette::Action::Power=>self.toggle_power(),
            palette::Action::Lock=>self.lock_now(),
            palette::Action::FindTerminal=>{self.launcher.open=false;self.send(Command::Terminal);let i=self.focus_tab();self.tabs[i].open_search();}
            palette::Action::CopyLastOutput=>{self.launcher.open=false;self.copy_last_output();}
            palette::Action::SaveSession=>self.save_session(),
            palette::Action::RestoreSession=>self.restore_session(),
            palette::Action::RunInTerminal(argv)=>self.run_argv(&argv),
            // The palette already put it on the clipboard; this just says so.
            palette::Action::Copy(text)=>self.notices.post("Copied",&text,false),
            palette::Action::Terminate(pid)=>match commands::terminate(pid){
                Ok(())=>self.notices.post("Process asked to end",&format!("SIGTERM sent to process {pid}."),false),
                Err(e)=>self.notices.post("Process not ended",&e,true),
            },
            palette::Action::Unit{name,user,op}=>{
                let outcomes=self.launcher.outcomes();let ctx=self.ctx.clone();
                std::thread::spawn(move ||{
                    let outcome=match commands::act_on_unit(&name,user,op){
                        Ok(())=>("Unit changed".to_string(),format!("{name}: {} done.",op.word()),false),
                        Err(e)=>(format!("Could not {} {name}",op.word()),e,true),
                    };
                    outcomes.lock().unwrap_or_else(|e|e.into_inner()).push(outcome);ctx.request_repaint();
                });
            }
        }
    }
    /// Opens a terminal tab running a program with its arguments, no shell between;
    /// the tab closes with the program. With no room for a tab, only a notice.
    pub(super) fn run_argv(&mut self,argv:&[String]){
        self.launcher.open=false;
        let program=argv.first().map_or("",String::as_str);
        if self.tabs.len()>=MAX_TABS{self.notices.post("No room for another tab",&format!("Close a terminal tab to run {program}."),true);return;}
        let cwd=self.tabs.get(self.focus_tab()).and_then(|t|t.cwd()).unwrap_or_else(||PathBuf::from("/"));
        match TermTab::spawn_command(argv,&cwd,self.ctx.clone(),self.config.scrollback){
            Ok(tab)=>{self.tabs.push(tab);self.show_tab(self.tabs.len()-1);self.send(Command::Terminal);}
            Err(e)=>self.notices.post("Could not run program",&format!("{program}: {e}"),true),
        }
    }
    pub(super) fn pump_documents(&mut self){
        let mut errors=Vec::new();
        self.document_openers.retain(|rx|match rx.try_recv(){
            Ok(Ok(()))=>false,Ok(Err(error))=>{errors.push(error);false},
            Err(std::sync::mpsc::TryRecvError::Disconnected)=>{errors.push("Document opener stopped unexpectedly.".into());false},
            Err(std::sync::mpsc::TryRecvError::Empty)=>true,
        });
        for error in errors{self.notices.post("Could not open document",&error,true);}
        for (title,body,urgent) in self.launcher.take_outcomes(){self.notices.post(&title,&body,urgent);}
    }
    pub(super) fn save_session(&mut self){
        let home=std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(||PathBuf::from("/"));
        let terminals=session_restore::TerminalLayout {directories:self.tabs.iter().map(|t|t.cwd().unwrap_or_else(||home.clone())).collect(),active:self.active,side:self.side,stacked:self.stacked};
        let state=self.desktop.as_ref().map(|d|d.state.clone()).unwrap_or_default();
        let session=session_restore::Session::capture(&state,&apps::scan(),terminals);
        match session_restore::save(&session){
            Ok(())=>self.notices.post("Session saved","Application placement and terminal folders are ready to restore.",false),
            Err(e)=>self.notices.post("Session not saved",&format!("{e}. Session storage requires a private, user-owned state directory."),true),
        }
    }
    pub(super) fn restore_session(&mut self){
        if self.restore.is_some(){self.notices.post("Restore in progress","Waiting for the applications already requested.",false);return;}
        let session=match session_restore::load(){Ok(s)=>s,Err(e)=>{self.notices.post("Cannot restore session",&format!("Save a session first, or check the saved session file: {e}"),true);return;}};
        let state=self.desktop.as_ref().map(|d|d.state.clone()).unwrap_or_default();
        if !session.apps.is_empty() && !self.desktop.as_ref().is_some_and(|d|d.connected()){
            self.notices.post("Desktop unavailable","Connect to the compositor before restoring application placement.",true);return;
        }
        let entries=apps::scan();
        let mut restore=session_restore::Restore::new(&session,&entries,&state,Instant::now());
        let mut used=std::collections::HashSet::new();let mut mapping=Vec::new();let mut unavailable=0;
        for cwd in session.terminals.directories.iter().take(MAX_TABS){
            if let Some(i)=self.tabs.iter().enumerate().find_map(|(i,t)|(!used.contains(&i)&&t.cwd().as_ref()==Some(cwd)).then_some(i)){used.insert(i);mapping.push(i);continue;}
            if self.tabs.len()>=MAX_TABS || !cwd.is_dir(){unavailable+=1;mapping.push(self.active);continue;}
            match TermTab::spawn(&self.shell,cwd,self.ctx.clone(),self.config.scrollback){Ok(tab)=>{let i=self.tabs.len();self.tabs.push(tab);used.insert(i);mapping.push(i);},Err(_)=>{unavailable+=1;mapping.push(self.active);}}
        }
        if let Some(&i)=mapping.get(session.terminals.active){self.active=i;}
        self.side=session.terminals.side.and_then(|i|mapping.get(i).copied()).filter(|i|*i!=self.active);
        self.stacked=session.terminals.stacked;self.side_focused=false;
        for place in std::mem::take(&mut restore.placements){
            if let Some(app)=state.apps.get(place.index){self.restore_placements.push_back((place,app.app_id.clone()));}
        }
        for i in std::mem::take(&mut restore.launch){if let Err(e)=apps::launch(&entries[i]){self.notices.post("Application not restored",&format!("{}: {e}",entries[i].name),true);}}
        if !restore.missing.is_empty(){self.notices.post("Applications unavailable",&restore.missing.join(", "),false);}
        if unavailable>0{self.notices.post("Some terminal folders unavailable","Existing terminals were kept; missing folders or the tab limit prevented some restored tabs.",false);}
        self.restore_workspace=self.desktop.as_ref().is_some_and(|d|d.connected()).then_some(session.workspace);
        if !restore.pending() && self.restore_placements.is_empty() && self.restore_workspace.is_none(){self.notices.post("Session restored","Terminal folders have been restored.",false);}
        else{self.restore=Some((restore,entries,Instant::now()));}
    }
    pub(super) fn pump_restore(&mut self){
        if self.lock.locked{return;}
        let Some(state)=self.desktop.as_ref().filter(|d|d.connected()).map(|d|d.state.clone())else{
            if self.restore.as_ref().is_some_and(|(_,_,started)|started.elapsed()>=Duration::from_secs(30)){
                self.restore=None;self.restore_placements.clear();self.restore_workspace=None;self.notices.post("Restore stopped","The compositor did not reconnect within 30 seconds. Try restoring again when connected.",false);
            }
            return;
        };
        let Some((mut restore,entries,started))=self.restore.take()else{return;};
        if started.elapsed()>=Duration::from_secs(30){
            self.restore_placements.clear();self.restore_workspace=None;
            self.notices.post("Restore stopped","Some windows or placement commands could not be completed within 30 seconds. Existing applications and terminals were kept.",false);
            return;
        }
        for place in restore.poll(&state,&entries,Instant::now()){
            if let Some(app)=state.apps.get(place.index){self.restore_placements.push_back((place,app.app_id.clone()));}
        }
        for _ in 0..16 {
            let Some((place,identity))=self.restore_placements.front()else{break;};
            if state.apps.get(place.index).is_none_or(|app| &app.app_id!=identity){self.restore_placements.pop_front();continue;}
            let command=Command::PlaceApp(place.index,place.screen,place.workspace,place.place);
            if self.desktop.as_mut().is_some_and(|d|d.send(command).is_ok()){self.restore_placements.pop_front();}else{break;}
        }
        if !restore.pending() && self.restore_placements.is_empty(){
            if let Some(workspace)=self.restore_workspace {
                if state.workspace==workspace{self.restore_workspace=None;}
                else if let Some(desktop)=&mut self.desktop{let _=desktop.send(Command::Workspace(workspace));}
            }
        }
        if restore.pending() || !self.restore_placements.is_empty() || self.restore_workspace.is_some(){self.restore=Some((restore,entries,started));}
        else if started.elapsed()>=Duration::from_secs(30){self.notices.post("Restore finished with unavailable windows","Some applications did not create a matching window within 30 seconds.",false);}
        else{self.notices.post("Session restored","Applications have been placed in their saved workspaces and displays.",false);}
    }
}
