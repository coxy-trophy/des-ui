# Separated desktop panels

User direction: System is the initial right-panel tab at login. Desktop panels should have the narrow gaps, subtle outline and modest rounding shown in the supplied VS Code reference. Retain DEs-UI colours, typefaces, wallpaper and controls.

Implementation choice: 8 logical-pixel gaps and corner radius, with a 4px inner workspace rim. This keeps native application rectangles inside the rounded workspace surface without adding a compositor shader, animation or per-frame image work. Fullscreen applications and secondary display application areas remain edge-to-edge. All primary-display panel and workspace rectangles come from one common geometry owner used by both compositor and shell; collapse/hide and narrow displays remain bounded. The terminal and native app share the same inset rectangle. Left/right monitoring panels, the workspace header and Files/Chat keep existing behavior.

Alternatives considered: painting gaps in the shell alone would leave native windows misaligned; rounded GPU clipping would add rendering complexity and is unnecessary for the requested panel treatment. Shared rectangles and a restrained shell surface are the selected implementation choice.

Voice is a separate diagnostic repair. Preserve existing key storage, model choice, explicit microphone start, stop/lock behavior, billing limits and secrets redaction. Do not claim a real provider/audio connection works without evidence.
