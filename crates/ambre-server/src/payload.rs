// ambre-server/src/payload.rs
// The one type the relay holds a payload in, and every form a payload takes.

//! A payload is the bytes of a sealed envelope: a delta, an ephemeral, a message
//! to a client, a batch of deltas, a pending entry. The relay stores and
//! forwards them, and never logs one (`ZK-006`, `SRV-001`).
//!
//! [`Payload`] has no `Debug` and no `Display`, so a log line or a span that
//! names one does not compile, whatever the binding is called. Its bytes are
//! read here and in the stores that write them. `scripts/crate_boundaries.py`
//! refuses a log line in any file that reads them. The socket, a broadcast, an
//! object, a row and a list each hand their bytes over as a `Payload` here. So
//! no raw byte of a payload is bound in a file that logs.

use std::collections::BTreeMap;

use ambre_protocol::ws::{WireDecodeError, WsClientMsg, WsServerMsg};
use aws_sdk_s3::primitives::ByteStream;
use axum::extract::ws::{Message, WebSocket};
use futures_util::{Stream, StreamExt};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// The bytes of a payload. No `Debug`, no `Display`, no `Serialize`: the only
/// readings outside this file are its length and [`Payload::expose_bytes`].
#[derive(Clone, sqlx::Type)]
#[sqlx(transparent)]
pub struct Payload(Vec<u8>);

impl Payload {
    /// Takes `bytes` as a payload.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// A message to a client, encoded for the socket.
    pub fn server_msg(msg: &WsServerMsg) -> Self {
        Self(ambre_protocol::ws::encode_server_msg(msg))
    }

    /// `value` sealed in its stored encoding, as the relay keeps a delta.
    pub fn seal<T: Serialize>(value: &T) -> Result<Self, postcard::Error> {
        postcard::to_allocvec(value).map(Self)
    }

    /// The value [`Payload::seal`] stored.
    pub fn open<T: DeserializeOwned>(&self) -> Result<T, postcard::Error> {
        postcard::from_bytes(&self.0)
    }

    /// The client message a socket frame carries.
    pub fn client_msg(&self) -> Result<WsClientMsg, WireDecodeError> {
        ambre_protocol::ws::decode_client_msg(&self.0)
    }

    /// The length in bytes, the one reading a log may carry.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// The bytes, for a store that writes them. A file that calls this holds no
    /// log line (`scripts/crate_boundaries.py`).
    pub fn expose_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The socket frame that carries it.
    pub fn into_message(self) -> Message {
        Message::Binary(self.0.into())
    }

    /// The body of an object that holds it.
    pub fn into_body(self) -> ByteStream {
        ByteStream::from(self.0)
    }

    /// The payload an object's body holds.
    pub async fn read_body(
        body: ByteStream,
    ) -> Result<Self, aws_sdk_s3::primitives::ByteStreamError> {
        Ok(Self(body.collect().await?.into_bytes().to_vec()))
    }

    /// The payload a broadcast carries.
    pub fn broadcast(message: &rustis::client::PubSubMessage) -> Self {
        Self(message.payload().to_vec())
    }

    /// A batch: each delta's number and its sealed envelope, unchanged
    /// (`SRV-153`). The number is written as the `i64` the stored batches
    /// already hold: a varint spells a signed and an unsigned integer apart.
    pub fn batch(frames: &[(ambre_core::Seq, Payload)]) -> Result<Self, postcard::Error> {
        let frames: Vec<(i64, &[u8])> = frames
            .iter()
            .map(|(s, f)| (s.to_i64_saturating(), f.0.as_slice()))
            .collect();
        postcard::to_allocvec(&frames).map(Self)
    }

    /// The frames of a batch, by number. A negative number, which no relay
    /// writes, refuses the batch.
    pub fn unbatch(&self) -> Result<BTreeMap<ambre_core::Seq, Payload>, postcard::Error> {
        let frames: Vec<(i64, Vec<u8>)> = postcard::from_bytes(&self.0)?;
        frames
            .into_iter()
            .map(|(s, f)| {
                ambre_core::Seq::try_from(s)
                    .map(|s| (s, Self(f)))
                    .map_err(|_| postcard::Error::DeserializeBadVarint)
            })
            .collect()
    }

