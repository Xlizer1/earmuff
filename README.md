# Earmuff

Battery and settings for the HyperX Cloud Alpha Wireless on Linux, in place of
HyperX NGENUITY. Written in Rust with GTK4 and libadwaita; it talks to the USB
dongle directly.

Run `earmuff` (or "Earmuff" in the app menu) for the window:

- battery level and charging state
- mic mute (follows the button on the headset)
- mic monitoring (sidetone) on/off and level
- auto power-off time, voice prompts

It puts a headset icon in the system tray (Waybar's tray, KDE, GNOME with the
AppIndicator extension). Hovering shows the battery; right-click for mic mute,
Open and Quit. Like Discord, closing the window only hides it: the app keeps
running in the tray until you pick Quit. On a desktop without a tray, closing
the window quits.

## Command line

```
earmuff status [--json]      battery and all settings
earmuff sidetone on|off|-4..18
earmuff auto-off MINUTES     0-255, 0 = never
earmuff voice-prompts on|off
earmuff mute on|off
earmuff waybar               JSON for a Waybar custom module
earmuff monitor [--raw]      print what the headset reports, live
```

`monitor --raw` prints every report in hex. Use it to work out new ones:
leave it running, press a button or change something, and see which bytes move.

## Protocol

Every message is a 31-byte HID report on `/dev/hidrawN` (vendor 03f0, product 098d):
`21 BB <command> <value...>`, zero-padded. The headset answers each command with the
same command byte. The dongle drops commands sent back to back, so the app waits
for each answer (or 400 ms) before sending the next one.

| Get  | Set  | Event | Meaning                                                |
|------|------|-------|--------------------------------------------------------|
| 0x03 |      | 0x24  | wireless: 1 = headset off, 2 = connected               |
| 0x05 | 0x10 | 0x22  | mic monitoring on/off                                  |
| 0x06 | 0x11 |       | mic monitoring level, signed 16-bit big endian, -4..18 |
| 0x07 | 0x12 |       | auto power-off, minutes                                |
| 0x08 |      |       | mic boom attached                                      |
| 0x09 | 0x13 |       | voice prompts on/off                                   |
| 0x0A | 0x15 | 0x23  | mic muted (a set is answered by the 0x23 event)        |
| 0x0B |      | 0x25  | battery percent                                        |
| 0x0C |      | 0x26  | 0 = on battery, 1 = charging, 2 = full                 |
| 0x0E |      |       | color: 0 black, 1 white, 2 black/red                   |

Answers to a set echo the value that was sent, not the value the headset kept,
so the app follows each set with the matching get.

Measured on this headset (2026-10-27):

- **Sidetone level is two bytes.** The level is a signed 16-bit value in bytes 3-4,
  clamped to -4..18. HeadsetControl and HyperHeadset write a single byte, so any level
  from 1 to 127 lands on the maximum and 128 on the minimum.
- Auto power-off stores any value from 0 to 255. That 0 means "never" is an
  assumption, not tested yet.
- Bytes 4-5 of the battery answer look like the battery voltage in millivolts
  (0x103a = 4154, then 4150 a few minutes later). Not confirmed.

Command bytes come from [HeadsetControl](https://github.com/Sapd/HeadsetControl) and
[HyperHeadset](https://github.com/LennardKittner/HyperHeadset), both MIT licensed.

## Code

| File              | What it does                                                  |
|-------------------|---------------------------------------------------------------|
| `src/protocol.rs` | packet bytes and parsing; no I/O, so it's what the tests cover |
| `src/device.rs`   | finds and opens the dongle, sends commands one at a time      |
| `src/cli.rs`      | the subcommands                                               |
| `src/gui.rs`      | the window                                                    |
| `src/tray.rs`     | the tray icon and its menu                                    |

## Build and install

```
cargo test
cargo build --release
install -m755 target/release/earmuff ~/.local/bin/
cp io.github.xlizer1.Earmuff.desktop ~/.local/share/applications/
```

Rebuild and run `install` again after changing the code; the window picks up the
new binary the next time it opens.

## Files outside this folder

| File                                                | Remove with                                            |
|-----------------------------------------------------|--------------------------------------------------------|
| `~/.local/bin/earmuff` (copy of the binary)   | `rm ~/.local/bin/earmuff`                        |
| `~/.local/share/applications/io.github.xlizer1.Earmuff.desktop` | `rm ~/.local/share/applications/io.github.xlizer1.Earmuff.desktop` |

Access to `/dev/hidrawN` comes from HeadsetControl's udev rule
(`/usr/lib/udev/rules.d/70-headsets.rules`), so keep the `headsetcontrol` package installed.
