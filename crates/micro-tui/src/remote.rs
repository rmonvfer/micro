//! The seam a phone reaches the interface through.
//!
//! The interface owns the agent, so everything a phone wants done has to arrive here. It
//! arrives as channels rather than as a handle to whatever is holding the phone's end:
//! this crate stays free of the relay, the pairing and the protocol, and what crosses the
//! seam is only what the interface can actually act on — a line to submit, a turn to
//! stop — and what it can report back.

use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;

/// Something the phone asked the interface to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FromPhone {
    /// Text to submit, exactly as though it had been typed here.
    ///
    /// It goes through the same path a typed line does, so a line naming a command runs
    /// as that command — which is what makes the phone's palette worth offering at all.
    Submit(String),
    /// Text to put in front of whatever is running.
    Steer(String),
    /// Text to run once whatever is running has finished.
    FollowUp(String),
    /// Stop the turn in flight.
    Abort,
}

/// Something the interface tells whoever is holding the phone's end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToPhone {
    /// Whether a turn is running. The phone shows a prompt or a stop button on it, and
    /// refuses to start a turn inside another one.
    Running(bool),
}

/// Both halves of the seam.
pub struct Remote {
    /// What the phone has asked for.
    pub incoming: UnboundedReceiver<FromPhone>,
    /// What the interface has to say back.
    pub outgoing: UnboundedSender<ToPhone>,
}

impl Remote {
    /// Says whether a turn is running, when that has changed.
    ///
    /// A send that fails means nobody is holding the other end any more, which is not
    /// worth interrupting a session over.
    pub fn report_running(&self, running: bool) {
        let _ = self.outgoing.send(ToPhone::Running(running));
    }
}
