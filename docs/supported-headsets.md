# Supported headsets

## Driven by Earmuff itself

These use Earmuff's own driver: changes and button presses show up instantly, and
every setting is read back from the headset.

| Headset | USB ID |
|---|---|
| HyperX Cloud Alpha Wireless | `03f0:098d` |

## Through HeadsetControl

With [HeadsetControl](https://github.com/Sapd/HeadsetControl) installed, Earmuff also
drives every headset it supports. Earmuff asks HeadsetControl for the battery every
few seconds (every second on headsets with a ChatMix dial). Some settings can only
be written through HeadsetControl, so Earmuff shows them as set once the headset
accepted them.

This list comes from HeadsetControl 4.1.0 (`headsetcontrol -u`); newer versions of
HeadsetControl support more headsets, and Earmuff picks those up without changes.

| Headset | USB IDs |
|---|---|
| Audeze Maxwell | `3329:4b19`, `3329:4b18` |
| Audeze Maxwell 2 | `3329:4b29`, `3329:4b28` |
| Corsair Headset Device | `1b1c:1b1c`, `1b1c:1b27`, `1b1c:0a14`, `1b1c:0a16`, `1b1c:0a17`, `1b1c:0a1d`, `1b1c:0a1a`, `1b1c:1b2a`, `1b1c:1b23`, `1b1c:1b29`, `1b1c:0a55`, `1b1c:0a51`, `1b1c:0a52`, `1b1c:0a38`, `1b1c:0a4f`, `1b1c:0a2b`, `1b1c:0a75`, `1b1c:0a56` |
| Corsair Virtuoso XT/SE | `1b1c:0a64`, `1b1c:0a62`, `1b1c:0a3e`, `1b1c:0a3d` |
| Corsair Wireless V2 Headset Device | `1b1c:2a08`, `1b1c:2a02`, `1b1c:0a97` |
| HeadsetControl Test device | `f00b:a00c` |
| HyperX Cloud 3 | `03f0:089d`, `03f0:03cc` |
| HyperX Cloud Alpha Wireless | `03f0:098d` |
| HyperX Cloud Flight Wireless | `0951:16c4`, `0951:1723` |
| HyperX Cloud II Wireless | `03f0:0696` |
| HyperX Cloud II Wireless (Kingston) | `0951:1718` |
| Lenovo Wireless VoIP Headset | `17ef:a07d` |
| Logitech ASTRO A50 Gen 5 | `046d:0b1c` |
| Logitech G PRO Series | `046d:0aa7`, `046d:0aaa`, `046d:0aba`, `046d:0afb`, `046d:0afc` |
| Logitech G PRO X 2 LIGHTSPEED | `046d:0af7` |
| Logitech G431/G432/G433 | `046d:0a9b`, `046d:0a9c`, `046d:0a6d` |
| Logitech G522 LIGHTSPEED | `046d:0b18` |
| Logitech G533 | `046d:0a66` |
| Logitech G535 | `046d:0ac4` |
| Logitech G633/G635/G733/G933/G935 | `046d:0a5c`, `046d:0a89`, `046d:0a5b`, `046d:0a87`, `046d:0ab5`, `046d:0afe`, `046d:0b1f` |
| Logitech G930 | `046d:0a1f` |
| Logitech Zone Wired/Zone 750 | `046d:0aad`, `046d:0ade` |
| MCHOSE X9 Wireless | `3837:6045` |
| Plantronics Voyager 8200 UC (BT600) | `047f:02ee` |
| ROCCAT Elo 7.1 Air | `1e7d:3a37` |
| ROCCAT Elo 7.1 USB | `1e7d:3a34` |
| Sony INZONE Buds | `054c:0ec2` |
| Sony INZONE H5 | `054c:0ebf` |
| SteelSeries Arctis (1/7X/7P) Wireless | `1038:12b3`, `1038:12b6`, `1038:12d7`, `1038:12d5` |
| SteelSeries Arctis (7/Pro) | `1038:1260`, `1038:12ad`, `1038:1252`, `1038:1280` |
| SteelSeries Arctis 7+ | `1038:220e`, `1038:2212`, `1038:2216`, `1038:2236` |
| SteelSeries Arctis 9 | `1038:12c2` |
| SteelSeries Arctis GameBuds | `1038:230a` |
| SteelSeries Arctis Nova (5/5X) | `1038:2232`, `1038:2253` |
| SteelSeries Arctis Nova 3 | `1038:12ec` |
| SteelSeries Arctis Nova 3P Wireless | `1038:2269`, `1038:226d` |
| SteelSeries Arctis Nova 7 | `1038:2202`, `1038:22a1`, `1038:227e`, `1038:2206`, `1038:2258`, `1038:229e`, `1038:22ad`, `1038:223a`, `1038:22a9`, `1038:227a`, `1038:22a4`, `1038:22a5` |
| SteelSeries Arctis Nova 7P | `1038:220a`, `1038:22a7`, `1038:2298` |
| SteelSeries Arctis Nova Pro Wireless | `1038:12e0`, `1038:12e5` |
| SteelSeries Arctis Pro Wireless | `1038:1290` |

## Missing yours?

[Open an issue](https://github.com/Xlizer1/earmuff/issues/new?template=headset-support.yml)
with the output of `earmuff list` and `lsusb`. If you can, also run
`earmuff sniff VID:PID` (with the headset's USB ID from `lsusb`) while you press the
headset's buttons, turn it off and on, and change its settings in the vendor's app.
It prints every report the headset sends, without sending it anything; that's how a
native driver gets written (see [docs/protocols](protocols)). Your headset's dongle
may only be readable with `sudo` until Earmuff knows it.
