//! The call message of a channel
//! (`specs/5-Murmure/features/ambre-spec-murmure-calls.md`).
//!
//! A call message is a message whose `call` names a call. Its `call_end` map holds one entry
//! per member who wrote the end, keyed by that member, as `reactions` is: a member writes its
//! own key alone (`MURMURE-593`). The map comes empty with the message, so two members who end
//! a call at once write two keys of one map. Who is in the call is never written: the reader
//! asks the relay (`MURMURE-409`).

use ambre_core::crdt::CrdtName;
use ambre_core::id::{Call, Id, User};
use ambre_core::{Authored, Instant, Ms, TimeSpan};
use loro::{Container, ContainerID, Index, LoroDoc, LoroMap, ValueOrContainer};

use crate::message::{CallEnd, CallView};
use crate::schema::{self, Fields, MessageKey, Root};

/// The interval between two asks of who is in the live calls of a shown channel, the call
/// roster row of `ambre-spec-budgets.md section 1` (`MURMURE-595`).
pub const ROSTER_INTERVAL: TimeSpan<Ms> = TimeSpan::new(5_000);

/// The call a message names, with whether its end is written. `None` for a message that
/// names no call, or a call this build cannot read.
pub(crate) fn read(message: &Fields<MessageKey>) -> Option<CallView> {
    let call = Id::<Call>::parse_str(&schema::string(message, MessageKey::Call)?).ok()?;
    let written = end_map(message).is_some_and(|ends| !ends.is_empty());
    Some(CallView {
        call,
        end: if written {
            CallEnd::Written
        } else {
            CallEnd::Open
        },
        members: None,
    })
}

/// The `call_end` map of a call message, `None` when it holds none.
pub(crate) fn end_map(message: &Fields<MessageKey>) -> Option<LoroMap> {
    match message.get(MessageKey::CallEnd) {
        Some(ValueOrContainer::Container(Container::Map(ends))) => Some(ends),
        _ => None,
    }
}

/// Writes the end of `user` into `ends`, at `now`. `false` when that member wrote it
/// already, so nothing travels.
///
/// # Errors
/// A write Loro refuses.
pub(crate) fn write_end(
    ends: &LoroMap,
    user: Id<User>,
    now: Instant<Authored>,
) -> Result<bool, loro::LoroError> {
    let key = user.to_string();
    if ends.get(&key).is_some() {
        return Ok(false);
    }
    ends.insert(&key, schema::ts_value(now))?;
    Ok(true)
}

/// Whether the key `key` of a `call_end` map is the entry of `user`.
pub(crate) fn is_own_end(key: &str, user: Id<User>) -> bool {
    Id::<User>::parse_str(key).is_ok_and(|writer| writer == user && writer.to_string() == key)
}

/// Whether `id` is the `call_end` map of a message `doc` holds: the map under that key, in a
/// message of the `messages` list.
pub(crate) fn is_end_map(doc: &LoroDoc, id: &ContainerID) -> bool {
    let Some(path) = doc.get_path_to_container(id) else {
        return false;
    };
    match path.as_slice() {
        [(list, _), (_, Index::Seq(_)), (_, Index::Key(key))] => {
            MessageKey::parse(key) == Some(MessageKey::CallEnd)
                && matches!(list, ContainerID::Root { name, .. } if Root::parse(name) == Some(Root::Messages))
        }
        _ => false,
    }
}
