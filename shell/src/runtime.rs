//! Per-frame desktop coordination and rendering.

use super::*;

impl eframe::App for App {
    fn clear_color(&self, _visuals: &Visuals) -> [f32; 4] {
        ui::GROUND.to_normalized_gamma_f32()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Before anything reads input: what the on-screen keys pressed last frame.
        self.osk.inject(ctx);
        self.mon.tick();
        self.widgets.pump();
        // While the terminal in use is in an ssh session, the panels show that machine.
        let focused = self.focus_tab();
        self.remote.follow_tab(self.tabs.get(focused), ctx);
        let view = self.remote.view();
        let following = self.remote.target().map(|target| sysmon::Following {
            name: target.name(),
            live: view.is_some(),
            word: self.remote.state().map_or("", remote::State::word),
        });
        self.geo.set_remote_connections(view.as_ref().map(|view| view.connections.clone()));
        self.mon.set_remote(following, view.as_ref());
        // Threshold crossings the sampler noticed: heat, a full disk, memory, a failed unit.
        for alert in self.mon.take_alerts() {
            self.notices.post(&alert.title, &alert.body, alert.urgent);
        }
        self.pump_documents();
        if let Some(desktop) = &mut self.desktop {
            desktop.pump();
        }
        self.answer_control();
        self.clipboard.poll(self.launcher.open);
        if let Some(desktop) = &mut self.desktop {
            if !desktop.connected() { self.dock_area = None; self.overlay_area = None; self.secondary_areas.clear(); }
            if let Some(screen)=desktop.dock_screen_requested.take(){
                if screen==0{self.dock.reveal_keyboard();}else if screen<desktop.state.screens{self.secondary_docks.entry(screen).or_default().reveal_keyboard();}
            }
            if std::mem::take(&mut desktop.dock_requested) { self.dock.reveal_keyboard(); }
            if std::mem::take(&mut desktop.launcher_requested) && !self.launcher.open {
                self.settings.open = false;
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
                self.settings.open = false;
                self.notices.open = false;
                self.power_menu.open = false;
                self.music.open = true;
            }
            if std::mem::take(&mut desktop.power_requested) {
                self.launcher.open = false;
                self.settings.open = false;
                self.music.open = false;
                self.notices.open = false;
                self.power_menu.open = true;
            }
            if std::mem::take(&mut desktop.notifications_requested) && !self.notices.open {
                self.launcher.open = false;
                self.settings.open = false;
                self.music.open = false;
                self.notices.open = true;
            }
            // Bring the frame in line with the saved choice of panels: once at the
            // start, and again whenever that choice is changed in the settings.
            if !desktop.connected() { self.frame_synced = false; }
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
        for tab in &mut self.tabs {
            tab.pump();
        }
        self.scratchpad.pump();
        // Tabs whose shell has exited go; the ones in view keep their places, and a
        // split whose pane lost its tab becomes whole again.
        let alive: Vec<bool> = self.tabs.iter().map(|tab| !tab.exited).collect();
        let (active, side) = remap_panes(&alive, self.active, self.side);
        self.tabs.retain(|tab| !tab.exited);
        if self.tabs.is_empty() {
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }
        self.active = active;
        self.side = side;
        self.side_focused &= side.is_some();

        self.handle_screenshot(ctx);

        // The lock screen comes before everything else, and while it is up nothing
        // else is drawn or given any input.
        let (asked, compositor_locked, security_confirmed) = match &mut self.desktop {
            Some(desktop) => {
                let asked = std::mem::take(&mut desktop.lock_requested);
                if !desktop.connected() { self.lock_confirmed = false; }
                (asked, desktop.connected().then_some(desktop.locked), desktop.security_confirmed.take())
            }
            None => (false, None, None),
        };
        if asked && !self.lock.locked {
            self.lock_now();
        }
        if let Some(prepared) = self.lock.take_prepared() {
            match prepared {
                Ok(()) => self.finish_lock(),
                Err(why) => self.notices.post("Screen could not be locked", &why, true),
            }
        }
        match reconcile_lock(self.lock.locked, self.lock_confirmed, self.unlock_pending, security_confirmed == Some(false), compositor_locked) {
            LockStep::Engage => self.lock.engage(),
            LockStep::Confirm => self.lock_confirmed = true,
            LockStep::Release => { self.lock.release(); self.unlock_pending = false; self.lock_confirmed = false; }
            LockStep::Request => self.send(Command::Lock),
            LockStep::Wait => {},
        }
        self.notices.pump();
        if let Some(desktop) = &mut self.desktop {
            for shot in std::mem::take(&mut desktop.shots) {
                self.capture.take(shot);
            }
            for state in std::mem::take(&mut desktop.recording_news) {
                self.capture.recording_is(state, &mut self.notices);
            }
        }
        let saved = self.capture.report(&mut self.notices);
        let called_off = self.capture.take_called_off();
        self.answer_held(Some(saved), called_off);
        // Sleep is on its way, or the session was told to lock.
        if self.power.lock_wanted() && !self.lock.locked {
            let for_sleep = self.power.sleep_waiting();
            if !for_sleep || self.config.lock_on_sleep {
                self.lock_now();
            }
        }
        if self.power.sleep_waiting() {
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
            let state=self.desktop.as_ref().map(|d|d.state.clone());
            self.secondary_views(ctx,state.as_ref(),true);
            self.dock.reset();
            if self.dock_area != Some(None) {
                self.dock_area = Some(None);
                self.send(Command::Dock(None));
            }
            self.capture.cancel();
            self.scratchpad.close_now();
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
            let music = &mut self.music;
            if self.lock.show(ctx, &theme, |ui, within| music.lock_card(ui, &theme, within)) {
                if self.desktop.is_some() {
                    self.unlock_pending = true;
                    self.lock.await_unlock();
                }
                self.send(Command::Unlock);
            }
            self.powerup.publish(ctx, self.config.animate, true);
            return;
        }
        let stage = self.powerup.publish(ctx, self.config.animate, false);
        self.pump_restore();
        let events = self.take_shortcuts(ctx);
        let theme = self.theme.clone();
        let screen = ctx.screen_rect();
        let state = self.desktop.as_ref().map(|desktop| desktop.state.clone());
        // The compositor decides which panels are open; alone, the shell does.
        let open = state.as_ref().map_or(self.local_panels, |state| state.panels);
        let places = self.geo.snapshot();
        // Still, the globe faces this machine's side of the world.
        let rotation = if self.config.globe_rotation {
            self.started.elapsed().as_secs_f32() * globe::SPEED
        } else {
            places.home.as_ref().map_or(20.0, |home| -home.lon)
        };
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
        } else {
            None
        };
        let mut command = None;

        // Every region is placed from the shared geometry, so that the workspace is
        // exactly where the compositor puts application windows.
        let area = des_common::workspace_with_behavior(screen.width(), screen.height(), open, self.config.side_panel_behavior);
        let workspace = Rect::from_min_size(
            screen.min + vec2(area.x, area.y),
            vec2(area.width, area.height),
        );
        let status_top = screen.bottom() - des_common::STATUS_HEIGHT;
        // The on-screen keys sit above the status strip and push the frame up.
        let osk_height = self.osk.height();
        let keys_top = status_top - osk_height;
        let workspace = Rect::from_min_max(workspace.min, pos2(workspace.max.x, (workspace.max.y - osk_height).max(workspace.min.y)));
        let dock_keys = self.dock.reserve_keyboard(ctx);
        // While the scratchpad is down it has the keyboard, not the panes.
        let scratch_keys = self.scratchpad.visible();
        CentralPanel::default()
            .frame(Frame::new().fill(ui::GROUND))
            .show(ctx, |ui| {
                let status = Rect::from_min_max(pos2(screen.left(), status_top), screen.max);
                if osk_height > 0.0 {
                    self.osk.show(ui, &theme, Rect::from_min_max(pos2(screen.left(), keys_top), pos2(screen.right(), status_top)));
                }
                let toggled = region(ui, status, vec2(24.0, 0.0), Layout::left_to_right(Align::Center), |ui| {
                    panels::status(ui, &theme, &self.mon, state.as_ref(), open, overlay, self.notices.count(), self.capture.recording.map(|since| since.elapsed()), Some(&self.tray))
                });
                let (toggled, settings_clicked, music_clicked, notices_clicked, power_clicked, stop_clicked) = toggled;
                if stop_clicked {
                    self.toggle_recording();
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
                    let rect = Rect::from_min_max(screen.min, pos2(workspace.left(), keys_top));
                    region(ui, rect, column, Layout::top_down(Align::Min), |ui| {
                        panels::left(ui, &theme, &self.mon, &mut self.pet, &self.widgets)
                    });
                }
                if open.right {
                    let rect = Rect::from_min_max(pos2(workspace.right(), screen.top()), pos2(screen.right(), keys_top));
                    region(ui, rect, column, Layout::top_down(Align::Min), |ui| {
                        let music = &mut self.music;
                        let mut wanted = false;
                        panels::right(ui, &theme, &self.mon, state.as_ref(), rotation, &places, &self.widgets, |ui| {
                            wanted = music.panel(ui, &theme);
                        });
                        wanted
                    })
                    .then(|| self.toggle_music());
                }
                if self.config.side_panel_behavior == des_common::PanelBehavior::Collapse {
                    for (panel, visible, rect) in [
                        (des_common::Panel::Left, open.left, Rect::from_min_max(screen.min, pos2(workspace.left(), keys_top))),
                        (des_common::Panel::Right, open.right, Rect::from_min_max(pos2(workspace.right(), screen.top()), pos2(screen.right(), keys_top))),
                    ] {
                        if !visible && region(ui, rect, vec2(0.0, 0.0), Layout::top_down(Align::Center), |ui| panels::collapsed_rail(ui, &theme, panel)) {
                            match &self.desktop {
                                Some(_) => command = Some(Command::TogglePanel(panel)),
                                None => self.local_panels.toggle(panel),
                            }
                        }
                    }
                }
                let mut typed = None;
                if open.bottom {
                    let rect = Rect::from_min_max(workspace.left_bottom(), pos2(workspace.right(), keys_top));
                    typed = region(ui, rect, vec2(24.0, 16.0), Layout::top_down(Align::Min), |ui| {
                        let cwd = self.tabs[self.focus_tab()].cwd();
                        self.fs.show(ui, &theme, cwd.as_deref())
                    });
                }

                // The top bar stays in view while an application covers what is below
                // it, so it is also the way back to the terminal.
                let bar = Rect::from_min_max(pos2(workspace.left(), screen.top()), workspace.right_top());
                let actions = region(ui, bar, vec2(16.0, 0.0), Layout::left_to_right(Align::Center), |ui| {
                    self.top_bar(ui, &theme, state.as_ref())
                });
                if actions.find_terminal {
                    self.launcher.open=false;self.settings.open=false;self.music.open=false;self.notices.open=false;self.power_menu.open=false;
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
                    let (left, _) = area.halves();
                    mine.set_width(left.width);
                }
                let inner = mine.shrink2(vec2(24.0, 18.0));
                let inner = Rect::from_min_max(inner.min, inner.max.max(inner.min));

                if self.settings.open {
                    let changes = region(ui, mine, vec2(0.0, 0.0), Layout::top_down(Align::Min), |ui| {
                        let displays = self.desktop.as_ref().map(|desktop| &desktop.displays);
                        self.settings.set_music_status(self.music.connection_status());
                        self.settings.set_desktop_connected(self.desktop.as_ref().is_some_and(|d|d.connected()));
                        self.settings.show(ui, mine, &theme, &self.config, open, &self.mon, &places, displays, &self.input)
                    });
                    for change in changes {
                        self.apply(ctx, change);
                    }
                    return;
                }
                if self.music.open {
                    region(ui, mine, vec2(0.0, 0.0), Layout::top_down(Align::Min), |ui| {
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
                        self.launcher.show(ui, mine, &theme)
                    });
                    if let Some(action) = picked { self.palette_action(action); }
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
        if let Some(command) = command { self.send(command); }

        let look = self.config.term_look();
        let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from);
        let scratch = self.scratchpad.show(ctx, workspace, screen, &theme, &look, self.config.animate,
            (&self.shell, &home, self.config.scrollback), &events);
        let overlay_area = scratch.map(scratchpad::area);
        if scratchpad::changed(&mut self.overlay_area, overlay_area) {
            self.send(Command::Overlay(overlay_area));
        }

        // The one-pixel lines between the frame's regions, drawn over all of them.
        let lines = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("frame-lines")));
        let line = Stroke::new(1.0, ui::LINE);
        powerup::frame_lines(&lines, screen, workspace, keys_top, open,
            self.config.side_panel_behavior == des_common::PanelBehavior::Collapse, line, &stage);

        ctx.input_mut(|input| input.events.extend(dock_keys));
        let dock = if self.capture.marking() {
            self.dock.reset();
            dock::Output::default()
        } else {
            self.dock.show(ctx, screen, state.as_ref(), &self.config.dock_pins, &theme, self.config.animate)
        };
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
        if let Some(action) = dock.action { self.apply_dock_action(action); }
        if let Some(id) = self.launcher.take_pin_request() { self.apply_dock_action(dock::Action::Pin(id,true)); }

        // Toasts hang from the top right corner of the workspace. The compositor is
        // told where, so that it shows them above whatever application is open.
        let (mut covered, chosen) = self.notices.toasts(ctx, &theme, workspace.right_top() + vec2(-16.0, 12.0));
        // Marking out a piece for a screenshot takes the whole display, above everything.
        if self.capture.marking() {
            match self.capture.mark(ctx, theme.main) {
                capture::Marked::Record => self.toggle_recording(),
                capture::Marked::Colour(rgb) => {
                    if let Some(reply) = self.pending_color.take() {
                        reply.send(control::colour_answer(Some(rgb)));
                    }
                }
                capture::Marked::Cancelled => self.answer_held(None, true),
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

        self.secondary_views(ctx,state.as_ref(),false);

        // The globe turns continuously; without it, the clock sets the pace.
        let pace = if open.right && self.config.globe_rotation { 66 } else { 500 };
        ctx.request_repaint_after(Duration::from_millis(pace));
    }
}

impl App {
    /// Answers what `des-ui-ctl` asked. Locked, only the lock itself, `ping` and
    /// `state` are allowed, so no script opens anything over the lock screen.
    fn answer_control(&mut self) {
        use control::{Answer, Request};
        let requests = self.control.as_mut().map(|control| control.take()).unwrap_or_default();
        for (request, reply) in requests {
            let locked = self.lock_confirmed || self.unlock_pending;
            let compositor = self.desktop.as_ref().is_some_and(Desktop::connected);
            let answer = match request {
                Request::Ping => Answer::Ok,
                Request::State => Answer::Json(control::state_json(
                    self.desktop.as_ref().filter(|desktop| desktop.connected()).map(|desktop| &desktop.state),
                    locked,
                )),
                Request::Lock => {
                    if !locked { self.lock_now(); }
                    Answer::Ok
                }
                _ if locked => Answer::Error("locked".into()),
                Request::Dock | Request::Screenshot(_) | Request::ScreenshotFile(_) | Request::PickColor if self.capture.marking() => Answer::Error("a screenshot is being marked".into()),
                Request::Workspace(_) | Request::Panel(_) | Request::Screenshot(_) | Request::ScreenshotFile(_) | Request::PickColor if !compositor => Answer::Error("no compositor".into()),
                // These two are answered once the user has chosen, so the reply is held.
                Request::ScreenshotFile(_) if !self.pending_shot_replies.is_empty() => Answer::Error("busy".into()),
                Request::PickColor if self.pending_color.is_some() => Answer::Error("busy".into()),
                Request::ScreenshotFile(mode) => {
                    self.pending_shot_replies.push(reply);
                    self.send(Command::Shot(mode));
                    continue;
                }
                Request::PickColor => {
                    self.pending_color = Some(reply);
                    self.capture.pick_colour_next();
                    self.send(Command::Shot(des_common::ipc::ShotMode::Region));
                    continue;
                }
                Request::Workspace(n) => { self.send(Command::Workspace(n)); Answer::Ok }
                Request::Toast(text) => { self.notices.post("des-ui-ctl", &text, false); Answer::Ok }
                Request::Dock => { self.dock.reveal_keyboard(); Answer::Ok }
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
            if self.pending_shot_replies.is_empty() {
                break;
            }
            self.pending_shot_replies.remove(0).send(control::shot_answer(&result));
        }
        if called_off {
            for reply in self.pending_shot_replies.drain(..) {
                reply.send(control::Answer::Error("cancelled".into()));
            }
            if let Some(reply) = self.pending_color.take() {
                reply.send(control::colour_answer(None));
            }
        }
    }
}
