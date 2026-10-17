# warpr is Herdr-only: no window distinguishes Herdr from Warp

warpr is exclusively a Herdr client. No code asks whether a window is Herdr-backed, because every window is: the top-level tabs are Herdr workspaces, the agent panel is Herdr's, new tabs and worktrees are created through Herdr, and the strings that used to switch between "tab" and "workspace" say "workspace". The window-level predicate (`presents_herdr_workspaces`), the per-window wording switch (`herdr::wording`), the `HerdrWorkspaceTabs` flag and its Cargo feature are gone, and the Herdr session model is registered unconditionally at startup. Where warpr once kept Warp's own tab, agent and worktree behavior alive for windows without Herdr, that behavior is not reachable any more.

The one non-Herdr state is the refusal from ADR-0001's compatibility check (#9): a missing or incompatible Herdr is shown as a refusal in place of the workspaces. That is a connection state of the Herdr model, not a mode of the window. The app still starts when no Herdr session is reachable, as it did before; the window then shows the refusal or the connection notice.

## Upstream code versus warpr's own code

ADR-0001 keeps upstream features warpr does not use behind turned-off flags so the rolling fork keeps merging `warpdotdev/warp`. That rule is about upstream code. Branching on "is this window Herdr-backed" was warpr's own code, and it goes. The rule when the two meet:

- **warpr's own branches and strings are deleted.** Dual-mode wording, predicates and tests that pinned only the non-Herdr side are removed, not kept as dead alternatives.
- **In upstream code the minimal edit that makes Herdr behavior unconditional wins.** Replace the branch by its Herdr side in place; do not remove the upstream module, setting, action or flag around it.
- **Consequences the compiler flags are removed with it.** A match arm that the unconditional Herdr arm now shadows, and a private helper left without a caller, are deleted. Anything still reachable from elsewhere, including upstream actions and public items, stays.
- **What Herdr does not support stays off through a kill-switch flag, as ADR-0001 says.** Where the non-Herdr branch carried behavior warpr must keep hidden, it is expressed as a turned-off upstream behavior: `DisableUserTabGroups` (the user cannot create tab groups or move tabs into or out of them; Herdr owns the groupings and the order) and `DisableLocalSessions` (the command palette does not offer to start local shell sessions). Both are on in every warpr build. Turning one off does not restore Warp's behavior, because the code that only the Herdr branches bypassed is gone where nothing reaches it any more (see Consequences).

## Whether warpr code may depend on upstream `ai` types

Yes, where reusing them is thinner than duplicating them. The Herdr agent notifications are fed into `crate::ai::agent_management::AgentNotificationsModel` and its `NotificationItem` types, which is also what the toast stack and the mailbox read. warpr does not copy that model to avoid the dependency, and an unreachable-looking `ai` module is therefore not safe to delete or strip.

This decides the question the compile-time stripping spike (#61) left to this ticket. Stripping stays out: the hosted agent is the one feature large enough to be worth cutting from the build, and warpr's own Herdr code, the terminal, the settings pages and onboarding all import its types, so a cfg boundary would need a rewrite of upstream code and conflicts with the merge rule above. Runtime kill-switch flags remain the only mechanism. If the hosted agent is ever taken out as a hard fork boundary, that is a decision of its own and has to begin by moving the notification types warpr reuses out of `ai`. Doing this ticket first, before any stripping, removes the Herdr predicate from the surface that decision would have to cut through.

## Consequences

- Other tickets' acceptance criteria that say "non-Herdr windows unchanged" are moot. There is no non-Herdr window to leave unchanged.
- Tests that need a window use `initialize_app`, which registers a Herdr model whose session never connects; tests that connect a session of their own use `initialize_app_without_herdr` and add their own model. A test no longer toggles Herdr by a flag.
- The GUI integration tests start the app with a real Herdr model. Without a reachable Herdr they see the refusal and not Warp's own tabs, so they cannot exercise upstream tab behavior.
- Upstream tab, group and worktree code that the Herdr branches used to bypass is still in the tree where something reachable calls it. What the compiler showed nothing calls any more was deleted: the `AddDefaultTab`, `AddTerminalTab` and `AddTabWithShell` handlers that open local tabs, the local new-worktree flow (`handle_new_worktree_submit`, `build_worktree_config_toml`, `find_unused_worktree_config_path`, the sidecar's local-checkout filter), `add_tab_with_shell`, and the pin, move, close, rename and group sections of the tab-group context menu that a Herdr workspace group never showed.
