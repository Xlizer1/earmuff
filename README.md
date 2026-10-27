# Earmuff

Battery and settings for your gaming headset on Linux, in place of vendor apps
like HyperX NGENUITY. Written in Rust with GTK4 and libadwaita.

## Supported headsets

- **HyperX Cloud Alpha Wireless**, with Earmuff's own driver: instant updates and
  every setting read back from the headset.
- **About 40 more models** from Logitech, SteelSeries, Corsair, HyperX, Sony, Audeze,
  Jabra, Roccat and others, when [HeadsetControl](https://github.com/Sapd/HeadsetControl)
  is installed.

The full list with USB IDs is in [docs/supported-headsets.md](docs/supported-headsets.md).
No headset at hand? `earmuff --demo` runs everything with simulated ones.

## What it does

Run `earmuff`, or open "Earmuff" from the app menu. It shows what your headset has:

- battery level and charging state
- mic mute (in sync with the button on the headset), mic volume, noise filter,
  mute-when-raised and the mute light
- mic monitoring (hearing your own voice) on/off and level
- equalizer presets, ChatMix, volume limiter and lights
- auto power-off, voice prompts and the Bluetooth options

The window has a dark gaming look: a sidebar with every connected headset, a banner
with the headset's battery ring, and a tab per kind of setting. Pick the accent color
(red, cyan, green, purple or orange) in Preferences.

| Shortcut | Does |
|---|---|
| Ctrl+, | Preferences |
| F1 | About |
| Ctrl+W | Close the window (Earmuff stays in the tray) |
| Ctrl+Q | Quit |

Earmuff puts a headset icon in the system tray (Waybar's tray, KDE, or GNOME with the
AppIndicator extension). Hover it for the battery; right-click for mic mute, Open and
Quit. Like Discord, closing the window only hides it, and the app keeps running in
the tray until you pick Quit. On a desktop without a tray, closing the window quits.

## Command line

```
earmuff list [--json]             headsets Earmuff can reach
earmuff status [--json]           battery and every setting
earmuff set SETTING VALUE         e.g. set sidetone 40%, set eq-preset "Bass boost"
earmuff play-sound N              play one of the headset's notification sounds
earmuff waybar                    JSON for a Waybar custom module
earmuff monitor [--raw]           print what the headset reports, live
```

`-d VID:PID` picks a headset (see `earmuff list`); without it, Earmuff uses the
first one it finds. `earmuff set --help` lists every setting.

`monitor --raw` prints every report in hex: leave it running, press a button or change
a setting, and watch which bytes move. That's how new headsets get supported; see
[docs/protocols](docs/protocols).

## Install

You need Rust 1.88 or newer, GTK 4.16 and libadwaita 1.5 or newer.

```
make install        # into ~/.local; PREFIX=/usr/local for everyone
make install-udev   # lets you open the headset's dongle (asks for sudo)
```

`make install-udev` isn't needed if HeadsetControl is installed: its udev rule covers
the same dongles. `make uninstall` and `make uninstall-udev` remove everything again.

## Development

Earmuff follows clean architecture: a Cargo workspace whose crates only depend inward.
[docs/architecture.md](docs/architecture.md) explains the layers and the tests.

```
make check          # formatting, lints, unit, integration and end-to-end tests
make test-hardware  # tests on a real Cloud Alpha Wireless (switched on)
```

The window and tray tests run headless, on their own display and D-Bus session, so
nothing shows up on your desktop while they run.

Commits follow [Conventional Commits](https://www.conventionalcommits.org/):
`type(scope): summary`, for example `feat(ui): add battery history graph`.

## Credits

Command bytes for the Cloud Alpha Wireless come from
[HeadsetControl](https://github.com/Sapd/HeadsetControl) (GPL-3.0) and
[HyperHeadset](https://github.com/LennardKittner/HyperHeadset) (MIT).

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
