//! The channel plane: the second type parameter of the two UI to Logic
//! envelopes (`BUS-027`, `BUS-032`).
//!
//! Murmure's content rides envelope variants of its own, addressed to a
//! channel or a space rather than a document. Those variants name its command
//! and event types, which live in `ambre-murmure`. So the envelopes take them
//! through a [`ChannelPlane`], whose associated types Murmure's marker fixes.
//! Every other sender and receiver carries [`NoChannels`], where each type is
//! uninhabited. [`PlaneOf`] is how the launcher's plane embeds a view's, as
//! [`crate::Embed`] does for the document command.

use std::convert::Infallible;

use crate::{Embed, Mutation};

/// The channel types an envelope carries, one per variant that names them.
pub trait ChannelPlane {
    /// What `LogicCommand::Channel` carries to an open channel. The lossy
    /// class asks it the questions it asks a document command (`BUS-023`).
    type Command: Mutation;
    /// What `LogicCommand::Space` carries to an open space.
    type SpaceCommand;
    /// The direct message `LogicCommand::OpenDirectMessage` asks for.
    type DirectDraft;
    /// What `UiEvent::Channel` carries from an open channel.
    type Event;
    /// What `UiEvent::Space` carries from an open space.
    type SpaceEvent;
    /// The outcome `UiEvent::DirectMessage` carries.
    type DirectEvent;
    /// One channel's matches in `UiEvent::MessageSearch`.
    type Matches;
}

/// The plane of a sender or receiver that carries no channel: every type is
/// uninhabited, so its channel variants hold no value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoChannels {}

impl ChannelPlane for NoChannels {
    type Command = Infallible;
    type SpaceCommand = Infallible;
    type DirectDraft = Infallible;
    type Event = Infallible;
    type SpaceEvent = Infallible;
    type DirectEvent = Infallible;
    type Matches = Infallible;
}

/// A plane whose every channel type embeds the one of `P`.
///
/// The launcher's plane is `PlaneOf` each view's: Murmure's of its own, and
/// of [`NoChannels`] through the uninhabited embeds. A view's channel variant
/// enters the launcher's queue unchanged, and a channel event leaves it only
/// for a view whose plane holds it.
pub trait PlaneOf<P: ChannelPlane>: ChannelPlane {
    fn embed_command(command: P::Command) -> Self::Command;
    fn embed_space_command(command: P::SpaceCommand) -> Self::SpaceCommand;
    fn embed_draft(draft: P::DirectDraft) -> Self::DirectDraft;
    fn project_command(command: Self::Command) -> Option<P::Command>;
    fn project_space_command(command: Self::SpaceCommand) -> Option<P::SpaceCommand>;
    fn project_draft(draft: Self::DirectDraft) -> Option<P::DirectDraft>;
    fn project_event(event: Self::Event) -> Option<P::Event>;
    fn project_space_event(event: Self::SpaceEvent) -> Option<P::SpaceEvent>;
    fn project_direct_event(event: Self::DirectEvent) -> Option<P::DirectEvent>;
    fn project_matches(matches: Self::Matches) -> Option<P::Matches>;
}

impl<P, Q> PlaneOf<P> for Q
where
    P: ChannelPlane,
    Q: ChannelPlane,
    Q::Command: Embed<P::Command>,
    Q::SpaceCommand: Embed<P::SpaceCommand>,
    Q::DirectDraft: Embed<P::DirectDraft>,
    Q::Event: Embed<P::Event>,
    Q::SpaceEvent: Embed<P::SpaceEvent>,
    Q::DirectEvent: Embed<P::DirectEvent>,
    Q::Matches: Embed<P::Matches>,
{
    fn embed_command(command: P::Command) -> Q::Command {
        Embed::embed(command)
    }

    fn embed_space_command(command: P::SpaceCommand) -> Q::SpaceCommand {
        Embed::embed(command)
    }

    fn embed_draft(draft: P::DirectDraft) -> Q::DirectDraft {
        Embed::embed(draft)
    }

    fn project_command(command: Q::Command) -> Option<P::Command> {
        command.project().ok()
    }

    fn project_space_command(command: Q::SpaceCommand) -> Option<P::SpaceCommand> {
        command.project().ok()
    }

    fn project_draft(draft: Q::DirectDraft) -> Option<P::DirectDraft> {
        draft.project().ok()
    }

    fn project_event(event: Q::Event) -> Option<P::Event> {
        event.project().ok()
    }

    fn project_space_event(event: Q::SpaceEvent) -> Option<P::SpaceEvent> {
        event.project().ok()
    }

    fn project_direct_event(event: Q::DirectEvent) -> Option<P::DirectEvent> {
        event.project().ok()
    }

    fn project_matches(matches: Q::Matches) -> Option<P::Matches> {
        matches.project().ok()
    }
}

/// `Embed<Infallible>` for each listed type: nothing embeds into it from an
/// uninhabited part, and nothing ever projects out.
///
/// The orphan rule puts these impls in the crate of the type. So a sum or a
/// channel type downstream of the bus writes its own with this macro.
#[macro_export]
macro_rules! embed_nothing {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl $crate::Embed<::std::convert::Infallible> for $ty {
                fn embed(part: ::std::convert::Infallible) -> Self {
                    match part {}
                }

                fn project(self) -> Result<::std::convert::Infallible, Self> {
                    Err(self)
                }

                fn peek(&self) -> Option<&::std::convert::Infallible> {
                    None
                }
            }
        )+
    };
}
