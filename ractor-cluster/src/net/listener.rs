// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! TCP Server to accept incoming sessions

use std::collections::hash_map::Entry;
use std::collections::HashMap;

use ractor::rpc::CallResult;
use ractor::{Actor, ActorId, ActorRef, SupervisionEvent};
use tokio::net::TcpListener;
use tokio::time::Duration;

/// A Tcp Socket [Listener] responsible for accepting new connections and spawning [super::session::Session]s
/// which handle the message sending and receiving over the socket.
///
/// The [Listener] supervises all of the TCP [super::session::Session] actors and is responsible for logging
/// connects and disconnects as well as tracking the current open [super::session::Session] actors.
pub struct Listener {
    port: super::NetworkPort,
    session_manager: ActorRef<crate::node::NodeServer>,
}

impl Listener {
    /// Create a new `Listener`
    pub fn new(
        port: super::NetworkPort,
        session_manager: ActorRef<crate::node::NodeServer>,
    ) -> Self {
        Self {
            port,
            session_manager,
        }
    }
}

/// The Node listener's state
pub struct ListenerState {
    listener: Option<TcpListener>,
    nodes: HashMap<ActorId, std::net::SocketAddr>,
}

#[async_trait::async_trait]
impl Actor for Listener {
    type Msg = ();

    type State = ListenerState;

    async fn pre_start(&self, myself: ActorRef<Self>) -> Self::State {
        let addr = format!("0.0.0.0:{}", self.port);
        let listener = match TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(err) => {
                panic!("Error listening to socket: {}", err);
            }
        };

        // startup the event processing loop by sending an initial msg
        let _ = myself.cast(());

        // create the initial state
        Self::State {
            listener: Some(listener),
            nodes: HashMap::new(),
        }
    }

    async fn post_stop(&self, _myself: ActorRef<Self>, state: &mut Self::State) {
        // close the listener properly, in case anyone else has handles to the actor stopping
        // total droppage
        drop(state.listener.take());
    }

    async fn handle(&self, myself: ActorRef<Self>, _message: Self::Msg, state: &mut Self::State) {
        if let Some(listener) = &mut state.listener {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    // ask the session manager for a new session agent
                    let session = self
                        .session_manager
                        .call(
                            |tx| super::SessionManagerMessage::OpenSession(true, tx),
                            Some(Duration::from_millis(500)),
                        )
                        .await
                        .unwrap();
                    match session {
                        CallResult::Timeout => {
                            log::warn!("Timeout in trying to open session. Failed to retrieve a new Session handler from the SessionManager in {} ms. Refusing connection", 500);
                        }
                        CallResult::SenderError => {
                            log::error!("Sender error when trying to receive session handler");
                            myself.stop(Some("Session handler retrieval failure".to_string()));
                        }
                        CallResult::Success(session_handler) => {
                            // Spawn off the connection management actor and make me the supervisor of it
                            if let Some(actor) = super::session::Session::spawn_linked(
                                session_handler,
                                stream,
                                myself.get_cell(),
                            )
                            .await
                            {
                                state.nodes.insert(actor.get_id(), addr);
                            }
                        }
                    }
                }
                Err(socket_accept_error) => {
                    log::warn!(
                        "Error accepting socket {} on Node server",
                        socket_accept_error
                    );
                }
            }
        }

        // continue accepting new sockets
        let _ = myself.cast(());
    }

    async fn handle_supervisor_evt(
        &self,
        _myself: ActorRef<Self>,
        message: SupervisionEvent,
        state: &mut Self::State,
    ) {
        // sockets open, they close, the world goes round...
        match message {
            SupervisionEvent::ActorPanicked(actor, msg) => {
                match state.nodes.entry(actor.get_id()) {
                    Entry::Occupied(o) => {
                        log::error!("Connection with {} panicked with message: {}", o.get(), msg);
                        o.remove();
                    }
                    Entry::Vacant(_) => {
                        log::error!("Connection with ([unknown]) panicked with message: {}", msg);
                    }
                }
            }
            SupervisionEvent::ActorTerminated(actor, _, _) => {
                match state.nodes.entry(actor.get_id()) {
                    Entry::Occupied(o) => {
                        log::error!("Connection closed with {}", o.get());
                        o.remove();
                    }
                    Entry::Vacant(_) => {
                        log::error!("Connection with ([unknown]) closed");
                    }
                }
                let _ = self
                    .session_manager
                    .cast(super::SessionManagerMessage::CloseSession(actor.get_id()));
            }
            SupervisionEvent::ActorStarted(actor) => match state.nodes.entry(actor.get_id()) {
                Entry::Occupied(o) => {
                    log::error!("Connection opened with {}", o.get());
                }
                Entry::Vacant(_) => {
                    log::error!("Connection with ([unknown]) opened");
                }
            },
            _ => {
                //no-op
            }
        }
    }
}
