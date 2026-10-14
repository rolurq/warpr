# warpr is the layout client for the panes it shows

warpr plays the role Herdr's own terminal UI plays: it supplies the area, and therefore the cell size, for the panes it displays. Herdr keeps the layout tree and its proportions; warpr applies them to its own frames and holds each visible pane's control session at the resulting size, so the program in the pane reflows to its frame. It is not an observe-only mirror, and typing, scrolling and mouse go through the same control session with no confirmation step.

A pane that another client already owns is a contested pane. warpr shows it through the observe stream at Herdr's grid size, scaled to the frame, and marks it. Displaying or focusing a contested pane never takes control; the first input forwarded to it (a keypress, paste, scroll or mouse event delivered to its program) takes control immediately, with a visible notice and no dialog.

## Considered Options

- Observe-only, cropping or padding Herdr's grid: never touches shared state, but a sole-client session has no TUI to apply the layout, so panes keep their default PTY size while their frames follow the proportions, and the result is cropped or squashed.
- Observe with client-side scaling only: exact mirror of Herdr's grid, but a larger window never shows more columns.
- Control only after an explicit take-control action: safe by default, but the default state is wrong in the main case where warpr is the only client.

## Consequences

- Owning a pane blocks `agent attach` and `terminal attach` from another terminal unless it uses takeover, and ownership cannot be queried, so warpr cannot show who owns a pane it does not own.
- Every resize signals the program in the pane, so frame changes are debounced and snapped to whole cells.
- A controller's size outlives it: after warpr exits, panes keep the last size until a Herdr TUI client lays them out again.
- "Same layout as Herdr" means the same split structure and proportions, not identical cell sizes.
- This supersedes ADR-0002's observe-only default. The observe stream remains the fallback for contested panes, and ADR-0002's format and findings still apply.
- Layout-resize suppression, the attached-TUI interaction, resize storms and the ownership cost were observed in Spike #25 (below).

## Spike findings

Spike #25, run against Herdr 0.9.4 (protocol 22) on macOS arm64 in an isolated session `warpr-test-25` (a headless `herdr --session warpr-test-25 server`, stopped afterwards; the default session was never addressed, and every command used the isolated-socket prefix from ADR-0002). `herdr` below means that invocation. The Herdr TUI client was `herdr --session warpr-test-25` run in a Python `pty.fork()` pseudo-terminal sized with `TIOCSWINSZ` + `SIGWINCH` and rendered with `pyte`; a "controller" is `herdr terminal session control <pane> --cols C --rows R` with a held-open stdin pipe. PTY sizes were read as `TIOCGWINSZ` (rows, cols, xpixel, ypixel) by a probe program running in the pane, which also logged each `SIGWINCH`; the TUI's own layout was read from `herdr pane layout`. Not run: the warpr GUI, so none of this is verified through warpr itself.

### 1. Layout-resize suppression: confirmed, and total

Setup: `w1:p1` | `w1:p2` (`pane split --direction right`), controller on `w1:p1` at 100×30, then a TUI client attached at 200×50.

- **The owner's size is left alone.** Attached TUI layout gave `p1` a 87×49 rect and `p2` a 85×47 PTY, yet `p1` stayed `(30, 100)`. Then: `pane resize --pane w1:p1 --direction right --amount 0.2` (rects 122/52 wide), TUI resized to 150×40 (rects 87/37, area 124×39), and `pane split w1:p2 --direction down` (`p2` 37×20, `p3` 37×19). `p1` received no `SIGWINCH` through any of them and stayed `(30, 100)`.
- **Unowned panes follow the layout.** `p2`'s PTY went `47×85` → `47×50` → `37×35` → `18×35`, always the layout rect minus a 2-cell border on each axis.
- **Owner resizes still work.** `terminal.resize` 90×25 gave `(25, 90)` regardless of the layout.
- **The owner receives `full: true` frames when layout changes**, though its size is untouched (three full 100×30 frames for the three layout changes), so a controller must tolerate a full repaint at any time (ADR-0002 §1 already says so).
- **`pane list` / `pane get` report the owner's size** (`scroll.viewport_rows: 30` for `p1` while its rect is 49 rows tall), not the layout's.

### 2. A TUI client attaching to controller-owned panes

