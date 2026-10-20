# Search scope in warpr (#70)

Which of Warp's search features take their scope from Herdr (the session's workspaces, their working directories and their repositories), which are turned off, and which are already settled elsewhere. warpr has no non-Herdr window (ADR-0005), so "in warpr" means in a window that presents a Herdr session. The command palette's own sources and actions belong to #71 (`docs/research/command-palette-scan.md`); this document links to it for the shared surfaces and does not repeat its table.

## Result

| Decision | Features | Tickets |
|---|---|---|
| Scope to Herdr | 3 | #86 (file search, AI context menu files), #89 (find in a pane) |
| Disable | 2 | #87 `DisableCommandSearch`, #88 `DisableAIContextMenuCode` |
| Already handled | 13 | #39, #38/#51/#52, #48, #60, #71, `DisableWarpDrive`, `DisableHostedAgent`, scope-free |

The 0-workspace state turned up a bug of its own, #85: until Herdr presents a workspace the window keeps Warp's own local terminal tabs, so a search there is scoped to a Warp shell.

## The scope source

Every directory-scoped search reads one of two sources.

1. **Per tab: `WorkingDirectoriesModel`** (`app/src/pane_group/working_directories.rs`). `Workspace::refresh_working_directories_for_pane_group` feeds it from terminal cwds, editor paths and, since #39, `PaneGroup::herdr_workspace_directory` (#40). The Herdr directory is collapsed to its repository root through `DetectedRepositories` and maps to no Warp terminal. Global search and the file tree read this source.
2. **Per window: `ActiveSession`** (`app/src/workspace/active_session.rs`). Only `Workspace::update_active_session` sets it, from `pane_group.active_session_view(ctx)`, which is a Warp `TerminalView`. A Herdr workspace pane group has none, so the update is skipped and the window's working directory stays `None`, or keeps the value of a Warp terminal tab that existed earlier (#85). File search (`FileSearchModel`), the palette's Files source and the AI context menu read this source. #86 connects it.

**Default scope.** The default scope is the directory of the selected pane of the Herdr tab that the window's shown workspace displays: `HerdrWorkspaceView::focused_pane_directory`, reached through `PaneGroup::herdr_workspace_directory`. The selected pane is the pane the user picked in warpr, or Herdr's tab-focused pane until the user picks one. Herdr's focus is never moved to get it (#40). The repository is the one `DetectedRepositories` finds for that directory, which is the checkout the pane is in. `HerdrWorkspaceView::report_directory_change` runs that detection for every workspace view when its directory changes, and emits `PaneEvent::RepoChanged`, which re-scopes the tab.

