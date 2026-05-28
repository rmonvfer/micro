

use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;

/// Something the phone asked the interface to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FromPhone {
    /// Text to submit, exactly as though it had been typed here.
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
    /// Whether a turn is running.
    Running(bool),
}

/// Both halves of the seam.
pub struct Remote {
    
    pub incoming: UnboundedReceiver<FromPhone>,
    /// What the interface has to say back.
    pub outgoing: UnboundedSender<ToPhone>,
}

impl Remote {
    /// Says whether a turn is running, when that has changed.
    pub fn report_running(&self, running: bool) {
        let _ = self.outgoing.send(ToPhone::Running(running));
    }
}