- **The controller's size wins**, whether the controller is first or the TUI is. The TUI draws the pane at the controller's size inside its rect, anchored top-left: a 30×100 pane in an 85×47 rect was cropped to 85 columns with blank rows below; a 25×90 pane was cropped the same way. It does not scale, and shows no marker, border change or notice that the pane is owned.
- **Release returns the pane to the layout.** With the TUI attached, `terminal.release` made the PTY `(37, 85, 680, 592)` immediately, and `kill -9` of the controller did the same within 0.3 s. There is no lingering size.
- **Without a TUI client the size sticks** (ADR-0002 §1, re-observed): after the TUI was killed, a controller at 100×30 with 9×18 px cells released and the PTY stayed `(30, 100, 900, 540)`, pixel fields included.
- **TUI typing is not blocked by an owner and does not evict it.** With the controller holding `p1` and a TUI focused on it, `hello-from-tui\r` typed into the TUI reached a `cat` in the pane and the controller kept streaming (no `terminal.closed`).

### 3. Resize storms

Programs (each in `w1:p1` under a recording pty wrapper that forwards `SIGWINCH` and logs output bytes): `omp` 18.10.4, `claude` 2.1.299, `codex` 0.162.1. Resizes were sent through the controller with distinct sizes, 100±15 × 30±5, and 9×18 px cells.

- **Every distinct size is a `SIGWINCH`; nothing is coalesced.** 30 resizes in 1 s and 30 over 5 s were 30 `SIGWINCH` each in all three programs. Herdr drops exact repeats: 5 identical `terminal.resize` lines gave 0 further `SIGWINCH`; a change of only `cell_*_px` gave 1.
- **Each `SIGWINCH` costs one full repaint, independent of rate.**

  | burst | `omp` | `claude` | `codex` |
  |---|---|---|---|
  | one resize | 5.1 kB, 6 writes | 1.4 kB, 3 | 2.2 kB, 18 |
  | 30 in 1 s | 77.7 kB | 36.1 kB | 29.7 kB |
  | 30 over 5 s | 109.9 kB | 37.5 kB | 30.5 kB |

  Idle (no resize) 3 s: 0 output events for `omp` and `claude`; `codex` repaints continuously (spinner), which is the only reason its idle count is nonzero.
- **No hang and no persistent corruption.** After every burst each program answered a typed `ping`, and its final screen matched the final size (separator and box rows exactly `cols` wide, footer on the last row). 200 back-to-back resizes (under 10 ms) made `omp` settle in 0.18 s; `codex` took 2.9 s to settle and a read taken at 3 s caught its footer mid-repaint (`› ←Pfor-agentse·a?lforhshortcuts`). The same read was clean on every later poll and after repeating the 200-burst and waiting 4 s, so the scramble was transient. I did not establish whether it came from `codex` or from Herdr's grid.
- **Throttling a 2 s, 60 Hz drag cuts the work without leaving the program stale.** Sending the latest size at most every *T* ms, plus one trailing send, cost (`omp`, 120 distinct sizes; `codex` in parentheses, 120 events with ~38 distinct sizes):

  | *T* | resizes sent | `omp` output | `codex` output |
  |---|---|---|---|
  | 0 (every event) | 120 | 158 kB | (29.6 kB) |
  | 100 ms | 20 | 50 kB | (15.9 kB) |
  | 200 ms | 11 | 42 kB | (9.2 kB) |
  | 300 ms | 8 | 38 kB | (7.7 kB) |

  The program settled within 0.12 s of the last send in every row, so the cost of a longer *T* is only how stale the PTY size looks mid-drag.
- **Recommendation.** Keep the layout pass and the frame scaling live, but send `terminal.resize` through a **100 ms leading-and-trailing throttle** (first change immediately, then at most one per 100 ms carrying the latest size, always one final send after the last change). It roughly thirds `omp`'s redraw volume for a 0.1 s lag; 200 ms halves it again at a visibly laggier reflow and is the fallback if a program misbehaves. **Snap to whole cells:** `cols = max(1, floor(frame_width_px / cell_width_px))`, `rows = max(1, floor(frame_height_px / cell_height_px))` (zero is rejected by Herdr), keep the remainder as padding inside the frame, and skip the send when `(cols, rows, cell_width_px, cell_height_px)` equals the last value sent. Herdr already drops exact repeats, but warpr should not spend a pipe write and a throttle slot on them.

### 4. Ownership cost

