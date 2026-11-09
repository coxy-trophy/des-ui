//! Per-frame desktop coordination and rendering.

use super::*;

pub(crate) fn frame_lines_layer() -> egui::LayerId {
    egui::LayerId::new(egui::Order::Middle, egui::Id::new("frame-lines"))
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &Visuals) -> [f32; 4] {
        ui::GROUND.to_normalized_gamma_f32()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Before anything reads input: what the on-screen keys pressed last frame.
        self.osk.inject(ctx);
        let connect_visible=self.right_connect && self.desktop.as_ref().map_or(self.local_panels.right, |d| d.state.panels.right)
            && !self.lock.locked && !self.lock_confirmed && !self.unlock_pending && !self.capture.marking();
        self.connect.tick(ctx, connect_visible);
        if self.files_browser.is_some() && !connect_visible { self.connect.keep_fresh(ctx); }
        self.mon.set_processor_temperature_warnings(self.config.processor_temperature_warnings);
        self.mon.tick();
        // Volume and brightness changed from outside the shell's sliders get a meter.
        let quiet = self.settings.open || self.quick.open;
        let now = Instant::now();
        self.osd.observe(osd::Kind::Volume, self.mon.volume(), quiet, now);
        self.osd.observe(osd::Kind::Brightness, self.mon.brightness, quiet, now);
        self.widgets.pump();
        for message in self.settings.poll_devices(ctx) {
            self.notices.post("Devices", &message, false);
        }
        // While the terminal in use is in an ssh session, the panels show that machine.
        let focused = self.focus_tab();
        self.remote.follow_tab(self.tabs.get(focused), ctx);
        let view = self.remote.view();
        let following = self.remote.target().map(|target| sysmon::Following {
            name: target.name(),
            live: view.is_some(),
            word: self.remote.state().map_or("", remote::State::word),
        }).or_else(|| (self.remote.state() == Some(remote::State::Unsupported)).then(|| sysmon::Following {
            name: "SSH".into(), live: false, word: remote::State::Unsupported.word(),
        }));
        self.geo.set_remote_connections(view.as_ref().map(|view| view.connections.clone()));
        self.mon.set_remote(following, view.as_ref());
        // Threshold crossings the sampler noticed: heat, a full disk, memory, a failed unit.
        for alert in self.mon.take_alerts() {
            self.notices.post(&alert.title, &alert.body, alert.urgent);
        }
        self.pump_documents();
        self.pump_file_operations(ctx);
        if let Some(desktop) = &mut self.desktop {
            desktop.pump();
        }
        self.answer_control();
        self.clipboard.poll(self.launcher.open);
        self.companion.pump(ctx);
        if std::mem::take(&mut self.companion.panel_requested) {self.toggle_companion(ctx);}
        let mut companion_requested = false;
        let mut companion_voice_requested = false;
        let mut companion_controls = Vec::new();
        let mut shortcut_requested = false;
        let mut apps_requested = None;
        if let Some(desktop) = &mut self.desktop {
            if std::mem::take(&mut desktop.companion_blur_requested) {self.companion.blur_text();}
            for screen in std::mem::take(&mut desktop.dock_dismissed) {
                if screen==0 {self.dock.dismiss_keyboard(ctx);}
                else if let Some(dock)=self.secondary_docks.get_mut(&screen){dock.dismiss_keyboard(ctx);}
            }
            companion_controls=std::mem::take(&mut desktop.companion_controls);
            companion_voice_requested=std::mem::take(&mut desktop.companion_voice_requested);
            companion_requested = std::mem::take(&mut desktop.companion_requested);
            if !desktop.connected() || desktop.locked || desktop.lock_requested || desktop.idle_lock_requested || desktop.shots.iter().any(|shot| !companion::Companion::accepts_capture(shot.request)) {self.companion.close();}
            else if desktop.launcher_requested || desktop.settings_requested || desktop.music_requested || desktop.notifications_requested || desktop.quick_requested || desktop.power_requested {self.companion.blur_text();}
            shortcut_requested = std::mem::take(&mut desktop.shortcuts_requested);
            if desktop.launcher_requested || desktop.settings_requested || desktop.music_requested || desktop.notifications_requested || desktop.quick_requested || desktop.power_requested { self.shortcut_reference.close(); }
            if !desktop.connected() { self.dock_area = None; self.overlay_area = None; self.secondary_areas.clear();self.dock_keyboard.clear(); }
            if let Some(screen)=desktop.dock_screen_requested.take(){
                if screen<desktop.state.screens {apps_requested=Some(screen); }
            }
            if std::mem::take(&mut desktop.dock_requested) { apps_requested=Some(0); }
            if std::mem::take(&mut desktop.launcher_requested) && !self.launcher.open {
                self.world.expanded = false;
                self.connect.expanded = false;
                self.settings.open = false; self.files_browser = None;
                self.music.open = false;
                self.notices.open = false;
                self.power_menu.open = false;
                self.launcher.open();
            }
            if std::mem::take(&mut desktop.settings_requested) && !self.settings.open {
                self.launcher.open = false;
                self.music.open = false;
                self.notices.open = false;
                self.power_menu.open = false;
                self.settings.open(&self.config);
            }
            if std::mem::take(&mut desktop.music_requested) && !self.music.open {
                self.launcher.open = false;
                self.settings.open = false; self.files_browser = None;
                self.notices.open = false;
                self.power_menu.open = false;
                self.music.open = true;
            }
            if std::mem::take(&mut desktop.power_requested) {
                self.launcher.open = false;
                self.settings.open = false; self.files_browser = None;
                self.music.open = false;
                self.notices.open = false;
                self.power_menu.open = true;
            }
            if std::mem::take(&mut desktop.quick_requested) {
                self.quick.toggle();
            }
            if std::mem::take(&mut desktop.notifications_requested) && !self.notices.open {
                self.launcher.open = false;
                self.settings.open = false; self.files_browser = None;
                self.music.open = false;
                self.notices.open = true;
            }
            // Bring the frame in line with the saved choice of panels: once at the
            // start, and again whenever that choice is changed in the settings.
            if !desktop.connected() { self.frame_synced = false; self.shortcuts_synced = false; self.keyboard_reserved = None; }
            let height = if self.lock.locked { 0 } else { self.osk.height_for(ctx.screen_rect().height()).round() as u16 };
            if desktop.connected() && self.keyboard_reserved != Some(height) && desktop.send(Command::KeyboardHeight(height)).is_ok() {
                self.keyboard_reserved = Some(height);
            }
            if !self.shortcuts_synced && desktop.connected() {
                self.shortcuts_synced = desktop.send(Command::ReloadInput).is_ok();
            }
            if !self.frame_synced && desktop.connected() {
                self.frame_synced = desktop.send(Command::ReloadFrame).is_ok();
            }
            if !self.panels_synced && desktop.connected() {
                let (now, wanted) = (desktop.state.panels, self.config.panels);
                for (panel, differs) in [
                    (des_common::Panel::Left, now.left != wanted.left),
                    (des_common::Panel::Right, now.right != wanted.right),
                    (des_common::Panel::Bottom, now.bottom != wanted.bottom),
                ] {
                    if differs {
                        if let Err(error) = desktop.send(Command::TogglePanel(panel)) {
                            eprintln!("des-ui: panel state could not be sent: {error}");
                        }
                    }
                }
                self.panels_synced = true;
            }
        }
        if !self.lock.locked && !self.polkit.active() && !self.capture.marking() {
            for action in companion_controls { self.companion.control_from_desktop(ctx, action); }
        }
        if companion_voice_requested {self.summon_voice(ctx);}
        if companion_requested { self.toggle_companion(ctx); }
        if let Some(screen)=apps_requested {self.focus_apps(screen);}
        if shortcut_requested && !self.polkit.active() { self.toggle_shortcut_reference(); }
        for tab in &mut self.tabs {
            tab.set_completions(self.config.terminal_completions);
            tab.pump();
        }
        self.scratchpad.set_completions(self.config.terminal_completions);
        self.scratchpad.pump();
        // Tabs whose shell has exited go; the ones in view keep their places, and a
        // split whose pane lost its tab becomes whole again.
        let alive: Vec<bool> = self.tabs.iter().map(|tab| !tab.exited).collect();
        let (active, side) = remap_panes(&alive, self.active, self.side);
        self.tabs.retain(|tab| !tab.exited);
        if self.tabs.is_empty() {
            self.checkpoint_live_terminals();
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }
        self.active = active;
        self.side = side;
        self.side_focused &= side.is_some();

        self.handle_screenshot(ctx);

        // The lock screen comes before everything else, and while it is up nothing
        // else is drawn or given any input.
        let (asked, idle, compositor_locked, security_confirmed, saver_approved) = match &mut self.desktop {
            Some(desktop) => {
                let asked = std::mem::take(&mut desktop.lock_requested);
                let idle = std::mem::take(&mut desktop.idle_lock_requested);
                if !desktop.connected() { self.lock_confirmed = false; }
                (asked, idle, desktop.connected().then_some(desktop.locked),
                    desktop.security_confirmed.take(), desktop.saver_active)
            }
            None => (false, false, None, None, false),
        };
        if compositor_locked.is_none() {
            self.idle_saver.cancel();
        }
        let system_lock = self.power.lock_wanted();
        let sleep_waiting = self.power.sleep_waiting();
        if asked || system_lock || sleep_waiting {
            self.cancel_idle_saver();
        }
        if !self.lock.locked {
            if asked || (system_lock && (!sleep_waiting || self.config.lock_on_sleep)) {
                self.lock_now();
            } else if idle && !sleep_waiting && !self.lock.preparing() {
                self.close_saver_preview();
                self.companion.close();
                if self.config.screensaver != config::Saver::Off {
                    self.idle_saver.begin();
                }
                self.lock.prepare(ctx);
            }
        }
        if let Some(prepared) = self.lock.take_prepared() {
            match prepared {
                Ok(()) => self.finish_lock(),
                Err(why) => {
                    self.cancel_idle_saver();
                    self.notices.post("Screen could not be locked", &why, true);
                }
            }
        }
        match reconcile_lock(self.lock.locked, self.lock_confirmed, self.unlock_pending, security_confirmed == Some(false), compositor_locked) {
            LockStep::Engage => self.lock.engage(),
            LockStep::Confirm => self.lock_confirmed = true,
            LockStep::Release => {
                self.lock.release();
                self.unlock_pending = false;
                self.lock_confirmed = false;
                self.idle_saver.cancel();
            }
            LockStep::Request => self.send(Command::Lock),
            LockStep::Wait => {},
        }
        let saver_can_show = self.lock.locked && self.lock_confirmed && !self.unlock_pending;
        match self.idle_saver.update(saver_can_show, saver_approved, now) {
            saver_flow::Action::Show => self.send(Command::ShowSaver),
            saver_flow::Action::Dismiss => self.send(Command::HideSaver),
            saver_flow::Action::None => {}
        }
        self.notices.pump();
        // What starts within a quarter of an hour is announced, once.
        self.agenda.pump(&mut self.notices);
        if let Some(desktop) = &mut self.desktop {
            for shot in std::mem::take(&mut desktop.shots) {
                if companion::Companion::accepts_capture(shot.request) { self.companion.receive_capture(ctx, shot); }
                else { self.capture.take(shot); }
            }
            for (request, reason) in std::mem::take(&mut desktop.shot_failures) {
                if companion::Companion::accepts_capture(request) { self.companion.capture_failed(request.unwrap(), &reason); }
                else { self.capture.failed(request, reason); }
            }
            for state in std::mem::take(&mut desktop.recording_news) {
                self.capture.recording_is(state, &mut self.notices);
            }
        }
        let saved = self.capture.report(&mut self.notices);
        let called_off = self.capture.take_called_off();
        self.answer_held(Some(saved), called_off);
        if sleep_waiting {
            // The machine may sleep once the lock is what is on screen, or at once if
            // there is to be no lock.
            let up = self.lock.locked && (self.lock_confirmed || self.desktop.is_none());
            if up || (!self.lock.locked && !self.lock.preparing()) {
                self.power.let_sleep();
            } else {
                ctx.request_repaint_after(Duration::from_millis(30));
            }
        }
        let held = self.power.holding();
        if held != self.idle_held {
            self.idle_held = held;
            self.send(Command::HoldIdle(held));
        }
        let screens = self.desktop.as_ref().filter(|desktop| desktop.connected()).map(|desktop| desktop.state.screens);
        if self.watch.check(&mut self.notices, self.mon.power, !self.mon.ip.is_empty(), screens) {
            power::Act::Suspend.run();
        }
        if self.lock.locked {
            self.close_saver_preview();
            self.companion.close();
            self.shortcut_reference.close();
            self.settings.appearance_editor.sync_visibility(false);
            let state=self.desktop.as_ref().map(|d|d.state.clone());
            self.secondary_views(ctx,state.as_ref(),true);
            self.dock.reset();
            self.connect.suspend_preview();
            if self.dock_area != Some(None) {
                self.dock_area = Some(None);
                self.send(Command::Dock(None));
            }
            self.capture.cancel();
            self.agenda.open = false;
            self.scratchpad.close_now();
            self.quick.open = false;
            if self.overlay_area != Some(None) {
                self.overlay_area = Some(None);
                self.send(Command::Overlay(None));
            }
            // Nothing is shown over the lock; what arrives waits in the list.
            if self.toast_area != Some(None) {
                self.toast_area = Some(None);
                self.send(Command::Toast(None));
            }
            let theme = self.theme.clone();
            if let Some(started) = self.idle_saver.started() {
                self.draw_saver(ctx, started);
            } else {
                let glance = crate::lock::Glance { battery: self.mon.power.map(|power| (power.percent, power.plugged)), online: !self.mon.ip.is_empty(), notices: self.notices.count() };
                let music = &mut self.music;
                if self.lock.show(ctx, &theme, glance, |ui, within| music.lock_card(ui, &theme, within)) {
                    if self.desktop.is_some() {
                        self.unlock_pending = true;
                        self.lock.await_unlock();
                    }
                    self.send(Command::Unlock);
                }
            }
            self.powerup.publish(ctx, self.config.animate, true);
            // A password request waits; nothing is drawn over the lock.
            self.polkit.show(ctx, &theme, true);
            self.checkpoint_live_terminals();
            if self.desktop.as_ref().is_some_and(|desktop| desktop.connected()) { self.readiness.notify(); }
            return;
        }
        if let Some(started) = self.saver_preview {
            if !self.settings.open || self.polkit.active() || self.capture.marking()
                || self.desktop.as_ref().is_some_and(|desktop| desktop.overview.is_some())
            {
                self.close_saver_preview();
            } else {
                let exit = ctx.input(|input| !input.focused || input.events.iter().any(|event| matches!(
                    event,
                    Event::Key { pressed: true, repeat: false, .. }
                    | Event::PointerButton { pressed: true, .. }
                    | Event::Touch { phase: egui::TouchPhase::Start, .. }
                    | Event::MouseWheel { .. }
                )));
                let state = self.desktop.as_ref().map(|desktop| desktop.state.clone());
                self.secondary_views(ctx, state.as_ref(), false);
                self.draw_saver(ctx, started);
                if exit {
                    self.close_saver_preview();
                    // End this frame here; the dismissal is not a Settings action.
                    ctx.input_mut(|input| input.events.clear());
                    ctx.request_repaint();
                } else {
                    let area = Some(scratchpad::area(ctx.screen_rect()));
                    if scratchpad::changed(&mut self.overlay_area, area) {
                        self.send(Command::Overlay(area));
                    }
                    ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("saver-preview-hint"))).text(
                        ctx.screen_rect().center_bottom() - vec2(0.0, 24.0),
                        egui::Align2::CENTER_BOTTOM, "Click or press a key to exit preview",
                        ui::display(14.0), ui::TEXT,
                    );
                }
                return;
            }
        }
        if let Some(error) = self.wallpaper.update(ctx, &self.config.appearance) {
            self.notices.post("Wallpaper unavailable", &format!("{error}. Choose another image in Settings → Appearance."), false);
        }
        self.wallpaper.publish(ctx, &self.config.appearance);
        shortcuts::publish(ctx, &self.config.shortcuts);
        let animate = self.config.animate && !self.config.appearance.reduce_motion;
        let stage = self.powerup.publish(ctx, animate, false);
        self.pump_restore();
        self.settings.appearance_editor.sync_visibility(self.settings.open && !self.polkit.active());
        if let Some(path) = files::take_request(ctx) { self.open_files(path); }
        if self.polkit.active() { self.shortcut_reference.close(); }
        if self.polkit.active() || self.capture.marking() || self.lock.locked { self.companion.close(); }
        let events = self.take_shortcuts(ctx);
        let mut reference_input = shortcuts::take_modal_input(ctx, self.shortcut_reference.open || self.companion.open);
        if self.launcher.open || self.settings.open || self.music.open || self.notices.open || self.power_menu.open {
            self.world.expanded = false;
            self.connect.expanded = false;
            self.files_browser = None;
        }
        let theme = self.theme.clone();
        let screen = ctx.screen_rect();
        let state = self.desktop.as_ref().map(|desktop| desktop.state.clone());
        // The compositor decides which panels are open; alone, the shell does.
        let open = state.as_ref().map_or(self.local_panels, |state| state.panels);
        let places = self.geo.snapshot();
        let rotating = self.config.globe_rotation && animate;
        self.world.tick(self.started.elapsed().as_secs_f64(), rotating, places.home.as_ref());
        let overlay = if self.launcher.open {
            Some("LAUNCHER")
        } else if self.settings.open {
            Some("SETTINGS")
        } else if self.music.open {
            Some("MUSIC")
        } else if self.notices.open {
            Some("NOTIFICATIONS")
        } else if self.power_menu.open {
            Some("POWER")
        } else if self.files_browser.is_some() {
            Some("FILES")
        } else if self.connect.expanded {
            Some("DEVICE MIRROR")
        } else if self.world.expanded {
            Some("WORLD VIEW")
        } else {
            None
        };
        let mut command = None;

        // The compositor places native windows in this same inset workspace.
        let osk_height = self.osk.height_for(screen.height()).round();
        let frame_layout = des_common::frame::Layout::with_keyboard(screen.width(), screen.height(), open, self.config.side_panel_behavior, osk_height);
        let on_screen = |rect: des_common::Rect| Rect::from_min_size(screen.min+vec2(rect.x,rect.y),vec2(rect.width,rect.height));
        let workspace = on_screen(frame_layout.workspace);
        let center = on_screen(frame_layout.center);
        let status_top = screen.bottom() - des_common::STATUS_HEIGHT;
        let keys_top = frame_layout.keyboard.map(|rect| on_screen(rect).top()).unwrap_or(status_top);
        let mut apps_output = dock::Output::default();
        let mut left_rail_output=None;
        // What the right panel's quick settings asked for, done once the frame is drawn.
        let mut panel_actions: Vec<quick::Action> = Vec::new();
        let mut open_network = false;
        ctx.data_mut(|data| data.insert_temp(egui::Id::new("window-shelf-capable"), self.desktop.as_ref().is_some_and(|d| d.connected() && d.window_shelf)));
        // While the scratchpad is down it has the keyboard, not the panes.
        let mut scratch_keys = self.companion.open || self.shortcut_reference.open || self.scratchpad.visible() || self.polkit.active() || self.fs.has_keyboard() || self.connect.has_keyboard() || self.quick.open;
        let mut chat_area=None;
        CentralPanel::default()
            .frame(Frame::new().fill(ui::GROUND))
            .show(ctx, |ui| {
                material::paint(ctx, ui.painter(), screen, 0.0, material::Role::Background);
                material::paint(ctx, ui.painter(), center, des_common::frame::RADIUS, material::Role::Panel);
                let status = Rect::from_min_max(pos2(screen.left(), status_top), screen.max);
                if let Some(rect) = frame_layout.keyboard {
                    self.osk.show(ui, &theme, on_screen(rect));
                }
                // What plays, and whether anything else on screen shows the time.
                let playing = self.music.now().track.map(|track| (track.name, track.artists));
                let clock = !open.left && self.config.side_panel_behavior != des_common::PanelBehavior::Collapse;
                let clicks = region(ui, status, vec2(24.0, 0.0), Layout::left_to_right(Align::Center), |ui| {
                    panels::status(ui, &theme, &self.mon, state.as_ref(), open, if self.dock.captures_keyboard() { Some("APPS") } else { overlay }, self.notices.count(), self.capture.recording.map(|since| since.elapsed()), self.desktop.as_ref().map_or(des_common::ipc::Hands::Off, |desktop| desktop.hands), Some(&self.tray), self.osk.open, self.quick.open, playing.as_ref().map(|(track, artists)| (track.as_str(), artists.as_str())), clock)
                });
                let (toggled, settings_clicked, music_clicked, notices_clicked, power_clicked, stop_clicked) =
                    (clicks.toggled, clicks.settings, clicks.music, clicks.notices, clicks.power, clicks.stop);
                if clicks.keyboard {
                    self.toggle_osk();
                }
                if clicks.quick {
                    self.quick.toggle();
                }
                if stop_clicked {
                    self.toggle_recording();
                }
                if clicks.hands {
                    command = Some(Command::ToggleHands);
                }
                if power_clicked {
                    self.toggle_power();
                }
                if notices_clicked {
                    self.toggle_notices();
                }
                if settings_clicked {
                    self.toggle_settings();
                }
                if music_clicked {
                    self.toggle_music();
                }
                if let Some(panel) = toggled {
                    match &self.desktop {
                        Some(_) => command = Some(Command::TogglePanel(panel)),
                        None => self.local_panels.toggle(panel),
                    }
                }

                let column = vec2(24.0, 24.0);
                if open.left {
                    let next_event = self.agenda.next_today();
                    let rect = on_screen(frame_layout.left.expect("open left panel"));
                    frame_region(ui, rect, column, Layout::top_down(Align::Min), |ui| {
                        panels::left(ui, &theme, &self.mon, &mut self.pet, &self.widgets, next_event.as_ref())
                    });
                }
                if open.right {
                    let rect = on_screen(frame_layout.right.expect("open right panel"));
                    frame_region(ui, rect, column, Layout::top_down(Align::Min), |ui| {
                        ui.horizontal_wrapped(|ui| {
                            match panels::tabs(ui, &theme, &["SYSTEM", "CONNECT"], usize::from(self.right_connect)) {
                                Some(0) if self.right_connect => {self.right_connect=false;self.connect.expanded=false;self.connect.suspend_preview();self.dock.dismiss_keyboard(ctx);}
                                Some(1) if !self.right_connect => {self.right_connect=true;self.dock.dismiss_keyboard(ctx);}
                                _ => {}
                            }
                            if !self.right_connect && self.connect.sharing() && crate::ui::button(ui,&theme,"Stop",None,false).clicked() { self.connect.stop_sharing(ctx); }
                        });
                        ui.add_space(12.0);
                        if self.right_connect && !self.capture.marking() {
                            let expanded=self.connect.expanded;
                            self.connect.panel(ui, &theme);
                            scratch_keys |= self.connect.has_keyboard();
                            if !expanded && self.connect.expanded {
                                self.files_browser=None;self.settings.open=false;self.launcher.open=false;
                                self.music.open=false;self.notices.open=false;self.power_menu.open=false;self.world.expanded=false;
                                self.send(Command::Terminal);
                            }
                            return false;
                        }
                        let music_height = self.music.preferred_panel_height();
                        let music = &mut self.music;
                        let world_was_open = self.world.expanded;
                        let mut wanted = false;
                        let readings = quick::Readings {
                            volume: self.mon.volume(),
                            brightness: self.mon.brightness,
                            night_light: self.config.night_light,
                            do_not_disturb: self.notices.quiet,
                            keyboard: self.osk.open,
                            power: self.mon.power,
                            battery_health: self.mon.battery_health,
                        };
                        let quick_card = &mut self.quick;
                        let mut asked = Vec::new();
                        let choose_network = panels::right(ui, &theme, &self.mon, state.as_ref(), &mut self.world, rotating, &places, &self.widgets, music_height,
                            |ui, room| { asked = quick_card.panel_card(ui, &theme, &readings, room); },
                            |ui| { wanted = music.panel(ui, &theme); });
                        panel_actions = asked;
                        if choose_network { panel_actions.push(quick::Action::Settings); open_network = true; }
                        if !world_was_open && self.world.expanded {
                            self.files_browser = None;
                            self.settings.open = false; self.files_browser = None;
                            self.launcher.open = false;
                            self.music.open = false;
                            self.notices.open = false;
                            self.power_menu.open = false;
                            self.send(Command::Terminal);
                        }
                        wanted
                    })
                    .then(|| self.toggle_music());
                }
                if self.config.side_panel_behavior == des_common::PanelBehavior::Collapse {
                    for (panel, visible, rect) in [
                        (des_common::Panel::Left, open.left, frame_layout.left.map(on_screen)),
                        (des_common::Panel::Right, open.right, frame_layout.right.map(on_screen)),
                    ] {
                        if !visible {
                            if let Some(rect)=rect {
                                let clicked=if panel==des_common::Panel::Left && !self.capture.marking() {
                                    let vitals=crate::dock::Vitals{cpu:self.mon.cpu_hist.back().copied().unwrap_or(0.0),memory:self.mon.mem_used as f32*100.0/self.mon.mem_total.max(1) as f32,failed:self.mon.failed_units.len()};
                                    let rail=frame_region(ui,rect,vec2(0.0,0.0),Layout::top_down(Align::Center),|ui|self.dock.rail_on(ui,state.as_ref(),&theme,Some(&vitals)));
                                    left_rail_output=Some(rail.output);rail.expand
                                } else {frame_region(ui,rect,vec2(0.0,0.0),Layout::top_down(Align::Center),|ui|panels::collapsed_rail(ui,&theme,panel))};
                                if clicked {match &self.desktop {Some(_)=>command=Some(Command::TogglePanel(panel)),None=>self.local_panels.toggle(panel)}}
                            }
                        }
                    }
                }
                let mut typed = None;
                if open.bottom {
                    let rect=on_screen(frame_layout.bottom.expect("open bottom panel"));
                    typed=frame_region(ui,rect,vec2(24.0,10.0),Layout::top_down(Align::Min),|ui|{
                        // FILES and CHAT: which the panel shows.
                        let chat=self.companion.chat_selected;
                        let mut switch=None;
                        if chat {
                            ui.horizontal(|ui|{
                                ui.spacing_mut().item_spacing.x=14.0;
                                switch=panels::tabs(ui,&theme,&["FILES","CHAT"],1);
                                if self.companion.voice_active(){panels::pill(ui,"● VOICE ACTIVE",theme.main);}
                                ui.with_layout(Layout::right_to_left(Align::Center),|ui|{
                                    if crate::ui::icon_button(ui,&theme,crate::ui::Icon::Settings,28.0,false).on_hover_text("Companion settings").clicked(){self.companion.settings_requested=true;self.companion.blur_text();}
                                });
                            });
                            ui.add_space(4.0);
                            if switch==Some(0){self.companion.select_files();}
                        }
                        if chat && switch!=Some(0) {
                            self.fs.blur();
                            if self.companion.open && !self.shortcut_reference.open {ctx.input_mut(|i|i.events.extend(std::mem::take(&mut reference_input)));}
                            self.companion.animate=animate;
                            self.companion.panel_ui(ui,&theme);
                            if self.companion.open {let _=shortcuts::take_modal_input(ctx,true);}
                            if self.companion.open {chat_area=Some(rect);}
                            None
                        } else {
                            let cwd=self.tabs[self.focus_tab()].cwd();
                            let typed=self.fs.show(ui,&theme,cwd.as_deref(),|ui|{switch=panels::tabs(ui,&theme,&["FILES","CHAT"],0);});
                            if switch==Some(1){self.fs.blur();self.companion.open(ctx);}
                            typed
                        }
                    });
                } else {
                    self.fs.blur();self.companion.blur_text();
                }
                scratch_keys |= self.companion.open;
                if self.companion.open && ctx.input(|i|i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p|workspace.contains(p))) {
                    self.companion.blur_text();
                }

                // The top bar stays in view while an application covers what is below
                // it, so it is also the way back to the terminal.
                let bar = on_screen(frame_layout.header);
                let actions = region(ui, bar, vec2(16.0, 0.0), Layout::left_to_right(Align::Center), |ui| {
                    self.top_bar(ui, &theme, state.as_ref())
                });
                if actions.find_terminal {
                    self.launcher.open=false;self.settings.open=false;self.files_browser=None;self.music.open=false;self.notices.open=false;self.power_menu.open=false;self.connect.expanded=false;
                    let i=self.focus_tab();self.tabs[i].open_search();
                }
                if actions.open_tab {
                    self.new_tab();
                }
                if actions.split_terminal {
                    self.split_terminal(false);
                }
                if actions.toggle_launcher {
                    self.toggle_launcher();
                }
                command = actions.command.or(command.take());

                let mut mine = workspace;
                if state.as_ref().is_some_and(|state| state.terminal_split) {
                    // An application has the right half; keep the terminal out from under it.
                    let (left, _) = frame_layout.workspace.halves();
                    mine.set_width(left.width);
                }
                let inner = mine.shrink2(vec2(24.0, 18.0));
                let inner = Rect::from_min_max(inner.min, inner.max.max(inner.min));

                if self.settings.open {
                    let changes = region(ui, mine, vec2(0.0, 0.0), Layout::top_down(Align::Min), |ui| {
                        let displays = self.desktop.as_ref().map(|desktop| &desktop.displays);
                        self.settings.set_music_status(self.music.connection_status());
                        self.settings.set_live_terminal_counts(self.tabs.iter().filter(|tab| tab.persistent_id.is_some()).count(), self.tabs.len());
                        self.settings.set_desktop_connected(self.desktop.as_ref().is_some_and(|d|d.connected()));
                        self.settings.show(ui, mine, &theme, &self.config, open, &self.mon, &places, displays, &self.input, &mut self.companion)
                    });
                    for change in changes {
                        self.apply(ctx, change);
                    }
                    return;
                }
                if self.files_browser.is_some() {
                    let action = region(ui, mine, vec2(20.0, 16.0), Layout::top_down(Align::Min), |ui| {
                        let progress = self.file_job.as_ref().map(|task| task.progress.load(std::sync::atomic::Ordering::Relaxed));
                        let browser = self.files_browser.as_mut().unwrap();
                        browser.configure_operations(self.file_clipboard.clone(), self.file_work.view(progress.unwrap_or(0)));
                        browser.set_phones(self.connect.file_targets());
                        browser.show_with_keyboard(ui, &theme, !scratch_keys && !self.dock.captures_keyboard())
                    });
                    match action {
                        Some(files::Event::Close) => self.files_browser = None,
                        Some(files::Event::Clipboard(clipboard)) => self.file_clipboard = Some(clipboard),
                        Some(files::Event::Drag(path)) => self.start_file_drag(ctx, &path),
                        Some(files::Event::Operate(operations)) => { self.file_work.enqueue(operations); self.pump_file_queue(); },
                        Some(files::Event::Undo) => { if self.file_work.undo().is_some() { self.pump_file_queue(); } },
                        Some(files::Event::Resolve(choice, all)) => { self.file_work.resolve(choice, all); self.pump_file_queue(); },
                        Some(files::Event::PauseTransfers(paused)) => { self.file_work.set_paused(paused); self.pump_file_queue(); },
                        Some(files::Event::ClearTransfers) => { self.file_work.clear(); },
                        Some(files::Event::SendTo(device, paths)) => {
                            let name = self.connect.file_targets().into_iter().find(|(id, _)| *id == device).map(|(_, name)| name).unwrap_or_default();
                            match self.connect.send_files(ctx, &device, &paths) {
                                Ok(count) => self.notices.post("Files", &format!("Offering {count} file(s) to {name}. Accept on the device; progress shows in CONNECT."), false),
                                Err(error) => self.notices.post("Not sent", &error, false),
                            }
                        },
                        Some(files::Event::Terminal(folder)) => { self.files_browser = None; self.new_tab_in(folder); },
                        Some(files::Event::CancelOperation) => { if let Some(task) = &self.file_job { task.cancel(); } },
                        Some(files::Event::Open(path)) => {
                            if self.document_openers.len() < 32 { self.document_openers.push(apps::open_document(path, ctx.clone())); }
                            else { self.notices.post("File not opened", "Wait for existing document viewers to respond.", false); }
                        },
                        _ => {},
                    }
                    return;
                }
                if self.music.open {
                    region(ui, mine, vec2(0.0, 0.0), Layout::top_down(Align::Min), |ui| {
                        self.music.set_lyrics_motion(self.config.animate && !self.config.appearance.reduce_motion);
                        self.music.show(ui, mine, &theme)
                    });
                    return;
                }
                if self.power_menu.open {
                    let act = region(ui, mine, vec2(0.0, 0.0), Layout::top_down(Align::Min), |ui| {
                        self.power_menu.show(ui, mine, &theme, self.mon.power)
                    });
                    match act {
                        Some(power::Act::Lock) => self.lock_now(),
                        Some(power::Act::LogOut) => ctx.send_viewport_cmd(ViewportCommand::Close),
                        Some(act) => act.run(),
                        None => {}
                    }
                    return;
                }
                if self.notices.open {
                    let (chosen, quiet) = region(ui, mine, vec2(0.0, 0.0), Layout::top_down(Align::Min), |ui| {
                        self.notices.show(ui, mine, &theme)
                    });
                    if let Some(quiet) = quiet {
                        if let Err(e) = config::set("do_not_disturb", quiet.then_some("on")) {
                            eprintln!("des-ui: cannot save the setting: {e}");
                        }
                    }
                    if let Some(chosen) = chosen {
                        self.follow(chosen);
                    }
                    return;
                }
                if self.launcher.open {
                    let picked = region(ui, mine, vec2(0.0, 0.0), Layout::top_down(Align::Min), |ui| {
                        self.launcher.set_state(state.clone());
                        self.launcher.set_pins(&self.config.dock_pins);
                        self.launcher.show(ui, mine, &theme)
                    });
                    if let Some(action) = picked { self.palette_action(action); }
                    return;
                }

                if self.connect.expanded {
                    region(ui, mine, vec2(24.0, 18.0), Layout::top_down(Align::Min), |ui| self.connect.expanded_view(ui, &theme));
                    return;
                }

                if self.world.expanded {
                    region(ui, mine, vec2(24.0, 18.0), Layout::top_down(Align::Min), |ui| {
                        self.world.workspace(ui, &theme, &places, rotating, self.config.lookups);
                    });
                    return;
                }

                // One continuous wallpaper sample covers the workspace padding and
                // split gaps as well as the terminal cells. No bright picture frame.
                material::paint(ctx, ui.painter(), mine, 0.0, material::Role::Terminal);
                // Keep pumping every PTY above, but do not lay out text, acquire
                // keyboard focus or blink a cursor behind a covering native app.
                let focused = self.focus_tab();
                if skip_covered_terminal(state.as_ref(), &mut typed, |bytes| self.tabs[focused].input(bytes)) {
                    return;
                }
                // One pane, or two with a gap and a rule between them.
                let panes: Vec<(usize, Rect)> = match self.side {
                    None => vec![(self.active, inner)],
                    Some(side) => {
                        let gap = 25.0;
                        let (first, second) = if self.stacked {
                            let height = ((inner.height() - gap) / 2.0).floor().max(1.0);
                            let rule = inner.top() + height + gap / 2.0;
                            ui.painter().hline(inner.x_range(), rule, Stroke::new(1.0, ui::LINE));
                            (
                                Rect::from_min_size(inner.min, vec2(inner.width(), height)),
                                Rect::from_min_max(pos2(inner.left(), inner.top() + height + gap), inner.max),
                            )
                        } else {
                            let width = ((inner.width() - gap) / 2.0).floor().max(1.0);
                            let rule = inner.left() + width + gap / 2.0;
                            ui.painter().vline(rule, inner.y_range(), Stroke::new(1.0, ui::LINE));
                            (
                                Rect::from_min_size(inner.min, vec2(width, inner.height())),
                                Rect::from_min_max(pos2(inner.left() + width + gap, inner.top()), inner.max),
                            )
                        };
                        vec![(self.active, first), (side, second)]
                    }
                };
                let responses: Vec<Response> = panes
                    .iter()
                    .enumerate()
                    .map(|(n, (_, rect))| ui.interact(*rect, ui.id().with(("terminal-pane", n)), Sense::click_and_drag()))
                    .collect();
                // A click in a pane gives it the keyboard.
                for (n, response) in responses.iter().enumerate() {
                    if panes.len() > 1 && (response.clicked() || response.drag_started()) {
                        self.side_focused = n == 1;
                    }
                }
                let focused = if self.side_focused && panes.len() > 1 { 1 } else { 0 };
                if panes.len() > 1 {
                    // A mark over the pane in use.
                    let rect = panes[focused].1;
                    let mark = Rect::from_min_size(pos2(rect.left(), rect.top() - 10.0), vec2(36.0, 2.0));
                    ui.painter().rect_filled(mark, 0.0, theme.main);
                }
                let response = &responses[focused];

                // The terminal owns the keyboard: keep focus on it and stop egui from
                // using Tab, arrows and Escape for its own widget navigation.
                if !self.dock.captures_keyboard() && !scratch_keys && !self.tabs[panes[focused].0].searching() { ui.memory_mut(|m| {
                    m.request_focus(response.id);
                    m.set_focus_lock_filter(
                        response.id,
                        EventFilter {
                            tab: true,
                            horizontal_arrows: true,
                            vertical_arrows: true,
                            escape: true,
                        },
                    );
                });

                }
                let (alt_held, shift_held) = ui.input(|i| (i.modifiers.alt, i.modifiers.shift));
                let tab = &mut self.tabs[panes[focused].0];
                if !self.dock.captures_keyboard() && !scratch_keys { tab.handle_events(&events, alt_held, shift_held, ctx); }
                if let Some(text) = typed.take() {
                    tab.input(text.as_bytes());
                }
                let look = self.config.term_look();
                for (n, (index, rect)) in panes.iter().enumerate() {
                    self.tabs[*index].show(ui, *rect, &theme, &responses[n], &look, n == focused);
                }
            });
        // Only one lifted dock surface is active at a time. Never union disjoint
        // left/right rectangles: doing so would cover native windows between them.
        for action in panel_actions {
            if open_network && action == quick::Action::Settings {
                self.launcher.open = false; self.music.open = false; self.notices.open = false; self.power_menu.open = false;
                self.settings.open_at(&self.config, "wifi");
                self.send(Command::Terminal);
            } else {
                self.quick_action(ctx, action);
            }
        }
        if let Some(rail)=left_rail_output {
            if rail.action.is_some() || rail.overlay.is_some() {apps_output=rail;}
        }
        if let Some(command) = command { self.send(command); }
        if let Some(output) = self.tabs.iter_mut().find_map(TermTab::take_explain_output) {
            self.companion.prepare_terminal_explanation(ctx, &output);
        }

        let look = self.config.term_look();
        let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from);
        let scratch = if self.companion.open || self.shortcut_reference.open { None } else { self.scratchpad.show(ctx, workspace, screen, &theme, &look, animate,
            (&self.shell, &home, self.config.scrollback, self.config.keep_terminal_sessions && self.live_terminal_lease.is_some()), &events) };
        if let Some(output) = self.scratchpad.take_explain_output() { self.companion.prepare_terminal_explanation(ctx, &output); }
        // The quick settings, when open, take the lifted area: they are the one in use.
        let readings = quick::Readings {
            volume: self.mon.volume(),
            brightness: self.mon.brightness,
            night_light: self.config.night_light,
            do_not_disturb: self.notices.quiet,
            keyboard: self.osk.open,
            power: self.mon.power,
            battery_health: self.mon.battery_health,
        };
        for action in self.quick.show(ctx, &theme, screen, keys_top, &readings) {
            self.quick_action(ctx, action);
        }
        let quick_area = self.quick.drawn_area();


        // Subtle rounded outlines belong to the same shared panel rectangles.
        let lines = ctx.layer_painter(frame_lines_layer());
        let line = Stroke::new(1.0, ui::LINE);
        powerup::frame_lines(&lines, screen, workspace, status_top, osk_height, open,
            self.config.side_panel_behavior == des_common::PanelBehavior::Collapse, line, &stage);

        if self.capture.marking() { self.dock.reset(); }
        let dock = apps_output;
        let dock_area = dock.overlay.map(|rect| (
            rect.left().floor() as i32, rect.top().floor() as i32,
            rect.width().ceil() as i32 + 1, rect.height().ceil() as i32 + 1,
        ));
        if self.dock_area != Some(dock_area) {
            match &mut self.desktop {
                Some(desktop) => {
                    if desktop.send(Command::Dock(dock_area)).is_ok() { self.dock_area=Some(dock_area); }
                }
                None => self.dock_area=Some(dock_area),
            }
        }
        if self.dock_keyboard.get(&0).copied()!=Some(dock.keyboard) {
            if let Some(desktop)=&mut self.desktop {
                if desktop.send(Command::DockKeyboard {screen:0,active:dock.keyboard}).is_ok(){self.dock_keyboard.insert(0,dock.keyboard);}
            } else {self.dock_keyboard.insert(0,dock.keyboard);}
        }
        if let Some(action) = dock.action { self.apply_dock_action(action); }
        if let Some(id) = self.launcher.take_pin_request() { self.apply_dock_action(dock::Action::Pin(id,true)); }

        // Toasts hang from the top right corner of the workspace. The compositor is
        // told where, so that it shows them above whatever application is open.
        // The volume or brightness meter hangs first; the toasts follow below it.
        let corner = workspace.right_top() + vec2(-16.0, 12.0);
        let meter = self.osd.show(ctx, &theme, corner, animate);
        let toasts_at = meter.map_or(corner, |meter| corner + vec2(0.0, meter.height() + 8.0));
        let (mut covered, chosen) = self.notices.toasts(ctx, &theme, toasts_at);
        if let Some(meter) = meter {
            covered = Some(covered.map_or(meter, |toasts| toasts.union(meter)));
        }
        // The calendar opened from the clock; lifted with the toasts, so it shows over applications.
        if let Some(anchor) = agenda::take_request(ctx) {
            self.agenda.toggle(ctx, anchor);
        }
        if let Some(rect) = self.agenda.show(ctx, &theme) {
            covered = Some(covered.map_or(rect, |all| all.union(rect)));
        }
        // Marking out a piece for a screenshot takes the whole display, above everything.
        if self.capture.marking() {
            let request_owned = self.capture.request_marking();
            match self.capture.mark(ctx, theme.main) {
                capture::Marked::Record => self.toggle_recording(),
                capture::Marked::Colour(rgb) => {
                    if request_owned {
                        self.capture.cancel_request();
                        if let Some(reply) = self.pending_color.take() {
                            reply.send(control::colour_answer(Some(rgb)));
                        }
                    }
                }
                capture::Marked::Cancelled => { if request_owned { self.answer_held(None, true); } },
                capture::Marked::Going => {}
            }
            covered = Some(screen);
        }
        let area = covered.map(|rect| {
            (rect.left().floor() as i32, rect.top().floor() as i32, rect.width().ceil() as i32 + 1, rect.height().ceil() as i32 + 1)
        });
        if self.toast_area != Some(area) {
            self.toast_area = Some(area);
            self.send(Command::Toast(area));
        }
        if let Some(chosen) = chosen {
            self.follow(chosen);
        }

        // The names and the selection around the compositor's pictures of the windows.
        if let Some(layout) = self.desktop.as_ref().and_then(|desktop| desktop.overview.as_ref()) {
            overview::show(ctx, &theme, layout, state.as_ref());
        }
        self.secondary_views(ctx,state.as_ref(),false);
        // The file panel's preview, over the workspace, with the terminal brought into
        // view beneath it.
        if self.fs.take_uncover() {
            self.send(Command::Terminal);
        }
        self.fs.look.show(ctx, workspace, &theme);
        // A file dragged out of the panel: the compositor carries it to whatever is
        // under the pointer when the button is let go.
        if let Some(path) = self.fs.take_drag() {
            self.start_file_drag(ctx, &path);
        }
        if self.polkit.active() { self.shortcut_reference.close(); }
        ctx.input_mut(|input| input.events.extend(reference_input));
        if self.shortcut_reference.show(ctx, &theme, &self.config.shortcuts) {
            self.files_browser = None; self.launcher.open = false; self.music.open = false; self.notices.open = false; self.power_menu.open = false;
            self.settings.open_at(&self.config, "shortcuts");
            self.send(Command::Terminal);
        }
        // A reference modal owns the entire display, including pointer input,
        // while applications beneath it retain their workspace/process state.
        let companion_suppressed=self.lock.locked || self.polkit.active() || self.capture.marking() || self.capture.recording.is_some() || self.notices.quiet || self.shortcut_reference.open || self.quick.open || self.scratchpad.visible() || self.launcher.open || self.settings.open || self.music.open || self.notices.open || self.power_menu.open || (ctx.dragged_id().is_some() || ctx.input(|i|!i.focused)) || self.desktop.as_ref().is_some_and(|d|!d.connected() || d.state.focus.is_some() || d.state.covering().is_some() || d.overview.is_some());
        let _companion_bubble=self.companion.social_tick(ctx,companion_suppressed,&theme,animate);
        let voice_allowed=!self.lock.locked && !self.polkit.active() && !self.capture.marking();
        if let Some(pointer)=self.desktop.as_mut().and_then(|d|d.companion_pointer.take()) {self.companion.set_voice_pointer(pointer);}
        let voice_caption=if voice_allowed {self.companion.voice_bubble(ctx,&theme,animate)}else{None};
        for command in self.companion.cursor_commands(voice_caption,voice_allowed,Instant::now()) {self.send(command);}
        if std::mem::take(&mut self.companion.settings_requested) {
            self.settings.open_at(&self.config, "companion");
            self.send(Command::Terminal);
        }
        if self.companion.take_capture_request() {
            let id = self.companion.begin_capture();
            if self.lock.locked || self.lock.preparing() || self.desktop.as_ref().is_some_and(|desktop| desktop.locked || desktop.lock_requested || desktop.idle_lock_requested) || self.capture.marking() {
                self.companion.capture_failed(id, "Screen capture is unavailable right now.");
            } else if let Some(desktop) = self.desktop.as_mut() {
                if desktop.send(Command::ShotRequest { id, mode: des_common::ipc::ShotMode::Screen }).is_err() {
                    self.companion.capture_failed(id, "Could not request a screen capture.");
                }
            } else { self.companion.capture_failed(id, "Screen capture requires the DEs-UI compositor."); }
        }
        let overlay_area = if self.shortcut_reference.open {Some(screen)} else {quick_area.or(chat_area).or(scratch)}.map(scratchpad::area);
        if scratchpad::changed(&mut self.overlay_area, overlay_area) {
            self.send(Command::CompanionChat(self.companion.open && chat_area.is_some() && quick_area.is_none()));
            self.send(Command::Overlay(overlay_area));
        }
        // Last, so that it is above everything else.
        self.polkit.show(ctx, &theme, false);

        // The globe turns continuously; without it, the clock sets the pace.
        let pace = if self.world.turning(rotating) { 66 } else { 500 };
        ctx.request_repaint_after(Duration::from_millis(pace));
        self.checkpoint_live_terminals();
        if self.desktop.as_ref().is_some_and(|desktop| desktop.connected()) { self.readiness.notify(); }
    }
}

