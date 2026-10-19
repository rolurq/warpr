use std::collections::{HashMap, HashSet};
use std::iter::once;
use std::path::{Path, PathBuf};

use herdr_client::{HerdrCli, HerdrTabId, SessionName, SessionTarget, WorkspaceId};
use herdr_fake::{FakeHerdr, FakeSession, PaneTree, SessionScript, SplitDirection, TabScript};
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::Vector2F;
use settings::Setting as _;
use warp_core::features::FeatureFlag;
use warp_core::ui::appearance::Appearance;
use warpui::elements::PositionedElementOffsetBounds;
use warpui::platform::WindowStyle;
use warpui::{App, EntityId, SingletonEntity as _, TypedActionView as _, ViewHandle};

use super::{
    AgentTabTextPreference, BranchLineBadges, HerdrRowDetails, HerdrRowTitle, SummaryPaneKind,
    SummaryPaneKindIcons, TerminalAgentText, TerminalPrimaryLineData, TerminalPrimaryLineFont,
    VerticalTabsDetailTarget, VerticalTabsDetailTargetKind, VerticalTabsPanelState,
    VerticalTabsSummaryBranchEntry, VerticalTabsSummaryData, VerticalTabsSummaryPrimaryLabel,
    branch_label_display, coalesce_summary_branch_entries, code_detail_kind_label,
    compact_branch_subtitle_display, detail_sidecar_width_and_bounds,
    detail_target_for_hovered_row, group_display_name, group_name_highlight_indices,
    group_subtitle_text, herdr_agent_line, herdr_compact_subtitle, herdr_details_below_title,
    herdr_pane_compact_subtitle, herdr_row_details_from, herdr_row_title, herdr_shown_title,
    matched_group_ids, merge_group_name_matches, non_terminal_search_text_fragments,
    pane_ids_for_display_granularity, pane_search_text_fragments, preferred_agent_tab_titles,
    push_normalized_unique_summary_label, render_settings_popup_menu, render_summary_branch_line,
    search_fragments_contain_query, select_summary_pane_kind_icons,
    should_keep_detail_sidecar_visible_for_mouse_position, should_show_tab_group_header,
    shows_synced_inputs_indicator, sort_summary_primary_labels_status_first,
    summary_overflow_count, summary_search_text_fragments, tab_admitted_by_group_name,
    terminal_kind_badge_label, terminal_primary_line_data, terminal_pull_request_badge_label,
    terminal_search_text_fragments, terminal_title_fallback_font, uses_outer_group_container,
    visible_pane_ids_for_detail_target, vtab_diff_stats_text,
};
use crate::ai::agent::conversation::ConversationStatus;
use crate::app_state::{HerdrWorkspaceContents, HerdrWorkspaceSnapshot};
use crate::context_chips::display_chip::GitLineChanges;
use crate::herdr::{
    DirectoryGitContext, HerdrSessionModel, HerdrWorkspaceView, HerdrWorkspaceViewAction,
};
use crate::pane_group::pane::IPaneType;
use crate::pane_group::{PaneId, TerminalPaneId};
use crate::safe_triangle::SafeTriangle;
use crate::tab::{ShortcutModifierKind, reveals_shortcut_hints};
use crate::terminal::CLIAgent;
use crate::test_util::assert_eventually;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspace::tab_group::{TabGroup, TabGroupId};
use crate::workspace::tab_settings::{
    TabSettings, VerticalTabsCompactSubtitle, VerticalTabsDisplayGranularity,
    VerticalTabsPrimaryInfo, VerticalTabsTabItemMode, VerticalTabsViewMode,
};
use crate::workspace::view::vertical_tabs::telemetry::VerticalTabsChipEntrypoint;

fn label(text: &str) -> VerticalTabsSummaryPrimaryLabel {
    VerticalTabsSummaryPrimaryLabel {
        text: text.to_string(),
        status: None,
    }
}

fn pane_id() -> PaneId {
    TerminalPaneId::dummy_terminal_pane_id().into()
}
fn code_summary_kind(title: &str) -> SummaryPaneKind {
    SummaryPaneKind::Code {
        title: title.to_string(),
    }
}

#[test]
fn summary_pane_kind_icons_render_single_icon_for_homogeneous_tabs() {
    assert_eq!(
        select_summary_pane_kind_icons([
            (EntityId::from_usize(10), SummaryPaneKind::Terminal),
            (EntityId::from_usize(20), SummaryPaneKind::Terminal),
        ]),
        Some(SummaryPaneKindIcons::Single(SummaryPaneKind::Terminal))
    );
}

#[test]
fn summary_pane_kind_icons_pick_two_oldest_distinct_pane_kinds() {
    assert_eq!(
        select_summary_pane_kind_icons([
            (EntityId::from_usize(30), SummaryPaneKind::Terminal),
            (EntityId::from_usize(20), code_summary_kind("main.rs")),
            (
                EntityId::from_usize(40),
                SummaryPaneKind::Notebook { is_plan: false },
            ),
            (EntityId::from_usize(10), SummaryPaneKind::Terminal),
        ]),
        Some(SummaryPaneKindIcons::Pair {
            primary: SummaryPaneKind::Terminal,
            secondary: code_summary_kind("main.rs"),
        })
    );
}

#[test]
fn summary_pane_kind_icons_recompute_when_oldest_kind_is_removed() {
    assert_eq!(
        select_summary_pane_kind_icons([
            (EntityId::from_usize(20), code_summary_kind("main.rs")),
            (EntityId::from_usize(30), SummaryPaneKind::Terminal),
        ]),
        Some(SummaryPaneKindIcons::Pair {
            primary: code_summary_kind("main.rs"),
            secondary: SummaryPaneKind::Terminal,
        })
    );
}

#[test]
fn summary_pane_kind_icons_distinguish_agent_terminals_from_plain_terminals() {
    assert_eq!(
        select_summary_pane_kind_icons([
            (EntityId::from_usize(10), SummaryPaneKind::Terminal),
            (
                EntityId::from_usize(20),
                SummaryPaneKind::CLIAgent {
                    agent: CLIAgent::Claude,
                    is_ambient: false,
                },
            ),
            (
                EntityId::from_usize(30),
                SummaryPaneKind::OzAgent { is_ambient: false },
            ),
        ]),
        Some(SummaryPaneKindIcons::Pair {
            primary: SummaryPaneKind::Terminal,
            secondary: SummaryPaneKind::CLIAgent {
                agent: CLIAgent::Claude,
                is_ambient: false,
            },
        })
    );
}

#[test]
fn summary_pane_kind_icons_distinguish_ambient_claude_from_local_claude() {
    // A local Claude session and a cloud-mode Claude session should count as distinct kinds
    // so they render with different icons (claude.svg vs claude_cloud.svg).
    assert_eq!(
        select_summary_pane_kind_icons([
            (
                EntityId::from_usize(10),
                SummaryPaneKind::CLIAgent {
                    agent: CLIAgent::Claude,
                    is_ambient: false,
                },
            ),
            (
                EntityId::from_usize(20),
                SummaryPaneKind::CLIAgent {
                    agent: CLIAgent::Claude,
                    is_ambient: true,
                },
            ),
        ]),
        Some(SummaryPaneKindIcons::Pair {
            primary: SummaryPaneKind::CLIAgent {
                agent: CLIAgent::Claude,
                is_ambient: false,
            },
            secondary: SummaryPaneKind::CLIAgent {
                agent: CLIAgent::Claude,
                is_ambient: true,
            },
        })
    );
}

#[test]
fn preferred_agent_tab_titles_default_to_title_like_text() {
    let agent_text = TerminalAgentText {
        conversation_display_title: Some("Generated Warp Agent title".to_string()),
        conversation_latest_user_prompt: Some("Latest Warp Agent prompt".to_string()),
        cli_agent_title: Some("CLI summary".to_string()),
        cli_agent_latest_user_prompt: Some("Latest CLI prompt".to_string()),
        is_oz_agent: true,
        cli_agent: Some(CLIAgent::Claude),
    };

    assert_eq!(
        preferred_agent_tab_titles(&agent_text, AgentTabTextPreference::ConversationTitle),
        (
            Some("Generated Warp Agent title".to_string()),
            Some("CLI summary".to_string())
        )
    );
}

