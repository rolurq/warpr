# Herdr is the source of truth; warpr is a client

warpr owns no terminals, layout or agents. Workspaces, Herdr tabs, panes and agents all live in a Herdr session, and warpr only presents and drives them through Herdr's API. Warp's account, Drive and hosted-agent services are gated off with feature flags rather than deleted, so a rolling fork can keep merging upstream `warpdotdev/warp` without conflicts in code we no longer use. Revisiting Warp's own agent integration is deliberately deferred.

## Consequences

- Warp's block model depends on shell hooks injected when Warp spawns the PTY. Herdr owns the PTY, so panes have no blocks until a later change injects the hook through Herdr's pane environment.
- The first slice is a read-only mirror; creating, splitting and closing panes and prompting agents come after.
- Switching sessions is a UI action (a session picker listing every Herdr session), not a setting. warpr never stops a session on quit.
