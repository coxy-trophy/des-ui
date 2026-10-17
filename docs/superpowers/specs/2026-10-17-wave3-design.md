# Wave 3: foundation, daily surfaces, identity

User request: do all of the 17 October proposal, foundation first. Same boundaries as Apex:
Signal visual language, owners in UX-CONTRACT.md, authenticated IPC, lock behaviour, bounded
off-thread work, tests with every piece, hardware cases in docs/UNTESTED.md.

## A. Foundation (what applications assume)
1. Protocols: fractional-scale + viewporter, presentation-time, cursor-shape, primary-selection,
   xdg-activation (focus on activation token, never over the lock), xdg-foreign, tearing-control,
   single-pixel-buffer, xdg-output where smithay offers them.
2. Input protocols: text-input-v3 + input-method-v2 (IME, with the popup surface placed at the
   cursor rectangle), pointer-constraints + relative-pointer (released on lock and Super keys),
   keyboard-shortcuts-inhibit (Super chords still reach the compositor for lock and session).
3. wlr-layer-shell: background, bottom, top, overlay layers on each display; exclusive zones
   shrink the workspace, never the shell's frame; overlay layer surfaces are hidden on lock.
4. Polkit authentication agent in the shell, Signal-style dialog, authenticated through
   polkit-agent-helper-1; never shown over the lock screen.
5. System tray: StatusNotifierWatcher + host; icons, tooltips, activate, context menus
   (com.canonical.dbusmenu) in the status strip.
6. XDG autostart: ~/.config/autostart and /etc/xdg/autostart honouring Hidden, OnlyShowIn,
   NotShowIn, TryExec, X-GNOME-Autostart-enabled; started once per session.

## B. Daily surfaces
Overview with live thumbnails (Super alone / three-finger swipe up), Alt+Tab by recency with
previews, resizable split divider and three-column / master-stack layouts per workspace, quick
settings pull-down and volume/brightness OSD, touchpad gestures, clock pop-up with agenda,
notification grouping/inline reply/DND schedule, file quick-look and drag-and-drop out of the
file panel.

## C. Identity
Kitty graphics and sixel images in the terminal; OSC 9;4 progress in the status strip and a
notification when a long command finishes unseen; ambient idle mode before blanking; named
workspaces with pinned folders; compositor magnifier, high-contrast palette, text scaling.
