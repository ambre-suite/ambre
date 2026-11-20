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

use std::collections::{BTreeMap, HashMap, HashSet};

use ambre_core::id::{Call, Channel, Id, User};
use ambre_core::{
    Authored, ExchangePublicKey, FixedBytes, Instant, KeyGeneration, Ms, PublicKey,
    SENDER_INDEX_LIMIT, SenderIndex, Seq, TimeSpan, UnixMs,
};
use ambre_crypto::call::{
    CallExchangeKey, Departures, JoinClaim, KeyHandoff, SenderKey, SignedJoin, WrapBinding,
    WrappedSenderKey, unwrap_sender_key, wrap_sender_key,
};
use ed25519_dalek::SigningKey;

use crate::membership_log::Log;

/// The delay between a leave and the fresh key in use. Late wraps arrive within it. The
/// receiver closes the older epochs on the same figure, so `ambre-crypto` holds it.
pub(crate) use ambre_crypto::call::SWITCH_DELAY;

/// How far the clock of a later joiner may lag behind the clock of this participant: the
/// join clock skew row of `ambre-spec-budgets.md section 8`. Past it, a join that arrives
/// after this participant's own is older than its own, and is refused (`CALL-124`).
pub(crate) const JOIN_CLOCK_SKEW: TimeSpan<Ms> = TimeSpan::new(60_000);

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
    /// The join arrives after this participant's own, but is older than it past the clock
    /// skew. A relay replays a join of an earlier session (`CALL-124`).
    Stale,
    /// The wrap comes from no participant this client admitted, or does not open.
    UnknownSender,
    /// The sender index or the generation is past the width of a key identifier.
    Exhausted,
    /// The wrap carries a key its sender drew before a removal, and the switch delay of that
    /// removal passed. A removed session holds that key (`CALL-157`).
    Retired,
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

/// One generation of the sender key of this participant, and the sessions it had departed
/// when it drew the key of that epoch (`CALL-157`).
#[derive(Clone)]
struct Held {
    generation: Seq<KeyGeneration>,
    key: SenderKey,
    departures: Departures,
}

impl Held {
    fn ratcheted(&self) -> Option<Self> {
        Some(Self {
            generation: self.generation.ratcheted()?,
            key: self.key.ratcheted(),
            departures: self.departures.clone(),
        })
    }
}

/// A removal this client heeded: the sessions it departed, the participants it still held,
/// and the end of its switch delay.
struct RemovalMark {
    sessions: Vec<Seq<SenderIndex>>,
    /// The participants this client held at the removal. A later joiner drew its keys after
    /// this client heeded the removal, and its own log chose whom it wraps to: the mark
    /// spares it.
    held: HashSet<Seq<SenderIndex>>,
    at: Instant<UnixMs>,
    /// For each sender, the oldest epoch of a key drawn after the removal. Every epoch of that
    /// sender from it up was drawn after, since the departures of a sender only grow.
    cleared: HashMap<Seq<SenderIndex>, Seq<KeyGeneration>>,
}

