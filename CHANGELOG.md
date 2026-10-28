# Changelog

Notable changes to Earmuff. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [0.1.0] - 2026-10-28

The first release.

### Headsets

- HyperX Cloud Alpha Wireless, with Earmuff's own driver: changes and button presses
  show up instantly, and every setting is read back from the headset. A command the
  dongle drops is sent again.
- About 40 more models through HeadsetControl, when it's installed.
- `earmuff --demo`, with a simulated Cloud Alpha Wireless and a headset with every
  feature.

### The window

- A dark gaming look with five accent colors: a sidebar with every connected headset,
  a banner with the battery ring, and Audio, Microphone, Lighting and Power tabs with
  only the controls the headset has.
- Mic mute, volume, noise filter, mute-when-raised and the mute light; mic monitoring
  and its level; ChatMix; the volume limiter; lights; auto power-off; voice prompts;
  the notification sound; the Bluetooth options.
- The headset's equalizer: its presets, and a curve editor with a live preview.
- A software equalizer for headsets without one, running in PipeWire.
- A mic test with a level meter that records five seconds and plays them back.
- Battery history for the last day or week, and how long the battery has left.
- The headset's model, USB ID and how Earmuff reaches it.

### Around it

- A tray icon with the battery, mic mute, Open and Quit. Closing the window keeps
  Earmuff in the tray; without a tray, it quits.
- Notifications when the battery runs low and when it's charged.
- Each headset's settings are remembered and put back when it reconnects.
- Starting at login, in the tray.
- Optionally switching the computer's speakers and mic to a headset when it turns on.

### Command line

- `list`, `status`, `set`, `play-sound` and `monitor`, with `--json` where it helps.
- `set mute toggle` for keyboard shortcuts.
- `waybar`, for a Waybar module.
- `sniff`, which prints every report any USB device sends, for supporting new headsets.

[0.1.0]: https://github.com/Xlizer1/earmuff/releases/tag/v0.1.0
