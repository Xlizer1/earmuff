# Architecture

Earmuff follows clean architecture. Each layer is its own crate, and dependencies only
point inward, so the compiler rejects an inner layer that reaches for an outer one.

```
earmuff (bin)  ──►  earmuff-infrastructure  ──►  earmuff-protocols
     │                     │                           │
     └──────────────►  earmuff-application  ──►  earmuff-domain  ◄──┘
```

| Crate | Layer | What it holds | Depends on |
|---|---|---|---|
| `earmuff-domain` | entities | headsets, capabilities, settings, status, events; validated values like `Percent` | nothing |
| `earmuff-application` | use cases | the ports (`Backend`, `Session`), the `DeviceManager`, one-shot use cases for scripts | domain |
| `earmuff-protocols` | adapters (pure) | byte codecs for natively driven headsets | domain |
| `earmuff-infrastructure` | adapters (I/O) | hidraw access, the native backend, simulated demo headsets | application, protocols, domain |
| `earmuff` | presentation + composition root | window, tray icon, command line; picks the backends | everything |

No crate uses `unsafe`: the workspace forbids it.

## How a setting reaches the headset

1. The window turns a switch into a `Setting` and hands it to the `DeviceManager`.
2. The manager passes it to the headset's own worker thread and wakes it.
3. The worker calls `Session::apply`. The native session asks the codec for the
   reports, queues them, and sends one at a time, waiting for each answer.
4. Answers, and anything the headset reports on its own (a button press, the
   battery), come back from `Session::poll` as domain `Event`s.
5. The worker forwards the events that changed something as `AppEvent::Changed`.
   The window folds them into its view model and redraws.

The GTK main loop never waits on a headset: the manager's threads do the waiting,
and a `Waker` (an eventfd for hidraw) interrupts a waiting poll as soon as there is
something to send.

## Backends

- **Native** (`NativeBackend`): finds dongles in `/sys/class/hidraw` whose USB ID has
  a codec in `earmuff-protocols`, and talks to them through `/dev/hidraw*`.
- **HeadsetControl** (`HeadsetControlBackend`): runs the `headsetcontrol` program
  and reads its JSON API (1.x), for the ~40 models it supports. Discovery only runs
  it again when the hidraw devices change; sessions poll it for the battery,
  connection, ChatMix and sidetone, and report write-only settings as set once
  HeadsetControl confirms them. A headset both backends reach stays native.
- **Demo** (`DemoBackend`): a byte-level simulator of the Cloud Alpha Wireless,
  driven through the same native session and codec as the real headset, and a
  "headset with every feature" simulated at the domain level. Run `earmuff --demo`
  to try them; the tests use them everywhere.

## Tests

| Kind | Where | What runs |
|---|---|---|
| Unit | `#[cfg(test)]` in every crate | value rules, codecs against captured bytes, use cases and the manager with fakes, view models, CLI formatting |
| Integration | `crates/earmuff-infrastructure/tests/` | native session + codec + use cases against the simulator; the manager against a demo headset; discovery in a fake sysfs; the HeadsetControl backend against the real program's mock headset |
| End to end | `crates/earmuff/tests/` | the `earmuff` binary (`cli.rs`); the real window driven like a user (`window.rs`); the tray icon over D-Bus (`tray.rs`) |
| Hardware | `crates/earmuff-infrastructure/tests/hardware.rs` | opt-in, against a real Cloud Alpha Wireless; puts every setting back afterwards |

`window.rs` and `tray.rs` re-run themselves as a child process with its own headless
display (GTK's Broadway backend, `gtk4-broadwayd`), its own private D-Bus daemon and
its own runtime directory, so they never show a window or touch the real session.
`tray.rs` registers a fake StatusNotifierWatcher on that bus and drives the icon the
way Waybar's tray does.
