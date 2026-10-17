# Can disabled features be stripped from the build? (issue #61)

Spike for [#61](https://github.com/rolurq/warpr/issues/61). Research only: no source, `Cargo.toml` or flag was changed, and no build was run.

## Answer

**Keep runtime flags as the only mechanism for every disabled feature. Compile-time stripping is not worth its merge-conflict cost for any of them.**

- The one feature that is already compile-time strippable, `crash_reporting` (Sentry), is already stripped. No warpr build path enables it.
- The only large saving is the hosted agent. It is about 22% of the compiled source, but the terminal, the settings pages, onboarding and warpr's own Herdr agent panel all depend on its types. It cannot be cfg-gated without a rewrite of upstream code.
- The remaining features are each at most about 3% of the source and remove at most 2 workspace crates and 0 external crates.
- If a stripped build is ever wanted, the feasible lever is an ADR-level decision to take the hosted agent out as a hard fork boundary. That decision belongs with #74, not with a Cargo feature (see [Interaction with #74](#interaction-with-74-herdr-only)).

All numbers below are **estimates** unless marked "counted". Their basis is stated in [Method](#method-and-limits).

## Method and limits

- **Inventory.** Walked the warpr kill-switch flags, `docs/adr/0001`–`0004`, `CONTEXT.md`, the issue #1 spec, and all 49 issues from `gh -R rolurq/warpr issue list --state all`. Read the gating commits `0b7a91bfc`, `aaf63490b`, `87a3367a0`, `b4a509928`, `1216c6668`, `cc8d09706`.
- **Code surface (counted).** Non-test `.rs` lines (`*_tests.rs`, `mod_test.rs`, `tests/` excluded) in each feature's modules and crates. "External files" are non-test files outside the feature's own modules that match a regex of its main types and paths. The regexes are approximate: they over-count common names and miss some indirect use, so read them as an order of magnitude.
- **Closure size (counted).** The `warp` package's normal-dependency closure for `x86_64-unknown-linux-gnu`, from `cargo metadata --offline --filter-platform`: 1,200 packages (1,125 external, 75 workspace), about 1.20M non-test source lines (app plus the workspace crates in that closure). `cargo tree -p warp -e normal` shows 938 distinct crate names for the same package; I did not reconcile this with the metadata count. All percentages use the metadata closure on both sides.
- **Dependency saving (estimate).** Removing a crate from the metadata graph and recomputing the closure gives the packages reachable only through it. It is an upper bound, because it assumes every source dependency on the crate was already cut.
- **Binary size and build time (estimate).** Taken as at most the line-count share of the closure. Real figures depend on monomorphisation, LTO and codegen units, and were not measured.
- **Compilability (inference).** Judged from reference counts, enum-variant and `match` spread, crate dependency edges and the persistence schema. No `cfg` experiment was compiled, so every "does not compile without it" verdict is `[INFERENCE]`.
- **Merge-conflict risk proxy.** Upstream commits since 2026-07-01 that touch each feature's paths (`git log upstream/master`, 1,233 commits in total, upstream tip `522481cd0` of 2026-10-14). A gate that touches a busy path conflicts often. This checkout's merge-base with upstream is `714c51f1e` of 2026-10-13, so a few later upstream commits are not in it.
- **Side effect.** The repo pins Rust 1.92.0 in its toolchain file; the first `cargo metadata` auto-installed that toolchain through rustup. Nothing in the repo changed.
- **Not delegated.** The `research` skill suggests a background agent. This spike was done in one agent session because the questions share one dependency graph.

## How warpr disables features today

- ADR-0001 (`docs/adr/0001-herdr-is-the-source-of-truth.md`) says Warp's account, Drive and hosted-agent services "are gated off with feature flags rather than deleted" so the rolling fork merges upstream.
- `AGENTS.md` says to prefer `FeatureFlag::X.is_enabled()` over `#[cfg]` unless the code cannot compile without it.
- warpr adds negative "kill-switch" flags to `crates/warp_features/src/lib.rs` (`DisableWarpAccount` … `DisableWarpUpdates`, lines 1014–1035; `TabLayoutChoice`, line 764). `enabled_features()` turns them on in one list (`app/src/features.rs:546-554`).
- The existing warpr footprint in upstream files is 108 files changed (2,978 insertions, 692 deletions) outside `app/src/herdr`, `crates/herdr_client`, `crates/herdr_fake` and tests (counted with `git diff --shortstat $(git merge-base HEAD upstream/master) HEAD`). Runtime gating already touches upstream call sites; cfg-gating touches the same sites and adds stubs.
- A Cargo feature in `app/Cargo.toml` usually only maps to a runtime flag (`app/src/features.rs`, `#[cfg(feature = "…")]` entries). Leaving it off does not remove code, only skips enabling the flag. `crash_reporting` is the one feature that removes code and dependencies: `dep:sentry`, `dep:sentry-log`, `dep:minidumper`, `dep:crash-handler`, plus the `crash_reporting` features of `warp_logging`, `warp_errors`, `warp_terminal` and `ai` (`app/Cargo.toml:491-500`).

## Inventory: disabled features and where each was found

| # | Feature | How it is disabled | Found in |
|---|---|---|---|
| 1 | Warp account (login wall, sign-up, account/billing/teams/referral pages) | `DisableWarpAccount` on | ticket #4 (closed), commit `0b7a91bfc`, `app/src/features.rs:549`, `app/src/settings_view/mod.rs:2114` |
| 2 | Warp Drive | `DisableWarpDrive` on | #4, `0b7a91bfc`, `aaf63490b` (Drive keybindings, auth deep links), `settings_view/mod.rs:2142` |
| 3 | Hosted agent (Agent Mode, conversations, cloud/ambient agents, AI suggestions, related settings pages) | `DisableHostedAgent` on | #5 (closed), `87a3367a0`, `b4a509928`, `settings_view/mod.rs:2127` |
| 4 | Usage telemetry and crash reporting | `DisableUsageTelemetry` on; `crash_reporting` Cargo feature never enabled | #5, `87a3367a0`, `app/src/settings/privacy.rs:201`, `app/src/bin/oss.rs:18-19` (`telemetry_config: None`, `crash_reporting_config: None`) |
| 5 | Warp updates and changelog | `DisableWarpUpdates` on; `autoupdate` Cargo feature not in `default` | `b4a509928`, `app/src/features.rs:26` |
| 6 | Horizontal/vertical tab layout choice | `TabLayoutChoice` off | #63 (closed), `cc8d09706`, `warp_features/src/lib.rs:764` |
| 7 | Privacy settings section | not yet gated; only the telemetry value is forced off | #58 (open); the page is missing from `should_render_page` (`settings_view/mod.rs:2113-2146`) |
| 8 | Account settings section (the "Account" page; `main_page.rs`) | not yet gated beyond feature 1's pages | #58 (open) |
| 9 | Warp links: What's New, Documentation, Warp logs, Slack, Feedback | not yet gated | #56 (open), `app/src/app_menus.rs:960-969`, `app/src/util/links.rs` |
| 10 | Blocks, Warpify and Warp's input editor on Herdr panes | not yet decided; "disabled if they cannot be supported" | #60 (open), ADR-0001 Consequences |
| 11 | Workspace config (`tab_configs`, launch configs) | not yet decided | #49 (open) |
| 12 | Reopen closed session (`undo_close`) | not yet decided | #50 (open) |
| 13 | Command-palette actions that do not apply to Herdr | not yet decided | #71 (open) |
| 14 | Warp-local search sources | not yet decided | #70 (open) |
| 15 | User groups and group pinning in Herdr windows | window-level Herdr predicate, not a flag | commits `9eb7aa557`, `9ed6fb3c8`; #74 |
| 16 | Local PTY and shell spawning (dead once warpr is Herdr-only) | not disabled; candidate only | ADR-0001 ("warpr owns no terminals"), #74, `app/Cargo.toml:795` |

Rows 7–14 are *planned* disables from open issues. Each ends with the same convention ("a feature flag turned off, never deleting it"), so the same conclusion applies to them.

## Per-feature findings

Columns: **Surface** = own non-test lines and files (counted); **Spread** = external files / matches of the regex (counted, approximate); **Churn** = upstream commits since 2026-07-01 touching the paths. Percentages are of the 1.20M-line closure.

### 1. Warp account

- **Surface:** `app/src/auth`, `crates/warp_server_auth`, `crates/firebase`, `app/src/billing`, `app/src/workspaces`, `app/src/pricing`, `app/src/usage`: 38 files, 15.7k lines (1.3%). The account, billing, teams and referral settings pages are another 20.0k lines (1.7%).
- **Spread:** `auth` is used by 138 external files / 678 matches, mostly `ai` (32), `settings_view` (16), `terminal` (16), `server` (11). `AuthStateProvider` appears in 101 non-test files. `workspaces`/billing types appear in 214 external files / 330 matches (`ai` 80).
- **Compiles without it?** No `[INFERENCE]`. `crates/cloud_objects` and `crates/warp_server_client` depend on `warp_server_auth`. `User` and team types appear in persistence (`teams`, `workspace_teams` tables in `crates/persistence/src/schema.rs`) and in server API signatures. The only existing precedent is `skip_login`, which replaces the token source, not the types (`app/Cargo.toml:846`, `app/src/workspace/mod.rs:1517`).
- **Estimated saving:** at most 3% of source. 0 external crates; 2 workspace crates (`warp_server_auth`, `firebase`) become unreachable only if `cloud_objects`/`warp_server_client` are also cut.
- **Merge-conflict risk:** high. 138 files, auth-adjacent `workspaces`/`auth` churn 74 commits.
- **Recommendation:** runtime flag only. Finish #58 by adding Account and Privacy to `should_render_page`.

### 2. Warp Drive

- **Surface:** `app/src/drive`, `app/src/cloud_object`, `crates/cloud_object_{client,models,persistence}`, `crates/cloud_objects`: 86 files, 32.4k lines (2.7%). Notebooks, workflows and env vars (Drive object types, also reachable outside Drive) add 30.9k lines (2.6%).
- **Spread:** 226 external files / 1,822 matches, mainly `ai` (49), `search` (31), `terminal` (19), `server` (13), `workflows` (13). `CloudModel` appears in 109 non-test files. `ai` depends on Drive in 49 files and Drive depends on `ai` in 7.
- **Compiles without it?** No `[INFERENCE]`. `crates/cloud_object_models` depends on the `ai` crate and `warp_cli`. Persistence keeps the Drive tables and 141 migrations that cannot be cfg-gated (Diesel `schema.rs`, 50 tables). Workflows and notebooks double as local features.
- **Estimated saving:** 2.7–5.3% of source; 0 external crates, 4 workspace crates (`cloud_object_*`, `cloud_objects`) at most.
- **Merge-conflict risk:** medium-high. 226 files, but Drive churn is low (29 commits in `drive`/`cloud_object*`; 18 in notebooks/workflows).
- **Recommendation:** runtime flag only. Revisit only if the hosted agent is cut (the two share the `ai`↔Drive cycle).

### 3. Hosted agent

- **Surface:** `app/src/ai` (419 files, 228k lines), `ai_assistant`, `suggestions`, `crates/ai`, `ai_types`, `warp_multi_agent_client`, `computer_use`, `mcp`, `warp_harness_usage`: 553 files, 268.8k lines (22.5%). Related settings pages are in the 20.0k line figure under row 1/8.
- **Spread:** 354 external files / 1,892 matches. `terminal` alone has 133, `settings_view` 26, `warp_tui` 20, `workspace` 19, `pane_group` 16, `search` 16. 129 of the 327 non-test `terminal` files import `crate::ai`.
- **Compiles without it?** No `[INFERENCE]`. Three blockers:
  1. The terminal block list uses the AI block types and `BlocklistAIHistoryModel`.
  2. warpr's Herdr notifications import `crate::ai::agent_management::{AgentNotificationsModel, notifications::…}` (`app/src/herdr/notifications.rs:13-14`), and `app/src/workspace/view.rs:178-185` mounts `AgentManagementView`. The Herdr agent panel *is* the repurposed agent-management UI (issue #1, "Agent panel").
  3. `crates/onboarding` and `crates/cloud_object_models` depend on the `ai` crate.
- **Estimated saving:** the largest. Up to about 22% of source (so, as an upper bound, up to about 22% of binary text and compile time). Dependencies: 15 external crates (`rmcp`, `rmcp-macros`, `sse-stream`, `ashpd`, `priority-queue`, `line-span`, `process-wrap`, `pastey`, `warp_multi_agent_api`, …) and 6 workspace crates. That is 1.3% of external packages: the heavy shared stack (`tokio`, `reqwest`, `diesel`, `wgpu`) stays.
- **Merge-conflict risk:** very high. `ai` paths take 460 of 1,233 upstream commits (37%). Any `cfg` boundary through `terminal`, `settings_view` or `workspace` also collides with the 282 / 105 / 107 commits there.
- **Recommendation:** runtime flag only. Not feasible as a cfg gate without a rewrite that decouples the Herdr agent panel from `ai::agent_management`. See #74.

### 4. Usage telemetry and crash reporting

- **Surface:** `app/src/server/telemetry` (8.4k lines of which `events.rs` is 7.2k lines, 0.7%), `crates/warp_core/src/telemetry.rs`, `app/src/crash_reporting` (1.2k lines).
- **Spread:** `send_telemetry*!` macros: 867 call sites in 193 files (counted). The macros live in one file (`app/src/server/telemetry/macros.rs`), so a no-op swap would be a small diff, but the `TelemetryEvent` types and `PrivacySettings` (50 files / 224 matches) stay.
- **Compiles without it?** Partly. `crash_reporting` already compiles out (33 `cfg` uses in 18 files, plus `crates/warp_errors`, `warp_logging`, `ai`, `warp_terminal`). Usage telemetry cannot be stripped without deleting `TelemetryEvent` construction at all 867 sites.
- **Estimated saving:** crash reporting: already realized. `crash_reporting` is in no `default` list; the only scripts that enable it are the remote-server deploy scripts (`script/deploy_remote_server`, `…_to_test_vm`). Sentry crates are not in the closure. Usage telemetry: at most 0.7% of source, 0 attributable external crates. The `opentelemetry*` crates (6 external) are in the closure unconditionally; I did not trace whether they serve usage telemetry or only tracing, so they are not attributed.
- **Merge-conflict risk:** low for the macro, high if the call sites are touched. Telemetry paths have 28 upstream commits; the call sites are everywhere.
- **Recommendation:** no change. The `oss` channel already ships `telemetry_config: None` and `crash_reporting_config: None` (`app/src/bin/oss.rs:18-19`), so the kill switch is belt and braces. Do not add `crash_reporting` to any warpr release build; that is the whole compile-time lever.

### 5. Warp updates and changelog

- **Surface:** `app/src/autoupdate`, `changelog_model.rs`, `crates/channel_versions`: 11 files, 3.8k lines (0.3%).
- **Spread:** 19 external files / 85 matches (`warp_tui` 4, `resource_center` 3, `workspace` 2).
- **Compiles without it?** Likely `[INFERENCE]`; the small spread and the existing `autoupdate` Cargo feature suggest a clean cut, but it was not tried.
- **Estimated saving:** at most 0.3% of source; 1 workspace crate (`channel_versions`), 0 external crates. The runtime flag `FeatureFlag::Autoupdate` is already off because the `autoupdate` Cargo feature is not in `default` (`app/src/autoupdate/mod.rs:144`, `app/src/features.rs:26`).
- **Merge-conflict risk:** low (6 upstream commits).
- **Recommendation:** runtime flag only. The gain is too small.

### 6. Tab layout choice (horizontal bar)

- **Surface:** the horizontal tab bar code is the Herdr tab strip after #63, so it is live code.
- **Spread:** `TabLayoutChoice` has 13 production references in 7 files (counted).
- **Compiles without it?** Not applicable.
- **Estimated saving:** about 0.
- **Merge-conflict risk:** n/a.
- **Recommendation:** runtime flag only; nothing to strip.

### 7–8. Privacy and Account settings sections (#58)

- **Surface:** `privacy_page.rs` 2.0k lines, `main_page.rs` (account) 1.3k lines, `settings/privacy.rs` 0.9k lines. Combined with row 1 pages.
- **Spread:** `PrivacySettings` is referenced from 50 external files / 224 matches (`terminal` 10, `ai` 6, `server` 6, `auth` 5). `SettingsPageViewHandle` and `SettingsSection` variants are matched in 7 non-test files (21 matches for `Teams` alone); a `cfg` on a variant needs a cfg on every arm.
- **Compiles without it?** No for privacy `[INFERENCE]`: the setting is read in the terminal, AI and server layers. Yes for the pages (leaf views), but a variant removal touches the `match` arms.
- **Estimated saving:** about 0.4% of source (the two pages), 0 crates.
- **Merge-conflict risk:** medium. `settings_view` has 105 upstream commits.
- **Recommendation:** runtime flag only: add `SettingsSection::Account` and `Privacy` to `should_render_page`, as the existing flags do (`settings_view/mod.rs:2113-2146`).

### 9. Warp links (#56)

- **Surface:** link constants in `app/src/util/links.rs`, the Help menu (`app_menus.rs:960-969`), `resource_center` (13 files, 4.4k lines) and the What's New and changelog code. Hard-coded `warp.dev` URLs appear in 111 non-test files; they are tooltips and doc links, not features.
- **Spread:** `links::` is used in 13 files; `resource_center` in 9.
- **Compiles without it?** Not meaningful: these are string constants and menu entries.
- **Estimated saving:** 0.4% (`resource_center` plus `tips`), 0 crates.
- **Merge-conflict risk:** low (5 upstream commits in `app_menus.rs`, 4 in `resource_center`).
- **Recommendation:** runtime flag only.

### 10. Blocks, Warpify and Warp's input editor (#60)

- **Surface:** Warpify (`app/src/terminal/warpify`, `terminal/ssh`, `settings_view/warpify_page.rs`) is 8 files, 2.8k lines (0.2%). The block model (`terminal/model/block*`, 10.4k lines) is the terminal grid model.
- **Spread:** Warpify has 40 external files / 296 matches (`terminal` 23). The block model has 177 external files / 991 matches, including `warp_tui` (10) and `crates/editor` (9). The Herdr module itself uses `terminal::model` (9 imports in `app/src/herdr`).
- **Compiles without it?** Blocks: no. warpr feeds Herdr output into the same terminal model (ADR-0003), so blocks are used as the grid. Warpify: plausible `[INFERENCE]` but 40 files import it.
- **Estimated saving:** blocks: none. Warpify: 0.2%, 0 crates.
- **Merge-conflict risk:** blocks: very high (`terminal` has 282 upstream commits). Warpify: low churn (5 commits) but wide imports.
- **Recommendation:** runtime flag only; blocks stay.

### 11. Workspace config (#49)

- **Surface:** `app/src/tab_configs`, `launch_configs`: 15 files, 5.4k lines (0.5%).
- **Spread:** 20 external files / 54 matches (`workspace` 5, `search` 4, `user_config` 4).
- **Compiles without it?** Plausible `[INFERENCE]`. The Cargo feature `tab_configs` already exists but only enables a runtime flag.
- **Estimated saving:** at most 0.5%, 0 crates.
- **Merge-conflict risk:** low-medium (9 upstream commits).
- **Recommendation:** runtime flag only.

### 12. Reopen closed session (#50)

- **Surface:** `app/src/undo_close`: 3 files, 467 lines (0.04%).
- **Spread:** 12 external files / 13 matches.
- **Compiles without it?** Plausible `[INFERENCE]`; the Cargo feature `undo_closed_panes` only maps to a runtime flag.
- **Estimated saving:** negligible. **Risk:** low (7 commits).
- **Recommendation:** runtime flag only.

### 13–14. Command palette and search (#71, #70)

These are per-action and per-source decisions inside shared code (`palette.rs`, `command_palette.rs`, `search`), not separable features. No module or crate boundary exists to cut.
- **Recommendation:** runtime flag only.

### 15. User groups and group pinning

- **Surface:** window-level branches in the workspace and tab code; 8 non-test files use `presents_herdr_workspaces` or `herdr::wording`.
- **Compiles without it?** Not applicable: it is a branch, not a module.
- **Recommendation:** remove with #74. This is warpr's own dual-mode code, which the #74 text says should go.

### 16. Local PTY and shell spawning (candidate)

- **Surface:** `local_tty` is a Cargo feature that `build.rs` sets for native platforms (`app/Cargo.toml:792-795`), with 185 `cfg` uses in 31 files.
- **Compiles without it?** Not investigated. warpr reuses the byte-fed terminal manager that has no local PTY (issue #1), so the cfg boundary may already be partly in place `[INFERENCE]`.
- **Estimated saving:** not estimated.
- **Recommendation:** open a follow-up spike once #74 lands. Unlike rows 1–15 this is not a disabled upstream feature; the cfg already exists, so the conflict cost is lowest here.

## Summary

| Feature | Own lines (% of closure) | External crates saved (upper bound) | Conflict risk | Compiles without it? | Recommendation |
|---|---|---|---|---|---|
| Account | 15.7k (1.3%) + 20.0k pages (1.7%) | 0 | high | no `[INFERENCE]` | runtime flag |
| Drive | 32.4k (2.7%) | 0 | medium-high | no `[INFERENCE]` | runtime flag |
| Hosted agent | 268.8k (22.5%) | 15 | very high | no `[INFERENCE]` | runtime flag |
| Usage telemetry / crash reporting | 8.4k (0.7%) | 0 attributable | high at call sites | partly | none; keep `crash_reporting` off |
| Updates / changelog | 3.8k (0.3%) | 0 | low | likely | runtime flag |
| Tab layout choice | live code | 0 | n/a | n/a | runtime flag |
| Privacy / Account sections | about 4.2k (0.4%) | 0 | medium | pages yes, setting no | runtime flag |
| Warp links | 4.4k (0.4%) | 0 | low | not meaningful | runtime flag |
| Warpify (blocks stay) | 2.8k (0.2%) | 0 | low churn, wide imports | plausible | runtime flag |
| Workspace config | 5.4k (0.5%) | 0 | low-medium | plausible | runtime flag |
| Reopen closed session | 0.5k (0.04%) | 0 | low | plausible | runtime flag |
| Palette / search | n/a | 0 | n/a | n/a | runtime flag |
| Groups / pinning | n/a | 0 | n/a | n/a | remove in #74 |
| Local PTY | not estimated | not estimated | low | not investigated | follow-up spike after #74 |

Combined, removing every crate from rows 1–5 gives at most 56 packages (39 external, 17 workspace) out of 1,200, about 4.7%, and the line-count share of rows 1–5 is about 29% (hosted agent dominates). Both figures are upper bounds and assume every dependent was already cut.

## Interaction with #74 (Herdr-only)

[#74](https://github.com/rolurq/warpr/issues/74) removes warpr's *own* Herdr/non-Herdr branching and proposes an ADR amendment: upstream code is disabled behind flags, warpr's own dual-mode code is removed, and where removal touches upstream the minimal edit that makes the Herdr behavior unconditional wins.

- **No conflict with this note's answer.** #74 removes warpr-authored branches, not upstream feature modules, so runtime flags stay the default for rows 1–14.
- **It changes row 16.** Once no window can be non-Herdr, the local PTY path (and the local new-tab and worktree flows that #74 lists) is dead code. That is the first place where a compile-time boundary could pay off, and `local_tty` is an existing cfg. Evaluate it after #74 lands.
- **It makes the ADR the right place for a hosted-agent decision.** If the project ever wants to remove the 22% hosted agent, the Herdr agent panel's dependency on `ai::agent_management` has to be broken first. The ADR #74 asks for is the place to record whether warpr's own code may depend on upstream `ai` types or must be moved out.
- **Risk of doing both at once.** #74 edits `workspace/view.rs`, `app_menus.rs` and `tab` code that also holds the kill-switch checks. A compile-time strip in the same files would stack a second conflict surface; sequence #74 first.
- **Sequencing note.** Several open tickets (#49, #50, #60, #71) still say "non-Herdr windows unchanged". Those lines become moot under #74, which also removes the "in Herdr windows" conditions that gate rows 11–13.

## Open questions

- Does `cargo build` with the hosted agent cut even partly reduce build time, given LTO and codegen units? Needs a measured build, out of scope here.
- Why does `cargo tree` (938 names) differ from the metadata closure (1,093 names)? Not reconciled; it does not change any recommendation, only the percentage denominators.
- Local PTY (row 16): is there already a `cfg(not(local_tty))` configuration that compiles and passes the Herdr tests?
