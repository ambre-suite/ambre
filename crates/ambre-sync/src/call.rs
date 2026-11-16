//! The sender keys of one call, as the Sync worker keeps them
//! (`ambre-spec-call-media.md section 3`, the join and the leave).
//!
//! The Sync worker holds the identity of the member and the membership log of the
//! channel. It checks each joiner against that log, and wraps its sender key to each
//! participant alone. It switches to a fresh key when a participant leaves or the log
//! removes one. The call worker receives the symmetric keys alone, as handoffs, and seals
//! and opens the chunks.
//!
//! Nothing here sends a byte: the caller carries each wrap to the relay route, and each
//! handoff to the call worker.

use std::collections::{HashMap, HashSet};

use ambre_core::id::{Call, Channel, Id, User};
use ambre_core::{
    Authored, ExchangePublicKey, FixedBytes, Instant, KeyGeneration, Ms, PublicKey,
    SENDER_INDEX_LIMIT, SenderIndex, Seq, TimeSpan, UnixMs,
};
use ambre_crypto::call::{
    CallExchangeKey, JoinClaim, KeyHandoff, SenderKey, SignedJoin, WrapBinding, WrappedSenderKey,
    unwrap_sender_key, wrap_sender_key,
};
use ed25519_dalek::SigningKey;

use crate::membership_log::Log;

/// The delay between a leave and the fresh key in use: the target of the call sender key
/// switch row of `ambre-spec-budgets.md section 1`. Late wraps arrive within it.
pub(crate) const SWITCH_DELAY: TimeSpan<Ms> = TimeSpan::new(5_000);

/// How a joiner meets this participant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Arrival {
    /// The joiner was in the call before this participant joined: this participant
    /// learns of it from the roster, and moves no key.
    Present,
    /// The joiner arrives after this participant: this participant moves its key one
    /// generation up before it wraps (`CALL-017`).
    New,
}

/// Why a join or a wrap was refused. No key is wrapped for a refused joiner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// The signature does not verify under the identity the join names.
    Signature,
    /// The join names another call or another channel.
    OtherCall,
    /// The membership log of the channel does not hold the joiner under that identity
    /// (`CALL-019`).
    NotAMember,
    /// The client saw this exchange key or this sender index already (`CALL-020`).
    Replayed,
    /// The wrap comes from no participant this client admitted, or does not open.
    UnknownSender,
    /// The sender index or the generation is past the width of a key identifier.
    Exhausted,
}

/// A wrap for one participant, which the relay route forwards.
#[derive(Debug)]
pub(crate) struct OutgoingWrap {
    /// The participant the wrap is for.
    pub(crate) to: Seq<SenderIndex>,
    /// The wrapped sender key.
    pub(crate) wrapped: WrappedSenderKey,
}

/// What a change of the call hands out: the wraps to forward, the handoffs to the call
/// worker.
#[derive(Debug, Default)]
pub(crate) struct Moves {
    pub(crate) wraps: Vec<OutgoingWrap>,
    pub(crate) handoffs: Vec<KeyHandoff>,
}

/// One generation of the sender key of this participant.
#[derive(Clone)]
struct Held {
    generation: Seq<KeyGeneration>,
    key: SenderKey,
}

impl Held {
    fn ratcheted(&self) -> Option<Self> {
        Some(Self {
            generation: self.generation.ratcheted()?,
            key: self.key.ratcheted(),
        })
    }
}

/// A fresh key that waits for the switch delay before it seals.
struct Pending {
    held: Held,
    at: Instant<UnixMs>,
}

/// A participant this client admitted.
struct Peer {
    member: Id<User>,
    identity: FixedBytes<PublicKey>,
    exchange: FixedBytes<ExchangePublicKey>,
}

/// The sender keys of this participant in one call, and what it knows of the others.
pub(crate) struct CallKeys {
    call: Id<Call>,
    channel: Id<Channel>,
    own: Seq<SenderIndex>,
    exchange: CallExchangeKey,
    join: SignedJoin,
    current: Held,
    pending: Option<Pending>,
    peers: HashMap<Seq<SenderIndex>, Peer>,
    /// Every exchange key and sender index a join of this call carried, refused or not.
    seen_keys: HashSet<FixedBytes<ExchangePublicKey>>,
    seen_senders: HashSet<Seq<SenderIndex>>,
}