impl App {
    pub(super) fn draw_saver(&self, ctx: &egui::Context, started: Instant) {
        screensaver::show(
            ctx,
            self.config.screensaver,
            started.elapsed(),
            !self.config.animate || self.config.appearance.reduce_motion,
            self.mon.power.is_some_and(|battery| !battery.plugged),
        );
    }

    pub(super) fn cancel_idle_saver(&mut self) {
        if self.idle_saver.cancel() {
            self.send(Command::HideSaver);
        }
    }

    pub(super) fn close_saver_preview(&mut self) {
        if self.saver_preview.take().is_some() && self.overlay_area.take().is_some() {
            self.send(Command::Overlay(None));
        }
    }

    /// Answers what `des-ui-ctl` asked. Locked, only the lock itself, `ping` and
    /// `state` are allowed, so no script opens anything over the lock screen.
    fn answer_control(&mut self) {
        use control::{Answer, Request};
        let cancelled_shot = self.pending_shot_replies.iter().any(control::Reply::is_cancelled);
        let cancelled_color = self.pending_color.as_ref().is_some_and(control::Reply::is_cancelled);
        if cancelled_shot || cancelled_color {
            self.capture.cancel_request();
            self.pending_shot_replies.retain(|reply| !reply.is_cancelled());
            if cancelled_color { self.pending_color = None; }
        }
        let requests = self.control.as_mut().map(|control| control.take()).unwrap_or_default();
        for (request, reply) in requests {
            if reply.is_cancelled() { continue; }
            let locked = self.lock_confirmed || self.unlock_pending;
            let compositor = self.desktop.as_ref().is_some_and(Desktop::connected);
            let answer = match request {
                Request::Ping => Answer::Ok,
                Request::State => Answer::Json(control::state_json(
                    self.desktop.as_ref().filter(|desktop| desktop.connected()).map(|desktop| &desktop.state),
                    locked,
                )),
                Request::Lock => {
                    if locked { self.cancel_idle_saver(); } else { self.lock_now(); }
                    Answer::Ok
                }
                _ if locked => Answer::Error("locked".into()),
                Request::Dock | Request::Screenshot(_) | Request::ScreenshotFile(_) | Request::PickColor if self.capture.marking() => Answer::Error("a screenshot is being marked".into()),
                Request::Workspace(_) | Request::Panel(_) | Request::Screenshot(_) | Request::ScreenshotFile(_) | Request::PickColor | Request::Overview if !compositor => Answer::Error("no compositor".into()),
                Request::Screenshot(_) | Request::ScreenshotFile(_) | Request::PickColor
                    if !self.pending_shot_replies.is_empty() || self.pending_color.is_some() => Answer::Error("busy".into()),
                // These two are answered once the user has chosen, so the reply is held.
                Request::ScreenshotFile(mode) => {
                    let id = self.capture.begin_request(mode);
                    self.pending_shot_replies.push(reply);
                    if let Some(desktop) = self.desktop.as_mut() {
                        if let Err(error) = desktop.send(Command::ShotRequest { id, mode }) {
                            self.capture.failed(Some(id), format!("cannot request screenshot: {error}"));
                        }
                    }
                    continue;
                }
                Request::PickColor => {
                    let id = self.capture.begin_request(des_common::ipc::ShotMode::Region);
                    self.pending_color = Some(reply);
                    self.capture.pick_colour_next();
                    if let Some(desktop) = self.desktop.as_mut() {
                        if let Err(error) = desktop.send(Command::ShotRequest { id, mode: des_common::ipc::ShotMode::Region }) {
                            self.capture.failed(Some(id), format!("cannot request colour picker: {error}"));
                        }
                    }
                    continue;
                }
                Request::Workspace(n) => { self.send(Command::Workspace(n)); Answer::Ok }
                Request::Toast(text) => { self.notices.post("des-ui-ctl", &text, false); Answer::Ok }
                Request::Dock => { self.focus_apps(0); Answer::Ok }
                Request::Launcher => { if !self.launcher.open { self.toggle_launcher(); } Answer::Ok }
                Request::Settings(None) => { if !self.settings.open { self.toggle_settings(); } Answer::Ok }
                Request::Settings(Some(section)) => {
                    self.launcher.open = false;
                    self.music.open = false;
                    self.notices.open = false;
                    self.power_menu.open = false;
                    self.settings.open_at(&self.config, &section);
                    self.send(Command::Terminal);
                    Answer::Ok
                }
                Request::Scratchpad => { self.scratchpad.toggle(); Answer::Ok }
                Request::Overview => { self.send(Command::Overview); Answer::Ok }
                Request::Quick => { self.quick.toggle(); Answer::Ok }
                Request::Music(word) => { self.music.command(word.word()); Answer::Ok }
                Request::Panel(panel) => { self.send(Command::TogglePanel(panel)); Answer::Ok }
                Request::Screenshot(mode) => { self.send(Command::Shot(mode)); Answer::Ok }
            };
            reply.send(answer);
        }
    }

