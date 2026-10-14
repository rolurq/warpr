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
- Layout-resize suppression while a controller owns a pane is taken from Herdr's source and was not observed. Its interaction with an attached TUI client, resize storms and the ownership cost are tracked in issue #25.
