# Command palette scan (issue #71)

Research for [#71](https://github.com/rolurq/warpr/issues/71). No product code, flag or test was changed. Every source the command palette can mix in and every action it lists in warpr gets a decision: **connect** it to Herdr (naming the Herdr data or request), **disable** it behind a kill-switch flag that is on in every warpr build (ADR-0001, ADR-0005; never delete), or mark it **already handled** (citing the ticket or flag). warpr has no non-Herdr window (ADR-0005), so "in warpr" means in a window that presents a Herdr session.

A fourth outcome turned up and is kept separate rather than forced into the three: **keep** marks entries that do not touch terminals, sessions, layout or Warp services (font size, themes, settings pages, accessibility, debug tooling). They work in warpr today and need no Herdr counterpart.

## Result

| Decision | Actions | Sources | Tickets |
|---|---|---|---|
| Connect to Herdr | 11 | 4 (Sessions, Ctrl+Tab tabs, Files, Repos) | #92, #93 (after #86), #94, #95, #96 |
| Disable | 22 | 0 | #97 (existing flags extended, plus a new `DisableResourceCenter`), #87 |
| Already handled | 101 | 5 (Launch configurations, New session, Warp Drive, Conversations, zero-state suggestions) | #2/#43, #11, #31, #36, #39, #49, #50, #54, #55, #56, #58, #60, #63, #65, #68, #70, #73; `DisableLocalSessions`, `DisableWarpDrive`, `DisableHostedAgent` |
| Keep (Herdr-independent) | 69 | 1 (recent items) | — (three keep rows only need rewording, in #92) |
| **Total** | **203** | **10** | 6 filed: #92–#97 |

Highlights:

- **The Sessions source is always empty** in warpr: it lists Warp `TerminalPane`s, and a warpr tab holds a `HerdrWorkspacePane`. It should list Herdr workspaces and say "workspaces" (#92).
- **The Files source is empty** for the same reason one level down: `FileSearchModel` reads `ActiveSession`, which only a Warp `TerminalView` sets. #86 (filed by #70) fixes the scope; #93 does the palette side and the question of where an opened file appears.
- **"Open Repository" opens a Warp local terminal tab** (`add_tab_with_pane_layout(SingleTerminal)` has no Herdr branch), and the Repos source does nothing on accept (#94, a bug).
- **Closing a workspace from the palette never tells Herdr** (`remove_tab` only; `herdr_client` has no workspace close request) (#95).
- **Most rows are already settled**: #68 wired split, resize, zoom and directional focus; #39/#70 own the tools panel and search; #60 owns the many terminal, input and block settings toggles that the palette exposes.

## Method

- **Code.** Read `app/src/search/command_palette/` (`data_sources.rs`, `view.rs`, `mixer.rs`, each source module), `app/src/search/action/data_source.rs`, `Workspace::handle_palette_event`/`open_palette`/`set_command_palette_binding_source` (`app/src/workspace/view.rs`), the binding registrations (`app/src/workspace/mod.rs`, `app/src/pane_group/mod.rs`) and the handlers named in the table. Base: `fork/master` at `5f7d397ff` (includes #68 and #53).
- **How the action list is built.** `CommandBindingDataSource` takes `ctx.key_bindings_for_view(window, focused_view)` and keeps every binding with a description (`CommandBinding::from_lens`); bindings whose `with_enabled` predicate is false are never returned. The focused view is captured when the palette opens (`Workspace::set_command_palette_binding_source`). In warpr that is the `HerdrWorkspaceView`: its `focus_contents` calls `focus_self`, and selecting a pane does too. The palette therefore lists the bindings of the contexts `RootView → Workspace → PaneGroup → HerdrWorkspaceView`, deduplicated by keystroke.
- **Runtime probe (the action list).** A throwaway unit test (not committed) built the app the way `initialize_app` does: the test models from `initialize_app_without_herdr`, then every keybinding `init` that `app/src/lib.rs` calls (all 40 ran without a panic), a `herdr_fake` session with workspaces `api` (one Herdr tab, two panes) and `web`, a real `Workspace` window, focus on the shown `HerdrWorkspaceView` (responder chain of 4 views), and every flag from warpr's `enabled_features()` turned on (the warpr kill switches, the Cargo-feature flags and, as the debug `warp-oss` build does, `DEBUG_FLAGS`). It then listed `key_bindings_for_view` through `CommandBinding::from_lens`: **208 rows, 203 distinct** ("Show Warp Network Log" is registered four times with the same action and is one row below). Adding the 32 further `init` calls changed nothing in this list; they only add bindings for other focus targets (688 described bindings in total across all contexts).
- **Not run.** warpr itself was not run against an isolated Herdr session for this scan, and nothing was seen in the GUI. Behaviour that the code does not settle is listed under [Not established](#not-established-needs-the-gui-or-a-run) and marked `[INFERENCE]` in the table, not guessed.

## Data sources

Mixed in by `DataSourceStore::reset_search_mixer` (and `reset_ctrl_tab_mixer` for Ctrl+Tab):

| # | Source (module) | Filter / chip | Gate in warpr | What it does today in warpr | Decision | Herdr data or request / ticket | Reason |
|---|---|---|---|---|---|---|---|
| S1 | Actions: `search::action::CommandBindingDataSource` | Actions | always | Lists the 203 bindings in the table below | per action | see [Actions](#actions) | — |
| S2 | Sessions: `command_palette::navigation::DataSource` | Sessions ("Search sessions", chip "sessions") | always | `SessionNavigationData::all_sessions` → `PaneGroup::pane_sessions` → `panes_of::<TerminalPane>()`: **empty**, since warpr tabs hold a `HerdrWorkspacePane`. Same for the Ctrl+Tab palette in "Cycle most recent session" mode | **Connect** | The window's workspaces from the presented Herdr session (`HerdrSessionModel`, one row per workspace tab; directory from `PaneGroup::herdr_workspace_directory`); accept = `root_view:activate_tab_by_pane_group_id`, no Herdr focus request. Reword to "workspaces". #92 | The list exists only as Herdr data |
| S3 | Ctrl+Tab tabs: `command_palette::tabs::DataSource` | Tabs (Ctrl+Tab only) | `CtrlTabBehavior::CycleMostRecentTab` | Lists workspace tabs in MRU order and activates one: works. Subtitle comes from `active_session_path` (a `TerminalView`), so it is empty; rows say "Tab N" | **Connect** | Subtitle from `herdr_workspace_directory`; "Workspace N". #92 | Data is Herdr's; wording |
| S4 | Files: `command_palette::files::FileDataSource` (+ "Create file" item) | Files | `CommandPaletteFileSearch`, not a shared-session viewer | `FileSearchModel::repo_root_location` / `get_folder_contents` read `ActiveSession::working_directory`, `None` for Herdr tabs: **lists nothing**. Accepting a file → `open_code` (Warp code pane as a new Warp tab or a split beside the `HerdrWorkspacePane`) | **Connect** | The selected Herdr pane's `cwd` (#40) collapsed to its repository by `DetectedRepositories` (#39), through #86; palette side and file placement in #93 | Directory context is Herdr's |
| S5 | Repos: `command_palette::repos::RepoDataSource` | Repos (no chip) | always | Lists `PersistedWorkspace` (Warp's codebase-index workspaces). Accept → `OpenDirectory` → `active_session_view` → `TerminalView::open_repo_folder`: **no effect** without a `TerminalView` | **Connect** | The session's repositories (`HerdrRepositories`, #48); accept = show the workspace on that repository, else `herdr workspace create --cwd`. #94 | Repositories are known from Herdr workspaces |
| S6 | Launch configurations: `command_palette::launch_config::DataSource` | Launch configurations | `ContextFlag::LaunchConfigurations` (on) | Lists `WarpConfig::launch_configs`; accept → `root_view:open_launch_config` (Warp windows/tabs/sessions) | **Already handled** | #49 | Workspace config is decided there |
| S7 | New session: `command_palette::new_session::NewSessionDataSource` | Actions | `ShellSelector && local_tty && !DisableLocalSessions` | Not created | **Already handled** | `DisableLocalSessions` | Local shells belong to Herdr |
| S8 | Warp Drive: `command_palette::warp_drive::DataSource` (workflows, notebooks, env vars, agent-mode workflows) | Workflows, Notebooks, Plans, Drive, Env vars | `WarpDriveSettings::is_warp_drive_enabled` | Not added (`DisableWarpDrive` makes `is_warp_drive_available` false); chips hidden | **Already handled** | `DisableWarpDrive` | Killed |
| S9 | Conversations: `command_palette::conversations::DataSource` (+ new/fork conversation items) | Conversations | `AISettings::is_any_ai_enabled` | Not added (`DisableHostedAgent` makes it false); chip hidden | **Already handled** | `DisableHostedAgent` | Killed |
| S10 | Zero state: suggested actions (`SUGGESTED_ACTIONS`) and recent items (`SelectedItems`) | — | always | Suggestions are `input:toggle_input_type` (Input context, not in warpr's chain), `workspace:create_personal_workflow` (Drive, gated) and `workspace:show_theme_chooser`: only the theme picker can show. Recent items rehydrate from the sources above | **Already handled** / keep | follows S1–S5 | Nothing of its own |

## Herdr-backed data the palette needs

| Need | Herdr data | Exists in warpr | Gap |
|---|---|---|---|
| Workspaces list (S2, "workspaces" wording) | presented workspaces of the `HerdrSessionModel` session; each is a Warp tab (`Workspace::tabs`) | yes (#2) | Not read by the Sessions source (#92) |
| Selected pane's directory (S3 subtitle, S4, "Copy Current Path", "New File") | pane `cwd` → `HerdrWorkspaceView::focused_pane_directory` → `PaneGroup::herdr_workspace_directory` | yes (#40) | `ActiveSession` does not follow it (#86); palette consumers (#92, #93, #96) |
| Repository of that directory (S4) | `DetectedRepositories`, fed by `HerdrWorkspaceView::report_directory_change` | yes (#39) | through #86 |
| Session repositories (S5) | `HerdrRepositories` / `PresentedRepository` | yes (#48) | Repos source and "Open Repository" (#94) |
| Workspace create in a folder | `herdr workspace create --cwd` | `create_workspace` takes no `cwd` | #94 |
| Workspace close | `herdr workspace close` | no request | #95 |
| Pane close | `pane.close` (`herdr_client::close_pane`) | yes (#65 close button) | palette entry (#96) |
| Pane order of a Herdr tab | `PresentedHerdrTab` panes | yes | next/previous pane (#96) |

## Actions

Columns: **Command** as the palette shows it; **Binding** (the `EditableBinding` name, or the action for unnamed settings toggles); **Today in warpr**; **Decision**; **Ticket / flag / Herdr data**; **Reason**. Bindings are registered in `app/src/workspace/mod.rs` (`workspace:*`), `app/src/pane_group/mod.rs` (`pane_group:*`), `app/src/workspace/sync_inputs.rs`, `app/src/terminal/input.rs` and the settings pages' toggle bindings (`DispatchToSettingsTab`, `app/src/settings_view/`). "Debug-only" rows exist because the debug `warp-oss` build turns on `DEBUG_FLAGS` (`app/src/bin/oss.rs`).

| # | Command | Binding | Today in warpr | Decision | Ticket / flag / Herdr data | Reason |
|---|---|---|---|---|---|---|
| 1 | Close Current Session | `pane_group:close_current_session` | `PaneGroupAction::RemoveActive` → `close_active_pane_with_confirmation` on the Warp pane, which is the whole `HerdrWorkspacePane`; not routed to Herdr | **Connect** | #96 | Should close the selected Herdr pane with `pane.close` (`herdr_client::close_pane`, already used by the pane close button, #65) and say "Close current pane" |
| 2 | Activate Next Pane | `pane_group:navigate_next` | `NavigateNext` → `navigate_next_pane` over Warp's pane tree, which holds one pane (the Herdr workspace view): no effect [INFERENCE] | **Connect** | #96 | Should move the warpr selection to the next pane of the shown Herdr tab in Herdr's pane order (`PresentedHerdrTab` panes); selection is view state, so no Herdr request (#40 rule) |
| 3 | Activate Previous Pane | `pane_group:navigate_prev` | `NavigatePrev` → `navigate_prev_pane`, same as above | **Connect** | #96 | As above, previous pane |
| 4 | Close the Current Workspace | `workspace:close_active_tab` | `CloseActiveTab` → `close_tabs` → `remove_tab`: drops the Warp tab only; `herdr_client` has no `workspace.close` request | **Connect** | #95 | Should send `herdr workspace close` and let the tab go through Herdr's `workspace.closed` event |
| 5 | Close Other Workspaces | `workspace:close_other_tabs` | `CloseOtherTabs` → local `remove_tab` for every other tab; nothing sent to Herdr | **Connect** | #95 | As above, one `workspace close` per workspace, behind the existing confirmation |
| 6 | Close Workspaces Below | `workspace:close_tabs_right_active_tab` | "Close Workspaces Below" → local `remove_tab`; nothing sent to Herdr | **Connect** | #95 | As above |
| 7 | Copy Current Path | `workspace:copy_current_path` | `path_from_focused_pane` reads a file/code pane or a `TerminalView` pwd; a Herdr workspace view has neither, so nothing is copied | **Connect** | #96 | Copy `PaneGroup::herdr_workspace_directory` (the selected pane's `cwd`, #40) |
| 8 | New File | `workspace:new_file` | `NewCodeFile` opens Warp's editor on a new file whose default directory is the `TerminalView` pwd, else `$HOME` | **Connect** | #93 | Default directory should be the selected Herdr pane's `cwd`; where the editor opens is the open question under Files |
| 9 | Open Repository | `workspace:open_repository` | Opens a folder picker, then `handle_open_repository` → `add_tab_with_pane_layout(SingleTerminal)`, which has no Herdr branch: a Warp local terminal tab outside Herdr [INFERENCE: not run] | **Connect** | #94 | Should send `herdr workspace create --cwd <folder>` |
| 10 | Toggle Files Palette | `workspace:toggle_files_palette` | Opens the palette on the Files filter; the source reads `ActiveSession`, which is `None` for Herdr tabs, so it lists nothing | **Connect** | #93 (needs #86) | Follows the Files source row (#86) |
| 11 | Toggle Navigation Palette | `workspace:toggle_navigation_palette` | Opens the palette on the Sessions filter, which lists only Warp `TerminalPane`s: always empty in warpr | **Connect** | #92 | Follows the Sessions source row: list Herdr workspaces, say "workspaces" |
| 12 | Copy Access Token to Clipboard | `workspace:copy_access_token_to_clipboard` | Copies the Warp account access token | **Disable** | `DisableWarpAccount` (extend); #97 | Warp account surface; `DisableWarpAccount` hides account UI but this binding is not gated |
| 13 | Install Oz CLI Globally for Use Outside of Warp | `workspace:install_cli` | Installs the Oz (hosted agent) CLI | **Disable** | `DisableHostedAgent` (extend); #97 | Hosted-agent surface not gated by its kill switch |
| 14 | Log Out | `workspace:log_out` | Logs out of the Warp account | **Disable** | `DisableWarpAccount` (extend); #97 | As above |
| 15 | Move Workspace Down | `workspace:move_tab_right` | "Move Workspace Down" reorders the Warp tab locally; Herdr has no workspace reorder request (`herdr workspace --help`: list, create, get, focus, rename, report-metadata, close) | **Disable** | `DisableUserTabGroups` (extend to tab order); #97 | Herdr owns the order (the `DisableUserTabGroups` doc already says so); a local move diverges from Herdr |
| 16 | Open Settings: AI | `workspace:show_ai_settings_page` | Opens the AI settings page, which `should_render_page` hides under `DisableHostedAgent` | **Disable** | `DisableHostedAgent` (extend); #97 | The palette still offers a page warpr hides; what it shows is unverified |
| 17 | Open Settings: MCP Servers | `workspace:show_mcp_servers_settings_page` | Opens the MCP servers page (hidden under `DisableHostedAgent`) | **Disable** | `DisableHostedAgent` (extend); #97 | As above |
| 18 | Open Settings: Billing and Usage | `workspace:show_settings_billing_and_usage_page` | Opens billing (hidden under `DisableWarpAccount` and `DisableHostedAgent`) | **Disable** | `DisableWarpAccount` (extend); #97 | As above |
| 19 | Open Settings: Environments | `workspace:show_settings_environments_page` | Opens the cloud environments page (hidden under `DisableHostedAgent`) | **Disable** | `DisableHostedAgent` (extend); #97 | As above |
| 20 | Open Settings: Referrals | `workspace:show_settings_referrals_page` | Opens referrals (hidden under `DisableWarpAccount`) | **Disable** | `DisableWarpAccount` (extend); #97 | As above |
| 21 | Open Settings: Shared Blocks | `workspace:show_settings_shared_blocks_page` | Opens shared blocks (hidden under `DisableWarpAccount`) | **Disable** | `DisableWarpAccount` (extend); #97 | As above |
| 22 | Open Settings: Teams | `workspace:show_settings_teams_page` | Opens teams (hidden under `DisableWarpAccount`) | **Disable** | `DisableWarpAccount` (extend); #97 | As above |
| 23 | Toggle Resource Center | `workspace:toggle_resource_center` | Opens Warp's resource center (Warp feature tips, what's new, links) | **Disable** | new `DisableResourceCenter`; #97 | Content is about Warp features warpr removes (blocks, AI, Drive) and links to Warp; keyboard shortcuts stay reachable from settings |
| 24 | Undo Global Oz CLI Installation (Oz Will Still Work Within Warp) | `workspace:uninstall_cli` | Removes the Oz CLI | **Disable** | `DisableHostedAgent` (extend); #97 | As above |
| 25 | Disable '@' Context Menu in Terminal Mode | `DispatchToSettingsTab(FeaturesPageToggle(ToggleAtContextMenu` | Flips the `FeaturesPageToggle::ToggleAtContextMenuInTerminalMode` setting | **Disable** | `DisableHostedAgent` / #88; #97 | The @ menu has no host in warpr |
| 26 | Disable App Analytics | `DispatchToSettingsTab(PrivacyPageToggle(ToggleTelemetry))` | Flips the `PrivacyPageToggle::ToggleTelemetry` setting | **Disable** | `DisableUsageTelemetry` (extend), #58; #97 | Telemetry and crash reporting are killed; the toggle is not gated |
| 27 | Disable Crash Reporting | `DispatchToSettingsTab(PrivacyPageToggle(ToggleCrashReporting` | Flips the `PrivacyPageToggle::ToggleCrashReporting` setting | **Disable** | `DisableUsageTelemetry` (extend), #58; #97 | Telemetry and crash reporting are killed; the toggle is not gated |
| 28 | Disable Restore Windows, Tabs, and Panes on Startup | `DispatchToSettingsTab(FeaturesPageToggle(ToggleRestoreSessio` | Flips the `FeaturesPageToggle::ToggleRestoreSession` setting (restore Warp windows, tabs and panes) | **Disable** | `DisableLocalSessions` (extend), #85; #97 | Herdr persists workspaces; restored Warp tabs are local shells (#85) |
| 29 | Enable AI | `DispatchToSettingsTab(WarpAgent(ToggleGlobalAI))` | Flips the `WarpAgent::ToggleGlobalAI` setting ("Enable AI") | **Disable** | `DisableHostedAgent` (extend); #97 | Hosted agent is killed; the toggle is not gated |
| 30 | Enable Agent Font Matching Terminal Font | `DispatchToSettingsTab(AppearancePageToggle(ToggleMatchAIToTe` | Flips the `AppearancePageToggle::ToggleMatchAIToTerminalFontFamily` setting | **Disable** | `DisableHostedAgent` (extend); #97 | Hosted-agent setting |
| 31 | Enable Global Workflows in Command Search | `DispatchToSettingsTab(FeaturesPageToggle(ToggleGlobalWorkflo` | Flips the `FeaturesPageToggle::ToggleGlobalWorkflowsInUniversalSearch` setting | **Disable** | `DisableCommandSearch` (#87); #97 | Command search is disabled |
| 32 | Enable Settings Sync | `DispatchToSettingsTab(MainPageToggle(ToggleSettingsSync))` | Flips the `MainPageToggle::ToggleSettingsSync` setting (Warp settings sync) | **Disable** | `DisableWarpAccount` (extend); #97 | Needs a Warp account |
| 33 | Enable Warp Drive | `DispatchToSettingsTab(WarpDrive(ToggleShowWarpDrive))` | Flips the `WarpDrive::ToggleShowWarpDrive` setting (Warp Drive on/off) | **Disable** | `DisableWarpDrive` (extend); #97 | Drive is killed; the toggle is not gated |
| 34 | Reopen Closed Session | `app:reopen_closed_session` | Restores Warp's last closed tab/pane from `UndoCloseStack` | **Already handled** | #50 | Restoring a closed Herdr workspace, tab or pane is decided there |
| 35 | Toggle Hidden Files in Project Explorer | `file_tree:toggle_hidden_files` | Toggles hidden files in the tools-panel file tree | **Already handled** | #39 | The file tree follows the focused Herdr pane since #39 |
| 36 | Split Pane Down | `pane_group:add_down` | `PaneGroupAction::Add(Down)` → `HerdrPaneCommand::Split` → `pane.split --direction down` | **Already handled** | #68 | Wired through Herdr |
| 37 | Split Pane Right | `pane_group:add_right` | `Add(Right)` → `pane.split --direction right` | **Already handled** | #68 | Wired through Herdr |
| 38 | Switch Panes Down | `pane_group:navigate_down` | `NavigateDown` → `HerdrPaneCommand::SelectTowards(Down)` | **Already handled** | #68 | Wired through Herdr |
| 39 | Switch Panes Left | `pane_group:navigate_left` | `NavigateLeft` → `SelectTowards(Left)` | **Already handled** | #68 | Wired through Herdr |
| 40 | Switch Panes Right | `pane_group:navigate_right` | `NavigateRight` → `SelectTowards(Right)` | **Already handled** | #68 | Wired through Herdr |
| 41 | Switch Panes Up | `pane_group:navigate_up` | `NavigateUp` → `SelectTowards(Up)` | **Already handled** | #68 | Wired through Herdr |
| 42 | Resize Pane > Move Divider Down | `pane_group:resize_down` | `ResizeDown` → `pane.resize` | **Already handled** | #68 | Wired through Herdr |
| 43 | Resize Pane > Move Divider Left | `pane_group:resize_left` | `ResizeLeft` → `pane.resize` | **Already handled** | #68 | Wired through Herdr |
| 44 | Resize Pane > Move Divider Right | `pane_group:resize_right` | `ResizeRight` → `pane.resize` | **Already handled** | #68 | Wired through Herdr |
| 45 | Resize Pane > Move Divider Up | `pane_group:resize_up` | `ResizeUp` → `pane.resize` | **Already handled** | #68 | Wired through Herdr |
| 46 | Toggle Pane Zoom | `pane_group:toggle_maximize_pane` | "Toggle pane zoom" → `HerdrPaneCommand::ToggleZoom` → `pane.zoom` | **Already handled** | #68 | Wired through Herdr |
| 47 | Switch to 8th Workspace | `workspace:activate_eighth_tab` | Selects a workspace tab locally (never moves Herdr focus) | **Already handled** | #2, #43 | Tabs are Herdr workspaces; wording already says workspace |
| 48 | Switch to 5th Workspace | `workspace:activate_fifth_tab` | Selects a workspace tab locally (never moves Herdr focus) | **Already handled** | #2, #43 | Tabs are Herdr workspaces; wording already says workspace |
| 49 | Switch to 1st Workspace | `workspace:activate_first_tab` | Selects a workspace tab locally (never moves Herdr focus) | **Already handled** | #2, #43 | Tabs are Herdr workspaces; wording already says workspace |
| 50 | Switch to 4th Workspace | `workspace:activate_fourth_tab` | Selects a workspace tab locally (never moves Herdr focus) | **Already handled** | #2, #43 | Tabs are Herdr workspaces; wording already says workspace |
| 51 | Switch to Last Workspace | `workspace:activate_last_tab` | Selects a workspace tab locally (never moves Herdr focus) | **Already handled** | #2, #43 | Tabs are Herdr workspaces; wording already says workspace |
| 52 | Activate Next Workspace | `workspace:activate_next_tab` | Selects a workspace tab locally (never moves Herdr focus) | **Already handled** | #2, #43 | Tabs are Herdr workspaces; wording already says workspace |
| 53 | Activate Previous Workspace | `workspace:activate_prev_tab` | Selects a workspace tab locally (never moves Herdr focus) | **Already handled** | #2, #43 | Tabs are Herdr workspaces; wording already says workspace |
| 54 | Switch to 2nd Workspace | `workspace:activate_second_tab` | Selects a workspace tab locally (never moves Herdr focus) | **Already handled** | #2, #43 | Tabs are Herdr workspaces; wording already says workspace |
| 55 | Switch to 7th Workspace | `workspace:activate_seventh_tab` | Selects a workspace tab locally (never moves Herdr focus) | **Already handled** | #2, #43 | Tabs are Herdr workspaces; wording already says workspace |
| 56 | Switch to 6th Workspace | `workspace:activate_sixth_tab` | Selects a workspace tab locally (never moves Herdr focus) | **Already handled** | #2, #43 | Tabs are Herdr workspaces; wording already says workspace |
| 57 | Switch to 3rd Workspace | `workspace:activate_third_tab` | Selects a workspace tab locally (never moves Herdr focus) | **Already handled** | #2, #43 | Tabs are Herdr workspaces; wording already says workspace |
| 58 | Stop Synchronizing Any Panes | `workspace:disable_terminal_input_syncing` | Stops Warp's input syncing across Warp terminal inputs | **Already handled** | #60 | Herdr panes have no Warp input; input is #60's decision |
| 59 | Left Panel: Global Search | `workspace:left_panel_global_search` | Opens global search in the tools panel | **Already handled** | #39, #70 | Scoped to the focused Herdr pane's repository |
| 60 | Left Panel: Project Explorer | `workspace:left_panel_project_explorer` | Opens the file tree | **Already handled** | #39 | Scoped to the focused Herdr pane |
| 61 | View Privacy Policy (Opens External Link) | `workspace:link_to_privacy_policy` | Opens Warp's privacy policy | **Already handled** | #56 | Warp external link |
| 62 | Join Our Slack Community (Opens External Link) | `workspace:link_to_slack` | Opens Warp's Slack | **Already handled** | #56 | Warp external link |
| 63 | View User Docs (Opens External Link) | `workspace:link_to_user_docs` | Opens Warp's docs | **Already handled** | #56 | Warp external link |
| 64 | Create New Workspace | `workspace:new_tab` | "Create New Workspace" → `create_herdr_workspace` → `workspace create` | **Already handled** | #36 | Wired through Herdr |
| 65 | New Terminal Workspace | `workspace:new_terminal_tab` | `AddTerminalTab` → `create_herdr_workspace` (ADR-0005) | **Already handled** | #36, ADR-0005 | Wired through Herdr; duplicates "Create New Workspace" |
| 66 | Open Global Search | `workspace:open_global_search` | Opens global search | **Already handled** | #39, #70 | As above |
| 67 | Save New Launch Configuration | `workspace:open_launch_config_save_modal` | Saves Warp's window layout as a launch configuration | **Already handled** | #49 | Workspace config is decided there |
| 68 | Pin Current Tab | `workspace:pin_active_tab` | Pins the active Warp tab | **Already handled** | #55 | Pin wording and group behaviour are decided there |
| 69 | Rename the Current Pane | `workspace:rename_active_pane` | Sets a Warp-local pane title; not sent to Herdr | **Already handled** | #54 | Rename through Herdr is decided there |
| 70 | Rename the Current Tab | `workspace:rename_active_tab` | Sets the Warp tab's custom title (`PaneGroup::set_title`); not `workspace rename`. Says "Tab" | **Already handled** | #54, #73 | Workspace rename through Herdr is decided there; Herdr-tab rename is #73 |
| 71 | Send Feedback (Opens External Link) | `workspace:send_feedback` | Opens Warp's feedback form | **Already handled** | #56 | #56 points feedback at the warpr GitHub project |
| 72 | Open Settings: Account | `workspace:show_settings_account_page` | Opens the Account page | **Already handled** | #58 | Account section is decided there |
| 73 | Open Settings: Privacy | `workspace:show_settings_privacy_page` | Opens the Privacy page | **Already handled** | #58 | Privacy section is decided there |
| 74 | Open Settings: Warpify | `workspace:show_settings_warpify_page` | Opens Warpify settings | **Already handled** | #60 | Warpify is decided there |
| 75 | Toggle Sticky Command Header | `workspace:toggle_block_snackbar` | Sticky command header of Warp blocks | **Already handled** | #60 | Herdr panes have no blocks |
| 76 | Launch Configuration Palette | `workspace:toggle_launch_config_palette` | Opens the launch-configuration palette | **Already handled** | #49 | As above |
| 77 | Open Left Panel | `workspace:toggle_left_panel` | Toggles the tools panel | **Already handled** | #39 | As above |
| 78 | Toggle Notification Mailbox | `workspace:toggle_notification_mailbox` | Opens the agent notification mailbox | **Already handled** | #11 | Fed by Herdr agent notifications |
| 79 | Turn Notifications On | `workspace:toggle_notifications_on` | Turns `SessionSettings.notifications` on and asks for OS permission | **Already handled** | #31 | Herdr agent OS notifications are #31; whether they honour this setting is unverified |
| 80 | Toggle Project Explorer | `workspace:toggle_project_explorer` | Toggles the file tree | **Already handled** | #39 | As above |
| 81 | Toggle Code Review | `workspace:toggle_right_panel` | Toggles the code review panel | **Already handled** | #39 | Scoped to the focused pane's repository |
| 82 | Toggle Synchronizing All Panes in All Tabs | `workspace:toggle_sync_all_terminal_inputs_in_all_tabs` | Warp input syncing | **Already handled** | #60 | As above |
| 83 | Toggle Synchronizing All Panes in Current Tab | `workspace:toggle_sync_terminal_inputs_in_tab` | Warp input syncing | **Already handled** | #60 | As above |
| 84 | Open Workspace Configs Menu | `workspace:toggle_tab_configs_menu` | Opens the workspace configs menu | **Already handled** | #49 | As above |
| 85 | Toggle Vertical Tabs Panel | `workspace:toggle_vertical_tabs_panel` | Toggles the vertical panel of workspaces | **Already handled** | #63 | The panel lists Herdr workspaces |
| 86 | View Warp Logs | `workspace:view_logs` | Opens Warp's log directory | **Already handled** | #56 | Listed with the Warp links in #56 |
| 87 | Disable Auto Show or Hide Rich Input Based on Agent Status | `DispatchToSettingsTab(CLIAgents(ToggleAutoToggleRichInput))` | Flips the `CLIAgents::ToggleAutoToggleRichInput` setting (Warp rich input / toolbar for CLI agents in a Warp terminal) | **Already handled** | #60 | Needs Warp's input on a pane |
| 88 | Disable Autocomplete Quotes, Parentheses, and Brackets | `DispatchToSettingsTab(FeaturesPageToggle(ToggleAutocompleteS` | Flips the `FeaturesPageToggle::ToggleAutocompleteSymbols` setting | **Already handled** | #60 | Warp input, completions or shell integration |
| 89 | Disable Autosuggestion Keybinding Hint | `DispatchToSettingsTab(FeaturesPageToggle(ToggleAutosuggestio` | Flips the `FeaturesPageToggle::ToggleAutosuggestionKeybindingHint` setting | **Already handled** | #60 | Warp input, completions or shell integration |
| 90 | Disable Autosuggestions | `DispatchToSettingsTab(FeaturesPageToggle(ToggleAutosuggestio` | Flips the `FeaturesPageToggle::ToggleAutosuggestions` setting | **Already handled** | #60 | Warp input, completions or shell integration |
| 91 | Disable Block Dividers | `DispatchToSettingsTab(AppearancePageToggle(ToggleShowBlockDi` | Flips the `AppearancePageToggle::ToggleShowBlockDividers` setting | **Already handled** | #60 | Warp input / blocks |
| 92 | Disable Code Review Button | `DispatchToSettingsTab(EditorAndCodeReview(ToggleCodeReviewPa` | Flips the `EditorAndCodeReview::ToggleCodeReviewPanel` setting | **Already handled** | #39 | Tools panel / code review follow the focused Herdr pane |
| 93 | Disable Coding Agent Toolbar | `DispatchToSettingsTab(CLIAgents(ToggleCLIAgentToolbar))` | Flips the `CLIAgents::ToggleCLIAgentToolbar` setting (Warp rich input / toolbar for CLI agents in a Warp terminal) | **Already handled** | #60 | Needs Warp's input on a pane |
| 94 | Disable Command Corrections | `DispatchToSettingsTab(FeaturesPageToggle(ToggleCommandCorrec` | Flips the `FeaturesPageToggle::ToggleCommandCorrections` setting | **Already handled** | #60 | Warp input, completions or shell integration |
| 95 | Disable Copy on Select Within the Terminal | `DispatchToSettingsTab(FeaturesPageToggle(ToggleCopyOnSelect)` | Flips the `FeaturesPageToggle::ToggleCopyOnSelect` setting; whether warpr's Herdr pane input/grid honours it is unverified | **Already handled** | #60 | Terminal-surface setting of Warp's `TerminalView` |
| 96 | Disable Cursor Blink | `DispatchToSettingsTab(AppearancePageToggle(ToggleCursorBlink` | Flips the `AppearancePageToggle::ToggleCursorBlink` setting; whether warpr's Herdr pane grid honours it is unverified | **Already handled** | #60 | Terminal-surface setting of Warp's `TerminalView` |
| 97 | Disable Custom Padding in Alt-Screen | `DispatchToSettingsTab(AppearancePageToggle(ToggleAltScreenPa` | Flips the `AppearancePageToggle::ToggleAltScreenPadding` setting; whether warpr's Herdr pane grid honours it is unverified | **Already handled** | #60 | Terminal-surface setting of Warp's `TerminalView` |
| 98 | Disable Diff Stats on Code Review Button | `DispatchToSettingsTab(EditorAndCodeReview(ToggleShowCodeRevi` | Flips the `EditorAndCodeReview::ToggleShowCodeReviewDiffStats` setting | **Already handled** | #39 | Tools panel / code review follow the focused Herdr pane |
| 99 | Disable Error Underlining | `DispatchToSettingsTab(FeaturesPageToggle(ToggleErrorUnderlin` | Flips the `FeaturesPageToggle::ToggleErrorUnderlining` setting | **Already handled** | #60 | Warp input, completions or shell integration |
| 100 | Disable Focus Reporting | `DispatchToSettingsTab(FeaturesPageToggle(ToggleFocusReportin` | Flips the `FeaturesPageToggle::ToggleFocusReporting` setting; whether warpr's Herdr pane input/grid honours it is unverified | **Already handled** | #60 | Terminal-surface setting of Warp's `TerminalView` |
| 101 | Disable Global File Search | `DispatchToSettingsTab(EditorAndCodeReview(ToggleGlobalSearch` | Flips the `EditorAndCodeReview::ToggleGlobalSearch` setting | **Already handled** | #39 | Tools panel / code review follow the focused Herdr pane |
| 102 | Disable Input Hint Text | `DispatchToSettingsTab(FeaturesPageToggle(ToggleShowInputHint` | Flips the `FeaturesPageToggle::ToggleShowInputHintText` setting | **Already handled** | #60 | Warp input, completions or shell integration |
| 103 | Disable Jump to Bottom of Block Button | `DispatchToSettingsTab(AppearancePageToggle(ToggleJumpToBotto` | Flips the `AppearancePageToggle::ToggleJumpToBottomOfBlockButton` setting | **Already handled** | #60 | Warp input / blocks |
| 104 | Disable Middle-Click Paste | `DispatchToSettingsTab(FeaturesPageToggle(ToggleMiddleClickPa` | Flips the `FeaturesPageToggle::ToggleMiddleClickPaste` setting; whether warpr's Herdr pane input/grid honours it is unverified | **Already handled** | #60 | Terminal-surface setting of Warp's `TerminalView` |
| 105 | Disable Mouse Reporting | `DispatchToSettingsTab(FeaturesPageToggle(ToggleMouseReportin` | Flips the `FeaturesPageToggle::ToggleMouseReporting` setting; whether warpr's Herdr pane input/grid honours it is unverified | **Already handled** | #60 | Terminal-surface setting of Warp's `TerminalView` |
| 106 | Disable Native Shell Completions | `DispatchToSettingsTab(FeaturesPageToggle(ToggleNativeShellCo` | Flips the `FeaturesPageToggle::ToggleNativeShellCompletions` setting | **Already handled** | #60 | Warp input, completions or shell integration |
| 107 | Disable Project Explorer | `DispatchToSettingsTab(EditorAndCodeReview(ToggleProjectExplo` | Flips the `EditorAndCodeReview::ToggleProjectExplorer` setting | **Already handled** | #39 | Tools panel / code review follow the focused Herdr pane |
| 108 | Disable SSH Warpification | `DispatchToSettingsTab(WarpifyPageToggle(ToggleSshWarpificati` | Flips the `WarpifyPageToggle::ToggleSshWarpification` setting | **Already handled** | #60 | Warpify |
| 109 | Disable Scroll Reporting | `DispatchToSettingsTab(FeaturesPageToggle(ToggleScrollReporti` | Flips the `FeaturesPageToggle::ToggleScrollReporting` setting; whether warpr's Herdr pane input/grid honours it is unverified | **Already handled** | #60 | Terminal-surface setting of Warp's `TerminalView` |
| 110 | Disable Show Hidden Files in Project Explorer | `DispatchToSettingsTab(EditorAndCodeReview(ToggleShowHiddenFi` | Flips the `EditorAndCodeReview::ToggleShowHiddenFiles` setting | **Already handled** | #39 | Tools panel / code review follow the focused Herdr pane |
| 111 | Disable Smart Select | `DispatchToSettingsTab(FeaturesPageToggle(ToggleSmartSelectio` | Flips the `FeaturesPageToggle::ToggleSmartSelection` setting; whether warpr's Herdr pane input/grid honours it is unverified | **Already handled** | #60 | Terminal-surface setting of Warp's `TerminalView` |
| 112 | Disable Syntax Highlighting | `DispatchToSettingsTab(FeaturesPageToggle(ToggleSyntaxHighlig` | Flips the `FeaturesPageToggle::ToggleSyntaxHighlighting` setting | **Already handled** | #60 | Warp input, completions or shell integration |
| 113 | Disable Terminal Input Message Line | `DispatchToSettingsTab(FeaturesPageToggle(ToggleShowTerminalI` | Flips the `FeaturesPageToggle::ToggleShowTerminalInputMessageLine` setting | **Already handled** | #60 | Warp input, completions or shell integration |
| 114 | Disable Warp Completions | `DispatchToSettingsTab(FeaturesPageToggle(ToggleWarpCompletio` | Flips the `FeaturesPageToggle::ToggleWarpCompletions` setting | **Already handled** | #60 | Warp input, completions or shell integration |
| 115 | Disable in-App Agent Notifications | `DispatchToSettingsTab(FeaturesPageToggle(ToggleAgentInAppNot` | Flips the `FeaturesPageToggle::ToggleAgentInAppNotifications` setting | **Already handled** | #11 | Herdr agent notifications use `AgentNotificationsModel` |
| 116 | Enable Alias Expansion | `DispatchToSettingsTab(FeaturesPageToggle(ToggleAliasExpansio` | Flips the `FeaturesPageToggle::ToggleAliasExpansion` setting | **Already handled** | #60 | Warp input, completions or shell integration |
| 117 | Enable Audible Terminal Bell | `DispatchToSettingsTab(FeaturesPageToggle(ToggleUseAudibleBel` | Flips the `FeaturesPageToggle::ToggleUseAudibleBell` setting; whether warpr's Herdr pane input/grid honours it is unverified | **Already handled** | #60 | Terminal-surface setting of Warp's `TerminalView` |
| 118 | Enable Auto Dismiss Rich Input After Prompt Submission | `DispatchToSettingsTab(CLIAgents(ToggleAutoDismissRichInputAf` | Flips the `CLIAgents::ToggleAutoDismissRichInputAfterSubmit` setting (Warp rich input / toolbar for CLI agents in a Warp terminal) | **Already handled** | #60 | Needs Warp's input on a pane |
| 119 | Enable Auto Open Code Review Panel | `DispatchToSettingsTab(EditorAndCodeReview(ToggleAutoOpenCode` | Flips the `EditorAndCodeReview::ToggleAutoOpenCodeReviewPane` setting | **Already handled** | #39 | Tools panel / code review follow the focused Herdr pane |
| 120 | Enable Auto Open Rich Input When a Coding Agent Session Starts | `DispatchToSettingsTab(CLIAgents(ToggleAutoOpenRichInputOnCLI` | Flips the `CLIAgents::ToggleAutoOpenRichInputOnCLIAgentStart` setting (Warp rich input / toolbar for CLI agents in a Warp terminal) | **Already handled** | #60 | Needs Warp's input on a pane |
| 121 | Enable Autosuggestion Ignore Button | `DispatchToSettingsTab(FeaturesPageToggle(ToggleShowAutosugge` | Flips the `FeaturesPageToggle::ToggleShowAutosuggestionIgnoreButton` setting | **Already handled** | #60 | Warp input, completions or shell integration |
| 122 | Enable Completions While Typing | `DispatchToSettingsTab(FeaturesPageToggle(ToggleCompletionsOp` | Flips the `FeaturesPageToggle::ToggleCompletionsOpenWhileTyping` setting | **Already handled** | #60 | Warp input, completions or shell integration |
| 123 | Enable Dim Inactive Panes | `DispatchToSettingsTab(AppearancePageToggle(ToggleDimInactive` | Flips the `AppearancePageToggle::ToggleDimInactivePanes` setting | **Already handled** | #65 | Herdr pane chrome dims inactive panes |
| 124 | Enable Editing Commands With Vim Keybindings | `DispatchToSettingsTab(FeaturesPageToggle(ToggleVimMode))` | Flips the `FeaturesPageToggle::ToggleVimMode` setting | **Already handled** | #60 | Warp input, completions or shell integration |
| 125 | Enable Focus Follows Mouse | `DispatchToSettingsTab(AppearancePageToggle(ToggleFocusPaneOn` | Flips the `AppearancePageToggle::ToggleFocusPaneOnHover` setting; whether warpr's Herdr pane grid honours it is unverified | **Already handled** | #60 | Terminal-surface setting of Warp's `TerminalView` |
| 126 | Enable Left Option Key Is Meta | `DispatchToSettingsTab(FeaturesPageToggle(ToggleLeftMetaKey))` | Flips the `FeaturesPageToggle::ToggleLeftMetaKey` setting; whether warpr's Herdr pane input/grid honours it is unverified | **Already handled** | #60 | Terminal-surface setting of Warp's `TerminalView` |
| 127 | Enable Preserve Input Focus on Block Selection | `DispatchToSettingsTab(FeaturesPageToggle(TogglePreserveInput` | Flips the `FeaturesPageToggle::TogglePreserveInputFocusOnBlockSelection` setting | **Already handled** | #60 | Warp input, completions or shell integration |
| 128 | Enable Reuse Existing SSH ControlMaster in the Warp SSH Wrapper | `DispatchToSettingsTab(FeaturesPageToggle(ToggleSshReuseContr` | Flips the `FeaturesPageToggle::ToggleSshReuseControlMaster` setting | **Already handled** | #60 | Warp input, completions or shell integration |
| 129 | Enable Right Option Key Is Meta | `DispatchToSettingsTab(FeaturesPageToggle(ToggleRightMetaKey)` | Flips the `FeaturesPageToggle::ToggleRightMetaKey` setting; whether warpr's Herdr pane input/grid honours it is unverified | **Already handled** | #60 | Terminal-surface setting of Warp's `TerminalView` |
| 130 | Enable Secret Redaction | `DispatchToSettingsTab(PrivacyPageToggle(ToggleSafeMode))` | Flips the `PrivacyPageToggle::ToggleSafeMode` setting (secret redaction of Warp block output) | **Already handled** | #60 | Applies to Warp blocks |
| 131 | Pin Input to the Bottom | `DispatchToSettingsTab(AppearancePageToggle(SetInputMode { ne` | Flips the `AppearancePageToggle::SetInputMode` setting | **Already handled** | #60 | Warp input / blocks |
| 132 | Pin Input to the Top | `DispatchToSettingsTab(AppearancePageToggle(SetInputMode { ne` | Flips the `AppearancePageToggle::SetInputMode` setting | **Already handled** | #60 | Warp input / blocks |
| 133 | Start Input at the Top | `DispatchToSettingsTab(AppearancePageToggle(SetInputMode { ne` | Flips the `AppearancePageToggle::SetInputMode` setting | **Already handled** | #60 | Warp input / blocks |
| 134 | Toggle Input Mode (Warp/Classic) | `DispatchToSettingsTab(AppearancePageToggle(ToggleInputMode))` | Flips the `AppearancePageToggle::ToggleInputMode` setting | **Already handled** | #60 | Warp input / blocks |
| 135 | Show Warp Network Log | `input:insert_network_logging_workflow` | Opens Warp's in-app network log as a Warp pane split into the active pane group | **Keep** | — | Warp-local diagnostics; no Herdr counterpart. Note it adds a Warp pane next to the Herdr workspace view (same placement question as opened files, see Files) |
| 136 | Close Window | `workspace:close_window` | Closes the warpr window | **Keep** | — | Window-level; warpr never stops the session on close (ADR-0001) |
| 137 | Crash the App (For Testing Sentry-Cocoa) | `workspace:crash` | Crashes the app for Sentry-Cocoa testing | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on; `crash_reporting` is never compiled into warpr, so it is a no-op |
| 138 | Cycle Current Tab Color | `workspace:cycle_active_tab_color` | Cycles the colour of the active Warp tab, i.e. the workspace row | **Keep** | wording in #92 | View-only decoration of a workspace; Herdr has no colour. Wording says "Tab" (see wording ticket) |
| 139 | [Debug] Un-Dismiss AWS Login Banner | `workspace:debug_reset_aws_bedrock_login_banner_dismissed` | Debug tooling | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on |
| 140 | Decrease Font Size | `workspace:decrease_font_size` | Font size | **Keep** | — | Herdr-independent |
| 141 | Increase Font Size | `workspace:increase_font_size` | Font size | **Keep** | — | Herdr-independent |
| 142 | [Debug] Install OpenCode Warp Plugin | `workspace:install_opencode_warp_plugin` | Debug tooling | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on |
| 143 | [Debug] Log Review Comment Send Status for Active Tab | `workspace:log_review_comment_send_status_for_active_tab` | Debug tooling | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on |
| 144 | Create New Window | `workspace:new_window` | Opens a new warpr window presenting the same Herdr session | **Keep** | — | Window-level |
| 145 | [Debug] Open Warp Agent CLI Launch Modal | `workspace:open_agent_cli_launch_modal` | Debug modal tooling | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on |
| 146 | [Debug] Open Auto-Handoff Sleep Modal | `workspace:open_auto_handoff_sleep_modal` | Debug modal tooling | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on |
| 147 | [Debug] Open Build Plan Migration Modal | `workspace:open_build_plan_migration_modal` | Debug modal tooling | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on |
| 148 | [Debug] Open ChatGPT Plan Modal | `workspace:open_chatgpt_plan_modal` | Debug modal tooling | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on |
| 149 | [Debug] Open Feature Intro Modal | `workspace:open_feature_intro_modal` | Debug modal tooling | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on |
| 150 | [Debug] Open Free AI Removal Modal | `workspace:open_free_ai_removal_modal` | Debug modal tooling | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on |
| 151 | [Debug] Open OpenWarp Launch Modal | `workspace:open_openwarp_launch_modal` | Debug modal tooling | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on |
| 152 | [Debug] Open Orchestration Launch Modal | `workspace:open_orchestration_launch_modal` | Debug modal tooling | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on |
| 153 | [Debug] Open Oz Launch Modal | `workspace:open_oz_launch_modal` | Debug modal tooling | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on |
| 154 | [Debug] Open Session Config Modal | `workspace:open_session_config_modal` | Debug modal tooling | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on |
| 155 | Open Settings File | `workspace:open_settings_file` | Opens settings.toml | **Keep** | — | Herdr-independent |
| 156 | Open View Tree Debugger | `workspace:open_view_tree_debug_view` | Debug tooling | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on |
| 157 | Trigger a Panic (For Testing Sentry-Rust) | `workspace:panic` | Panics for Sentry-Rust testing | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on |
| 158 | [Debug] Reset Warp Agent CLI Launch Modal State | `workspace:reset_agent_cli_launch_modal_state` | Debug modal tooling | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on |
| 159 | [Debug] Reset Auto-Handoff Sleep Modal State | `workspace:reset_auto_handoff_sleep_modal_state` | Debug modal tooling | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on |
| 160 | [Debug] Reset Build Plan Migration Modal State | `workspace:reset_build_plan_migration_modal_state` | Debug modal tooling | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on |
| 161 | [Debug] Reset ChatGPT Plan Modal State | `workspace:reset_chatgpt_plan_modal_state` | Debug modal tooling | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on |
| 162 | [Debug] Reset Feature Intro Modal State | `workspace:reset_feature_intro_modal_state` | Debug modal tooling | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on |
| 163 | Reset Font Size to Default | `workspace:reset_font_size` | Font size | **Keep** | — | Herdr-independent |
| 164 | [Debug] Reset Free AI Removal Modal State | `workspace:reset_free_ai_removal_modal_state` | Debug modal tooling | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on |
| 165 | [Debug] Reset OpenWarp Launch Modal State | `workspace:reset_openwarp_launch_modal_state` | Debug modal tooling | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on |
| 166 | [Debug] Reset Orchestration Launch Modal State | `workspace:reset_orchestration_launch_modal_state` | Debug modal tooling | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on |
| 167 | [Debug] Reset Oz Launch Modal State | `workspace:reset_oz_launch_modal_state` | Debug modal tooling | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on |
| 168 | Sample Process | `workspace:sample_process` | Samples the warpr process | **Keep** | — | Diagnostics |
| 169 | [a11y] Set Concise Accessibility Announcements | `workspace:set_a11y_concise_verbosity_level` | Accessibility | **Keep** | — | Herdr-independent |
| 170 | [a11y] Set Verbose Accessibility Announcements | `workspace:set_a11y_verbose_verbosity_level` | Accessibility | **Keep** | — | Herdr-independent |
| 171 | Switch Focus to Left Panel | `workspace:shift_focus_left` | Moves focus to the left panel | **Keep** | — | Panel focus |
| 172 | Switch Focus to Right Panel | `workspace:shift_focus_right` | Moves focus to the right panel | **Keep** | — | Panel focus |
| 173 | [Debug] Start HOA Onboarding Flow | `workspace:show_hoa_onboarding_flow` | Debug tooling | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on |
| 174 | Open Keybindings Editor | `workspace:show_keybinding_settings` | Opens the keybindings editor | **Keep** | — | Herdr-independent |
| 175 | Open Settings | `workspace:show_settings` | Opens settings | **Keep** | — | Herdr-independent |
| 176 | Open Settings: About | `workspace:show_settings_about_page` | Settings page | **Keep** | — | Herdr-independent |
| 177 | Open Settings: Appearance | `workspace:show_settings_appearance_page` | Settings page | **Keep** | — | Herdr-independent |
| 178 | Open Settings: Code | `workspace:show_settings_code_page` | Opens the Code (editor and code review) page | **Keep** | — | Editor and tools panel stay |
| 179 | Open Settings: Features | `workspace:show_settings_features_page` | Settings page | **Keep** | — | Herdr-independent |
| 180 | Open Settings: Keyboard Shortcuts | `workspace:show_settings_keyboard_shortcuts_page` | Settings page | **Keep** | — | Herdr-independent |
| 181 | Open Theme Picker | `workspace:show_theme_chooser` | Theme picker (also the only zero-state suggestion warpr can show) | **Keep** | — | Herdr-independent |
| 182 | Quit Warp | `workspace:terminate_app` | Quits warpr ("Quit Warp") | **Keep** | — | warpr never stops the session on quit (ADR-0001) |
| 183 | Toggle Command Palette | `workspace:toggle_command_palette` | Opens/closes the palette | **Keep** | — | — |
| 184 | Toggle Keyboard Shortcuts | `workspace:toggle_keybindings_page` | Toggles the keyboard shortcuts page | **Keep** | — | Herdr-independent |
| 185 | [Debug] Trigger Auto-Handoff to Cloud | `workspace:trigger_auto_handoff_to_cloud` | Debug tooling | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on |
| 186 | [Debug] Use Local OpenCode Warp Plugin (Testing Only) | `workspace:use_local_opencode_warp_plugin` | Debug tooling | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on |
| 187 | Configure Global Hotkey | `ScrollToSettingsWidget { page: Features, widget_id: "warp::s` | Opens the global hotkey setting | **Keep** | — | Window-level |
| 188 | Disable Notebook Font Size Matching Terminal Font Size | `DispatchToSettingsTab(AppearancePageToggle(ToggleMatchNotebo` | Flips the `AppearancePageToggle::ToggleMatchNotebookToMonospaceFontSize` setting | **Keep** | — | Herdr-independent appearance |
| 189 | Disable Quit Warning Modal | `DispatchToSettingsTab(FeaturesPageToggle(ToggleShowWarningBe` | Flips the `FeaturesPageToggle::ToggleShowWarningBeforeQuitting` setting | **Keep** | — | Herdr-independent |
| 190 | Disable Show Tooltip on Click on Links | `DispatchToSettingsTab(FeaturesPageToggle(ToggleLinkTooltip))` | Flips the `FeaturesPageToggle::ToggleLinkTooltip` setting | **Keep** | — | Herdr-independent |
| 191 | Disable Tab Indicators | `DispatchToSettingsTab(AppearancePageToggle(ToggleTabIndicato` | Flips the `AppearancePageToggle::ToggleTabIndicators` setting | **Keep** | — | Herdr-independent appearance |
| 192 | Disable Tools Panel Visibility Across Tabs | `DispatchToSettingsTab(AppearancePageToggle(ToggleLeftPanelVi` | Flips the `AppearancePageToggle::ToggleLeftPanelVisibility` setting | **Keep** | — | Herdr-independent appearance |
| 193 | Dump Debug Info | `DumpDebugInfo` | Writes debug info | **Keep** | — | Diagnostics |
| 194 | Enable Code as Default Editor | `DispatchToSettingsTab(FeaturesPageToggle(ToggleCodeAsDefault` | Flips the `FeaturesPageToggle::ToggleCodeAsDefaultEditor` setting | **Keep** | — | Herdr-independent |
| 195 | Enable Compact Mode | `DispatchToSettingsTab(AppearancePageToggle(ToggleCompactMode` | Flips the `AppearancePageToggle::ToggleCompactMode` setting | **Keep** | — | Herdr-independent appearance |
| 196 | Enable Open New Windows With Custom Size | `DispatchToSettingsTab(AppearancePageToggle(ToggleOpenWindows` | Flips the `AppearancePageToggle::ToggleOpenWindowsAtCustomSize` setting | **Keep** | — | Herdr-independent appearance |
| 197 | Enable Preserve Active Tab Color for New Tabs | `DispatchToSettingsTab(AppearancePageToggle(TogglePreserveAct` | Flips the `AppearancePageToggle::TogglePreserveActiveTabColor` setting | **Keep** | — | Herdr-independent appearance |
| 198 | Enable Themes: Sync With OS | `DispatchToSettingsTab(AppearancePageToggle(ToggleRespectSyst` | Flips the `AppearancePageToggle::ToggleRespectSystemTheme` setting | **Keep** | — | Herdr-independent appearance |
| 199 | Show Initialization Block | `DispatchToSettingsTab(Debug(ToggleInitializationBlock))` | Flips the `Debug::ToggleInitializationBlock` setting | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on; Warp block internals |
| 200 | Show in-Band Command Blocks | `DispatchToSettingsTab(Debug(ToggleInBandCommandBlocks))` | Flips the `Debug::ToggleInBandCommandBlocks` setting | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on; Warp block internals |
| 201 | Switch to Next Tab | `CycleNextSession` | "Switch to Next/Previous Tab": with the default `CtrlTabBehavior::ActivatePrevNextTab` selects the next/previous workspace; with the MRU settings it opens the Ctrl+Tab palette | **Keep** | wording in #92 | Works on workspaces; wording says "Tab" (wording ticket) |
| 202 | Switch to Previous Tab | `CyclePrevSession` | "Switch to Next/Previous Tab": with the default `CtrlTabBehavior::ActivatePrevNextTab` selects the next/previous workspace; with the MRU settings it opens the Ctrl+Tab palette | **Keep** | wording in #92 | Works on workspaces; wording says "Tab" (wording ticket) |
| 203 | [Debug] View First-Time User Experience | `AddGetStartedTab` | Adds Warp's local Get Started tab | **Keep** | — | Debug-only: registered behind `DEBUG_FLAGS`, which only the debug `warp-oss` build turns on |

## Proposed implementation tickets (filed)

All are `needs-triage`, link #71 as parent, and include the "tests cover which actions are offered" and "verified against an isolated `warpr-test` session" criteria.

| Ticket | Category | Scope | Rows |
|---|---|---|---|
| #92 | enhancement | Sessions source lists Herdr workspaces; "workspaces" wording; Ctrl+Tab subtitle and "Workspace N"; remaining tab/session strings | S2, S3, navigation palette, 3 wording rows |
| #93 | enhancement | Files source after #86; "New File" default directory; where opened files appear | S4, Files palette, New File |
| #94 | bug | "Open Repository" and Repos source through `herdr workspace create --cwd` | S5, Open Repository |
| #95 | enhancement | Close current / other / below workspaces through `herdr workspace close` | 3 close rows |
| #96 | enhancement | Close pane (`pane.close`), next/previous pane, copy current path on the selected Herdr pane | 4 pane rows |
| #97 | enhancement | Gate the 22 disable rows with existing kill switches (`DisableWarpAccount`, `DisableHostedAgent`, `DisableWarpDrive`, `DisableUsageTelemetry`, `DisableUserTabGroups`, `DisableLocalSessions`, `DisableCommandSearch` from #87) and a new `DisableResourceCenter`; one test pins the offered list | all Disable rows |

Related tickets filed by #70: #86 (file-search scope), #87 (`DisableCommandSearch`), #85 (Warp local tabs before the first workspace).

## Not established (needs the GUI or a run)

warpr was not run against an isolated Herdr session for this scan, so nothing below was observed:

- What "Close Current Session" (`RemoveActive` on the `HerdrWorkspacePane`) does: whether a confirmation shows and whether the workspace view is removed from its tab.
- Whether a workspace closed with "Close the Current Workspace" / "Other" / "Below" comes back on the next Herdr refresh or stays hidden until restart.
- That "Activate Next/Previous Pane" does nothing (inferred from a one-pane Warp pane tree).
- That "Open Repository" creates a local Warp terminal tab (inferred from `add_tab_with_pane_layout` having no Herdr branch; the folder picker was not driven).
- What the palette's "Open Settings: …" entries show for pages `should_render_page` hides (Billing, Referrals, Shared Blocks, Teams, AI, MCP Servers, Environments).
- Whether warpr's Herdr pane grid honours the terminal-surface settings the palette toggles (mouse/scroll/focus reporting, copy on select, middle-click paste, option-as-meta, cursor blink, alt-screen padding, audible bell, smart select, focus follows mouse); these are filed under #60.
- Whether "Turn Notifications On" affects Herdr's OS notifications (#31).
- Where a file opened from the Files palette appears on screen and how the vertical panel shows a Warp editor tab beside the workspaces (#93).
- The action list when the palette is opened with focus elsewhere (tools panel, code editor, settings, a rename editor): those chains add their own bindings and were not enumerated; only the `HerdrWorkspaceView` chain was.
- The list in a release build: the probe included `DEBUG_FLAGS` like the debug `warp-oss`. A build without them should drop the 32 rows marked debug-only; that each of them is behind `DEBUG_FLAGS` was read from where it is registered, not tested.
- The probe used the test auth state; bindings gated on login state may differ for an anonymous warpr user.
