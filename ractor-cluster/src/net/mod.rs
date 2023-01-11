// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! TCP server and session actors which transmit [prost::Message] encoded messages

// TODO: we need a way to identify which session messages are coming from + going to. Therefore
// we should actually have a notification when a new session is launched, which can be used
// to match which session is tied to which actor id

use ractor::{ActorId, ActorRef, RpcReplyPort};

pub mod listener;
pub mod session;

/// Messages to/from the session aggregator
pub enum SessionManagerMessage {
    /// Notification when a new session is opened, and the handle to communicate with it
    /// Returns the actor which will be responsible for handling messages on this session
    ///
    /// Flags are a flag denoting if it comes from the TCP server socket and the RPC reply port
    OpenSession(bool, RpcReplyPort<ActorRef<crate::node::NodeSession>>),

    /// Notification when a session is closed, and the id of the actor to cleanup
    CloseSession(ActorId),
}

/// Message from the TCP [session::Session] actor and the
/// monitoring Sesson actor
pub enum SessionMessage {
    /// The Session actor is setting it's handle
    SetSession(ActorRef<session::Session>),

    /// A network message was received from the network
    MessageReceived(crate::protocol::NetworkMessage),
}

/// A trait which implements [prost::Message], [Default], and has a static lifetime
/// denoting protobuf-encoded messages which can be transmitted over the wire
pub trait NetworkMessage: prost::Message + Default + 'static {}
impl<T: prost::Message + Default + 'static> NetworkMessage for T {}

/// A network port
pub type NetworkPort = u16;
