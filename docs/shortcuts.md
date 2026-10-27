# Keyboard shortcuts

`earmuff set mute toggle` mutes the headset's mic if it's on, and turns it back on if
it's muted. It prints `Mic muted` or `Mic on`, so you can pass that on to a
notification. Bind it to a key on your desktop like this. `XF86AudioMicMute` is the
mic-mute key many keyboards have; any other key works too.

Add `-d VID:PID` (from `earmuff list`) if you have more than one headset.

The mute goes to the headset itself, the same as its own mute button, so the mute light
and the voice prompt follow, and the Earmuff window and tray update too.

## Hyprland

In `~/.config/hypr/hyprland.conf`:

```
bind = , XF86AudioMicMute, exec, earmuff set mute toggle
# or with a notification
bind = SUPER SHIFT, M, exec, notify-send "$(earmuff set mute toggle)"
```

## Sway

In `~/.config/sway/config`:

```
bindsym XF86AudioMicMute exec earmuff set mute toggle
```

## GNOME

Settings → Keyboard → View and Customize Shortcuts → Custom Shortcuts → Add Shortcut.
Name it "Headset mic mute", set the command to `earmuff set mute toggle`, and pick a key.

## KDE Plasma

System Settings → Keyboard → Shortcuts → Add New → Command or Script. Set the command
to `earmuff set mute toggle` and pick a key.

## Headsets that don't report their mute

Some headsets that Earmuff reaches through HeadsetControl can be muted but don't say
whether they are. `toggle` can't work for those, and says so. Bind two keys instead:
`earmuff set mute on` and `earmuff set mute off`.
