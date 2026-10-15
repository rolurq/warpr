use warpui::{AppContext, ModelHandle, View, ViewContext, ViewHandle};

use super::PaneId;
use crate::app_state::{HerdrWorkspaceContents, LeafContents};
use crate::herdr::HerdrWorkspaceView;
use crate::pane_group::pane::{ShareableLink, ShareableLinkError};
use crate::pane_group::{BackingView, PaneConfiguration, PaneContent, PaneGroup, PaneView};

pub struct HerdrWorkspacePane {
    view: ViewHandle<PaneView<HerdrWorkspaceView>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
}

impl HerdrWorkspacePane {
    pub fn new<V: View>(workspace: HerdrWorkspaceContents, ctx: &mut ViewContext<V>) -> Self {
        let workspace_view =
            ctx.add_typed_action_view(|ctx| HerdrWorkspaceView::new(workspace, ctx));
        let pane_configuration = workspace_view.as_ref(ctx).pane_configuration();
        let pane_view = ctx.add_typed_action_view(|ctx| {
            let pane_id = PaneId::from_herdr_workspace_pane_ctx(ctx);
            PaneView::new(pane_id, workspace_view, (), pane_configuration.clone(), ctx)
        });
        Self {
            view: pane_view,
            pane_configuration,
        }
    }

    pub fn workspace_view(&self, ctx: &AppContext) -> ViewHandle<HerdrWorkspaceView> {
        self.view.as_ref(ctx).child(ctx)
    }
}

impl PaneContent for HerdrWorkspacePane {
    fn id(&self) -> PaneId {
        PaneId::from_herdr_workspace_pane_view(&self.view)
    }

    fn attach(
        &self,
        _group: &PaneGroup,
        focus_handle: crate::pane_group::focus_state::PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        self.view
            .update(ctx, |view, ctx| view.set_focus_handle(focus_handle, ctx));
        let child = self.view.as_ref(ctx).child(ctx);

        let pane_id = self.id();
        ctx.subscribe_to_view(&child, move |pane_group, _, event, ctx| {
            pane_group.handle_pane_event(pane_id, event, ctx);
        });
    }

    fn detach(
        &self,
        _group: &PaneGroup,
        _detach_type: super::DetachType,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        let child = self.view.as_ref(ctx).child(ctx);
        ctx.unsubscribe_to_view(&child);
    }

    fn snapshot(&self, ctx: &AppContext) -> LeafContents {
        LeafContents::HerdrWorkspace(self.view.as_ref(ctx).child(ctx).as_ref(ctx).snapshot())
    }

    fn has_application_focus(&self, ctx: &mut ViewContext<PaneGroup>) -> bool {
        self.view.is_self_or_child_focused(ctx)
    }

    fn focus(&self, ctx: &mut ViewContext<PaneGroup>) {
        self.view
            .as_ref(ctx)
            .child(ctx)
            .update(ctx, BackingView::focus_contents)
    }

    fn shareable_link(
        &self,
        _ctx: &mut ViewContext<PaneGroup>,
    ) -> Result<ShareableLink, ShareableLinkError> {
        Ok(ShareableLink::Base)
    }

    fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    fn is_pane_being_dragged(&self, ctx: &AppContext) -> bool {
        self.view.as_ref(ctx).is_being_dragged()
    }
}
