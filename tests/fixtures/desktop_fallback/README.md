Topologies in the shape of `hyprctl -j monitors all` and `hyprctl -j workspaces` (fields the fallback reads).
Recorded by T15 on 28 Oct 2026: output names, sizes and positions as this machine's compositor reported them
(HDMI-A-1 1920x1080 at 0x0, DP-1 3440x1440 at 1920x0); the workspace placement is the arrangement the directive names.
- two-output: both on; workspaces 1, 2 on HDMI-A-1, 3, 4, 5 on DP-1.
- one-output-dp1: HDMI-A-1 turned off (disabled, power state off), workspaces still listed where they were, the
  state the handlers meet at the event.
- two-output-returned: HDMI-A-1 back on with every workspace still on DP-1.
`monitors.lua` is a copy of the Omarchy default monitor file with no Soliton block, for the replace round trip.
- two-output-dp2-asleep: both outputs listed; the second screen (HDMI-A-1, this machine's other output) asleep
  (`dpmsStatus false`, not disabled). Stands for a screen blanked by idle (DPMS off): still present.
- two-output-dp2-disabled: the same screen `disabled true`. Stands for a screen the compositor has turned off: gone.
- sleep-wake-trace.jsonl: one step per line, the outputs the compositor lists after it and the event it raises: sleep,
  wake (the second screen's power state only), unplug (the second screen no longer listed, `monitor.removed`), replug
  (`monitor.added`). A sleep or wake is given the layout event, the worst case: the handlers run and must move nothing.
