# Contributing

Thanks for helping. Bug reports, headset captures and code are all welcome.

## Asking for a headset

Most of the work in supporting a new headset is learning what it sends and wants.
[Open a "Support my headset" issue](https://github.com/Xlizer1/earmuff/issues/new?template=headset-support.yml)
with the output of `earmuff sniff VID:PID`: it prints every report the headset sends,
in hex, without sending it anything. Press its buttons, turn it off and on and plug
in the charger while it runs, and say what you did when.

Captures of the vendor's own app changing a setting help most. On Windows,
[Wireshark](https://www.wireshark.org/) with USBPcap records them.

## Setting up

You need Rust 1.88 or newer, GTK 4.16, libadwaita 1.5 and `glib-compile-resources`.
The end-to-end tests also need `gtk4-broadwayd` (part of GTK 4) and `dbus-daemon`,
and the HeadsetControl tests need `headsetcontrol`. On Arch:

```
sudo pacman -S rust gtk4 libadwaita headsetcontrol
```

Then:

```
cargo run -- --demo         # the window, with simulated headsets
cargo run -- --demo status  # the command line, likewise
make check                  # everything CI runs
```

## Before you send a change

- `make check` passes: formatting (`cargo fmt`), lints (`cargo clippy -D warnings`)
  and every unit, integration and end-to-end test.
- New behavior comes with tests at the layer it lives in. Most logic belongs in
  `earmuff-application` or the view model (`crates/earmuff/src/ui/view_model.rs`),
  where it's tested without GTK or hardware; the window and CLI tests then check that
  it's wired up.
- Code follows the layers in [docs/architecture.md](docs/architecture.md): inner
  crates never depend on outer ones, and the outside world is reached through a port.
- No `unsafe`; the workspace forbids it.
- If the window changed, `make screenshots` retakes the README's pictures.

## Commits

Commits follow [Conventional Commits](https://www.conventionalcommits.org/):
`type(scope): summary`, in the imperative and lower case, for example
`feat(ui): add battery history graph` or `fix(infrastructure): send a report again
when the dongle drops it`. Types are `feat`, `fix`, `docs`, `test`, `refactor`,
`perf`, `build`, `ci` and `chore`. One change per commit; the body says why.

## Adding a native driver

A native driver is a `Codec` in `earmuff-protocols`: pure functions from queries and
settings to reports, and from reports to events, unit-tested against captured bytes.
[docs/protocols](docs/protocols) has the Cloud Alpha Wireless as an example. Register
the codec in `earmuff_protocols::find`, add the USB ID to the udev rule in `data/udev`,
and the headset to [docs/supported-headsets.md](docs/supported-headsets.md).

## License

By contributing, you agree that your contributions are licensed under the
GPL-3.0-or-later, like the rest of Earmuff.
