# Earmuff

Battery and settings for your gaming headset on Linux, in place of vendor apps
like HyperX NGENUITY. Written in Rust with GTK4 and libadwaita.

## Supported headsets

| Headset | Connection |
|---|---|
| HyperX Cloud Alpha Wireless | USB dongle (03f0:098d) |

Support for many more headsets is on the way.

## What it does

Run `earmuff`, or open "Earmuff" from the app menu:

- battery level and charging state
- mic mute, in sync with the button on the headset
- mic monitoring (hearing your own voice) on/off and level
- auto power-off time and voice prompts

Earmuff puts a headset icon in the system tray (Waybar's tray, KDE, or GNOME with the
AppIndicator extension). Hover it for the battery; right-click for mic mute, Open and
Quit. Like Discord, closing the window only hides it, and the app keeps running in
the tray until you pick Quit. On a desktop without a tray, closing the window quits.

## Command line

```
earmuff status [--json]           battery and all settings
earmuff sidetone on|off|0-100%    mic monitoring
earmuff auto-off MINUTES          0-255, 0 = never
earmuff voice-prompts on|off
earmuff mute on|off
earmuff waybar                    JSON for a Waybar custom module
earmuff monitor [--raw]           print what the headset reports, live
```

`monitor --raw` prints every report in hex: leave it running, press a button or change
a setting, and watch which bytes move. That's how new headsets get supported; see
[docs/protocols](docs/protocols).

## Install

You need Rust 1.88 or newer, GTK 4 and libadwaita 1.4 or newer.

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
