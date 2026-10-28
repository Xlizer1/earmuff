# Brand guidelines

How Earmuff looks and talks, so the window, tray, notifications, command line and docs
stay one product. The colors themselves live in [`data/style.css`](../data/style.css);
this page says how to use them.

## Name and tagline

- **Earmuff**, one word, capital E. Never "EarMuff" or "earmuff" in prose (the binary and
  the package are `earmuff`).
- Tagline: **Battery and settings for your gaming headset, in place of vendor apps like
  HyperX NGENUITY.** Used as is in the README, the About dialog, the package and
  Cargo.toml. The desktop file and AppStream summary use its first half.
- Earmuff isn't affiliated with any headset maker, and says so wherever it names their
  products (README, About, AppStream).

## Voice

Plain, friendly and direct, like a friend who knows headsets.

- Talk to the user ("Hear your own voice in the headset while you talk"), in short
  sentences.
- Say what happens, not how it works: "Runs on this computer", not "PipeWire filter chain".
  Technical names belong in docs and the command line.
- Sentence case for everything: titles, buttons, menu items. The capitals on panel
  titles and the status badge come from CSS (`text-transform`), never from the text, so
  screen readers read words.
- Failures start with "Couldn't …", then the reason: "Couldn't switch the sound: …".
- `…` (one character) for something in progress: "Connecting…", "Reading the battery…".

### Words we use

| Say | Not | Notes |
|---|---|---|
| headset | headphones | it has a mic |
| mic | microphone | "Microphone" only as the tab and section name |
| USB dongle | receiver, adapter | |
| mic monitoring | sidetone | the CLI accepts both: `set sidetone`, `set mic-monitoring` |
| turn off when not in use | auto-off | in the window; "auto power-off" in lists of features, `auto-off` on the command line |
| fully charged | full, charged | |
| ChatMix | chat mix, chatmix | |
| software equalizer | DSP, filter chain | |

## Color

Graphite surfaces with one glacier-teal accent, used only for what's active: the chosen
preset, a switch that's on, a value, the curve. No glows and no gradients; depth comes
from the surface steps and hairlines. Every color is a libadwaita CSS variable in
`data/style.css`, and code that draws (the battery ring, graph, equalizer curve) takes its
color from CSS, never a literal.

| Role | Value |
|---|---|
| Window / header / panels / controls in panels | `#0d1012` / `#0b0e10` / `#14181b` / `#1a1f23` |
| Hairlines (panel edges, dividers) | white at 6% |
| Text / dimmed text | `#e6ecee` / the same at 60% |
| Accent: text, lines, the curve | `#3ddcc4` |
| Accent fill (switches), with text on it | `#19b3a0`, text `#04201c` |
| Low battery / muted mic | libadwaita's `--warning-color` / `--error-color` |

- There's one accent, Earmuff's own, away from the vendors' reds, oranges and blues.
- States that must stand out use the warning and error colors, never the accent.
- Accent text is above 10:1 against panels; text on the accent fill is dark, not white.
- Brand color outside the app (AppStream banners): `#c4f1e9` in light, `#10302b` in dark.

## Type

IBM Plex Sans, bundled with the app (data/fonts, OFL), so it looks the same on every
desktop. Big numbers at 600 with tabular figures; panel titles at 12 px, 600, in capitals
with 0.06 em spacing and dimmed; row titles at 500; body text regular.

## Layout

One page of panels in a six-column grid, placed by the view model from what the headset
has, never from its state. Panels in a row share its height, so their edges line up, and
a control never appears, disappears or changes size when the user acts on it: inactive
controls are dimmed, not hidden. Narrow windows get one column.

## Icon

The Equalizer E: an E built from three level bars, Earmuff's initial and what it does.
Teal on a graphite tile for the app icon and the tray; a one-color glyph for the
symbolic icon and the header. The headset glyph stays for "headset" itself (the empty
pages).

- `data/icons/scalable/apps/io.github.xlizer1.Earmuff.svg`: the app icon
- `…/io.github.xlizer1.Earmuff-symbolic.svg`: the mark as a glyph
- `data/icons/scalable/actions/earmuff-headset-symbolic.svg`: the headset glyph
- `data/icons/tray/earmuff-64.png`: the tray picture, rendered from the app icon with
  `rsvg-convert -w 64 -h 64`; render it again whenever the icon changes
- Symbolic icons use filled shapes only (GTK recolors fills), and must read at 16 px.

## Screenshots

`make screenshots` retakes the README's pictures with the demo headsets: each a whole
dashboard. Retake them whenever the window changes, and don't edit them by hand.
