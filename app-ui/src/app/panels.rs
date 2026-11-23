//! Free-floating panels, used by every tab (Operating, QSL, Logbook).
//!
//! Each tab has its own [`Panels`]: a set of panels on that tab's canvas.
//! Each panel is an independent rectangle on the canvas: it can
//! be moved, resized, raised, closed and reopened without affecting any
//! other panel. Panels may overlap and may leave gaps — nothing tiles.
//!
//! Geometry is stored as fractions of the canvas (0.0..=1.0) rather than
//! pixels, so an arrangement keeps its proportions when the window is
//! resized and can never end up off-screen.
//!
//! # Adding a panel
//!
//! 1. Add a variant to [`PanelKind`] and list it in its tab's list
//!    ([`PanelKind::OPERATING`], [`PanelKind::QSL`] or
//!    [`PanelKind::LOGBOOK`]).
//! 2. Give it a `title`, a `default_rect` and a `default_open`.
//! 3. Render its body in `views::floating::App::panel_body`.
//!
//! Saved layouts that predate the new panel pick it up automatically with
//! its defaults (see [`Panels::from_saved`]).

use iced::{Point, Size};
use serde::{Deserialize, Serialize};

/// Smallest size a panel can be resized to, in pixels.
const MIN_WIDTH: f32 = 180.0;
const MIN_HEIGHT: f32 = 90.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PanelKind {
    Spots,
    Entry,
    Keyer,
    Station,
    Recent,
    CondX,
    // QSL tab
    QslServices,
    QslPending,
    QslConfirmations,
    // Logbook tab
    LogSearch,
    LogResults,
    LogDetail,
    LogAwards,
    LogTools,
}

impl PanelKind {
    /// The Operating tab's panels, in the order they appear in its
    /// panel bar.
    pub(super) const OPERATING: &[PanelKind] = &[
        PanelKind::Entry,
        PanelKind::Spots,
        PanelKind::Recent,
        PanelKind::Keyer,
        PanelKind::Station,
        PanelKind::CondX,
    ];

    /// The QSL tab's panels.
    pub(super) const QSL: &[PanelKind] = &[
        PanelKind::QslServices,
        PanelKind::QslPending,
        PanelKind::QslConfirmations,
    ];

    /// The Logbook tab's panels.
    pub(super) const LOGBOOK: &[PanelKind] = &[
        PanelKind::LogSearch,
        PanelKind::LogResults,
        PanelKind::LogDetail,
        PanelKind::LogAwards,
        PanelKind::LogTools,
    ];