/// Who joins, where and when: the fields of the join this client signs.
pub(crate) struct Joining {
    pub(crate) call: Id<Call>,
    pub(crate) channel: Id<Channel>,
    pub(crate) member: Id<User>,
    pub(crate) sender: Seq<SenderIndex>,
    pub(crate) at: Instant<Authored>,
}

impl CallKeys {
    /// Joins a call: draws the exchange key and the first sender key of this session, and
    /// signs the join with `identity`.
    ///
    /// # Errors
    ///
    /// [`Refusal::Exhausted`] for a sender index past the width of a key identifier.
    pub(crate) fn start(
        identity: &SigningKey,
        joining: Joining,
    ) -> Result<(Self, KeyHandoff), Refusal> {
        if joining.sender >= SENDER_INDEX_LIMIT {
            return Err(Refusal::Exhausted);
        }
        let exchange = CallExchangeKey::generate();
        let claim = JoinClaim {
            call: joining.call,
            channel: joining.channel,
            member: joining.member,
            identity: FixedBytes::from_array(identity.verifying_key().to_bytes()),
            at: joining.at,
            sender: joining.sender,
            exchange: exchange.public(),
        };
        let join = SignedJoin::sign(identity, claim).map_err(|_| Refusal::Signature)?;
        let current = Held {
            generation: Seq::ZERO,
            key: SenderKey::generate(),
        };
        let handoff = KeyHandoff::Seal {
            sender: joining.sender,
            generation: current.generation,
            key: current.key.clone(),
        };
        let keys = Self {
            call: joining.call,
            channel: joining.channel,
            own: joining.sender,
            seen_keys: HashSet::from([exchange.public()]),
            seen_senders: HashSet::from([joining.sender]),
            exchange,
            join,
            current,
            pending: None,
            peers: HashMap::new(),
        };
        Ok((keys, handoff))
    }

    /// The signed join of this session, which the relay route hands each participant.
    pub(crate) const fn join(&self) -> &SignedJoin {
        &self.join
    }

    /// The sender index of this session.
    pub(crate) const fn own(&self) -> Seq<SenderIndex> {
        self.own
    }

    /// Admits a joiner: checks its signature, its call, the membership log of the
    /// channel and its freshness, then wraps this participant's keys to it alone. A
    /// joiner that `arrival` names new first moves this participant one generation up.
    ///
    /// # Errors
    ///
    /// Each [`Refusal`] of a join. A refused joiner gets no wrap and changes no key.
    pub(crate) fn admit(
        &mut self,
        join: &SignedJoin,
        log: &Log,
        arrival: Arrival,
    ) -> Result<Moves, Refusal> {
        let claim = join.verified().map_err(|_| Refusal::Signature)?;
        if claim.call != self.call || claim.channel != self.channel {
            return Err(Refusal::OtherCall);
        }
        let fresh_key = !self.seen_keys.contains(&claim.exchange);
        let fresh_sender = !self.seen_senders.contains(&claim.sender);
        self.seen_keys.insert(claim.exchange);
        self.seen_senders.insert(claim.sender);
        if !(fresh_key && fresh_sender) {
            return Err(Refusal::Replayed);
        }
        if log.present_key(claim.member) != Some(claim.identity) {
            return Err(Refusal::NotAMember);
        }
        let mut moves = Moves::default();
        if arrival == Arrival::New {
            let current = self.current.ratcheted().ok_or(Refusal::Exhausted)?;
            let pending = match &self.pending {
                Some(pending) => Some(Pending {
                    held: pending.held.ratcheted().ok_or(Refusal::Exhausted)?,
                    at: pending.at,
                }),
                None => None,
            };
            self.current = current;
            self.pending = pending;
            moves.handoffs.push(KeyHandoff::Seal {
                sender: self.own,
                generation: self.current.generation,
                key: self.current.key.clone(),
            });
        }
        self.peers.insert(
            claim.sender,
            Peer {
                member: claim.member,
                identity: claim.identity,
                exchange: claim.exchange,
            },
        );
        let held = std::iter::once(&self.current).chain(self.pending.as_ref().map(|p| &p.held));
        for held in held {
            moves
                .wraps
                .push(self.wrap_to(claim.sender, &claim.exchange, held)?);
        }
        Ok(moves)
    }

