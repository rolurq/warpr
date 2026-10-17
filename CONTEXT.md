# warpr

A fork of the Warp terminal whose interface is a client of Herdr. Herdr owns every terminal, layout and coding agent; warpr presents them in Warp's UI.

## Language

**warpr**:
The fork. Warp's GUI acting as a Herdr client, with Warp's own account, cloud-drive and hosted-agent services removed.
_Avoid_: Warp (when the fork is meant), Herdr UI

**Herdr session**:
A named, persistent Herdr server that warpr connects to. It outlives warpr.
_Avoid_: connection, profile, instance

**Workspace**:
Herdr's top-level grouping of tabs and panes. Presented in warpr as an entry in the vertical tabs panel.
_Avoid_: window, project, Warp tab

**Herdr tab**:
A grouping of panes inside a workspace. Presented in warpr as a tab in the horizontal tab bar, which lists the tabs of the workspace the window shows.
_Avoid_: tab (unqualified)

**Pane**:
One terminal surface owned by Herdr, positioned by Herdr's layout.
_Avoid_: split, terminal, session

**Pane owner**:
The one client that currently sets a pane's size and sends its input. warpr is the owner of the panes it shows when no other client is.
_Avoid_: controller, attached client, lock holder

**Contested pane**:
A pane whose owner is another client. warpr displays it without taking ownership until input is forwarded to it.
_Avoid_: locked pane, busy pane

**Agent**:
A coding agent that Herdr recognizes running in a pane.
_Avoid_: Warp agent, assistant, bot

**Agent state**:
Herdr's lifecycle classification of an agent: idle, working, blocked, done, or unknown.
_Avoid_: status, health