**Behaviour by number of workspaces** (the behaviour #86 tests and #85 fixes):

| Workspaces | Default scope | Session repositories |
|---|---|---|
| 0 (connecting, refused, failed, or connected to an empty session) | none: searches show their empty state. Today the window instead keeps Warp's local terminal tabs and scopes to their shell (#85). | none |
| 1 | that workspace's selected-pane directory and its repository. A pane with no `cwd` from Herdr gives no scope and no error. | that repository |
| several | the workspace the window shows, never the one Herdr has focused. Switching tabs re-scopes, and each window has its own scope. | every workspace's directory resolved through `DetectedRepositories`, de-duplicated in Herdr's order, with a linked worktree as a root of its own |

**Search stays per workspace.** Warp scopes search per tab (`WorkingDirectoriesModel` is keyed by pane group, `GlobalSearchView` exists per pane group), and a warpr tab is a workspace. A search across all workspaces would mix unrelated repositories and would return every file of a repository once per open worktree. No search feature takes the session-wide list today. The table above defines it in case a cross-workspace scope is ever wanted.

**Is #48's repository list the right source? It is for pickers, not for search roots.** `HerdrRepositories::of` (`app/src/workspace/view/herdr_tabs.rs`) is the right source for "which repositories exist" (the New worktree picker, #48, and the palette's Repos source, #71). It is the wrong source for search, for two reasons:

- It resolves a linked worktree to its **main checkout**: `PresentedSession::repositories` gives `repo_root`, and `repository_root` follows `commondir`. A search from a worktree workspace would then return the main checkout's files, which belong to another branch.
- Its directory is `PresentedWorkspace::directory`, which is **Herdr's** focused pane of Herdr's selected tab, not the pane selected in warpr.

`DetectedRepositories` has neither problem, costs nothing extra, and already holds every workspace's repository. In the run below it indexed `repo-a` and its linked worktree `repo-a-wt` as separate repositories.

## Features

"Entry points" counts only those reachable in warpr. Palette rows that open a feature are listed in #71.

| # | Feature | Entry points | Scope source today | Decision | Reason |
|---|---|---|---|---|---|
| 1 | **Global search** (`workspace/view/global_search`, ripgrep over roots) | View menu item `CustomAction::ToggleGlobalSearch`, `OpenGlobalSearch` / `ToggleGlobalSearch` bindings, tools panel button, palette (#71) | `LeftPanelView::set_active_pane_group` → `WorkingDirectoriesModel::most_recent_directories_for_pane_group` → `GlobalSearchView::set_root_directories`. For a Herdr tab the root is the selected pane's repository root, or the directory outside a repository (#39). `should_enable_file_tree_and_global_search_for_pane_group` counts a Herdr workspace pane, so the panel is `Enabled` | **Already handled (#39)** | Follows the default scope per workspace. Remote roots (`remote_server::ripgrep_search`) only come from Warp SSH sessions, which Herdr panes never have |
| 2 | **File search** (`FileSearchModel`): the palette's Files source and the Files palette (`CustomAction::FilesPalette`) | View menu item `CustomAction::FilesPalette`, palette (#71) | `ActiveSession::working_directory(window)` in `repo_root_location` / `get_folder_contents`, and `path_if_local` for "Create file" in `FileDataSource::run_fuzzy_search_query`. It is `None` for Herdr tabs | **Scope to Herdr: default scope (#86)** | The data exists: `herdr_workspace_directory` plus `DetectedRepositories`, and the repository index is already built (log below). The palette decides the listing and wording (#71) |
| 3 | **Command search** (Ctrl-R, `search/command_search`): history, workflows, env-var collections, AI queries and Warp AI sources | View menu items `CustomAction::History` and `CustomAction::CommandSearch`, `workspace:show_command_search` and `input:search_command_history` (Input context only), local control `SurfaceCommandSearchOpen`, the input's `#` trigger | Warp `SessionId`: `show_command_search` returns with `report_error!("…no session is active!")` when `active_session_id` is `None`, which it is for every Herdr tab | **Disable: `DisableCommandSearch` (#87)** | Every source needs a Warp terminal session (history comes from Warp's shell hooks) or a Warp service (Drive, Warp AI). Herdr exposes no command history, so there is no cheap Herdr-backed source. Ctrl-R in a pane keeps reaching the shell |
| 4 | **Command search's projects source** (`ProjectDataSource`, `ProjectManagementModel`) | none: no mixer adds it (`search_code`: only its own module references `ProjectDataSource`) | `ProjectManagementModel::all_projects`, filled by `handle_open_repository` | **Already handled: unreachable** | No change needed. It disappears with #87 in any case |
| 5 | **Vertical tabs search** (workspace/pane navigation, `VerticalTabsPanelState::matching_tab_indices`) | search field of the vertical tabs panel | Herdr rows match on label, custom title, working directory, branch, PR label, diff stats, command (`herdr_details_search_fragments`) and, in Panes view, each pane's title, details and agent line (`PaneProps::rendered_search_text_fragments`, `TypedPane::HerdrWorkspace`) | **Already handled (#38, #51, #52)** | Searches across all workspaces of the window by design. Matching uses Herdr data only |
| 6 | **Palette navigation, Ctrl+Tab, Repos and the title-bar search bar** | palette | sessions/tabs, `PersistedWorkspace` (Repos) | **Already handled (#71)** | Owned by the palette scan. For Repos, #48's list is the right source (see above) |
| 7 | **AI context menu ("@ menu"): Files** (RepoFiles / CurrentFolderFiles) | `@` in Warp's input editor: `terminal::input::Input`, `CompactAgentInput` (the only hosts with `include_ai_context_menu: true`) | `ActiveSession::working_directory` for the category choice (`get_categories_for_mode`'s `is_active_dir_in_git_repo`) and for `snapshot_last_opened` | **Scope to Herdr: default scope (#86)** | Same source as row 2. A Herdr pane has no Warp input, so the menu has no host until #60 decides input. Once it has one, its files are already scoped |
| 8 | **AI context menu: Code** (symbols) | as row 7 | `ActiveSession` (`is_active_dir_in_git_repo`) plus `RepoOutlines`, built for every watched repository when `outline_codebase_symbols_for_at_context_menu` is on | **Disable: `DisableAIContextMenuCode` (#88)** | No host can show it, yet outlines are parsed for every workspace's repository (log below). #60 can turn the flag off if it gives CLI agents a Warp input |
| 9 | **AI context menu: DiffSet, Conversations, Skills** | as row 7, AI input mode only | `ActiveSession` (DiffSet) | **Already handled (`DisableHostedAgent`)** | Offered only in `is_ai_or_autodetect_mode`, which is Warp's hosted agent |
| 10 | **AI context menu: Workflows, Notebooks, Plans, Rules**, and the Drive palette source | as row 7, palette | Warp Drive | **Already handled (`DisableWarpDrive`)** | `show_warp_drive` is false |
| 11 | **AI context menu: Blocks, Commands** | as row 7 | Warp block list of the session | **Already handled (#60)** | Herdr panes have no blocks (ADR-0001) |
| 12 | **Find in a pane** (Cmd-F, `terminal/find`) | find bar binding in `TerminalView` | `TerminalView` block list / alt screen. A Herdr pane is a `herdr::PaneTerminal` (`AltScreen` only), so it has no find today | **Scope to Herdr: the selected pane's screen (#89)** | Cheap: `run_find_on_alt_screen` runs over `PaneTerminal::screen()`. Scrollback is not in the frame stream (ADR-0002 §3) |
| 13 | **Input-hosted searches**: block filter (`terminal/block_filter`, `blocklist_filter`), inline history, slash commands, external secrets, `ShellWidgetHandoff` Ctrl-T / Alt-C to fzf | Warp input / `TerminalView` | Warp session | **Already handled (#60)** | All require Warp's input and shell hooks |
| 14 | **Find in an opened buffer**: code editor find (`code/editor/find`), LSP find references, code review find (`code_review/find_model`), notebook/Markdown find bar | the opened editor, diff or file | the buffer itself | **Already handled: scope-free** | Nothing to scope. The buffers are opened from the #39-scoped file tree, global search and code review |
| 15 | **Warp Drive panel search** | Drive panel | Warp Drive | **Already handled (`DisableWarpDrive`)** | Not shown |
| 16 | **Hosted-agent searches**: conversation list search, codebase search, grep and web-search tools | agent conversation list, agent tools | Warp AI / codebase index | **Already handled (`DisableHostedAgent`)** | Off in every warpr build. `CodebaseIndexManager::handle_active_session_changed` is only reached from a Warp terminal, and #86 must not reach it |
| 17 | **Settings and keybindings search** (`search_terms`) | Settings | settings widgets | **Already handled: scope-free** | Not about directories |
| 18 | **New worktree repository picker filter** | New worktree modal | `HerdrRepositories` | **Already handled (#48)** | Lists the session's repositories, with the shown workspace's repository preselected |

## Evidence from running warpr

The run used an isolated session and never the default one. The setup was `herdr --session warpr-test-70 server` (Herdr 0.9.5, protocol 22), started with `nohup` under a throwaway `HERDR_CONFIG_PATH` (an empty file in a temp directory), and warpr's debug `warp-oss` from this commit, launched with `WARPR_HERDR_SESSION=warpr-test-70`. The fixture was `repo-a` (git, `main`), a linked worktree `repo-a-wt` on `feature`, and a non-git `plain-dir`. The session was stopped and deleted afterwards, and the temp directory removed.

- **Isolation.** `lsof -a -p <warpr> -U` showed warpr's only two Herdr connections, both to `~/.config/herdr/sessions/warpr-test-70/herdr.sock`. The server log shows warpr's `events.subscribe` (`request_id="warpr-events"`).
- **Warp local tabs before the first workspace (#85).** At launch the session was empty. warpr's terminal server bootstrapped two local Warp shells: `Shell is bootstrapped with session_id …` twice in `warp-oss.log`, and two `-zsh -g --no_rcs` children. These were restored app-state tabs. They stayed alive after `workspace create` and ended when warpr first presented workspaces (`Did not find child shell process … assuming it has already terminated`).
- **All workspaces' repositories are detected, worktrees separately.** After three workspaces were created (`repo-a` shown, `repo-a-wt` and `plain` not focused), the log had `Successfully indexed repository: …/repo-a with 1 files` and `…/repo-a-wt with 3 files`. `plain-dir` was not indexed. Detection runs for every workspace view, not only the shown one.
- **Outlines are built with no consumer (#88).** The log then showed `Parsing symbols for repo outline` and `Successfully constructed symbols outline for repo`, twice.
- **Incidental, not established.** The first workspace was created at 16:53:33 and was not presented until 16:55:20. That was right after warpr's `warpr-events` stream closed (16:55:08) and was reopened, and right after the next two creates. The screen was locked during the run (App Nap or the lock may explain it), so this was not filed.

## Not established (needs the GUI)

The screen was locked for the whole run, so nothing could be seen or clicked. These items come from code reading only:

- Global search results in a Herdr tab: whether hits come from the selected pane's repository, and whether picking a pane in another directory re-scopes an open global search, as #39 verified for the file tree.
- Palette file search in a Herdr tab today: the code gives `ActiveSession` `None`, so `repo_root` is `None` and the folder contents are empty. Whether the palette shows an empty list or another fallback was not observed.
- Whether the View menu's "Command Search" / "History" items are enabled with a Herdr pane focused, and what choosing one does.
- Vertical tabs search on Herdr rows (covered by #51/#52 tests, not exercised by hand here).
- Cmd-F in a Herdr pane (expected: nothing happens).
- What the window shows with 0 workspaces (#85): the restored local shells were observed, the tabs were not.
- Whether the delayed first presentation above is a warpr bug.

## Implementation tickets

| Ticket | Kind | Covers |
|---|---|---|
| #86 | enhancement | `ActiveSession`'s working directory follows the focused Herdr pane: file search, the palette's Files source, the AI context menu files. Tests for 0, 1 and several workspaces. Uses `DetectedRepositories`, not `HerdrRepositories` |
| #87 | enhancement | `DisableCommandSearch` kill switch, on in every warpr build: View menu items, bindings, local control, `show_command_search` |
| #88 | enhancement | `DisableAIContextMenuCode` kill switch, on in every warpr build: no `RepoOutlines` parsing, no Code category |
| #89 | enhancement | Find in a Herdr pane over its screen grid |
| #85 | bug | With no Herdr workspace presented, the window keeps Warp's local terminal tabs; 0 workspaces must mean no scope |
