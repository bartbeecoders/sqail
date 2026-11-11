//! sqail patch: an Attention that can be requested while a request runs.
//!
//! [`Client::cancel_query`](crate::Client::cancel_query) needs the client,
//! which the running request's future borrows, so it can only cancel after
//! that future was dropped, and dropping a future mid-token desyncs the
//! connection. An [`AttentionHandle`] is a separate, cloneable handle: its
//! [`request`](AttentionHandle::request) wakes the connection, whose next
//! read of the response first writes the Attention packet. The response then
//! runs on to the acknowledging DONE as usual, so nothing is dropped
//! mid-message.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Waker;

#[derive(Debug, Default)]
pub(crate) struct AttentionShared {
    requested: AtomicBool,
    waker: Mutex<Option<Waker>>,
    /// Packets read on the connection, so that a canceller can tell a
    /// response still arriving from a server that went quiet.
    packets: AtomicU64,
}

impl AttentionShared {
    pub(crate) fn is_requested(&self) -> bool {
        self.requested.load(Ordering::Acquire)
    }

    pub(crate) fn clear(&self) {
        self.requested.store(false, Ordering::Release);
    }

    pub(crate) fn note_packet(&self) {
        self.packets.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn register(&self, waker: &Waker) {
        let mut slot = self.waker.lock().unwrap_or_else(|e| e.into_inner());
        if !slot.as_ref().is_some_and(|w| w.will_wake(waker)) {
            *slot = Some(waker.clone());
        }
    }
}

/// Requests a TDS Attention on the connection it came from
/// ([`Client::attention_handle`](crate::Client::attention_handle)).
#[derive(Debug, Clone)]
pub struct AttentionHandle(pub(crate) Arc<AttentionShared>);

impl AttentionHandle {
    /// Ask the connection to send an Attention for the request in flight.
    /// It goes out on the connection's next read of that request's response
    /// (right away if a read is waiting on the server). Safe to call at any
    /// time; with no request in flight it waits for
    /// [`Client::finish_attention`](crate::Client::finish_attention), which
    /// clears it.
    pub fn request(&self) {
        self.0.requested.store(true, Ordering::Release);
        let waker = self
            .0
            .waker
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(w) = waker {
            w.wake();
        }
    }

    /// Whether an Attention was requested and not yet finished.
    pub fn is_requested(&self) -> bool {
        self.0.is_requested()
    }

    /// How many packets the connection has read so far. While it grows the
    /// server is still sending: after an Attention, the rest of what it sent
    /// before the Attention reached it, read on the way to the
    /// acknowledgement.
    pub fn packets_read(&self) -> u64 {
        self.0.packets.load(Ordering::Relaxed)
    }
}

/// Where the connection is with a requested Attention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum AttentionProgress {
    #[default]
    Idle,
    /// Written, being flushed.
    Flushing,
    /// On the wire; the acknowledgement has not been read yet.
    Sent,
    /// The acknowledging DONE was read.
    Acked,
}
