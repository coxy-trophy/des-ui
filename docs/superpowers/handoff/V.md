# Handoff V: system tray

Done on the branch: `shell/src/tray.rs` — a StatusNotifierWatcher (owned when no one
else has the name, otherwise the shell follows the existing one), a host
(`org.kde.StatusNotifierHost-<pid>`), item reading through one `Properties.GetAll`,
pixmaps or icon names (an item's `IconThemePath` first, then the shared
`app_icons::IconCache`), attention icons, tooltips, and dbusmenu menus. All D-Bus work
is on one thread plus small signal-listener threads; calls time out after 2 s.

Already wired (beyond the `mod` line, so that nothing is dead code):
- `App.tray: tray::Tray`, started in `App::new` with `tray::Tray::start(ctx.clone())`.
- `panels::status` takes `tray: Option<&tray::Tray>` and draws the icons left of the
  readings; the call in runtime.rs passes `Some(&self.tray)`.
- `app_icons::resolve` is `pub(crate)` so an item's own icon folder can be searched.

Clicks: left `Activate(x, y)` (or the menu, for `ItemIsMenu`), middle
`SecondaryActivate`, wheel `Scroll(delta, "vertical")`, right the menu (`AboutToShow`,
then a fresh `GetLayout`). A menu entry sends `Event(id, "clicked", 0, time)`.

Not done: icons drawn from SVG files fall back to the generic symbol, as elsewhere in
the shell (the image crate here reads PNG, JPEG and GIF). Passive items are hidden.

## README paragraph (under "What is in it")

> **The tray.** Applications that keep an icon in a tray (Discord, Steam, Nextcloud,
> Telegram, `nm-applet --indicator`) show it in the status strip, left of the readings.
> A click does what the application says, a right-click opens its menu, the middle
> button its second action and the wheel scrolls it. An icon asking for attention is
> underlined in the accent.

## For docs/UNTESTED.md

- [ ] Discord, Steam, Nextcloud, Telegram and `nm-applet --indicator` each show an icon,
      with the right picture and tooltip
- [ ] Left click opens or raises the application; right click opens its menu; entries
      with submenus, check marks and radio buttons work and close the menu
- [ ] An item that changes its icon or status (Nextcloud syncing, a new Telegram
      message) updates within a second; a Passive one disappears
- [ ] Quitting an application removes its icon; starting it again brings it back
- [ ] Restarting the shell (Super+Shift+R) brings every icon back without restarting
      the applications
- [ ] With another watcher running (e.g. `snixembed` or a KDE session's), the shell
      shows the same icons as a host