impl RemovalMark {
    /// Whether `sender` drew the key of `epoch`, which carries `departures`, after this
    /// removal. A key that names a removed session as departed was never wrapped to one.
    fn clears(
        &mut self,
        sender: Seq<SenderIndex>,
        epoch: Seq<KeyGeneration>,
        departures: &Departures,
    ) -> bool {
        if !self.held.contains(&sender) {
            return true;
        }
        if departures.names_any(&self.sessions) {
            let oldest = self.cleared.entry(sender).or_insert(epoch);
            *oldest = (*oldest).min(epoch);
            return true;
        }
        self.cleared.get(&sender).is_some_and(|&from| from <= epoch)
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
    /// The departures each epoch of its key carried, by the first generation of the epoch.
    epochs: BTreeMap<Seq<KeyGeneration>, Departures>,
}

/// The sender keys of this participant in one call, and what it knows of the others.
pub(crate) struct CallKeys {
    call: Id<Call>,
    channel: Id<Channel>,
    own: Seq<SenderIndex>,
    exchange: CallExchangeKey,
    join: SignedJoin,
    /// The time this session signed its join, on its own clock.
    joined: Instant<Authored>,
    current: Held,
    pending: Option<Pending>,
    peers: HashMap<Seq<SenderIndex>, Peer>,
    /// Every session this participant departed, the newest kept. A key it draws carries them.
    departed: Departures,
    /// Every removal this client heeded in the call (`CALL-157`).
    removals: Vec<RemovalMark>,
    /// Every exchange key, under the identity that signed it, and every sender index of a
    /// join this client admitted. A refused join marks nothing, so a join that claims the
    /// key or the index of another and fails a check evicts no one (`CALL-133`).
    seen_keys: HashSet<(FixedBytes<PublicKey>, FixedBytes<ExchangePublicKey>)>,
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
        let own_identity = FixedBytes::from_array(identity.verifying_key().to_bytes());
        let claim = JoinClaim {
            call: joining.call,
            channel: joining.channel,
            member: joining.member,
            identity: own_identity,
            at: joining.at,
            sender: joining.sender,
            exchange: exchange.public(),
        };
        let join = SignedJoin::sign(identity, claim).map_err(|_| Refusal::Signature)?;
        let current = Held {
            generation: Seq::ZERO,
            key: SenderKey::generate(),
            departures: Departures::default(),
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
            seen_keys: HashSet::from([(own_identity, exchange.public())]),
            seen_senders: HashSet::from([joining.sender]),
            exchange,
            join,
            joined: joining.at,
            current,
            pending: None,
            peers: HashMap::new(),
            departed: Departures::default(),
            removals: Vec::new(),
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
    /// channel and its freshness, and the time of a new joiner. Then wraps this
    /// participant's keys to the joiner alone. A joiner that `arrival` names new first
    /// moves this participant one generation up.
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
        let seen = (claim.identity, claim.exchange);
        if self.seen_keys.contains(&seen) || self.seen_senders.contains(&claim.sender) {
            return Err(Refusal::Replayed);
        }
        if log.present_key(claim.member) != Some(claim.identity) {
            return Err(Refusal::NotAMember);
        }
        // A joiner the roster names was in the call before this participant, so its join is
        // older by right. A new joiner signed after this participant did.
        if arrival == Arrival::New && self.joined.since(claim.at) > JOIN_CLOCK_SKEW {
            return Err(Refusal::Stale);
        }
        // Only an admitted join marks its key and its index (`CALL-133`).
        self.seen_keys.insert(seen);
        self.seen_senders.insert(claim.sender);
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
                epochs: BTreeMap::new(),
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

    /// Opens a wrap a participant this client admitted made for it, at `now`.
    ///
    /// A key the sender drew before a removal this client heeded reached a removed session.
    /// Inside the switch delay of that removal, the handoff to open it comes with a close at
    /// the end of the delay. Past it, the wrap is refused (`CALL-157`).
    ///
    /// # Errors
    ///
    /// [`Refusal::UnknownSender`] for a wrap from no admitted participant, or one that
    /// does not open, and [`Refusal::Retired`] for a key a removal closed.
    pub(crate) fn receive(
        &mut self,
        from: Seq<SenderIndex>,
        wrapped: &WrappedSenderKey,
        now: Instant<UnixMs>,
    ) -> Result<Vec<KeyHandoff>, Refusal> {
        let peer = self.peers.get_mut(&from).ok_or(Refusal::UnknownSender)?;
        let binding = WrapBinding {
            call: self.call,
            sender: from,
            recipient: self.own,
        };
        let opened = unwrap_sender_key(&self.exchange, &peer.exchange, binding, wrapped)
            .map_err(|_| Refusal::UnknownSender)?;
        let epoch = opened.generation.epoch_start();
        let mut within: Option<TimeSpan<Ms>> = None;
        for mark in &mut self.removals {
            if mark.clears(from, epoch, &opened.departures) {
                continue;
            }
            if now >= mark.at {
                return Err(Refusal::Retired);
            }
            let left = mark.at.since(now);
            within = Some(within.map_or(left, |w| w.min(left)));
        }
        peer.epochs.insert(epoch, opened.departures);
        let mut handoffs = vec![KeyHandoff::Open {
            sender: from,
            generation: opened.generation,
            key: opened.key,
        }];
        if let Some(within) = within {
            handoffs.push(KeyHandoff::Close {
                sender: from,
                generation: opened.generation,
                within,
            });
        }
        Ok(handoffs)
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
        self.depart_all(&[sender], now)
    }

    /// The sessions `senders` left at `now`, all at once: forgets each, then draws one fresh
    /// key and wraps it to each remaining participant alone. No fresh key is wrapped while one
    /// of `senders` is still held, so none reaches a second session of a removed member
    /// (`CALL-158`).
    fn depart_all(
        &mut self,
        senders: &[Seq<SenderIndex>],
        now: Instant<UnixMs>,
    ) -> Result<Moves, Refusal> {
        let mut moves = Moves::default();
        for &sender in senders {
            if self.peers.remove(&sender).is_some() {
                self.departed.depart(sender);
                moves.handoffs.push(KeyHandoff::Forget { sender });
            }
        }
        if moves.handoffs.is_empty() {
            return Ok(moves);
        }
        let top = self
            .pending
            .as_ref()
            .map_or(self.current.generation, |p| p.held.generation);
        let held = Held {
            generation: top.next_epoch().ok_or(Refusal::Exhausted)?,
            key: SenderKey::generate(),
            departures: self.departed.clone(),
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

    /// Reads a new state of the membership log of the channel. Every session of a member the
    /// log no longer holds departs at `now`, all at once (`CALL-028`, `CALL-158`).
    ///
    /// Each key a remaining participant drew before the removal reached a removed session.
    /// So each one closes once the switch delay passed, whether the fresh key of that
    /// participant arrives or not (`CALL-157`).
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
        if removed.is_empty() {
            return Ok(Moves::default());
        }
        let mut moves = self.depart_all(&removed, now)?;
        let mut mark = RemovalMark {
            sessions: removed,
            held: self.peers.keys().copied().collect(),
            at: now.plus(SWITCH_DELAY),
            cleared: HashMap::new(),
        };
        for (&index, peer) in &self.peers {
            let mut before = None;
            for (&epoch, departures) in &peer.epochs {
                if !mark.clears(index, epoch, departures) {
                    before = Some(epoch);
                }
            }
            if let Some(epoch) = before {
                // The close names the epoch: each step of it the receiver ratchets to closes
                // with it.
                moves.handoffs.push(KeyHandoff::Close {
                    sender: index,
                    generation: epoch,
                    within: SWITCH_DELAY,
                });
            }
        }
        self.removals.push(mark);
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
            &held.departures,
            &held.key,
        )
        .map_err(|_| Refusal::UnknownSender)?;
        Ok(OutgoingWrap { to: index, wrapped })
    }
}

#[cfg(test)]
#[path = "call_tests.rs"]
mod tests;