- **A refused `terminal attach` or `agent attach`** from another terminal (run in a PTY, no `--takeover`) toggles the outer terminal's alt-screen and mouse modes, then prints `herdr: server shut down: terminal attach failed: terminal term_… already has an attached client; retry with --takeover` and exits 1. `agent attach` accepts a pane id and resolves to the same terminal. The refusal returns before any resize: a refused attach at 20×60 left the owner's PTY at `(30, 100)`, and the owner kept streaming. The refusal says nothing about who the owner is.
- **An evicted controller gets exactly one record**: `{"type":"terminal.closed","reason":"terminal attach taken over"}`, then the process exits 0 with empty stderr. There is no warning before it and no event channel; the only other sign is a `BrokenPipeError` on the next stdin write. The reason is prose, as in ADR-0002 §2. The new owner's size wins at once (the PTY went `(37, 85)` → `(20, 60)` for a takeover from a 20×60 terminal).
- **Acquire cost.** Spawn to first frame: 37–268 ms over 10 runs (median ≈ 200 ms). The CLI takes only `--cols`/`--rows`, so acquiring zeroes the pixel fields: even at the pane's current size the program gets a `SIGWINCH` (pixels `680×592` → `0×0`), and the `terminal.resize` with `cell_*_px` that follows is a second one. Releasing with a TUI attached is a third (pixels return, size returns to layout).
- **Recommendation: release panes that are not visible, after a 10 s grace.** warpr cannot show a pane in a hidden Herdr tab or workspace, yet holding it keeps the owner slot and blocks other terminals' `terminal attach` and `agent attach` for no benefit. Re-acquiring is about 0.2 s plus two repaints, so a grace period is enough to absorb tab flicking without churn. A released pane keeps its last PTY size when no TUI is attached, so nothing reflows on hide; the pane is simply resized again on the next acquire. Visible panes stay owned for as long as they are visible. warpr SHOULD send `terminal.release` and wait for `terminal.closed` on quit rather than relying on `kill`, though both free the slot.

### 5. `cell_width_px` / `cell_height_px`

- **Programs see them.** With `terminal.resize` at 100×30 and 9×18 px, `TIOCGWINSZ` returned `(30, 100, 900, 540)` and the program received `CSI 14 t` → `ESC[4;540;900t`, `CSI 16 t` → `ESC[6;18;9t` and `CSI 18 t` → `ESC[8;30;100t`. At 110×35 and 7×15 the replies followed (`ESC[4;525;770t`, `ESC[6;15;7t`). A TUI client with a zero-pixel PTY gets Herdr's default 8×16 (`680×592` for 85×37).
- **Both or neither.** With either value omitted or `0` the pixel fields are zero (a width-only resize gave `(30, 100, 900, 0)`), and `CSI 14 t`, `16 t` and `18 t` all got no reply within 0.4 s. A later `terminal.resize` without the pixel fields clears them; they do not carry over. A resize that only changes the pixel fields is a `SIGWINCH`.
- **Recommendation.** Send the real size of one terminal cell in physical pixels on every `terminal.resize`: `cell_width_px` and `cell_height_px` as the integer metrics of the font warpr draws the grid with, multiplied by the display scale factor, with `cols`/`rows` derived from them as in section 3 so that `cols × cell_width_px` fits the frame. Image-aware programs size their output from these.

### Not answered

- Whether the transient `codex` footer scramble is `codex` or Herdr's grid.
- Resize storms against `vim` and `less` (not run); only `omp`, `claude` and `codex` were exercised.
- Whether Herdr's refusal or eviction behaves differently for a TUI client that is itself the owner (the TUI typed into an owned pane without claiming it, so it may not use the owner slot at all).

## Implementation notes (live pane output)

- **Where it lives.** `herdr_client::PaneStream` runs one pane's `control` or `observe` subprocess and applies the resize and ownership policy above; the policy is a pure state machine (`pane_machine`) driven with explicit times, so the throttle, snap and grace are tested without sleeping. `app/src/herdr/` feeds the decoded frames to Warp's ANSI `Processor` and paints the result.
- **The screen is an alternate-screen grid, not a `TerminalModel`.** A frame repaints the whole visible screen, so `AltScreen` (a `GridHandler` with no scrollback) is the whole model. `terminal/remote_tty` was not reused: it builds a full terminal model with block list and shell bootstrap, and writes a zsh init script into the byte stream, none of which applies to a pane Herdr already runs a shell in.
- **A contested pane is watched at the pane's layout rect in cells.** The API does not expose a pane's PTY size, and the rect is the PTY size plus Herdr's border, so the observe viewport pads rather than crops. It is drawn at the font's own cell size when that fits the frame and shrunk, font included, only when it does not; the rest of the frame stays blank. It carries a notice in the frame and a changed border.
- **Control is retried only after a release.** A refused or evicted pane stays on `observe` while it is shown; once it has been hidden past the grace and is shown again, control is tried again without `--takeover`. Any other end of a stream is shown and not retried until the pane has been hidden and shown.
- **A pane counts as visible when its Herdr tab is the one shown by a workspace view.** warpr has no signal for a whole workspace tab or window being hidden, so those panes are held until their view goes away.
