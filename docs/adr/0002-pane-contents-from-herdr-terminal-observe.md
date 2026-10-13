# Pane contents come from `herdr terminal session observe`

warpr renders a pane by running `herdr terminal session observe <pane> --cols N --rows N` as a subprocess and feeding each frame's decoded `bytes` into Warp's existing VT grid. Frames are JSON lines (`type: "terminal.frame"`, `seq`, `full`, `width`, `height`, `encoding: "ansi"`, base64 `bytes`): the first is a full repaint, later ones are incremental cursor-positioned repaints. This avoids both polling `pane.read` snapshots and speaking Herdr's undocumented binary protocol.

If `herdr status` reports the private protocol as incompatible with the installed binary, warpr refuses to connect and says why, rather than degrading to the public API.

## Considered Options

- Poll `pane.read --format ansi` over the public API: stable but not live, and weak for mouse, alt-screen and cursor fidelity.
- Speak Herdr's numbered binary protocol directly: same-install only and undocumented, so it would lock warpr to exact Herdr builds.
- Attach a Herdr client in a PTY and render that: faithful, but it brings the client's prefix key and single-owner input contention into every view.

## Consequences

- Frames were observed streaming over plain pipes (held-open and `/dev/null` stdin); no PTY is needed per pane.
- Input ownership is not yet exercised: `terminal session control` exited with `terminal.closed` / `detached` when given `/dev/null` as stdin, so the take-control path needs its own spike before the control slice.
- Wide characters, alt-screen applications and Kitty images through `observe` are unverified.
- warpr must pass an explicit socket for the chosen session; an inherited `HERDR_SOCKET_PATH` points at whichever session the launching shell lives in.
