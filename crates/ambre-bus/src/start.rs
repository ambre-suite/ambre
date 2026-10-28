//! The Main side of a worker's start (`SYNC-057`).
//!
//! A worker installs its inbound handler at the end of an asynchronous start,
//! so a command posted before then reaches no handler at all. The worker
//! posts [`ANNOUNCEMENT`] as the last step of its start. The Main side holds
//! every command in [`StartGate`] until it reads one.

use crate::conduit::Receiver;

/// What a worker posts once its inbound handler is installed: an empty
/// payload. An encoded event is never empty, so the two cannot be confused.
pub const ANNOUNCEMENT: &[u8] = &[];

/// Holds the commands bound for one worker until that worker announces it
/// is listening.
#[derive(Debug, Default)]
pub struct StartGate {
    /// Whether the worker has announced itself.
    open: bool,
}

impl StartGate {
    /// A gate the worker has not opened yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads one payload from the worker. The announcement opens the gate and
    /// yields nothing; any other payload is an event, handed back.
    pub fn read<'a>(&mut self, bytes: &'a [u8]) -> Option<&'a [u8]> {
        if bytes == ANNOUNCEMENT {
            self.open = true;
            return None;
        }
        Some(bytes)
    }

    /// Whether the worker has announced itself.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Hands every queued command to `post`, in order, once the gate is
    /// open. Before that, the commands stay in `commands`.
    pub fn drain<T>(&self, commands: &Receiver<T>, mut post: impl FnMut(T)) {
        if !self.open {
            return;
        }
        while let Ok(command) = commands.try_recv() {
            post(command);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conduit::{Doorbell, to_main};
    use crate::event::LogicEvent;

    /// `SYNC-057`: commands sent while the worker starts wait, and reach it
    /// in order once it announces itself. None is posted before.
    #[test]
    fn no_command_is_posted_before_the_worker_announces_itself() {
        let bell = Doorbell::new();
        let (tx, rx) = to_main::<u32>(crate::conduit::ChannelCapacity::new(8), &bell);
        let mut gate = StartGate::new();
        let mut posted = Vec::new();

        tx.send(1).unwrap();
        tx.send(2).unwrap();
        for _ in 0..3 {
            gate.drain(&rx, |command| posted.push(command));
        }
        assert!(posted.is_empty(), "the worker is not listening yet");

        let event = postcard::to_allocvec(&LogicEvent::SetupRequired).unwrap();
        assert_eq!(gate.read(&event), Some(event.as_slice()));
        assert!(!gate.is_open(), "an event is not the announcement");
        assert!(!event.is_empty(), "an encoded event is never empty");

        assert_eq!(gate.read(ANNOUNCEMENT), None);
        tx.send(3).unwrap();
        gate.drain(&rx, |command| posted.push(command));
        assert_eq!(posted, [1, 2, 3]);
    }
}