#[test]
fn preferred_agent_tab_titles_do_not_use_cli_prompt_when_disabled() {
    let agent_text = TerminalAgentText {
        conversation_display_title: None,
        conversation_latest_user_prompt: None,
        cli_agent_title: None,
        cli_agent_latest_user_prompt: Some("Latest CLI prompt".to_string()),
        is_oz_agent: false,
        cli_agent: Some(CLIAgent::Claude),
    };

    assert_eq!(
        preferred_agent_tab_titles(&agent_text, AgentTabTextPreference::ConversationTitle),
        (None, None)
    );
}

#[test]
fn terminal_primary_line_uses_terminal_title_when_disabled_cli_has_only_prompt() {
    let agent_text = TerminalAgentText {
        conversation_display_title: None,
        conversation_latest_user_prompt: None,
        cli_agent_title: None,
        cli_agent_latest_user_prompt: Some("Latest CLI prompt".to_string()),
        is_oz_agent: false,
        cli_agent: Some(CLIAgent::Claude),
    };
    let (conversation_title, cli_title) =
        preferred_agent_tab_titles(&agent_text, AgentTabTextPreference::ConversationTitle);

    let line = terminal_primary_line_data(
        false,
        conversation_title,
        cli_title,
        "Generated Claude Code title",
        "~/warp",
        terminal_title_fallback_font(&agent_text),
        Some("claude".to_string()),
    );

    assert_eq!(line.text(), "Generated Claude Code title");
    assert!(matches!(
        line,
        TerminalPrimaryLineData::Text {
            font: TerminalPrimaryLineFont::Ui,
            ..
        }
    ));
}

#[test]
fn preferred_agent_tab_titles_use_latest_prompt_when_enabled() {
    let agent_text = TerminalAgentText {
        conversation_display_title: Some("Generated Warp Agent title".to_string()),
        conversation_latest_user_prompt: Some("Latest Warp Agent prompt".to_string()),
        cli_agent_title: Some("CLI summary".to_string()),
        cli_agent_latest_user_prompt: Some("Latest CLI prompt".to_string()),
        is_oz_agent: true,
        cli_agent: Some(CLIAgent::Claude),
    };

    assert_eq!(
        preferred_agent_tab_titles(&agent_text, AgentTabTextPreference::LatestUserPrompt),
        (
            Some("Latest Warp Agent prompt".to_string()),
            Some("Latest CLI prompt".to_string())
        )
    );
}

#[test]
fn terminal_primary_line_uses_cli_prompt_when_enabled_cli_has_prompt() {
    let agent_text = TerminalAgentText {
        conversation_display_title: None,
        conversation_latest_user_prompt: None,
        cli_agent_title: None,
        cli_agent_latest_user_prompt: Some("Latest CLI prompt".to_string()),
        is_oz_agent: false,
        cli_agent: Some(CLIAgent::Claude),
    };
    let (conversation_title, cli_title) =
        preferred_agent_tab_titles(&agent_text, AgentTabTextPreference::LatestUserPrompt);

    let line = terminal_primary_line_data(
        false,
        conversation_title,
        cli_title,
        "Generated Claude Code title",
        "~/warp",
        terminal_title_fallback_font(&agent_text),
        Some("claude".to_string()),
    );

    assert_eq!(line.text(), "Latest CLI prompt");
}

#[test]
fn terminal_primary_line_uses_cli_prompt_when_enabled_cli_is_long_running() {
    let agent_text = TerminalAgentText {
        conversation_display_title: None,
        conversation_latest_user_prompt: None,
        cli_agent_title: None,
        cli_agent_latest_user_prompt: Some("Latest CLI prompt".to_string()),
        is_oz_agent: false,
        cli_agent: Some(CLIAgent::Claude),
    };
    let (conversation_title, cli_title) =
        preferred_agent_tab_titles(&agent_text, AgentTabTextPreference::LatestUserPrompt);

    let line = terminal_primary_line_data(
        true,
        conversation_title,
        cli_title,
        "Generated Claude Code title",
        "~/warp",
        terminal_title_fallback_font(&agent_text),
        Some("claude".to_string()),
    );

    assert_eq!(line.text(), "Latest CLI prompt");
}

#[test]
fn preferred_agent_tab_titles_fall_back_when_preferred_text_is_missing() {
    let agent_text = TerminalAgentText {
        conversation_display_title: Some("Generated Warp Agent title".to_string()),
        conversation_latest_user_prompt: None,
        cli_agent_title: None,
        cli_agent_latest_user_prompt: Some("Latest CLI prompt".to_string()),
        is_oz_agent: true,
        cli_agent: Some(CLIAgent::Claude),
    };

    assert_eq!(
        preferred_agent_tab_titles(&agent_text, AgentTabTextPreference::LatestUserPrompt),
        (
            Some("Generated Warp Agent title".to_string()),
            Some("Latest CLI prompt".to_string())
        )
    );
}

fn pane_type_supports_vertical_tabs_detail_sidecar(pane_type: IPaneType) -> bool {
    matches!(
        pane_type,
        IPaneType::Terminal
            | IPaneType::Code
            | IPaneType::Notebook
            | IPaneType::Workflow
            | IPaneType::EnvVarCollection
            | IPaneType::AIFact
            | IPaneType::AIDocument
    )
}

fn collect_normalized_unique_summary_texts(
    texts: impl IntoIterator<Item = impl AsRef<str>>,
) -> Vec<String> {
    texts
        .into_iter()
        .filter_map(|text| {
            let normalized = text
                .as_ref()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            (!normalized.is_empty()).then_some(normalized)
        })
        .fold(Vec::new(), |mut values, normalized| {
            if !values.contains(&normalized) {
                values.push(normalized);
            }
            values
        })
}

#[test]
fn detail_sidecar_supports_terminal_code_and_warp_drive_object_panes() {
    assert!(pane_type_supports_vertical_tabs_detail_sidecar(
        IPaneType::Terminal
    ));
    assert!(pane_type_supports_vertical_tabs_detail_sidecar(
        IPaneType::Code
    ));
    assert!(pane_type_supports_vertical_tabs_detail_sidecar(
        IPaneType::Notebook
    ));
    assert!(pane_type_supports_vertical_tabs_detail_sidecar(
        IPaneType::Workflow
    ));
    assert!(pane_type_supports_vertical_tabs_detail_sidecar(
        IPaneType::EnvVarCollection
    ));
    assert!(pane_type_supports_vertical_tabs_detail_sidecar(
        IPaneType::AIFact
    ));
    assert!(pane_type_supports_vertical_tabs_detail_sidecar(
        IPaneType::AIDocument
    ));
    assert!(!pane_type_supports_vertical_tabs_detail_sidecar(
        IPaneType::Settings
    ));
}

#[test]
fn code_detail_kind_label_uses_programming_language_display_name() {
    assert_eq!(
        code_detail_kind_label("block_id.rs"),
        Some("Rust".to_string())
    );
    assert_eq!(
        code_detail_kind_label("Dockerfile"),
        Some("Dockerfile".to_string())
    );
}

#[test]
fn code_detail_kind_label_returns_none_when_language_is_unknown() {
    assert_eq!(code_detail_kind_label("notes.txt"), None);
}

#[test]
fn detail_target_matches_panes_granularity() {
    let pane_group_id = EntityId::new();
    let hovered_pane_id = pane_id();

    assert_eq!(
        detail_target_for_hovered_row(
            pane_group_id,
            hovered_pane_id,
            VerticalTabsDisplayGranularity::Panes,
        ),
        VerticalTabsDetailTarget::Pane {
            pane_group_id,
            pane_id: hovered_pane_id,
        }
    );
}

#[test]
fn detail_target_matches_tabs_granularity() {
    let pane_group_id = EntityId::new();
    let hovered_pane_id = pane_id();

    assert_eq!(
        detail_target_for_hovered_row(
            pane_group_id,
            hovered_pane_id,
            VerticalTabsDisplayGranularity::Tabs,
        ),
        VerticalTabsDetailTarget::Tab {
            pane_group_id,
            source_pane_id: hovered_pane_id,
        }
    );
}

#[test]
fn pane_detail_target_returns_hovered_pane_when_supported() {
    let hovered_pane_id = pane_id();

    assert_eq!(
        visible_pane_ids_for_detail_target(
            &[hovered_pane_id],
            hovered_pane_id,
            VerticalTabsDetailTargetKind::Pane,
            |pane_id| pane_id == hovered_pane_id,
        ),
        Some(vec![hovered_pane_id])
    );
}

#[test]
fn pane_detail_target_returns_none_when_hovered_pane_is_not_supported() {
    let hovered_pane_id = pane_id();

    assert_eq!(
        visible_pane_ids_for_detail_target(
            &[hovered_pane_id],
            hovered_pane_id,
            VerticalTabsDetailTargetKind::Pane,
            |_| false,
        ),
        None
    );
}