    pub(super) fn title(self) -> &'static str {
        match self {
            PanelKind::Spots => "Spots",
            PanelKind::Entry => "Entry",
            PanelKind::Keyer => "CW Keyer",
            PanelKind::Station => "Station",
            PanelKind::Recent => "Recent QSOs",
            PanelKind::CondX => "CondX",
            PanelKind::QslServices => "Services",
            PanelKind::QslPending => "Pending uploads",
            PanelKind::QslConfirmations => "Recent confirmations",
            PanelKind::LogSearch => "Search",
            PanelKind::LogResults => "Results",
            PanelKind::LogDetail => "Detail",
            PanelKind::LogAwards => "Awards",
            PanelKind::LogTools => "Tools",
        }
    }

    /// Where the panel sits the first time it is shown (and after
    /// "Reset layout"). The panels open by default happen not to overlap:
    ///
    /// ```text
    /// ┌─────────────────────────┬─────────────────┐
    /// │                         │     Entry       │
    /// │                         ├─────────────────┤
    /// │         Spots           │    CW Keyer     │
    /// │                         ├─────────────────┤
    /// │                         │    Station      │
    /// ├─────────────────────────┴─────────────────┤
    /// │              Recent QSOs                  │
    /// └───────────────────────────────────────────┘
    /// ```
    ///
    /// CondX starts closed; when opened it sits over the right-hand end
    /// of Spots (the comment column), clear of the Entry form.
    ///
    /// QSL stacks its three panels full width: Services, then Pending
    /// uploads (the largest), then Recent confirmations. Logbook has
    /// Search, Results and Detail down the left and Awards over Tools on
    /// the right. Both match the fixed layouts those tabs had before
    /// their panels floated.
    pub(super) fn default_rect(self) -> Rect {
        let (x, y, w, h) = match self {
            PanelKind::Spots => (0.00, 0.00, 0.60, 0.68),
            PanelKind::Entry => (0.60, 0.00, 0.40, 0.30),
            PanelKind::Keyer => (0.60, 0.30, 0.40, 0.20),
            PanelKind::Station => (0.60, 0.50, 0.40, 0.18),
            PanelKind::Recent => (0.00, 0.68, 1.00, 0.32),
            PanelKind::CondX => (0.36, 0.00, 0.24, 0.42),
            PanelKind::QslServices => (0.00, 0.00, 1.00, 0.30),
            PanelKind::QslPending => (0.00, 0.30, 1.00, 0.38),
            PanelKind::QslConfirmations => (0.00, 0.68, 1.00, 0.32),
            PanelKind::LogSearch => (0.00, 0.00, 0.65, 0.29),
            PanelKind::LogResults => (0.00, 0.29, 0.65, 0.37),
            PanelKind::LogDetail => (0.00, 0.66, 0.65, 0.34),
            PanelKind::LogAwards => (0.65, 0.00, 0.35, 0.45),
            PanelKind::LogTools => (0.65, 0.45, 0.35, 0.55),
        };
        Rect { x, y, w, h }
    }

    /// Whether the panel is open before the operator has chosen. New,
    /// specialised panels should usually start closed.
    pub(super) fn default_open(self) -> bool {
        match self {
            PanelKind::Spots
            | PanelKind::Entry
            | PanelKind::Keyer
            | PanelKind::Station
            | PanelKind::Recent
            | PanelKind::QslServices
            | PanelKind::QslPending
            | PanelKind::QslConfirmations
            | PanelKind::LogSearch
            | PanelKind::LogResults
            | PanelKind::LogDetail
            | PanelKind::LogAwards
            | PanelKind::LogTools => true,
            // An optional network feed: off until the operator asks, so
            // existing layouts look the same as before it existed.
            PanelKind::CondX => false,
        }
    }
}

/// A rectangle in canvas fractions: `(0,0)` is the top-left of the canvas
/// and `(1,1)` the bottom-right.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    /// Pixel position and size on a canvas of the given size, snapped to
    /// whole pixels so text stays crisp.
    pub(super) fn to_pixels(self, canvas: Size) -> (Point, Size) {
        (
            Point::new((self.x * canvas.width).round(), (self.y * canvas.height).round()),
            Size::new((self.w * canvas.width).round(), (self.h * canvas.height).round()),
        )
    }

    /// Pulls the rectangle back inside the canvas: sizes are capped at the
    /// full canvas, then the position is shifted so nothing hangs off an
    /// edge. Also repairs hand-edited or corrupt saved values.
    fn clamped(self) -> Rect {
        let finite = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
        let w = finite(self.w, 0.3).clamp(0.02, 1.0);
        let h = finite(self.h, 0.3).clamp(0.02, 1.0);
        Rect {
            x: finite(self.x, 0.0).clamp(0.0, 1.0 - w),
            y: finite(self.y, 0.0).clamp(0.0, 1.0 - h),
            w,
            h,
        }
    }

    /// The rectangle moved by a pixel delta, kept inside the canvas.
    fn moved(self, dx: f32, dy: f32, canvas: Size) -> Rect {
        Rect {
            x: self.x + dx / canvas.width,
            y: self.y + dy / canvas.height,
            ..self
        }
        .clamped()
    }

    /// The rectangle with its bottom-right corner dragged by a pixel
    /// delta. The top-left stays put; the size is held between the
    /// minimum panel size and the canvas edge.
    fn resized(self, dx: f32, dy: f32, canvas: Size) -> Rect {
        let min_w = (MIN_WIDTH / canvas.width).min(1.0);
        let min_h = (MIN_HEIGHT / canvas.height).min(1.0);
        let max_w = (1.0 - self.x).max(min_w);
        let max_h = (1.0 - self.y).max(min_h);
        Rect {
            w: (self.w + dx / canvas.width).clamp(min_w, max_w),
            h: (self.h + dy / canvas.height).clamp(min_h, max_h),
            ..self
        }
        .clamped()
    }
}