    /// Opens a wrap a participant this client admitted made for it.
    ///
    /// # Errors
    ///
    /// [`Refusal::UnknownSender`] for a wrap from no admitted participant, or one that
    /// does not open.
    pub(crate) fn receive(
        &self,
        from: Seq<SenderIndex>,
        wrapped: &WrappedSenderKey,
    ) -> Result<KeyHandoff, Refusal> {
        let peer = self.peers.get(&from).ok_or(Refusal::UnknownSender)?;
        let binding = WrapBinding {
            call: self.call,
            sender: from,
            recipient: self.own,
        };
        let (generation, key) = unwrap_sender_key(&self.exchange, &peer.exchange, binding, wrapped)
            .map_err(|_| Refusal::UnknownSender)?;
        Ok(KeyHandoff::Open {
            sender: from,
            generation,
            key,
        })
    }

    /// A participant left at `now`: forgets it, draws a fresh key, and wraps that key to
    /// each remaining participant alone (`CALL-030`). The fresh key seals once the switch
    /// delay of the first leave since the last switch has passed (`CALL-027`).
    ///
    /// # Errors
    ///
    /// [`Refusal::Exhausted`] past the last epoch, and [`Refusal::UnknownSender`] when a
    /// wrap fails.
    pub(crate) fn depart(
        &mut self,
        sender: Seq<SenderIndex>,
        now: Instant<UnixMs>,
    ) -> Result<Moves, Refusal> {
        let mut moves = Moves::default();
        if self.peers.remove(&sender).is_none() {
            return Ok(moves);
        }
        moves.handoffs.push(KeyHandoff::Forget { sender });
        let top = self
            .pending
            .as_ref()
            .map_or(self.current.generation, |p| p.held.generation);
        let held = Held {
            generation: top.next_epoch().ok_or(Refusal::Exhausted)?,
            key: SenderKey::generate(),
        };
        let at = self
            .pending
            .as_ref()
            .map_or(now.plus(SWITCH_DELAY), |p| p.at);
        let remaining: Vec<_> = self
            .peers
            .iter()
            .map(|(&index, peer)| (index, peer.exchange))
            .collect();
        for (index, exchange) in remaining {
            moves.wraps.push(self.wrap_to(index, &exchange, &held)?);
        }
        self.pending = Some(Pending { held, at });
        Ok(moves)
    }

    /// Reads a new state of the membership log of the channel: each participant it no
    /// longer holds under its identity departs at `now` (`CALL-028`).
    ///
    /// # Errors
    ///
    /// Each error of [`Self::depart`].
    pub(crate) fn heed(&mut self, log: &Log, now: Instant<UnixMs>) -> Result<Moves, Refusal> {
        let removed: Vec<_> = self
            .peers
            .iter()
            .filter(|(_, peer)| log.present_key(peer.member) != Some(peer.identity))
            .map(|(&index, _)| index)
            .collect();
        let mut moves = Moves::default();
        for index in removed {
            let more = self.depart(index, now)?;
            moves.wraps.extend(more.wraps);
            moves.handoffs.extend(more.handoffs);
        }
        Ok(moves)
    }

    /// Whether a fresh key waits for its switch.
    pub(crate) fn switch_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Switches to the fresh key once its delay has passed at `now`.
    pub(crate) fn tick(&mut self, now: Instant<UnixMs>) -> Option<KeyHandoff> {
        let due = self.pending.as_ref().is_some_and(|p| p.at <= now);
        let pending = self.pending.take_if(|_| due)?;
        self.current = pending.held;
        Some(KeyHandoff::Seal {
            sender: self.own,
            generation: self.current.generation,
            key: self.current.key.clone(),
        })
    }

    /// Wraps one held key to the participant at `index`.
    fn wrap_to(
        &self,
        index: Seq<SenderIndex>,
        exchange: &FixedBytes<ExchangePublicKey>,
        held: &Held,
    ) -> Result<OutgoingWrap, Refusal> {
        let binding = WrapBinding {
            call: self.call,
            sender: self.own,
            recipient: index,
        };
        let wrapped = wrap_sender_key(
            &self.exchange,
            exchange,
            binding,
            held.generation,
            &held.key,
        )
        .map_err(|_| Refusal::UnknownSender)?;
        Ok(OutgoingWrap { to: index, wrapped })
    }
}

#[cfg(test)]
#[path = "call_tests.rs"]
mod tests;