#[test]
fn tab_detail_target_returns_all_visible_panes_when_every_pane_is_supported() {
    let pane_1 = pane_id();
    let pane_2 = pane_id();
    let pane_3 = pane_id();
    let visible_pane_ids = vec![pane_1, pane_2, pane_3];

    assert_eq!(
        visible_pane_ids_for_detail_target(
            &visible_pane_ids,
            pane_2,
            VerticalTabsDetailTargetKind::Tab,
            |_| true,
        ),
        Some(visible_pane_ids)
    );
}

#[test]
fn tab_detail_target_returns_none_for_mixed_support_tabs() {
    let pane_1 = pane_id();
    let pane_2 = pane_id();
    let pane_3 = pane_id();

    assert_eq!(
        visible_pane_ids_for_detail_target(
            &[pane_1, pane_2, pane_3],
            pane_2,
            VerticalTabsDetailTargetKind::Tab,
            |pane_id| pane_id != pane_3,
        ),
        None
    );
}

#[test]
fn panes_granularity_returns_all_visible_panes_in_order() {
    let pane_1 = pane_id();
    let pane_2 = pane_id();
    let pane_3 = pane_id();
    let visible_pane_ids = vec![pane_1, pane_2, pane_3];

    assert_eq!(
        pane_ids_for_display_granularity(
            &visible_pane_ids,
            pane_2,
            VerticalTabsDisplayGranularity::Panes,
        ),
        visible_pane_ids
    );
}

#[test]
fn tabs_granularity_returns_focused_pane_when_present() {
    let pane_1 = pane_id();
    let pane_2 = pane_id();
    let pane_3 = pane_id();

    assert_eq!(
        pane_ids_for_display_granularity(
            &[pane_1, pane_2, pane_3],
            pane_2,
            VerticalTabsDisplayGranularity::Tabs,
        ),
        vec![pane_2]
    );
}

#[test]
fn tabs_granularity_falls_back_to_first_visible_pane_when_focused_pane_is_absent() {
    let pane_1 = pane_id();
    let pane_2 = pane_id();
    let pane_3 = pane_id();
    let focused_pane = pane_id();

    assert_eq!(
        pane_ids_for_display_granularity(
            &[pane_1, pane_2, pane_3],
            focused_pane,
            VerticalTabsDisplayGranularity::Tabs,
        ),
        vec![pane_1]
    );
}

#[test]
fn tabs_granularity_returns_empty_for_empty_visible_panes() {
    assert_eq!(
        pane_ids_for_display_granularity(&[], pane_id(), VerticalTabsDisplayGranularity::Tabs,),
        Vec::<PaneId>::new()
    );
}

#[test]
fn detail_sidecar_uses_default_width_when_space_allows() {
    let (width, bounds) = detail_sidecar_width_and_bounds(400.);
    assert_eq!(width, 320.);
    assert!(matches!(
        bounds,
        PositionedElementOffsetBounds::WindowBySize
    ));
}

#[test]
fn detail_sidecar_shrinks_to_fit_before_hitting_min_width() {
    let (width, bounds) = detail_sidecar_width_and_bounds(280.);
    assert_eq!(width, 280.);
    assert!(matches!(
        bounds,
        PositionedElementOffsetBounds::WindowBySize
    ));
}

#[test]
fn detail_sidecar_stops_shrinking_at_min_width_and_allows_clipping() {
    let (width, bounds) = detail_sidecar_width_and_bounds(180.);
    assert_eq!(width, 240.);
    assert!(matches!(bounds, PositionedElementOffsetBounds::Unbounded));
}

#[test]
fn detail_sidecar_visibility_helper_keeps_sidecar_visible_inside_sidecar_bounds() {
    let row_rect = RectF::new(Vector2F::new(0., 100.), Vector2F::new(100., 40.));
    let sidecar_rect = RectF::new(Vector2F::new(120., 50.), Vector2F::new(180., 220.));
    let mut safe_triangle = SafeTriangle::new();

    assert!(should_keep_detail_sidecar_visible_for_mouse_position(
        Vector2F::new(200., 120.),
        Some(row_rect),
        Some(sidecar_rect),
        &mut safe_triangle,
    ));
}

#[test]
fn detail_sidecar_visibility_helper_keeps_sidecar_visible_in_safe_triangle() {
    let row_rect = RectF::new(Vector2F::new(0., 100.), Vector2F::new(100., 40.));
    let sidecar_rect = RectF::new(Vector2F::new(120., 50.), Vector2F::new(180., 220.));
    let mut safe_triangle = SafeTriangle::new();
    safe_triangle.set_target_rect(Some(sidecar_rect));
    safe_triangle.update_position(Vector2F::new(90., 120.));

    assert!(should_keep_detail_sidecar_visible_for_mouse_position(
        Vector2F::new(110., 120.),
        Some(row_rect),
        Some(sidecar_rect),
        &mut safe_triangle,
    ));
}

#[test]
fn detail_sidecar_visibility_helper_clears_sidecar_outside_row_sidecar_and_safe_triangle() {
    let row_rect = RectF::new(Vector2F::new(0., 100.), Vector2F::new(100., 40.));
    let sidecar_rect = RectF::new(Vector2F::new(120., 50.), Vector2F::new(180., 220.));
    let mut safe_triangle = SafeTriangle::new();
    safe_triangle.update_position(Vector2F::new(200., 120.));

    assert!(!should_keep_detail_sidecar_visible_for_mouse_position(
        Vector2F::new(340., 120.),
        Some(row_rect),
        Some(sidecar_rect),
        &mut safe_triangle,
    ));
}

#[test]
fn panes_granularity_uses_outer_group_container() {
    assert!(uses_outer_group_container(
        VerticalTabsDisplayGranularity::Panes
    ));
}

#[test]
fn tabs_granularity_does_not_use_outer_group_container() {
    assert!(!uses_outer_group_container(
        VerticalTabsDisplayGranularity::Tabs
    ));
}

// Regression coverage for #9098 ("Tab names not rendered in tab bar, only
// first tab shows name"). The header gate previously read `has_custom_title
// || is_being_renamed`, which collapsed to `false` for every tab without a
// user-set rename — leaving multi-pane tabs with auto-generated names
// looking like they had no tab label at all. The new gate keeps the existing
// triggers and adds "any multi-pane tab", so every multi-pane group has a
// stable tab-level identifier in `Panes` granularity.
#[test]
fn tab_group_header_shows_for_custom_title() {
    assert!(should_show_tab_group_header(true, false, 1));
    assert!(should_show_tab_group_header(true, false, 3));
}

#[test]
fn tab_group_header_shows_while_renaming() {
    // The inline rename editor must always be reachable, even on
    // single-pane tabs with no prior custom title.
    assert!(should_show_tab_group_header(false, true, 1));
}

#[test]
fn tab_group_header_shows_for_multi_pane_tabs_without_custom_title() {
    // The #9098 case: an auto-named multi-pane tab. Each row only shows the
    // per-pane title (e.g. `travelplan` + `main`), so without a group header
    // there is no way to tell two such tabs apart in the sidebar.
    assert!(should_show_tab_group_header(false, false, 2));
    assert!(should_show_tab_group_header(false, false, 5));
}

#[test]
fn tab_group_header_hidden_for_single_pane_without_custom_title() {
    // Single-pane groups already surface the pane title in their only row.
    // Rendering the same string again as a header would duplicate it
    // immediately above itself, so the gate stays closed in this shape.
    assert!(!should_show_tab_group_header(false, false, 1));
    // Defensive: `0` should not crash or accidentally render a header for
    // an empty group (this shape shouldn't reach the renderer in practice,
    // but the helper is total and stays closed).
    assert!(!should_show_tab_group_header(false, false, 0));
}

#[test]
fn tab_group_header_distinguishes_two_auto_named_multi_pane_tabs() {
    // Models the screenshot in #9098: tab 1 has a custom title
    // ("Humanfigure"), tabs 2 and 3 are auto-named multi-pane groups
    // ("travelplan + main", "deponti + release/development"). Before the
    // fix only tab 1 showed a header; after the fix every multi-pane tab
    // gets one so the user can tell them apart at a glance.
    let renders_header: Vec<bool> = vec![
        should_show_tab_group_header(true, false, 2),  // tab 1
        should_show_tab_group_header(false, false, 2), // tab 2
        should_show_tab_group_header(false, false, 2), // tab 3
    ];
    assert_eq!(renders_header, vec![true, true, true]);
}