/// One panel's persistent state.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Panel {
    pub kind: PanelKind,
    #[serde(flatten)]
    pub rect: Rect,
    pub open: bool,
}

impl Panel {
    fn default_for(kind: PanelKind) -> Self {
        Self {
            kind,
            rect: kind.default_rect(),
            open: kind.default_open(),
        }
    }
}

/// What a pointer drag is doing to a panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragMode {
    /// Dragging the title bar: reposition.
    Move,
    /// Dragging the corner grip: resize from the bottom-right.
    Resize,
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    kind: PanelKind,
    mode: DragMode,
    /// Canvas size when the drag began, to convert pixel deltas.
    canvas: Size,
    /// Pointer position at the first movement of the drag. The press
    /// itself carries no position, so the first move becomes the anchor.
    anchor: Option<Point>,
    /// The panel's rectangle when the drag began.
    origin: Rect,
}

/// One tab's set of panels and any drag in progress.
#[derive(Debug, Clone)]
pub struct Panels {
    /// The panels this tab has, in panel-bar order.
    kinds: &'static [PanelKind],
    /// Back-to-front: the last panel is drawn on top.
    panels: Vec<Panel>,
    drag: Option<Drag>,
}

impl Panels {
    /// The given panels at their default places.
    pub(super) fn new(kinds: &'static [PanelKind]) -> Self {
        Self {
            kinds,
            panels: kinds.iter().copied().map(Panel::default_for).collect(),
            drag: None,
        }
    }

    /// Rebuilds a tab's panels from a saved layout. Stacking order is
    /// preserved; duplicate entries and panels that belong to another
    /// tab are dropped; geometry is clamped back onto the canvas; and any
    /// panel the saved layout doesn't mention (added in a newer build) is
    /// appended with its defaults.
    pub(super) fn from_saved(kinds: &'static [PanelKind], saved: Vec<Panel>) -> Self {
        let mut panels: Vec<Panel> = Vec::with_capacity(kinds.len());
        for mut panel in saved {
            if kinds.contains(&panel.kind) && panels.iter().all(|p| p.kind != panel.kind) {
                panel.rect = panel.rect.clamped();
                panels.push(panel);
            }
        }
        for kind in kinds {
            if panels.iter().all(|p| p.kind != *kind) {
                panels.push(Panel::default_for(*kind));
            }
        }
        Self { kinds, panels, drag: None }
    }

