// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Erlang `node()` host communication for managing remote actor communication in
//! a cluster

/*
TODO:

Overview:

A `NodeServer` handles opening the TCP listener and managing incoming and outgoing `NodeSession` requests. `NodeSession`s
will represent a remote server locally.

We need to:
1. Authenticate the remote `node()` and determine if it's a valid node which can be connected to
2. Define and implement the remote node protocol
    1. Cast
    2. Call
3. Adjustments in the default Message type which allow messages to be serializable. (with `cluster` feature enabled)
4. Spawn all of the remote actors locally as "Remote Actors" (see actor_id.rs in `ractor`), and any requests to these
local representitive actors will be passed over the network to the remote actor.
5. Registry and pg groups need to be sync'd with local & remote actors across all nodes.

Additionally, you can open a session as a "client" by requesting a new session from the  NodeServer
after intially connecting a TcpStream to the desired endpoint and then attaching the NodeSession
to the TcpStream (and linking the actor).

What's there to do?
1. The client-connection behavior does not yet exist

*/

pub mod auth;

use std::collections::HashMap;

use ractor::{Actor, ActorId, ActorRef};

use crate::protocol::auth as auth_protocol;
use crate::protocol::node as node_protocol;

const DEFAULT_PORT: crate::net::NetworkPort = 1230;

/// Represents the server which is managing all incoming node session requests
///
/// The [NodeServer] supervises a single [crate::net::listener::Listener] actor which is
/// responsible for hosting a server port for incoming `node()` connections. It also supervises
/// all of the [NodeSession] actors which are tied to tcp sessions and manage the FSM around `node()`s
/// establishing inter connections.
pub struct NodeServer {
    maybe_port: Option<crate::net::NetworkPort>,
}

/// The state of the node server
pub struct NodeServerState {
    _listener: ActorRef<crate::net::listener::Listener>,
    node_sessions: HashMap<ActorId, ActorRef<NodeSession>>,
    node_id_counter: u64,
}

#[async_trait::async_trait]
impl Actor for NodeServer {
    type Msg = crate::net::SessionManagerMessage;
    type State = NodeServerState;
    async fn pre_start(&self, myself: ActorRef<Self>) -> Self::State {
        let listener = crate::net::listener::Listener::new(
            self.maybe_port.unwrap_or(DEFAULT_PORT),
            myself.clone(),
        );

        let (actor_ref, _) = Actor::spawn_linked(None, listener, myself.get_cell())
            .await
            .expect("Failed to start listener");

        Self::State {
            node_sessions: HashMap::new(),
            _listener: actor_ref,
            node_id_counter: 0,
        }
    }

    async fn handle(&self, myself: ActorRef<Self>, message: Self::Msg, state: &mut Self::State) {
        match message {
            Self::Msg::OpenSession(is_server, reply) => {
                let id = state.node_id_counter;
                if let Ok((actor, _)) = Actor::spawn_linked(
                    None,
                    NodeSession {
                        node_id: id,
                        is_server,
                    },
                    myself.get_cell(),
                )
                .await
                {
                    state.node_id_counter += 1;
                    let _ = reply.send(actor);
                } else {
                    // let the timeout take care of the child process and shut the channel down
                }
            }
            Self::Msg::CloseSession(id) => {
                // remove the stale session
                if let Some(session) = state.node_sessions.remove(&id) {
                    // session closed
                    session.stop(None);
                }
            }
        }
    }

    // TODO: fill out the supervision logic
}

enum AuthenticationState {
    Client(auth::ClientAuthenticationProcess),
    Server(auth::ServerAuthenticationProcess),
}

impl AuthenticationState {
    fn is_ok(&self) -> bool {
        match self {
            Self::Client(c) => matches!(c, auth::ClientAuthenticationProcess::Ok),
            Self::Server(s) => matches!(s, auth::ServerAuthenticationProcess::Ok(_)),
        }
    }

    fn is_close(&self) -> bool {
        match self {
            Self::Client(c) => matches!(c, auth::ClientAuthenticationProcess::Close),
            Self::Server(s) => matches!(s, auth::ServerAuthenticationProcess::Close),
        }
    }
}

/// Represents a session with a specific node
pub struct NodeSession {
    node_id: u64,
    is_server: bool,
}

/// The state of the node session
pub struct NodeSessionState {
    tcp: Option<ActorRef<crate::net::session::Session>>,
    name: Option<auth_protocol::NameMessage>,
    auth: AuthenticationState,
}

impl NodeSessionState {
    fn handle_auth(
        &mut self,
        message: auth_protocol::AuthenticationMessage,
        myself: ActorRef<NodeSession>,
    ) {
        if self.auth.is_ok() {
            // nothing to do, we're already authenticated
            return;
        }
        if self.auth.is_close() {
            // we need to shutdown, the session needs to be terminated
            myself.stop(Some("auth_fail".to_string()));
            if let Some(tcp) = &self.tcp {
                tcp.stop(Some("auth_fail".to_string()));
            }
        }

        match &self.auth {
            AuthenticationState::Client(client_auth) => {
                // TODO:
            }
            AuthenticationState::Server(server_auth) => {
                // TODO:
            }
        }
    }

    fn handle_node(&mut self, _message: node_protocol::NodeMessage, myself: ActorRef<NodeSession>) {
    }
}

#[async_trait::async_trait]
impl Actor for NodeSession {
    type Msg = crate::net::SessionMessage;
    type State = NodeSessionState;
    async fn pre_start(&self, _myself: ActorRef<Self>) -> Self::State {
        Self::State {
            tcp: None,
            name: None,
            auth: if self.is_server {
                AuthenticationState::Server(auth::ServerAuthenticationProcess::init())
            } else {
                AuthenticationState::Client(auth::ClientAuthenticationProcess::init())
            },
        }
    }

    async fn handle(&self, myself: ActorRef<Self>, message: Self::Msg, state: &mut Self::State) {
        match message {
            Self::Msg::SetSession(session) if state.tcp.is_none() => {
                state.tcp = Some(session);
            }
            Self::Msg::MessageReceived(maybe_network_message) if state.tcp.is_some() => {
                if let Some(network_message) = maybe_network_message.message {
                    match network_message {
                        crate::protocol::meta::network_message::Message::Auth(auth_message) => {
                            state.handle_auth(auth_message, myself);
                        }
                        crate::protocol::meta::network_message::Message::Node(node_message) => {
                            state.handle_node(node_message, myself);
                        }
                    }
                }
            }
            _ => {
                // no-op, ignore
            }
        }
    }
}

// /// A node server is run by every `node()` process to accept inter-node connection requests
// pub type NodeServer = crate::net::listener::Listener<crate::protocol::NetworkMessage>;
// /// A node session in an active open connection with a peer `node()`
// pub type NodeSession = crate::net::session::Session<crate::protocol::NetworkMessage>;