#[test]
fn terminal_primary_line_prefers_cli_agent_display_title() {
    let line = terminal_primary_line_data(
        false,
        None,
        Some("Review the failing tests".to_string()),
        "~/warp",
        "~/warp",
        TerminalPrimaryLineFont::Monospace,
        Some("cargo nextest run".to_string()),
    );

    assert_eq!(line.text(), "Review the failing tests");
}

#[test]
fn terminal_primary_line_prefers_cli_agent_display_title_over_conversation_title() {
    let line = terminal_primary_line_data(
        false,
        Some("Review the failing tests".to_string()),
        Some("Summarize the failures".to_string()),
        "~/warp",
        "~/warp",
        TerminalPrimaryLineFont::Monospace,
        Some("cargo nextest run".to_string()),
    );

    assert_eq!(line.text(), "Summarize the failures");
}

#[test]
fn terminal_primary_line_falls_through_to_terminal_title_when_cli_agent_has_no_plugin_data() {
    let line = terminal_primary_line_data(
        false,
        None,
        None,
        "codex - ~/warp",
        "~/warp",
        TerminalPrimaryLineFont::Monospace,
        Some("cargo nextest run".to_string()),
    );

    assert_eq!(line.text(), "codex - ~/warp");
}

#[test]
fn terminal_primary_line_uses_terminal_title_as_fallback() {
    let line = terminal_primary_line_data(
        false,
        None,
        None,
        "nvim src/workspace/view/vertical_tabs.rs",
        "~/warp",
        TerminalPrimaryLineFont::Monospace,
        Some("cargo nextest run".to_string()),
    );

    assert_eq!(line.text(), "nvim src/workspace/view/vertical_tabs.rs");
}

#[test]
fn terminal_primary_line_uses_last_completed_command_when_shell_title_matches_working_directory() {
    let line = terminal_primary_line_data(
        false,
        None,
        None,
        "~/warp",
        "~/warp",
        TerminalPrimaryLineFont::Monospace,
        Some("cargo nextest run".to_string()),
    );

    assert_eq!(line.text(), "cargo nextest run");
}

#[test]
fn terminal_primary_line_falls_back_to_new_session() {
    let line = terminal_primary_line_data(
        false,
        None,
        None,
        "~/warp",
        "~/warp",
        TerminalPrimaryLineFont::Monospace,
        None,
    );

    assert_eq!(line.text(), "New session");
    assert!(matches!(
        line,
        TerminalPrimaryLineData::Text {
            font: TerminalPrimaryLineFont::Ui,
            ..
        }
    ));
}

#[test]
fn terminal_primary_line_uses_monospace_for_last_completed_command() {
    let line = terminal_primary_line_data(
        false,
        None,
        None,
        "~/warp",
        "~/warp",
        TerminalPrimaryLineFont::Monospace,
        Some("cargo nextest run".to_string()),
    );

    assert!(matches!(
        line,
        TerminalPrimaryLineData::Text {
            font: TerminalPrimaryLineFont::Monospace,
            ..
        }
    ));
}

#[test]
fn terminal_search_fragments_include_rendered_terminal_badges() {
    let fragments = terminal_search_text_fragments(
        "Review the failing tests".to_string(),
        "~/warp".to_string(),
        Some("main".to_string()),
        terminal_kind_badge_label(false, Some(CLIAgent::Claude)),
        Some(terminal_pull_request_badge_label(
            "https://github.com/warpdotdev/warp-internal/pull/12345",
        )),
        Some(GitLineChanges {
            files_changed: 1,
            lines_added: 2,
            lines_removed: 3,
        }),
    );

    assert!(search_fragments_contain_query(&fragments, "claude"));
    assert!(search_fragments_contain_query(
        &fragments,
        "review the failing tests"
    ));
    assert!(search_fragments_contain_query(&fragments, "#12345"));
    assert!(search_fragments_contain_query(&fragments, "+2"));
    assert!(search_fragments_contain_query(&fragments, "-3"));
}

#[test]
fn pane_search_fragments_prepend_custom_title_and_keep_generated_metadata() {
    let fragments = pane_search_text_fragments(
        Some("Production API"),
        vec![
            "cargo nextest run".to_string(),
            "~/warp".to_string(),
            "Claude".to_string(),
        ],
    );

    assert_eq!(fragments[0], "Production API");
    assert!(search_fragments_contain_query(&fragments, "production api"));
    assert!(search_fragments_contain_query(&fragments, "cargo nextest"));
    assert!(search_fragments_contain_query(&fragments, "~/warp"));
    assert!(search_fragments_contain_query(&fragments, "claude"));
}

#[test]
fn pane_search_fragments_dedupe_custom_title_against_generated_text() {
    assert_eq!(
        pane_search_text_fragments(
            Some("  Production   API  "),
            vec![
                "Production API".to_string(),
                "~/warp".to_string(),
                "~/warp".to_string(),
            ],
        ),
        vec!["Production API".to_string(), "~/warp".to_string()]
    );
}

#[test]
fn non_terminal_search_fragments_only_include_rendered_text() {
    let fragments = non_terminal_search_text_fragments("Pane title", "and 2 more");

    assert!(search_fragments_contain_query(&fragments, "pane title"));
    assert!(search_fragments_contain_query(&fragments, "and 2 more"));
    assert!(!search_fragments_contain_query(&fragments, "notebook"));
    assert!(!search_fragments_contain_query(&fragments, "unsaved"));
}

#[test]
fn diff_stats_text_matches_rendered_badge_text() {
    assert_eq!(
        vtab_diff_stats_text(&GitLineChanges {
            files_changed: 1,
            lines_added: 2,
            lines_removed: 3,
        }),
        "+2 -3"
    );
    assert_eq!(
        vtab_diff_stats_text(&GitLineChanges {
            files_changed: 1,
            lines_added: 0,
            lines_removed: 0,
        }),
        "0"
    );
}

#[test]
fn branch_label_display_falls_back_without_branch_icon() {
    assert_eq!(
        branch_label_display(None, "~/warp"),
        ("~/warp".to_string(), false)
    );
    assert_eq!(
        branch_label_display(Some(""), "~/warp"),
        ("~/warp".to_string(), false)
    );
    assert_eq!(
        branch_label_display(Some("main"), "~/warp"),
        ("main".to_string(), true)
    );
}

#[test]
fn compact_branch_subtitle_falls_back_to_working_directory_without_branch_icon() {
    assert_eq!(
        compact_branch_subtitle_display(None, Some("~/warp")),
        Some(("~/warp".to_string(), false))
    );
    assert_eq!(
        compact_branch_subtitle_display(Some(""), Some("~/warp")),
        Some(("~/warp".to_string(), false))
    );
    assert_eq!(
        compact_branch_subtitle_display(Some("main"), Some("~/warp")),
        Some(("main".to_string(), true))
    );
}

#[test]
fn collect_normalized_unique_summary_texts_dedupes_after_whitespace_normalization() {
    assert_eq!(
        collect_normalized_unique_summary_texts([
            "  cargo   test  ",
            "cargo test",
            "",
            " git   status ",
        ]),
        vec!["cargo test".to_string(), "git status".to_string()]
    );
}

#[test]
fn collect_normalized_unique_summary_texts_preserves_first_seen_order() {
    assert_eq!(
        collect_normalized_unique_summary_texts([
            "~/warp-internal",
            "~/warp-server",
            "~/warp-internal",
            "~/warp-terraform",
        ]),
        vec![
            "~/warp-internal".to_string(),
            "~/warp-server".to_string(),
            "~/warp-terraform".to_string(),
        ]
    );
}