    /// Answers programs waiting on a screenshot: with the files just saved, and with
    /// "cancelled" when marking was called off.
    pub(super) fn answer_held(&mut self, saved: Option<Vec<Result<PathBuf, String>>>, called_off: bool) {
        for result in saved.unwrap_or_default() {
            if !self.pending_shot_replies.is_empty() {
                self.pending_shot_replies.remove(0).send(control::shot_answer(&result));
            } else if let Err(why) = result {
                if let Some(reply) = self.pending_color.take() {
                    reply.send(control::Answer::Error(why));
                }
            }
        }
        if called_off {
            self.capture.cancel_request();
            for reply in self.pending_shot_replies.drain(..) {
                reply.send(control::Answer::Error("cancelled".into()));
            }
            if let Some(reply) = self.pending_color.take() {
                reply.send(control::colour_answer(None));
            }
        }
    }
}

impl App {
    /// Does what the quick settings asked.
    fn quick_action(&mut self, ctx: &egui::Context, action: quick::Action) {
        match action {
            quick::Action::Change(change) => self.apply(ctx, change),
            quick::Action::Mute => {
                self.osd.hold(osd::Kind::Volume, Instant::now());
                spawn_quietly("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]);
            }
            quick::Action::Keyboard => self.toggle_osk(),
            quick::Action::DoNotDisturb(quiet) => {
                self.notices.quiet = quiet;
                if let Err(e) = config::set("do_not_disturb", quiet.then_some("on")) {
                    self.notices.post("Could not save Do not disturb", &e.to_string(), false);
                }
            }
            quick::Action::Settings => {
                self.quick.open = false;
                if !self.settings.open {
                    self.toggle_settings();
                }
            }
            quick::Action::Power => {
                self.quick.open = false;
                self.toggle_power();
            }
            quick::Action::Lock => {
                self.quick.open = false;
                self.lock_now();
            }
        }
    }
}

// A covered terminal still accepts commands explicitly chosen in the Files panel.
fn skip_covered_terminal(state: Option<&DesktopState>, typed: &mut Option<String>, mut send: impl FnMut(&[u8])) -> bool {
    let covered = state.is_some_and(|state| !state.terminal_split && state.covering().is_some());
    if covered {
        if let Some(text) = typed.take() { send(text.as_bytes()); }
    }
    covered
}

#[cfg(test)]
mod terminal_visibility_tests {
    use super::*;
    #[test]
    fn covered_terminal_keeps_explicit_files_commands() {
        let state = DesktopState::decode("state 1 0 0 0 0\t0 1:0:74657374 Test app\n").unwrap();
        let mut typed = Some("cd ..\r".to_string());
        let mut sent = Vec::new();
        assert!(skip_covered_terminal(Some(&state), &mut typed, |bytes| sent.extend_from_slice(bytes)));
        assert_eq!(sent, b"cd ..\r");
        assert!(typed.is_none(), "the explicit command must only be delivered once");
    }
    #[test]
    fn visible_terminal_keeps_commands_for_normal_input_order() {
        let mut state = DesktopState::default();
        let mut typed = Some("cd ..\r".to_string());
        assert!(!skip_covered_terminal(Some(&state), &mut typed, |_| panic!("ordinary input is routed later")));
        assert!(typed.is_some());
        state = DesktopState::decode("state 1 0 1 0 0\t0 1:0:74657374 Test app\n").unwrap();
        assert!(!skip_covered_terminal(Some(&state), &mut typed, |_| panic!("split terminal remains visible")));
    }
}
