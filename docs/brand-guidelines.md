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
- Sentence case for everything: titles, buttons, menu items. Capitals for headings,
  badges and the banner come from CSS (`text-transform`), never from the text, so screen
  readers read words.
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

Dark, deep charcoal surfaces with one glowing accent. Every color is a libadwaita CSS
variable in `data/style.css`; code that draws (the battery ring, graph, equalizer preview)
takes its color from CSS, never a literal.

| Role | Value |
|---|---|
| Window / headerbar and sidebar / cards | `#0f1015` / `#0b0c10` / `#181a21` |
| Text | `#eceef4` |
| Accent text and lines (red, the default) | `#ff5a5f` |
| Accent fill (red) | `#e3242b` |
| Other accents (text / fill) | cyan `#3fd4ec` / `#0098b3`, green `#4fdd7c` / `#1c9a45`, purple `#b18cff` / `#7c3aed`, orange `#ff9d42` / `#df6108` |
| Low battery | libadwaita's `--warning-color` |

- The accent is the user's choice; nothing may depend on it being red. States that must
  stand out (a low battery) use libadwaita's warning and error colors instead.
- Accent text is at least 5.7:1 against cards. White on the cyan, green and orange
  fills is only about 3.5:1, so don't put text on an accent fill.
- Brand color outside the app (AppStream banners): `#ffc2c4` in light, `#4a0f13` in dark.

## Type

The system's UI font, in heavy weights: 800–900 for headings and the banner, with wide
letter-spacing (0.08–0.16 em) on capitals. Body text stays regular; the dimmed text is
libadwaita's.

## Icon

A headset with a mic boom: red on a dark tile for the app icon, a single-color glyph for
the symbolic icons (the banner, the Audio tab and panels).

- `data/icons/scalable/apps/io.github.xlizer1.Earmuff.svg`: the app icon
- `…-symbolic.svg` and `data/icons/scalable/actions/earmuff-headset-symbolic.svg`: the glyph
- `data/icons/tray/earmuff-64.png`: the tray picture, rendered from the app icon with
  `rsvg-convert -w 64 -h 64`; render it again whenever the icon changes
- Symbolic icons use filled shapes only (GTK recolors fills), and must read at 16 px.

## Screenshots

`make screenshots` retakes the README's pictures with the demo headsets on the default
red accent. Retake them whenever the window changes, and don't edit them by hand.