#[test]
fn coalesce_summary_branch_entries_groups_by_repo_and_branch() {
    let repo_a = PathBuf::from("/tmp/repo-a");
    let repo_b = PathBuf::from("/tmp/repo-b");
    let entries = vec![
        VerticalTabsSummaryBranchEntry {
            repo_path: repo_a.clone(),
            branch_name: "main".to_string(),
            diff_stats: None,
            pull_request_label: None,
            pull_request_url: None,
        },
        VerticalTabsSummaryBranchEntry {
            repo_path: repo_a.clone(),
            branch_name: "main".to_string(),
            diff_stats: Some(GitLineChanges {
                files_changed: 1,
                lines_added: 2,
                lines_removed: 3,
            }),
            pull_request_label: Some("#123".to_string()),
            pull_request_url: Some("https://github.com/acme/repo-a/pull/123".to_string()),
        },
        VerticalTabsSummaryBranchEntry {
            repo_path: repo_b.clone(),
            branch_name: "main".to_string(),
            diff_stats: Some(GitLineChanges {
                files_changed: 4,
                lines_added: 5,
                lines_removed: 6,
            }),
            pull_request_label: Some("#456".to_string()),
            pull_request_url: Some("https://github.com/acme/repo-b/pull/456".to_string()),
        },
    ];

    assert_eq!(
        coalesce_summary_branch_entries(entries),
        vec![
            VerticalTabsSummaryBranchEntry {
                repo_path: repo_a,
                branch_name: "main".to_string(),
                diff_stats: Some(GitLineChanges {
                    files_changed: 1,
                    lines_added: 2,
                    lines_removed: 3,
                }),
                pull_request_label: Some("#123".to_string()),
                pull_request_url: Some("https://github.com/acme/repo-a/pull/123".to_string()),
            },
            VerticalTabsSummaryBranchEntry {
                repo_path: repo_b,
                branch_name: "main".to_string(),
                diff_stats: Some(GitLineChanges {
                    files_changed: 4,
                    lines_added: 5,
                    lines_removed: 6,
                }),
                pull_request_label: Some("#456".to_string()),
                pull_request_url: Some("https://github.com/acme/repo-b/pull/456".to_string()),
            },
        ]
    );
}

#[test]
fn summary_overflow_count_caps_visible_region() {
    assert_eq!(summary_overflow_count(5, 3), 2);
    assert_eq!(summary_overflow_count(3, 3), 0);
    assert_eq!(summary_overflow_count(2, 3), 0);
}

#[test]
fn primary_labels_dedupe_preserves_first_seen_status() {
    let mut values = Vec::new();
    let mut seen = std::collections::HashMap::new();
    push_normalized_unique_summary_label(&mut values, &mut seen, "  cargo   test  ", None);
    push_normalized_unique_summary_label(
        &mut values,
        &mut seen,
        "cargo test",
        Some(ConversationStatus::InProgress),
    );

    assert_eq!(
        values,
        vec![VerticalTabsSummaryPrimaryLabel {
            text: "cargo test".to_string(),
            status: None,
        }]
    );
}

#[test]
fn primary_labels_preserve_status_through_aggregation() {
    let mut values = Vec::new();
    let mut seen = std::collections::HashMap::new();
    push_normalized_unique_summary_label(
        &mut values,
        &mut seen,
        "Plan a refactor",
        Some(ConversationStatus::InProgress),
    );
    push_normalized_unique_summary_label(
        &mut values,
        &mut seen,
        "Investigate failure",
        Some(ConversationStatus::Success),
    );
    push_normalized_unique_summary_label(&mut values, &mut seen, "cargo build", None);

    assert_eq!(
        values,
        vec![
            VerticalTabsSummaryPrimaryLabel {
                text: "Plan a refactor".to_string(),
                status: Some(ConversationStatus::InProgress),
            },
            VerticalTabsSummaryPrimaryLabel {
                text: "Investigate failure".to_string(),
                status: Some(ConversationStatus::Success),
            },
            VerticalTabsSummaryPrimaryLabel {
                text: "cargo build".to_string(),
                status: None,
            },
        ]
    );
}

#[test]
fn sort_summary_primary_labels_moves_status_first_and_preserves_order() {
    let mut values = vec![
        VerticalTabsSummaryPrimaryLabel {
            text: "plain terminal".to_string(),
            status: None,
        },
        VerticalTabsSummaryPrimaryLabel {
            text: "first conversation".to_string(),
            status: Some(ConversationStatus::InProgress),
        },
        VerticalTabsSummaryPrimaryLabel {
            text: "code pane".to_string(),
            status: None,
        },
        VerticalTabsSummaryPrimaryLabel {
            text: "second conversation".to_string(),
            status: Some(ConversationStatus::Success),
        },
        VerticalTabsSummaryPrimaryLabel {
            text: "last terminal".to_string(),
            status: None,
        },
    ];

    sort_summary_primary_labels_status_first(&mut values);

    assert_eq!(
        values,
        vec![
            VerticalTabsSummaryPrimaryLabel {
                text: "first conversation".to_string(),
                status: Some(ConversationStatus::InProgress),
            },
            VerticalTabsSummaryPrimaryLabel {
                text: "second conversation".to_string(),
                status: Some(ConversationStatus::Success),
            },
            VerticalTabsSummaryPrimaryLabel {
                text: "plain terminal".to_string(),
                status: None,
            },
            VerticalTabsSummaryPrimaryLabel {
                text: "code pane".to_string(),
                status: None,
            },
            VerticalTabsSummaryPrimaryLabel {
                text: "last terminal".to_string(),
                status: None,
            },
        ]
    );
}

#[test]
fn synced_inputs_indicator_shows_on_synced_terminal_rows() {
    assert!(shows_synced_inputs_indicator(true, true, true));
}

#[test]
fn synced_inputs_indicator_hidden_when_tab_is_not_synced() {
    assert!(!shows_synced_inputs_indicator(true, false, true));
}

#[test]
fn synced_inputs_indicator_respects_tab_indicators_setting() {
    assert!(!shows_synced_inputs_indicator(true, true, false));
}

#[test]
fn synced_inputs_indicator_hidden_on_non_terminal_rows() {
    assert!(!shows_synced_inputs_indicator(false, true, true));
}

#[test]
fn reveals_shortcut_hints_requires_overlap_with_binding_modifiers() {
    let super_kind = once(ShortcutModifierKind::Super).collect();
    assert!(reveals_shortcut_hints(&super_kind, &super_kind));

    let alt_kind = once(ShortcutModifierKind::Alt).collect();
    assert!(!reveals_shortcut_hints(&alt_kind, &super_kind));

    let empty = std::collections::HashSet::new();
    assert!(!reveals_shortcut_hints(&empty, &super_kind));
}

#[test]
fn summary_search_fragments_include_hidden_overflow_values() {
    let summary = VerticalTabsSummaryData {
        primary_labels: vec![
            VerticalTabsSummaryPrimaryLabel {
                text: "Claude".to_string(),
                status: Some(ConversationStatus::InProgress),
            },
            label("Warp Agent"),
            label("cargo"),
            label("code review"),
            label("hidden work"),
        ],
        working_directories: vec!["~/warp-internal".to_string(), "~/warp-server".to_string()],
        branch_entries: vec![
            VerticalTabsSummaryBranchEntry {
                repo_path: PathBuf::from("/tmp/repo-a"),
                branch_name: "main".to_string(),
                diff_stats: Some(GitLineChanges {
                    files_changed: 1,
                    lines_added: 2,
                    lines_removed: 3,
                }),
                pull_request_label: Some("#123".to_string()),
                pull_request_url: Some("https://github.com/acme/repo-a/pull/123".to_string()),
            },
            VerticalTabsSummaryBranchEntry {
                repo_path: PathBuf::from("/tmp/repo-b"),
                branch_name: "feature/hidden".to_string(),
                diff_stats: None,
                pull_request_label: None,
                pull_request_url: None,
            },
            VerticalTabsSummaryBranchEntry {
                repo_path: PathBuf::from("/tmp/repo-c"),
                branch_name: "cleanup".to_string(),
                diff_stats: None,
                pull_request_label: None,
                pull_request_url: None,
            },
            VerticalTabsSummaryBranchEntry {
                repo_path: PathBuf::from("/tmp/repo-d"),
                branch_name: "hidden-branch".to_string(),
                diff_stats: None,
                pull_request_label: Some("#789".to_string()),
                pull_request_url: Some("https://github.com/acme/repo-d/pull/789".to_string()),
            },
        ],
        has_unread_activity: false,
    };

    let fragments = summary_search_text_fragments(&summary, Some("Custom tab"));

    assert!(search_fragments_contain_query(&fragments, "custom tab"));
    assert!(search_fragments_contain_query(&fragments, "claude"));
    assert!(search_fragments_contain_query(&fragments, "hidden work"));
    assert!(search_fragments_contain_query(&fragments, "hidden-branch"));
    assert!(search_fragments_contain_query(&fragments, "#789"));
    assert!(search_fragments_contain_query(&fragments, "+2"));
    assert!(search_fragments_contain_query(&fragments, "-3"));
}

#[test]
fn group_subtitle_counts_only_nested_worktrees() {
    assert_eq!(group_subtitle_text(1), None);
    assert_eq!(group_subtitle_text(2), Some("1 worktree".to_string()));
    assert_eq!(group_subtitle_text(4), Some("3 worktrees".to_string()));
}

