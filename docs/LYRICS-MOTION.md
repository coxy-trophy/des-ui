# Lyric motion prototype

The lyrics view remains native egui/Rust and uses the existing Spotify line timing.
There are no fabricated word timestamps and no new lyric provider or browser engine.

- Text wraps once at a fixed bold font size. Visible lines are painted with a small
  scale and brightness transition; highlighting does not reflow the song.
- A critically damped spring keeps the active line near the upper third. Its exact
  time integration gives equivalent movement at different frame rates.
- Wheel, touch and scrollbar browsing suspend following until **Return to current
  line** is selected. Browsing raises inactive text contrast.
- Clicking a timed line still seeks. Seeking or a large playback jump aligns without
  animating through the entire song; a pending target bridges playback acknowledgment.
- Highlight transitions can be interrupted by fast lines without restarting at full
  brightness. Pause and reduced motion produce a stable highlight.
- Layouts are cached for the current track/width/density. Only visible lines are
  painted. Frames are requested during transitions and near the next lyric timestamp,
  with no continuous background animation, blur pass, or backdrop rendering.

The visual reference was Apple's lyric experience. Implementation research included
AMLL's separate scroll, spring and word-emphasis components; this is an independent
native implementation, not an embedded web player or a port of that library.

Relevant references:
- https://www.apple.com/newsroom/2022/12/apple-introduces-apple-music-sing/
- https://github.com/amll-dev/applemusic-like-lyrics/tree/main/packages/core/src/lyric-player

This first pass deliberately covers line-level movement and focus. Word/syllable
highlighting, duet metadata, animated album colors and blur are outside its scope.

## Silent native preview

After `cargo build -p des-ui`, run `python3 scripts/lyrics-motion-demo.py`.
The demo uses original sample text and an isolated local music socket, configuration,
state and cache. It does not log into Spotify, emit audio, seek the real player or
attach to existing terminals. Pause/seek controls act only on the demo clock.
Use `--no-animation` for comparison or `--screenshot /tmp/lyrics.ppm` for a snapshot.
Closing the preview ends its local service and removes its temporary files.
