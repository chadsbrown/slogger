//! Per-pane state structs and view-side render methods that were
//! originally written as overlay drawers (R1-R3 transition). After R4
//! the drawer overlay pattern is gone — these modules now exclusively
//! supply the bodies of the Logbook tab's floating panels.
//!
//! Directory name kept as `drawers` for now to minimize churn; module
//! contents (logbook search/grid/detail, awards drill) are the canonical
//! home for that state.

pub(super) mod awards;
pub(super) mod logbook;
