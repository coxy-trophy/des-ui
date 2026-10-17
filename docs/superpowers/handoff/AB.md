# Handoff AB: quick-look and dragging files out of the file panel

Wired already: `shell/src/quicklook.rs`, the file panel's selection and keys
(`panels.rs` FsView), `Command::DragFiles` in `common/src/ipc.rs`, the compositor's
drag source in `compositor/src/dnd.rs` (applied in `policy.rs` pump_ipc), and the
runtime calls. Nothing further for the parent to wire.

## Behaviour change

A single click on a **file** in the bottom panel no longer types its path into the
terminal: it picks the file and gives the panel the keyboard. A double click (or Enter)
types the path as a click used to. Folders and `..` still `cd` on a single click.

## Deviations from the brief

- The drag line is bounded to 4,000 bytes, not 64 KiB: the compositor drops any IPC
  line of 4 KiB or more and disconnects the client, and raising that bound would loosen
  the authenticated channel for every command. One file per drag (the panel has one
  selection); the protocol carries up to 64.
- No WebP previews: the `image` crate's `webp` feature would pull a new dependency
  that is not in the lockfile. PNG, JPEG and GIF are shown.
- No drag icon: the pointer carries the drag without a picture.
- X11 targets: smithay 0.7's XWM does not bridge Wayland drags to X11 windows, so a
  drop works on Wayland applications only.

## README paragraph (under "The terminal", after the file panel)

> **Files.** A click on a file in the bottom panel picks it; **Space** shows it over the
> workspace (pictures, text and source, a PDF's first page with poppler-utils, what a zip
> or tar archive holds, how big a folder is, or at least what a file is), **← →** go
> through the folder, **Space** or **Escape** closes. **Enter** or a double click types
> its path into the terminal, as a click on a folder `cd`s into it. Drag a file out of
> the panel and drop it on an application to open or attach it. Nothing is ever run:
> the preview reads at most 50 MB, and the only helpers it uses are `file`,
> `pdftoppm`, `unzip` and `tar`, each with the file as an argument.

## For docs/UNTESTED.md

- [ ] Space on a PNG, a large JPEG, a GIF, a source file, a 10,000-line log, a PDF
      (with and without poppler-utils), a zip, a .tar.xz, a folder of 50,000 files and a
      binary; ← → through a folder of mixed files; Escape returns keys to the terminal
- [ ] The preview over a workspace showing an application: the terminal comes into
      view under it
- [ ] Drag a file from the panel into Firefox (an upload field), Nautilus, a GTK4 text
      editor and a Qt application; the drop arrives as the file, not as text
- [ ] Drag a file named with spaces and non-ASCII letters
- [ ] Drag onto an X11 application (expected: no drop) and onto the shell itself
- [ ] Locking while dragging: the drag is cancelled, nothing is dropped
- [ ] Clicking a file then typing in the terminal after a click there: keys reach the
      terminal again