fn tab_group(name: Option<&str>) -> TabGroup {
    TabGroup {
        name: name.map(str::to_string),
        ..TabGroup::new()
    }
}

#[test]
fn group_display_name_uses_the_group_name_when_set() {
    assert_eq!(group_display_name(&tab_group(Some("backend"))), "backend");
}

#[test]
fn group_display_name_falls_back_to_the_untitled_header_text() {
    assert_eq!(group_display_name(&tab_group(None)), "New Group");
}

#[test]
fn group_name_match_includes_members_that_did_not_match_on_their_own() {
    let group = TabGroupId::new();
    let tab_group_ids = vec![Some(group), Some(group), None];

    let merged = merge_group_name_matches(
        &tab_group_ids,
        &HashSet::from([group]),
        // Only the second tab matched on its own title.
        vec![(1, None)],
    );

    assert_eq!(merged, vec![(0, None), (1, None)]);
}

#[test]
fn group_name_match_keeps_results_ordered_by_tab_index() {
    let group = TabGroupId::new();
    // Ungrouped tabs on either side of the group.
    let tab_group_ids = vec![None, Some(group), Some(group), None];

    let merged = merge_group_name_matches(
        &tab_group_ids,
        &HashSet::from([group]),
        vec![(0, None), (3, None)],
    );

    assert_eq!(
        merged,
        vec![(0, None), (1, None), (2, None), (3, None)],
        "group members must be interleaved in tab order, not appended"
    );
}

#[test]
fn group_name_match_upgrades_a_pane_filtered_member_to_all_panes() {
    let group = TabGroupId::new();
    let tab_group_ids = vec![Some(group)];

    let merged = merge_group_name_matches(
        &tab_group_ids,
        &HashSet::from([group]),
        // The tab matched on one pane row only.
        vec![(0, Some(vec![pane_id()]))],
    );

    assert_eq!(
        merged,
        vec![(0, None)],
        "a group-name match shows the whole tab, not a pane-filtered slice"
    );
}

#[test]
fn group_name_match_for_a_group_with_no_members_changes_nothing() {
    let empty_group = TabGroupId::new();
    let tab_group_ids = vec![None, None];

    let merged = merge_group_name_matches(
        &tab_group_ids,
        &HashSet::from([empty_group]),
        vec![(1, None)],
    );

    assert_eq!(merged, vec![(1, None)]);
}

#[test]
fn group_name_match_leaves_tabs_outside_the_group_filtered() {
    let matched = TabGroupId::new();
    let other = TabGroupId::new();
    let tab_group_ids = vec![Some(matched), Some(other), None];

    let merged = merge_group_name_matches(&tab_group_ids, &HashSet::from([matched]), vec![]);

    assert_eq!(
        merged,
        vec![(0, None)],
        "only members of the name-matched group are force-included"
    );
}

#[test]
fn no_group_name_match_leaves_the_filtered_tabs_untouched() {
    let group = TabGroupId::new();
    let tab_group_ids = vec![Some(group), None];
    let own_matches = vec![(1, Some(vec![pane_id()]))];

    let merged = merge_group_name_matches(&tab_group_ids, &HashSet::new(), own_matches.clone());

    assert_eq!(merged, own_matches);
}

fn tab_groups_map(groups: Vec<TabGroup>) -> HashMap<TabGroupId, TabGroup> {
    groups.into_iter().map(|group| (group.id, group)).collect()
}

#[test]
fn matched_group_ids_selects_groups_whose_displayed_name_contains_the_query() {
    let backend = tab_group(Some("backend"));
    let frontend = tab_group(Some("frontend"));
    let backend_id = backend.id;
    let groups = tab_groups_map(vec![backend, frontend]);

    assert_eq!(
        matched_group_ids(&groups, "backend"),
        HashSet::from([backend_id])
    );
}

#[test]
fn matched_group_ids_is_case_insensitive() {
    let group = tab_group(Some("Backend Services"));
    let id = group.id;
    let groups = tab_groups_map(vec![group]);

    assert_eq!(matched_group_ids(&groups, "backend"), HashSet::from([id]));
}

#[test]
fn matched_group_ids_matches_the_untitled_group_placeholder() {
    let group = tab_group(None);
    let id = group.id;
    let groups = tab_groups_map(vec![group]);

    assert_eq!(matched_group_ids(&groups, "new group"), HashSet::from([id]));
}

#[test]
fn matched_group_ids_is_empty_when_nothing_matches() {
    let groups = tab_groups_map(vec![tab_group(Some("backend"))]);

    assert!(matched_group_ids(&groups, "nomatch").is_empty());
}

#[test]
fn tab_is_admitted_by_group_name_only_when_its_group_matched() {
    let matched = TabGroupId::new();
    let other = TabGroupId::new();
    let matched_groups = HashSet::from([matched]);

    assert!(tab_admitted_by_group_name(Some(matched), &matched_groups));
    assert!(!tab_admitted_by_group_name(Some(other), &matched_groups));
    assert!(
        !tab_admitted_by_group_name(None, &matched_groups),
        "an ungrouped tab is never admitted by a group-name match"
    );
}

#[test]
fn group_name_highlights_case_insensitive_substrings() {
    assert_eq!(
        group_name_highlight_indices("My Backend", "BACK"),
        vec![3, 4, 5, 6]
    );
}

#[test]
fn group_name_highlights_repeated_matches() {
    assert_eq!(
        group_name_highlight_indices("dev / DEV", "dev"),
        vec![0, 1, 2, 6, 7, 8]
    );
}

#[test]
fn group_name_highlights_overlapping_matches() {
    assert_eq!(
        group_name_highlight_indices("banana", "ana"),
        vec![1, 2, 3, 4, 5]
    );
}

#[test]
fn group_name_highlights_use_character_indices_after_multibyte_text() {
    assert_eq!(group_name_highlight_indices("🚀 Équipe", "éq"), vec![2, 3]);
}

#[test]
fn group_name_highlights_map_expanding_lowercase_to_original_characters() {
    assert_eq!(
        group_name_highlight_indices("İ Backend", "back"),
        vec![2, 3, 4, 5]
    );
    assert_eq!(group_name_highlight_indices("İ Backend", "i"), vec![0]);
}

#[test]
fn group_name_highlights_preserve_contextual_lowercasing() {
    assert_eq!(group_name_highlight_indices("ΟΣ", "ος"), vec![0, 1]);
}

#[test]
fn group_name_highlights_clear_for_empty_query() {
    assert_eq!(
        group_name_highlight_indices("Backend", ""),
        Vec::<usize>::new()
    );
}

#[test]
fn group_name_highlights_do_not_use_fuzzy_matching() {
    assert_eq!(
        group_name_highlight_indices("Backend", "bkd"),
        Vec::<usize>::new()
    );
}

#[test]
fn group_name_highlights_match_untitled_fallback() {
    assert_eq!(
        group_name_highlight_indices(&group_display_name(&tab_group(None)), "group"),
        vec![4, 5, 6, 7, 8]
    );
}

fn herdr_git(pull_request_url: Option<&str>) -> DirectoryGitContext {
    DirectoryGitContext {
        repo_path: PathBuf::from("/home/me/api"),
        branch: "feature/sidebar".to_string(),
        diff_stats: Some(GitLineChanges {
            files_changed: 2,
            lines_added: 10,
            lines_removed: 3,
        }),
        pull_request_url: pull_request_url.map(str::to_string),
    }
}

#[test]
fn herdr_row_outside_a_repository_shows_only_the_directory() {
    let details =
        herdr_row_details_from(Some(Path::new("/srv/logs")), None, None, Some("/home/me"));

    assert_eq!(
        details,
        HerdrRowDetails {
            working_directory: Some("/srv/logs".to_string()),
            branch_entry: None,
            command: None,
        }
    );
}

#[test]
fn herdr_row_in_a_repository_has_the_branch_entry_a_terminal_would_build() {
    let url = "https://github.com/acme/api/pull/42";
    let details = herdr_row_details_from(
        Some(Path::new("/home/me/api/src")),
        Some(herdr_git(Some(url))),
        None,
        Some("/home/me"),
    );

    assert_eq!(details.working_directory.as_deref(), Some("~/api/src"));
    assert_eq!(
        details.branch_entry,
        Some(VerticalTabsSummaryBranchEntry {
            repo_path: PathBuf::from("/home/me/api"),
            branch_name: "feature/sidebar".to_string(),
            diff_stats: herdr_git(None).diff_stats,
            pull_request_label: Some(terminal_pull_request_badge_label(url)),
            pull_request_url: Some(url.to_string()),
        })
    );
}

