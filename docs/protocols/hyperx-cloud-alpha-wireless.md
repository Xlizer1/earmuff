# HyperX Cloud Alpha Wireless

USB dongle `03f0:098d` (HP vendor ID). Codec:
`crates/earmuff-protocols/src/hyperx/cloud_alpha_wireless.rs`.

## Reports

Every message is a 31-byte HID report on the dongle's vendor interface (usage page
0xFF43): report ID `0x21`, then `0xBB`, a command byte and its value, zero-padded.
The same interface also carries report ID `0x01` (consumer control: the volume wheel
and media keys), which Earmuff ignores.

The headset answers each command with the same command byte. Answers to a SET echo
the value that was sent, not the value the headset kept, so Earmuff follows each SET
with the matching GET.

The dongle drops a command that arrives while it is still answering the previous one
(1 of 11 in a burst during testing), so Earmuff sends one command at a time and waits
for the answer, or 400 ms.

| Get  | Set  | Event | Meaning                                                |
|------|------|-------|--------------------------------------------------------|
| 0x03 |      | 0x24  | wireless: 1 = headset off, 2 = connected               |
| 0x04 |      |       | pairing ID (5 bytes)                                   |
| 0x05 | 0x10 | 0x22  | mic monitoring on/off                                  |
| 0x06 | 0x11 |       | mic monitoring level, signed 16-bit big endian, -4..18 |
| 0x07 | 0x12 |       | auto power-off, minutes                                |
| 0x08 |      |       | mic boom attached                                      |
| 0x09 | 0x13 |       | voice prompts on/off                                   |
| 0x0A | 0x15 | 0x23  | mic muted (a SET is answered with the 0x23 event)      |
| 0x0B |      | 0x25  | battery percent                                        |
| 0x0C |      | 0x26  | 0 = on battery, 1 = charging, 2 = full                 |
| 0x0E |      |       | color: 0 black, 1 white, 2 black/red                   |

Captured examples:

```
GET battery   → 21 bb 0b 63 10 3a 01        99%
SET level 40  → 21 bb 11 00 28 01           echo of what was sent
GET level     → 21 bb 06 00 12              18: what the headset kept
SET mute on   → 21 bb 23 01                 answered with the mute event
```

## Measured on the headset

- **Mic monitoring level is two bytes.** It is a signed 16-bit value in bytes 3-4,
  clamped to -4..18. HeadsetControl and HyperHeadset write a single byte, so any
  level from 1 to 127 lands on the maximum and 128 on the minimum. Earmuff maps
  0-100% onto the 23 steps.
- Auto power-off stores any value from 0 to 255. That 0 means "never" follows the
  convention of other HyperX headsets; it has not been confirmed by waiting.
- Bytes 4-5 of the battery answer look like the battery voltage in millivolts
  (0x103a = 4154, then 4150 a few minutes later). Not confirmed.

## Sources

Command bytes come from [HeadsetControl](https://github.com/Sapd/HeadsetControl)
(GPL-3.0) and [HyperHeadset](https://github.com/LennardKittner/HyperHeadset) (MIT).
The level encoding and value ranges were measured on a real headset.
