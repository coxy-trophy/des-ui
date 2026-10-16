# Handoff J: the frame follows the machine you are in

Done on the branch: `shell/src/remote.rs` (ssh command-line parsing, one bounded `ssh`
call every 2 s with BatchMode and a private ControlMaster socket, the fixed POSIX script
and its parser, rates from two samples, states), `SysMon::set_remote` and
`SysMon::uptime_shown` (sysmon.rs), `TermTab::foreground_command_line` (term.rs),
`Geo::set_remote_connections` (geo.rs), and the left column's REMOTE heading plus a quiet
LOCAL over the controls (panels.rs).

## To wire in the shell

1. `App` gets `remote: remote::Remote` (`remote::Remote::new()` in `App::new`).
2. In `runtime.rs`'s `update`, after `self.mon.tick();`:

   ```rust
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
   ```

   `follow_tab` looks at the tab at most twice a second, so this is cheap per frame;
   following stops (and the ControlMaster is closed with `ssh -O exit`) when the tab
   changes, ssh exits, or it becomes a tunnel. Until wired, rustc reports remote.rs as
   dead code.

## README paragraph (under "The terminal")

> When the terminal in use is running `ssh`, the side panels follow it: the processor,
> memory, processes, network, disks and the globe's connections are that machine's,
> under a REMOTE heading with its name. The clock, battery, volume and brightness stay
> yours (marked LOCAL). It asks the host every two seconds with one short `ssh` call of
> its own, reusing a private control connection, and never with a password: a host you
> reach only by typing a password shows NEEDS A KEY and the panels stay on this machine.
> Tunnels (`ssh -N`, `ssh -W`) are not followed. Temperatures, fans and the GPU are not
> read over ssh, so those sections fold away.

## For docs/UNTESTED.md

- [ ] `ssh` to a Linux host with a key or an agent: REMOTE heading, its processes, memory
      and network within a few seconds; back to this machine when ssh exits
- [ ] A password-only host shows NEEDS A KEY and the panels stay local; no password prompt
      appears anywhere
- [ ] A host that is down shows UNREACHABLE and is asked again only every 10 s
- [ ] A non-Linux host (macOS, a router) shows NOT LINUX
- [ ] Two tabs on two hosts: switching tabs switches the panels
- [ ] The globe draws the remote machine's public connections
- [ ] `ssh -N -L …` and `ssh -W` are not followed
- [ ] No `ssh-*` control sockets are left in `$XDG_RUNTIME_DIR/des-ui/` a minute after
      leaving