    /// A pending entry: the delta's number, its object key and its full sealed
    /// frame. The frame is what a replay and the flusher both read back.
    pub fn pending_entry(
        seq: ambre_core::Seq,
        s3_key: &ambre_core::ObjectKey,
        wire: &Payload,
    ) -> Self {
        use base64::Engine;
        let entry = serde_json::json!({
            "seq": seq,
            "s3_key": s3_key,
            "update_b64": base64::engine::general_purpose::URL_SAFE.encode(&wire.0),
        });
        Self(entry.to_string().into_bytes())
    }

    /// The number, the object key and the frame of a pending entry, or `None`
    /// when it does not parse.
    pub fn read_pending(&self) -> Option<(ambre_core::Seq, ambre_core::ObjectKey, Payload)> {
        use base64::Engine;
        let entry: serde_json::Value = serde_json::from_slice(&self.0).ok()?;
        let s3_key = ambre_core::ObjectKey(entry.get("s3_key")?.as_str()?.to_owned());
        let frame = base64::engine::general_purpose::URL_SAFE
            .decode(entry.get("update_b64")?.as_str()?)
            .ok()?;
        let seq = entry
            .get("seq")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        Some((ambre_core::Seq(seq), s3_key, Self(frame)))
    }

    /// The number a pending entry names, when it names one, even if the rest
    /// does not parse.
    pub fn pending_seq(&self) -> Option<ambre_core::Seq> {
        let entry: serde_json::Value = serde_json::from_slice(&self.0).ok()?;
        entry.get("seq")?.as_u64().map(ambre_core::Seq)
    }
}

/// The socket's next binary frame. `None` once the socket closes or fails;
/// a frame of another kind is skipped.
pub async fn next_frame<S>(stream: &mut S) -> Option<Payload>
where
    S: Stream<Item = Result<Message, axum::Error>> + Unpin,
{
    loop {
        match stream.next().await? {
            Ok(Message::Binary(data)) => return Some(Payload(data.to_vec())),
            Ok(Message::Close(_)) | Err(_) => return None,
            Ok(_) => continue,
        }
    }
}

/// The socket's first binary frame, read before the socket is split.
pub async fn first_frame(socket: &mut WebSocket) -> Option<Payload> {
    next_frame(socket).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A batch reads back frame for frame, in the encoding the batches already
    /// stored use.
    #[test]
    fn a_batch_reads_back_its_frames() {
        let frames = vec![
            (ambre_core::Seq(3), Payload::new(vec![1, 2])),
            (ambre_core::Seq(4), Payload::new(vec![])),
        ];
        let batch = Payload::batch(&frames).unwrap();
        let stored: Vec<(i64, Vec<u8>)> = vec![(3, vec![1, 2]), (4, vec![])];
        assert_eq!(
            batch.expose_bytes(),
            postcard::to_allocvec(&stored).unwrap()
        );
        let read: Vec<(i64, Vec<u8>)> = batch
            .unbatch()
            .unwrap()
            .into_iter()
            .map(|(s, f)| (s.to_i64_saturating(), f.0))
            .collect();
        assert_eq!(read, stored);
    }

    /// A negative number, which no relay writes, refuses the batch.
    #[test]
    fn a_batch_with_a_negative_number_is_refused() {
        let stored: Vec<(i64, Vec<u8>)> = vec![(-1, vec![1])];
        let batch = Payload::new(postcard::to_allocvec(&stored).unwrap());
        assert!(batch.unbatch().is_err());
    }

    /// A pending entry reads back its number, its key and its frame.
    #[test]
    fn a_pending_entry_reads_back() {
        let entry = Payload::pending_entry(
            ambre_core::Seq(7),
            &ambre_core::ObjectKey::from("d/7"),
            &Payload::new(vec![9, 8]),
        );
        let (seq, key, frame) = entry.read_pending().unwrap();
        assert_eq!(
            (seq, key.as_str(), frame.expose_bytes()),
            (ambre_core::Seq(7), "d/7", &[9u8, 8][..])
        );
        assert_eq!(
            Payload::new(b"{\"seq\":4}".to_vec()).pending_seq(),
            Some(ambre_core::Seq(4))
        );
        assert!(Payload::new(b"not json".to_vec()).read_pending().is_none());
    }
}
