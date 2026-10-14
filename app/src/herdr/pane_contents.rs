use std::collections::HashMap;
use std::sync::Arc;

use herdr_client::{
    GridSize, HerdrCli, PaneId, PaneOwnership, PaneStream, PaneStreamEvent, SessionName,
    SessionTarget,
};
use warpui::{Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use super::pane_terminal::PaneTerminal;
use super::{HerdrSessionEvent, HerdrSessionModel};

/// What one pane shows: the screen its stream draws and how warpr holds the pane.
pub(crate) struct HerdrPaneContents {
    /// Shared with the element that paints the pane, which reports the size it has.
    stream: Arc<PaneStream>,
    terminal: Option<PaneTerminal>,
    ownership: Option<PaneOwnership>,
}

pub(crate) enum HerdrPaneContentsEvent {
    Changed,
}

impl HerdrPaneContents {
    fn new(
        cli: HerdrCli,
        target: SessionTarget,
        pane: PaneId,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let (events_tx, events_rx) = async_channel::unbounded();
        let stream = PaneStream::start(cli, target, pane, move |event| {
            // Only fails once the model is gone, when nobody is left to show the event to.
            let _ = events_tx.try_send(event);
        });
        let _ = ctx.spawn_stream_local(events_rx, |me, event, ctx| me.apply(event, ctx), |_, _| {});
        Self {
            stream: Arc::new(stream),
            terminal: None,
            ownership: None,
        }
    }

    fn apply(&mut self, event: PaneStreamEvent, ctx: &mut ModelContext<Self>) {
        match event {
            PaneStreamEvent::Frame(frame) => self
                .terminal
                .get_or_insert_with(|| PaneTerminal::new(frame.width, frame.height))
                .apply(&frame),
            PaneStreamEvent::Ownership(ownership) => self.ownership = Some(ownership),
        }
        ctx.emit(HerdrPaneContentsEvent::Changed);
    }

    pub(crate) fn terminal(&self) -> Option<&PaneTerminal> {
        self.terminal.as_ref()
    }

    /// `None` until the stream has reported how it holds the pane.
    pub(crate) fn ownership(&self) -> Option<&PaneOwnership> {
        self.ownership.as_ref()
    }

    pub(crate) fn stream(&self) -> Arc<PaneStream> {
        Arc::clone(&self.stream)
    }
}

impl Entity for HerdrPaneContents {
    type Event = HerdrPaneContentsEvent;
}

/// The contents of every pane of the presented session, so that a pane keeps its stream when it
/// goes out of view and back, and loses it when Herdr closes it or another session is presented.
///
/// Creating a pane's contents starts nothing: its stream only runs while the pane is shown and
/// has been given a size.
#[derive(Default)]
pub(crate) struct HerdrPaneContentsRegistry {
    session: Option<SessionName>,
    panes: HashMap<PaneId, ModelHandle<HerdrPaneContents>>,
    /// The panes each view that presents panes currently shows.
    shown: HashMap<EntityId, Vec<PaneId>>,
}

impl HerdrPaneContentsRegistry {
    /// Follows the session model, which must exist.
    pub(crate) fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&HerdrSessionModel::handle(ctx), |me, _, event, ctx| {
            if matches!(event, HerdrSessionEvent::Presented) {
                me.sync(ctx);
            }
        });
        let mut registry = Self::default();
        registry.sync(ctx);
        registry
    }

    pub(crate) fn contents(&self, pane: &PaneId) -> Option<ModelHandle<HerdrPaneContents>> {
        self.panes.get(pane).cloned()
    }

    /// Records the panes `view` shows; every other pane is not shown by it.
    pub(crate) fn set_shown(
        &mut self,
        view: EntityId,
        panes: Vec<PaneId>,
        ctx: &mut ModelContext<Self>,
    ) {
        if panes.is_empty() {
            self.shown.remove(&view);
        } else {
            self.shown.insert(view, panes);
        }
        self.apply_visibility(ctx);
    }

    fn apply_visibility(&self, ctx: &mut ModelContext<Self>) {
        for (id, handle) in &self.panes {
            let visible = self.shown.values().any(|shown| shown.contains(id));
            handle.read(ctx, |contents, _| contents.stream.set_visible(visible));
        }
    }

    /// Makes the registry hold exactly the panes the session presents, each told Herdr's own grid
    /// for it.
    fn sync(&mut self, ctx: &mut ModelContext<Self>) {
        let session = HerdrSessionModel::as_ref(ctx);
        let source = session.stream_source();
        let current = source.as_ref().map(|(_, target)| target.name().clone());
        if current != self.session {
            self.panes.clear();
            self.shown.clear();
            self.session = current;
        }
        let Some((cli, target)) = source else {
            return;
        };
        let grids: Vec<_> = session
            .presented()
            .workspaces
            .iter()
            .flat_map(|workspace| &workspace.herdr_tabs)
            .filter_map(|tab| tab.layout.as_ref())
            .flat_map(|layout| layout.frames())
            .map(|pane| {
                let grid = GridSize {
                    cols: pane.rect.width,
                    rows: pane.rect.height,
                };
                (pane.id, grid)
            })
            .collect();

        self.panes
            .retain(|id, _| grids.iter().any(|(pane, _)| pane == id));
        for (pane, grid) in grids {
            let handle = match self.panes.get(&pane) {
                Some(handle) => handle.clone(),
                None => {
                    let handle = ctx.add_model(|ctx| {
                        HerdrPaneContents::new(cli.clone(), target.clone(), pane.clone(), ctx)
                    });
                    ctx.subscribe_to_model(&handle, |_, _, _, ctx| {
                        ctx.emit(HerdrPaneContentsRegistryEvent::PaneChanged);
                    });
                    self.panes.insert(pane, handle.clone());
                    handle
                }
            };
            handle.read(ctx, |contents, _| contents.stream.set_herdr_grid(grid));
        }
        self.apply_visibility(ctx);
    }
}

pub(crate) enum HerdrPaneContentsRegistryEvent {
    /// Some pane has something new to show.
    PaneChanged,
}

impl Entity for HerdrPaneContentsRegistry {
    type Event = HerdrPaneContentsRegistryEvent;
}

impl SingletonEntity for HerdrPaneContentsRegistry {}

#[cfg(all(test, unix))]
#[path = "pane_contents_tests.rs"]
mod tests;
