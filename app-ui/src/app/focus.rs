//! Widget identifiers used for keyboard focus management. iced's
//! `text_input::Id` lookups go through the runtime widget-operation
//! mechanism — see `iced::widget::operation::focus`. Each constant here
//! must be unique within the running view tree.

use iced::Task;
use iced::widget::Id;

use super::message::Message;

pub(super) const ENTRY_CALL_ID: &str = "entry-call";
pub(super) const ENTRY_FREQ_ID: &str = "entry-freq";
pub(super) const ENTRY_RST_SENT_ID: &str = "entry-rst-sent";
pub(super) const ENTRY_RST_RCVD_ID: &str = "entry-rst-rcvd";

/// Returns a Task that focuses the Call input. Useful after logging a QSO
/// (the operator is ready for the next callsign) or after a successful
/// QSO edit cancel.
pub(super) fn focus_call() -> Task<Message> {
    iced::widget::operation::focus(Id::new(ENTRY_CALL_ID))
}
