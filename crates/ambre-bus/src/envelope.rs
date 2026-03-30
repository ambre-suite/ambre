//! Envelope types for UI ↔ Logic communication.
//!
//! - `UiEnvelope`: UI Thread → Logic Worker
//! - `LogicEnvelope`: Logic Worker → UI Thread
//!
//! Both envelopes carry opaque `Vec<u8>` payloads that are serialized with postcard.
//! The payloads are application‑specific commands/events; the bus does not interpret them.

use serde::{Deserialize, Serialize};

/// Envelope sent from the UI Thread to the Logic Worker.
///
/// Contains a `Vec<u8>` that is a postcard‑serialized application command.
/// The `doc_id` identifies the target document (or `None` for session‑level commands).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiEnvelope {
    /// Target document identifier, if the command is document‑specific.
    pub doc_id: Option<crate::Uuid>,
    /// Opaque postcard‑serialized application command.
    pub payload: Vec<u8>,
}

/// Envelope sent from the Logic Worker to the UI Thread.
///
/// Contains a `Vec<u8>` that is a postcard‑serialized application event.
/// The `doc_id` identifies the source document (or `None` for session‑level events).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogicEnvelope {
    /// Source document identifier, if the event is document‑specific.
    pub doc_id: Option<crate::Uuid>,
    /// Opaque postcard‑serialized application event.
    pub payload: Vec<u8>,
}

impl UiEnvelope {
    /// Creates a new `UiEnvelope` with the given document ID and payload.
    pub fn new(doc_id: Option<crate::Uuid>, payload: Vec<u8>) -> Self {
        Self { doc_id, payload }
    }

    /// Creates a session‑level envelope (no document ID).
    pub fn session(payload: Vec<u8>) -> Self {
        Self {
            doc_id: None,
            payload,
        }
    }

    /// Creates a document‑specific envelope.
    pub fn document(doc_id: crate::Uuid, payload: Vec<u8>) -> Self {
        Self {
            doc_id: Some(doc_id),
            payload,
        }
    }
}

impl LogicEnvelope {
    /// Creates a new `LogicEnvelope` with the given document ID and payload.
    pub fn new(doc_id: Option<crate::Uuid>, payload: Vec<u8>) -> Self {
        Self { doc_id, payload }
    }

    /// Creates a session‑level envelope (no document ID).
    pub fn session(payload: Vec<u8>) -> Self {
        Self {
            doc_id: None,
            payload,
        }
    }

    /// Creates a document‑specific envelope.
    pub fn document(doc_id: crate::Uuid, payload: Vec<u8>) -> Self {
        Self {
            doc_id: Some(doc_id),
            payload,
        }
    }
}