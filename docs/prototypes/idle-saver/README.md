# Screen savers and sleep — design study

Open [index.html](index.html) locally. Choose any of the ten previews, or use
the arrow buttons to compare them in place, including in **Fullscreen**. In
the pattern studies, drag **Morph** to inspect the assembled logo, or choose
[Ink](index.html#silk-ink) and scrub from Soft to Deep. Each of the six
scenes has its own tuning slider. The lower timeline switches between example
AC and battery profiles. The page does not lock, blank or suspend the machine.

The studies use the actual DEs-UI boot wordmark from
`compositor/assets/wordmark.la`, with local Chakra Petch and JetBrains Mono
fonts. [React Bits PatternWaves](https://reactbits.dev/backgrounds/pattern-waves?color=ff00f1)
informed the appearance and pointer behavior. The prototype is its own Canvas
renderer; it installs no React, OGL, or browser runtime in the desktop. The
new scenes likewise use original Canvas drawing, with React Bits
references as starting points:

| Study | Starting point | What we changed |
| --- | --- | --- |
| [Polar Veil](index.html#polar-veil) | [Aurora](https://reactbits.dev/backgrounds/aurora) | A blended violet, blue and mint glow with the reference's soft edge, above the mountain silhouette. The light follows the pointer gently. |
| [Event Horizon](index.html#event-horizon) | [Orb](https://reactbits.dev/backgrounds/orb) | A dark eclipse with a slow ember corona, an offset highlight and sparse stars. |
| [Night Transit](index.html#night-transit) | [Hyperspeed](https://reactbits.dev/backgrounds/hyperspeed) | A rose and cyan night drive with a low skyline, curved lanes and adjustable speed. |
| [Liquid Prism](index.html#liquid-prism) | [Iridescence](https://reactbits.dev/backgrounds/iridescence) | Shaded ribbons of rose, jade and lilac glass that bend slightly under the pointer. |
| [Phosphor Sweep](index.html#phosphor-sweep) | [Radar](https://reactbits.dev/backgrounds/radar) | A green scope with fading contacts, an adjustable afterglow and a pointer marker. |
| [Rising Lines](index.html#rising-lines) | [Rising Lines](https://pro.reactbits.dev/docs/components/rising-lines) | White-tipped particles rise from the violet horizon, draw the DEs-UI logo, and drift away. Hover and click effects are for the gallery; click the fullscreen canvas to dismiss it. Reduced motion holds the assembled logo. |

## Product direction for review

Recommend **Silk Fold** as the first native saver. Keep Contour as a second
preset if it is useful for lower-power hardware; leave the third study as a
design option until we have measured it. Retain an Off choice and a static
wordmark for Reduce Motion and unsupported rendering backends. A time-only
overlay could be a separate option, with no notifications or private content.

There are three different actions to configure:

| Stage | Plugged in example | Battery example | What happens |
| --- | ---: | ---: | --- |
| Lock and show saver | 5 min | 2 min | Lock first, then show the chosen animation on the secure surface. |
| Display off | 10 min | 3 min | Stop the animation and turn physical outputs off. |
| Suspend | 30 min | 10 min | Ask logind to suspend, with the session already locked. |

Times are measured from the last activity. They're starting points, not
changes to anybody's configuration. Allow Never for each automatic stage,
show the resulting order in Settings, and flag combinations where an earlier
display-off deadline will skip the saver. The battery profile can disable the
animation entirely. Plugging or unplugging recomputes the deadlines without
unlocking or flashing the desktop.

An input event wakes the display and reveals the existing password prompt if
locked. A key or click that dismisses the saver must not land in an underlying
window or be entered as part of the password. The interactive ripples in HTML
are for preview mode. A real saver should dismiss on deliberate input instead
of trapping the cursor in an animation. Resume never restarts a prior capture
or goes back to the unlocked desktop when lock-before-sleep is enabled.

## Where it belongs

The compositor already tracks input, asks the shell to lock on idle, handles
Wayland idle inhibitors and can turn DRM displays off. The shell already
serves the D-Bus ScreenSaver inhibitor API and handles logind's sleep/lock
events. Default automatic lock is ten minutes and automatic screen-off is
disabled in the current source. The prototype's proposed profiles should
reuse these owners and preserve existing settings on upgrade.

- Give the compositor one explicit idle state machine with independent
  deadlines and a lock acknowledgement. The existing shell lock screen owns
  the password UI. Manual lock, explicit sleep, critical battery and lid
  behavior keep their own priority; distinguish idle/presentation inhibition
  from logind sleep inhibitors.
- Prefer one bounded native render pass, or a sparse batched point mesh using
  the current renderer. Use the HTML for visual tuning; a browser process and
  a full WebGL ripple simulation would add work just when the desktop is idle.
  Reuse the existing wordmark, cap animation around 24 fps on AC, and compare a
  static or lower-rate battery mode on the target laptop. Allocate resources
  on entry, release or suspend them on exit, and submit no saver frames after
  display-off or suspend.
- Validate lid and external-display combinations before changing policy for a
  docked laptop. Make presentation mode discoverable and show why an idle
  action is being held off. Test keyboard/mouse/touch wake, lock handshake,
  suspend failure, resume, display hotplug and disconnect, live inhibitors,
  and critical battery. Measure CPU, GPU and RAM with the saver idle, visible,
  and off. Confirm no pre-lock desktop frame appears on any output at wake.

## Native desktop

In **Settings → Session → Screensaver**, choose **Silk Fold · Ink**,
**Signal Contour**, **Terminal Weave**, **Night Transit**, **Rising Lines**,
**Event Horizon**, or **Polar Veil**. **Preview** plays the native scene without
locking. Click or press a key to return to Settings.

The saver runs after an automatic idle lock if the displays are still on.
Moving the pointer or using the keyboard or touch reveals the password prompt.
Manual lock and sleep use the usual lock screen. Existing timers are retained;
the saver defaults to Off. Desktop animation and Reduce motion settings apply,
and animation runs at a lower rate on battery.