    /// This tab's panels, in panel-bar order.
    pub(super) fn kinds(&self) -> &'static [PanelKind] {
        self.kinds
    }

    /// Whether `kind` is one of this tab's panels.
    pub(super) fn has(&self, kind: PanelKind) -> bool {
        self.kinds.contains(&kind)
    }

    /// The state to persist, in stacking order.
    pub(super) fn to_saved(&self) -> Vec<Panel> {
        self.panels.clone()
    }

    /// Open panels, back-to-front.
    pub(super) fn open(&self) -> impl Iterator<Item = &Panel> {
        self.panels.iter().filter(|p| p.open)
    }

    pub(super) fn is_open(&self, kind: PanelKind) -> bool {
        self.panels.iter().any(|p| p.kind == kind && p.open)
    }

    pub(super) fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Brings a panel to the front.
    pub(super) fn raise(&mut self, kind: PanelKind) {
        if let Some(i) = self.panels.iter().position(|p| p.kind == kind) {
            let panel = self.panels.remove(i);
            self.panels.push(panel);
        }
    }

    /// Opens a closed panel (on top, where it was last placed) or closes
    /// an open one.
    pub(super) fn toggle(&mut self, kind: PanelKind) {
        let Some(panel) = self.panels.iter_mut().find(|p| p.kind == kind) else {
            return;
        };
        panel.open = !panel.open;
        if panel.open {
            self.raise(kind);
        }
    }

    /// Restores every panel's default position, size and visibility.
    pub(super) fn reset(&mut self) {
        *self = Self::new(self.kinds);
    }

    /// Begins moving or resizing a panel, raising it first.
    pub(super) fn start_drag(&mut self, kind: PanelKind, mode: DragMode, canvas: Size) {
        if canvas.width < 1.0 || canvas.height < 1.0 {
            return;
        }
        self.raise(kind);
        let Some(panel) = self.panels.iter().find(|p| p.kind == kind) else {
            return;
        };
        self.drag = Some(Drag {
            kind,
            mode,
            canvas,
            anchor: None,
            origin: panel.rect,
        });
    }

    /// Applies pointer movement to the drag in progress, if any.
    pub(super) fn drag_to(&mut self, pointer: Point) {
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        let anchor = *drag.anchor.get_or_insert(pointer);
        let (dx, dy) = (pointer.x - anchor.x, pointer.y - anchor.y);
        let rect = match drag.mode {
            DragMode::Move => drag.origin.moved(dx, dy, drag.canvas),
            DragMode::Resize => drag.origin.resized(dx, dy, drag.canvas),
        };
        let kind = drag.kind;
        if let Some(panel) = self.panels.iter_mut().find(|p| p.kind == kind) {
            panel.rect = rect;
        }
    }

    /// Ends the drag in progress. Returns true if there was one, i.e. the
    /// layout may have changed and is worth saving.
    pub(super) fn end_drag(&mut self) -> bool {
        self.drag.take().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANVAS: Size = Size::new(1000.0, 500.0);

    fn rect_of(panels: &Panels, kind: PanelKind) -> Rect {
        panels.panels.iter().find(|p| p.kind == kind).unwrap().rect
    }

    fn top(panels: &Panels) -> PanelKind {
        panels.open().last().unwrap().kind
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn default_panels_fit_on_the_canvas_without_overlapping() {
        for kinds in [PanelKind::OPERATING, PanelKind::QSL, PanelKind::LOGBOOK] {
            let panels = Panels::new(kinds);
            for kind in kinds {
                let r = kind.default_rect();
                assert!(r.x >= 0.0 && r.y >= 0.0 && r.x + r.w <= 1.0001 && r.y + r.h <= 1.0001);
            }
            let rects: Vec<Rect> = panels.open().map(|p| p.rect).collect();
            let open_by_default = kinds.iter().filter(|k| k.default_open()).count();
            assert_eq!(rects.len(), open_by_default);
            for (i, a) in rects.iter().enumerate() {
                for b in &rects[i + 1..] {
                    let overlap_x = a.x < b.x + b.w - 1e-4 && b.x < a.x + a.w - 1e-4;
                    let overlap_y = a.y < b.y + b.h - 1e-4 && b.y < a.y + a.h - 1e-4;
                    assert!(!(overlap_x && overlap_y), "{a:?} overlaps {b:?}");
                }
            }
        }
    }

    /// The QSL and Logbook defaults fill their canvases with no gaps,
    /// as the fixed layouts they replace did.
    #[test]
    fn qsl_and_logbook_defaults_cover_the_whole_canvas() {
        for kinds in [PanelKind::QSL, PanelKind::LOGBOOK] {
            let area: f32 = kinds.iter().map(|k| k.default_rect()).map(|r| r.w * r.h).sum();
            assert!(close(area, 1.0), "{kinds:?} cover {area}");
        }
    }

    #[test]
    fn every_panel_belongs_to_exactly_one_tab() {
        let all: Vec<PanelKind> = [PanelKind::OPERATING, PanelKind::QSL, PanelKind::LOGBOOK]
            .concat();
        for kind in &all {
            assert_eq!(all.iter().filter(|k| *k == kind).count(), 1, "{kind:?}");
        }
    }

    #[test]
    fn a_saved_panel_from_another_tab_is_ignored() {
        let stray = vec![Panel {
            kind: PanelKind::Spots,
            rect: Rect { x: 0.0, y: 0.0, w: 0.5, h: 0.5 },
            open: true,
        }];
        let panels = Panels::from_saved(PanelKind::QSL, stray);
        assert_eq!(panels.to_saved().len(), PanelKind::QSL.len());
        assert!(!panels.to_saved().iter().any(|p| p.kind == PanelKind::Spots));
    }

    #[test]
    fn reset_restores_this_tabs_panels_only() {
        let mut panels = Panels::new(PanelKind::LOGBOOK);
        panels.toggle(PanelKind::LogTools);
        panels.reset();
        assert!(panels.is_open(PanelKind::LogTools));
        assert_eq!(panels.to_saved().len(), PanelKind::LOGBOOK.len());
    }

    #[test]
    fn moving_one_panel_leaves_the_others_alone() {
        let mut panels = Panels::new(PanelKind::OPERATING);
        let before: Vec<Panel> = panels.to_saved();
        panels.start_drag(PanelKind::Keyer, DragMode::Move, CANVAS);
        panels.drag_to(Point::new(700.0, 200.0));
        panels.drag_to(Point::new(600.0, 250.0));
        assert!(panels.end_drag());

        let moved = rect_of(&panels, PanelKind::Keyer);
        let origin = PanelKind::Keyer.default_rect();
        assert!(close(moved.x, origin.x - 0.1) && close(moved.y, origin.y + 0.1));
        assert!(close(moved.w, origin.w) && close(moved.h, origin.h));
        for p in before.iter().filter(|p| p.kind != PanelKind::Keyer) {
            assert_eq!(rect_of(&panels, p.kind), p.rect);
        }
    }

    #[test]
    fn a_panel_cannot_be_dragged_off_the_canvas() {
        let mut panels = Panels::new(PanelKind::OPERATING);
        panels.start_drag(PanelKind::Entry, DragMode::Move, CANVAS);
        panels.drag_to(Point::new(0.0, 0.0));
        panels.drag_to(Point::new(5000.0, -5000.0));
        let r = rect_of(&panels, PanelKind::Entry);
        assert!(close(r.x + r.w, 1.0) && close(r.y, 0.0));
    }

    #[test]
    fn resizing_respects_minimum_size_and_canvas_edge() {
        let mut panels = Panels::new(PanelKind::OPERATING);
        panels.start_drag(PanelKind::Spots, DragMode::Resize, CANVAS);
        panels.drag_to(Point::new(0.0, 0.0));
        panels.drag_to(Point::new(-5000.0, -5000.0));
        let r = rect_of(&panels, PanelKind::Spots);
        assert!(close(r.w, MIN_WIDTH / CANVAS.width) && close(r.h, MIN_HEIGHT / CANVAS.height));

        panels.drag_to(Point::new(5000.0, 5000.0));
        let r = rect_of(&panels, PanelKind::Spots);
        assert!(close(r.x + r.w, 1.0) && close(r.y + r.h, 1.0));
        // Resizing never moves the top-left corner.
        assert!(close(r.x, 0.0) && close(r.y, 0.0));
    }

    #[test]
    fn pointer_movement_without_a_drag_does_nothing() {
        let mut panels = Panels::new(PanelKind::OPERATING);
        let before = panels.to_saved();
        panels.drag_to(Point::new(300.0, 300.0));
        assert!(!panels.end_drag());
        assert_eq!(panels.to_saved(), before);
    }

    #[test]
    fn closing_and_reopening_keeps_placement_and_raises() {
        let mut panels = Panels::new(PanelKind::OPERATING);
        panels.start_drag(PanelKind::Station, DragMode::Move, CANVAS);
        panels.drag_to(Point::new(0.0, 0.0));
        panels.drag_to(Point::new(-200.0, 0.0));
        panels.end_drag();
        let placed = rect_of(&panels, PanelKind::Station);

        panels.toggle(PanelKind::Station);
        assert!(!panels.is_open(PanelKind::Station));
        panels.raise(PanelKind::Spots);
        panels.toggle(PanelKind::Station);
        assert!(panels.is_open(PanelKind::Station));
        assert_eq!(rect_of(&panels, PanelKind::Station), placed);
        assert_eq!(top(&panels), PanelKind::Station);
    }

    #[test]
    fn saved_layout_round_trips_through_json() {
        let mut panels = Panels::new(PanelKind::OPERATING);
        panels.toggle(PanelKind::Keyer);
        panels.raise(PanelKind::Entry);
        let json = serde_json::to_string(&panels.to_saved()).unwrap();
        let restored = Panels::from_saved(PanelKind::OPERATING, serde_json::from_str(&json).unwrap());
        assert_eq!(restored.to_saved(), panels.to_saved());
    }

    #[test]
    fn saved_layout_missing_a_panel_gains_it_with_defaults() {
        let saved = vec![Panel {
            kind: PanelKind::Spots,
            rect: Rect { x: 0.1, y: 0.1, w: 0.5, h: 0.5 },
            open: false,
        }];
        let panels = Panels::from_saved(PanelKind::OPERATING, saved);
        assert_eq!(panels.to_saved().len(), PanelKind::OPERATING.len());
        assert!(!panels.is_open(PanelKind::Spots));
        assert!(panels.is_open(PanelKind::Keyer));
        assert_eq!(rect_of(&panels, PanelKind::Keyer), PanelKind::Keyer.default_rect());
    }

    #[test]
    fn condx_starts_closed() {
        assert!(!Panels::new(PanelKind::OPERATING).is_open(PanelKind::CondX));
    }

    /// A layout saved before CondX existed lists only the original five
    /// panels. It must keep their arrangement, gain CondX closed at its
    /// default spot, and let the operator open it from the panel bar.
    #[test]
    fn layout_saved_before_condx_gains_it_closed_and_openable() {
        let old: Vec<Panel> = Panels::new(PanelKind::OPERATING)
            .to_saved()
            .into_iter()
            .filter(|p| p.kind != PanelKind::CondX)
            .collect();
        let mut panels = Panels::from_saved(PanelKind::OPERATING, old.clone());
        assert_eq!(panels.to_saved()[..old.len()], old[..]);
        assert!(!panels.is_open(PanelKind::CondX));
        assert_eq!(rect_of(&panels, PanelKind::CondX), PanelKind::CondX.default_rect());

        panels.toggle(PanelKind::CondX);
        assert!(panels.is_open(PanelKind::CondX));
        assert_eq!(top(&panels), PanelKind::CondX);
    }

    #[test]
    fn saved_geometry_is_pulled_back_onto_the_canvas() {
        let saved = vec![Panel {
            kind: PanelKind::Entry,
            rect: Rect { x: 0.9, y: -3.0, w: 0.4, h: f32::NAN },
            open: true,
        }];
        let r = rect_of(&Panels::from_saved(PanelKind::OPERATING, saved), PanelKind::Entry);
        assert!(close(r.x, 0.6) && close(r.y, 0.0) && close(r.w, 0.4) && r.h.is_finite());
    }
}