#[test]
fn herdr_row_without_a_pull_request_has_no_chip() {
    let details = herdr_row_details_from(
        Some(Path::new("/home/me/api")),
        Some(herdr_git(None)),
        None,
        Some("/home/me"),
    );

    let entry = details.branch_entry.expect("branch entry");
    assert_eq!(entry.pull_request_label, None);
    assert_eq!(entry.pull_request_url, None);
}

#[test]
fn herdr_row_without_a_directory_shows_nothing() {
    let details = herdr_row_details_from(None, None, None, Some("/home/me"));

    assert!(!details.is_present());
}

#[test]
fn herdr_compact_subtitle_follows_the_setting_like_a_terminal_row() {
    let in_repo = herdr_row_details_from(
        Some(Path::new("/home/me/api")),
        Some(herdr_git(None)),
        Some("claude".to_string()),
        Some("/home/me"),
    );
    let outside =
        herdr_row_details_from(Some(Path::new("/srv/logs")), None, None, Some("/home/me"));

    assert_eq!(
        herdr_compact_subtitle(&in_repo, VerticalTabsCompactSubtitle::Branch),
        Some(("feature/sidebar".to_string(), true))
    );
    assert_eq!(
        herdr_compact_subtitle(&outside, VerticalTabsCompactSubtitle::Branch),
        Some(("/srv/logs".to_string(), false))
    );
    assert_eq!(
        herdr_compact_subtitle(&in_repo, VerticalTabsCompactSubtitle::WorkingDirectory),
        Some(("~/api".to_string(), false))
    );
    assert_eq!(
        herdr_compact_subtitle(&in_repo, VerticalTabsCompactSubtitle::Command),
        Some(("claude".to_string(), false))
    );
    assert_eq!(
        herdr_compact_subtitle(&outside, VerticalTabsCompactSubtitle::Command),
        None
    );
}

fn titled_row() -> HerdrRowDetails {
    herdr_row_details_from(
        Some(Path::new("/home/me/api/src")),
        Some(herdr_git(None)),
        Some("claude".to_string()),
        Some("/home/me"),
    )
}

#[test]
fn herdr_row_title_follows_pane_title_as_like_a_terminal_row() {
    let row = titled_row();

    assert_eq!(
        herdr_row_title(&row, VerticalTabsPrimaryInfo::Command, "api"),
        HerdrRowTitle {
            text: "claude".to_string(),
            source: Some(VerticalTabsPrimaryInfo::Command),
        }
    );
    assert_eq!(
        herdr_row_title(&row, VerticalTabsPrimaryInfo::WorkingDirectory, "api"),
        HerdrRowTitle {
            text: "~/api/src".to_string(),
            source: Some(VerticalTabsPrimaryInfo::WorkingDirectory),
        }
    );
    assert_eq!(
        herdr_row_title(&row, VerticalTabsPrimaryInfo::Branch, "api"),
        HerdrRowTitle {
            text: "feature/sidebar".to_string(),
            source: Some(VerticalTabsPrimaryInfo::Branch),
        }
    );
}

#[test]
fn herdr_row_title_falls_back_to_the_workspace_label_when_the_chosen_value_is_unknown() {
    let outside_a_repository = herdr_row_details_from(
        Some(Path::new("/srv/logs")),
        None,
        Some("claude".to_string()),
        Some("/home/me"),
    );
    let nothing_known = HerdrRowDetails::default();
    let label_only = HerdrRowTitle {
        text: "api".to_string(),
        source: None,
    };

    assert_eq!(
        herdr_row_title(
            &outside_a_repository,
            VerticalTabsPrimaryInfo::Branch,
            "api"
        ),
        label_only
    );
    for primary in [
        VerticalTabsPrimaryInfo::Command,
        VerticalTabsPrimaryInfo::WorkingDirectory,
        VerticalTabsPrimaryInfo::Branch,
    ] {
        assert_eq!(herdr_row_title(&nothing_known, primary, "api"), label_only);
    }
    assert_eq!(
        herdr_row_title(
            &herdr_row_details_from(Some(Path::new("/srv/logs")), None, None, Some("/home/me")),
            VerticalTabsPrimaryInfo::Command,
            "api"
        ),
        label_only
    );
}

fn herdr_agent(
    name: Option<&str>,
    kind: Option<&str>,
    state: herdr_client::AgentState,
) -> herdr_client::PresentedAgent {
    herdr_client::PresentedAgent {
        pane: herdr_client::PaneId::from("w1:p1"),
        workspace: herdr_client::WorkspaceId::from("w1"),
        herdr_tab: herdr_client::HerdrTabId::from("w1:t1"),
        kind: kind.map(str::to_string),
        name: name.map(str::to_string),
        state,
    }
}

#[test]
fn herdr_agent_line_names_the_agent_then_its_kind_then_its_state() {
    use herdr_client::AgentState;

    assert_eq!(
        herdr_agent_line(&herdr_agent(
            Some("builder"),
            Some("claude"),
            AgentState::Working
        )),
        "builder · claude · working"
    );
    assert_eq!(
        herdr_agent_line(&herdr_agent(None, Some("codex"), AgentState::Idle)),
        "codex · idle"
    );
    assert_eq!(
        herdr_agent_line(&herdr_agent(None, None, AgentState::Blocked)),
        "blocked"
    );
}

#[test]
fn herdr_agent_line_does_not_repeat_a_name_that_is_its_kind() {
    assert_eq!(
        herdr_agent_line(&herdr_agent(
            Some("claude"),
            Some("claude"),
            herdr_client::AgentState::Done
        )),
        "claude · done"
    );
}

#[test]
fn herdr_pane_compact_subtitle_is_the_agent_when_there_is_one_else_follows_the_setting() {
    let in_repo = herdr_row_details_from(
        Some(Path::new("/home/me/api")),
        Some(herdr_git(None)),
        None,
        Some("/home/me"),
    );

    assert_eq!(
        herdr_pane_compact_subtitle(
            &in_repo,
            Some("claude · working"),
            VerticalTabsCompactSubtitle::Branch
        ),
        Some(("claude · working".to_string(), false))
    );
    assert_eq!(
        herdr_pane_compact_subtitle(&in_repo, None, VerticalTabsCompactSubtitle::Branch),
        Some(("feature/sidebar".to_string(), true))
    );
    assert_eq!(
        herdr_pane_compact_subtitle(
            &HerdrRowDetails::default(),
            None,
            VerticalTabsCompactSubtitle::Branch
        ),
        None
    );
}

#[test]
fn herdr_details_below_the_title_leave_out_what_the_title_already_shows() {
    let row = titled_row();
    let title = |primary| herdr_row_title(&row, primary, "api");

    let below = |primary| herdr_details_below_title(&row, Some(&title(primary)));

    assert_eq!(below(VerticalTabsPrimaryInfo::Command), row);
    assert_eq!(
        below(VerticalTabsPrimaryInfo::WorkingDirectory).working_directory,
        None
    );
    assert_eq!(
        below(VerticalTabsPrimaryInfo::WorkingDirectory).branch_entry,
        row.branch_entry
    );
    assert_eq!(below(VerticalTabsPrimaryInfo::Branch).branch_entry, None);
    assert_eq!(
        below(VerticalTabsPrimaryInfo::Branch).working_directory,
        row.working_directory
    );
    assert_eq!(
        herdr_details_below_title(
            &row,
            Some(&herdr_row_title(
                &HerdrRowDetails::default(),
                VerticalTabsPrimaryInfo::Branch,
                "api"
            ))
        ),
        row,
        "a title that fell back to the label shows nothing of the row"
    );
}

fn popup_text(
    app: &mut App,
    granularity: VerticalTabsDisplayGranularity,
    item_mode: VerticalTabsTabItemMode,
    view_mode: VerticalTabsViewMode,
) -> String {
    app.update(|ctx| {
        TabSettings::handle(ctx).update(ctx, |settings, ctx| {
            let _ = settings
                .vertical_tabs_display_granularity
                .set_value(granularity, ctx);
            let _ = settings
                .vertical_tabs_tab_item_mode
                .set_value(item_mode, ctx);
            let _ = settings.vertical_tabs_view_mode.set_value(view_mode, ctx);
        });
    });
    let state = VerticalTabsPanelState::default();
    app.read(|ctx| {
        render_settings_popup_menu(&state, ctx)
            .debug_text_content()
            .unwrap_or_default()
    })
}

#[test]
fn the_settings_popup_offers_pane_title_as_only_when_it_lists_panes() {
    App::test((), |mut app| async move {
        let _summary = FeatureFlag::VerticalTabsSummaryMode.override_enabled(true);
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| Appearance::mock());

        let panes = popup_text(
            &mut app,
            VerticalTabsDisplayGranularity::Panes,
            VerticalTabsTabItemMode::FocusedSession,
            VerticalTabsViewMode::Compact,
        );
        for expected in [
            "Pane title as",
            "Command / Conversation",
            "Working Directory",
            "Branch",
        ] {
            assert!(panes.contains(expected), "Panes: {panes:?}");
        }

        for item_mode in [
            VerticalTabsTabItemMode::FocusedSession,
            VerticalTabsTabItemMode::Summary,
        ] {
            let workspaces = popup_text(
                &mut app,
                VerticalTabsDisplayGranularity::Tabs,
                item_mode,
                VerticalTabsViewMode::Compact,
            );
            assert!(
                !workspaces.contains("Pane title as"),
                "Workspaces, {item_mode:?}: {workspaces:?}"
            );
        }
    });
}

#[test]
fn the_settings_popup_offers_density_and_the_show_toggles_for_workspaces_in_both_item_modes() {
    App::test((), |mut app| async move {
        let _summary = FeatureFlag::VerticalTabsSummaryMode.override_enabled(true);
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| Appearance::mock());

        for (item_mode, view_mode) in [
            (
                VerticalTabsTabItemMode::FocusedSession,
                VerticalTabsViewMode::Expanded,
            ),
            (
                VerticalTabsTabItemMode::Summary,
                VerticalTabsViewMode::Expanded,
            ),
            (
                VerticalTabsTabItemMode::Summary,
                VerticalTabsViewMode::Compact,
            ),
        ] {
            let text = popup_text(
                &mut app,
                VerticalTabsDisplayGranularity::Tabs,
                item_mode,
                view_mode,
            );
            for expected in ["Density", "PR link", "Diff stats"] {
                assert!(
                    text.contains(expected),
                    "{item_mode:?}, {view_mode:?}: {text:?}"
                );
            }
        }
    });
}

#[test]
fn pane_title_as_gives_only_the_rows_of_the_panes_view_a_herdr_title() {
    let row = titled_row();

    assert_eq!(
        herdr_shown_title(
            &row,
            VerticalTabsPrimaryInfo::Command,
            "api",
            VerticalTabsDisplayGranularity::Tabs
        ),
        None
    );
    assert_eq!(
        herdr_shown_title(
            &row,
            VerticalTabsPrimaryInfo::Command,
            "api",
            VerticalTabsDisplayGranularity::Panes
        )
        .map(|title| title.text),
        Some("claude".to_string())
    );
}

#[test]
fn a_branch_line_shows_the_pull_request_and_diff_stats_the_toggles_leave_on() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let url = "https://github.com/acme/api/pull/42";
        let entry = herdr_row_details_from(
            Some(Path::new("/home/me/api")),
            Some(herdr_git(Some(url))),
            None,
            Some("/home/me"),
        )
        .branch_entry
        .expect("branch entry");
        let text = |app: &App, badges| {
            app.read(|ctx| {
                render_summary_branch_line(
                    &entry,
                    None,
                    VerticalTabsChipEntrypoint::Pane,
                    badges,
                    Appearance::as_ref(ctx),
                )
                .debug_text_content()
                .unwrap_or_default()
            })
        };
        let pr_label = terminal_pull_request_badge_label(url);
        let diff_stats = "+10";

        let both = text(
            &app,
            BranchLineBadges {
                diff_stats: true,
                pr_link: true,
            },
        );
        assert!(
            both.contains(&pr_label) && both.contains(diff_stats.trim()),
            "{both:?}"
        );

        let only_diff_stats = text(
            &app,
            BranchLineBadges {
                diff_stats: true,
                pr_link: false,
            },
        );
        assert!(!only_diff_stats.contains(&pr_label), "{only_diff_stats:?}");
        assert!(
            only_diff_stats.contains(diff_stats.trim()),
            "{only_diff_stats:?}"
        );

        let only_pr_link = text(
            &app,
            BranchLineBadges {
                diff_stats: false,
                pr_link: true,
            },
        );
        assert!(only_pr_link.contains(&pr_label), "{only_pr_link:?}");
        assert!(
            !only_pr_link.contains(diff_stats.trim()),
            "{only_pr_link:?}"
        );
    });
}

fn herdr_titles_session(fake: &FakeHerdr) -> FakeSession {
    fake.start_session(
        "default",
        SessionScript::new().workspace_with_tabs(
            "api",
            [
                TabScript::new(
                    "logs",
                    PaneTree::split(
                        SplitDirection::Right,
                        0.5,
                        PaneTree::pane(),
                        PaneTree::pane(),
                    ),
                )
                .cwd(0, "/srv/api")
                .agent(
                    0,
                    herdr_fake::AgentScript::new("claude", herdr_fake::AgentState::Working),
                )
                .cwd(1, "/srv/logs")
                .terminal_title(1, "tail -f app.log"),
                TabScript::new("blank", PaneTree::pane()).without_cwd(0),
            ],
        ),
    )
}

/// The titles `primary` gives the rows of the workspace `w1`'s shown Herdr tab, as
/// `(pane, title)`.
fn herdr_row_titles(
    app: &App,
    view: &ViewHandle<HerdrWorkspaceView>,
    primary: VerticalTabsPrimaryInfo,
) -> Vec<(String, String)> {
    view.read(app, |view, ctx| {
        view.pane_rows(ctx)
            .into_iter()
            .map(|row| {
                let details = herdr_row_details_from(
                    row.directory.as_deref(),
                    row.git,
                    row.command,
                    Some("/home/me"),
                );
                (
                    row.pane.as_str().to_owned(),
                    herdr_row_title(&details, primary, "api").text,
                )
            })
            .collect()
    })
}

#[test]
fn herdr_rows_are_titled_from_what_herdr_reports_for_each_pane() {
    App::test((), |mut app| async move {
        let mut fake = FakeHerdr::new();
        fake.script_status(&herdr_fake::HerdrStatus::compatible());
        let _session = herdr_titles_session(&fake);
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| Appearance::mock());
        let target = SessionTarget::new(&fake.config_dir(), SessionName::default_session());
        app.update(|ctx| {
            ctx.add_singleton_model(|ctx| {
                HerdrSessionModel::connect(HerdrCli::new(fake.binary_path()), target, ctx)
            });
        });
        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            HerdrWorkspaceView::new(
                HerdrWorkspaceContents::Workspace(HerdrWorkspaceSnapshot {
                    workspace_id: WorkspaceId::from("w1"),
                    label: "api".to_owned(),
                }),
                ctx,
            )
        });
        assert_eventually!(
            600 => herdr_row_titles(&app, &view, VerticalTabsPrimaryInfo::Command)
                .first()
                .is_some_and(|(_, title)| title == "claude"),
            "the agents never arrived"
        );

        assert_eq!(
            herdr_row_titles(&app, &view, VerticalTabsPrimaryInfo::Command),
            [
                ("w1:p1".to_owned(), "claude".to_owned()),
                ("w1:p2".to_owned(), "tail -f app.log".to_owned()),
            ]
        );
        assert_eq!(
            herdr_row_titles(&app, &view, VerticalTabsPrimaryInfo::WorkingDirectory),
            [
                ("w1:p1".to_owned(), "/srv/api".to_owned()),
                ("w1:p2".to_owned(), "/srv/logs".to_owned()),
            ]
        );
        assert_eq!(
            herdr_row_titles(&app, &view, VerticalTabsPrimaryInfo::Branch),
            [
                ("w1:p1".to_owned(), "api".to_owned()),
                ("w1:p2".to_owned(), "api".to_owned()),
            ],
            "neither directory is in a git repository"
        );

        app.update(|ctx| {
            view.update(ctx, |view, ctx| {
                view.handle_action(
                    &HerdrWorkspaceViewAction::SelectTab(HerdrTabId::from("w1:t2")),
                    ctx,
                );
            });
        });
        for primary in [
            VerticalTabsPrimaryInfo::Command,
            VerticalTabsPrimaryInfo::WorkingDirectory,
            VerticalTabsPrimaryInfo::Branch,
        ] {
            assert_eq!(
                herdr_row_titles(&app, &view, primary),
                [("w1:p3".to_owned(), "api".to_owned())],
                "Herdr reports nothing for this pane: {primary:?}"
            );
        }
    });
}
