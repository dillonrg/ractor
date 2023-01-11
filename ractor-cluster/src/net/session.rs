// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! TCP session actor which is managing the specific communication to a node

// TODO: RUSTLS + Tokio : https://github.com/tokio-rs/tls/blob/master/tokio-rustls/examples/server/src/main.rs

use std::marker::PhantomData;

use bytes::Bytes;
use prost::Message;
use ractor::SupervisionEvent;
use ractor::{Actor, ActorCell, ActorRef};
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::ErrorKind;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;

use super::NetworkMessage;

/// Helper method to read exactly `len` bytes from the stream into a pre-allocated buffer
/// of bytes
async fn read_n_bytes(stream: &mut OwnedReadHalf, len: usize) -> Result<Vec<u8>, tokio::io::Error> {
    let mut buf = Vec::with_capacity(len);
    let mut c_len = 0;
    while c_len < len {
        let n = stream.read(&mut buf[c_len..]).await?;
        c_len += n;
    }
    Ok(buf)
}

// ========================= Node Session actor ========================= //

/// Represents a bi-directional tcp connection along with send + receive operations
///
/// The [Session] actor supervises two child actors, [SessionReader] and [SessionWriter]. Should
/// either the reader or writer exit, they will terminate the entire session.
pub struct Session {
    handler: ActorRef<crate::node::NodeSession>,
}

impl Session {
    pub(crate) async fn spawn_linked(
        handler: ActorRef<crate::node::NodeSession>,
        stream: TcpStream,
        supervisor: ActorCell,
    ) -> Option<ActorRef<Self>> {
        let actor = match Actor::spawn_linked(None, Session { handler }, supervisor).await {
            Err(err) => {
                log::error!("Failed to spawn session writer actor: {}", err);
                None
            }
            Ok((a, _)) => {
                // intiialize this actor & its children
                let _ = a.cast(SessionMessage::SetStream(stream));
                // return the actor handle
                Some(a)
            }
        };
        actor
    }
}

/// The node connection messages
pub enum SessionMessage {
    /// Set the session's tcp stream, which initializes all underlying states
    SetStream(TcpStream),

    /// Send a message over the channel
    Send(crate::protocol::NetworkMessage),

    /// An object was received on the channel
    ObjectAvailable(crate::protocol::NetworkMessage),
}

/// The node session's state
pub struct SessionState {
    writer: ActorRef<SessionWriter<crate::protocol::NetworkMessage>>,
    reader: ActorRef<SessionReader>,
}

#[async_trait::async_trait]
impl Actor for Session {
    type Msg = SessionMessage;
    type State = SessionState;

    async fn pre_start(&self, myself: ActorRef<Self>) -> Self::State {
        // spawn writer + reader child actors
        let (writer, _) = Actor::spawn_linked(
            None,
            SessionWriter::<crate::protocol::NetworkMessage> {
                _phantom: PhantomData,
            },
            myself.get_cell(),
        )
        .await
        .expect("Failed to start session writer");
        let (reader, _) = Actor::spawn_linked(
            None,
            SessionReader {
                session: myself.clone(),
            },
            myself.get_cell(),
        )
        .await
        .expect("Failed to start session reader");

        // notify the NodeSession about this TcpSession
        let _ = self
            .handler
            .cast(crate::net::SessionMessage::SetSession(myself.clone()));

        Self::State { writer, reader }
    }

    async fn handle(&self, _myself: ActorRef<Self>, message: Self::Msg, state: &mut Self::State) {
        match message {
            Self::Msg::SetStream(stream) => {
                let (read, write) = stream.into_split();
                // initialize the writer & reader state's
                let _ = state.writer.cast(SessionWriterMessage::SetStream(write));
                let _ = state.reader.cast(SessionReaderMessage::SetStream(read));
            }
            Self::Msg::Send(msg) => {
                let _ = state.writer.cast(SessionWriterMessage::WriteObject(msg));
            }
            Self::Msg::ObjectAvailable(msg) => {
                let _ = self
                    .handler
                    .cast(crate::net::SessionMessage::MessageReceived(msg));
            }
        }
    }

    async fn handle_supervisor_evt(
        &self,
        myself: ActorRef<Self>,
        message: SupervisionEvent,
        state: &mut Self::State,
    ) {
        // sockets open, they close, the world goes round... If a reader or writer exits for any reason, we'll start the shutdown procedure
        // which requires that all actors exit
        match message {
            SupervisionEvent::ActorPanicked(actor, panic_msg) => {
                if actor.get_id() == state.reader.get_id() {
                    log::error!("TCP Session's reader panicked with '{}'", panic_msg);
                } else if actor.get_id() == state.writer.get_id() {
                    log::error!("TCP Session's writer panicked with '{}'", panic_msg);
                } else {
                    log::error!("TCP Session received a child panic from an unknown child actor ({}) - '{}'", actor.get_id(), panic_msg);
                }
                myself.stop(Some("child_panic".to_string()));
            }
            SupervisionEvent::ActorTerminated(actor, _, exit_reason) => {
                if actor.get_id() == state.reader.get_id() {
                    log::error!("TCP Session's reader exited");
                } else if actor.get_id() == state.writer.get_id() {
                    log::error!("TCP Session's writer exited");
                } else {
                    log::error!("TCP Session received a child exit from an unknown child actor ({}) - '{:?}'", actor.get_id(), exit_reason);
                }
                myself.stop(Some("child_terminate".to_string()));
            }
            _ => {
                // all ok
            }
        }
    }
}

// ========================= Node Session writer ========================= //

struct SessionWriter<TMsg>
where
    TMsg: NetworkMessage,
{
    _phantom: PhantomData<TMsg>,
}

struct SessionWriterState {
    writer: Option<OwnedWriteHalf>,
}

enum SessionWriterMessage<TMsg>
where
    TMsg: NetworkMessage,
{
    /// Set the stream, providing a [TcpStream], which
    /// to utilize for this node's connection
    SetStream(OwnedWriteHalf),

    /// Write an object over the wire
    WriteObject(TMsg),
}

#[async_trait::async_trait]
impl<TMsg> Actor for SessionWriter<TMsg>
where
    TMsg: NetworkMessage,
{
    type Msg = SessionWriterMessage<TMsg>;

    type State = SessionWriterState;

    async fn pre_start(&self, _myself: ActorRef<Self>) -> Self::State {
        // OK we've established connection, now we can process requests

        Self::State { writer: None }
    }

    async fn post_stop(&self, _myself: ActorRef<Self>, state: &mut Self::State) {
        // drop the channel to close it should we be exiting
        drop(state.writer.take());
    }

    async fn handle(&self, _myself: ActorRef<Self>, message: Self::Msg, state: &mut Self::State) {
        match message {
            Self::Msg::SetStream(stream) if state.writer.is_none() => {
                state.writer = Some(stream);
            }
            Self::Msg::WriteObject(msg) if state.writer.is_some() => {
                if let Some(stream) = &mut state.writer {
                    stream.writable().await.unwrap();

                    let length = msg.encoded_len();

                    // encode the length into a buffer and first write those bytes
                    let mut length_buf = Vec::with_capacity(10);
                    prost::encode_length_delimiter(length, &mut length_buf).unwrap();
                    if let Err(write_err) = stream.write_all(&length_buf).await {
                        log::warn!("Error writing to the stream: '{}'", write_err);
                    } else {
                        // Serialize the full object and write it over the wire
                        let mut buf = Vec::with_capacity(length);
                        msg.encode(&mut buf).unwrap();
                        if let Err(write_err) = stream.write_all(&mut buf).await {
                            log::warn!("Error writing to the stream: '{}'", write_err);
                        }
                    }
                }
            }
            _ => {
                // no-op, wait for next send request
            }
        }
    }
}

// ========================= Node Session reader ========================= //

struct SessionReader {
    session: ActorRef<Session>,
}

/// The node connection messages
pub enum SessionReaderMessage {
    /// Set the stream, providing a [TcpStream], which
    /// to utilize for this node's connection
    SetStream(OwnedReadHalf),

    /// Wait for an object from the stream
    WaitForObject,

    /// Read next object off the stream
    ReadObject(usize),
}

struct SessionReaderState {
    reader: Option<OwnedReadHalf>,
}

#[async_trait::async_trait]
impl Actor for SessionReader {
    type Msg = SessionReaderMessage;

    type State = SessionReaderState;

    async fn pre_start(&self, _myself: ActorRef<Self>) -> Self::State {
        Self::State { reader: None }
    }

    async fn post_stop(&self, _myself: ActorRef<Self>, state: &mut Self::State) {
        // drop the channel to close it should we be exiting
        drop(state.reader.take());
    }

    async fn handle(&self, myself: ActorRef<Self>, message: Self::Msg, state: &mut Self::State) {
        match message {
            Self::Msg::SetStream(stream) if state.reader.is_none() => {
                state.reader = Some(stream);
                // wait for an incoming object
                let _ = myself.cast(SessionReaderMessage::WaitForObject);
            }
            Self::Msg::WaitForObject if state.reader.is_some() => {
                if let Some(stream) = &mut state.reader {
                    stream.readable().await.unwrap();
                    match read_n_bytes(stream, 10).await {
                        Ok(buf) => {
                            let bytes = Bytes::from(buf);
                            match prost::decode_length_delimiter(bytes) {
                                Ok(protobuf_len) => {
                                    let _ =
                                        myself.cast(SessionReaderMessage::ReadObject(protobuf_len));
                                    return;
                                }
                                Err(decode_err) => {
                                    log::warn!(
                                        "Failed to decode protobuf object length with {}",
                                        decode_err
                                    );
                                    // continue processing
                                }
                            }
                        }
                        Err(err) if err.kind() == ErrorKind::UnexpectedEof => {
                            // EOF, close the stream by dropping the stream
                            drop(state.reader.take());
                            myself.stop(Some("channel_closed".to_string()));
                        }
                        Err(_other_err) => {
                            // some other TCP error, more handling necessary
                        }
                    }
                }

                let _ = myself.cast(SessionReaderMessage::WaitForObject);
            }
            Self::Msg::ReadObject(length) if state.reader.is_some() => {
                if let Some(stream) = &mut state.reader {
                    match read_n_bytes(stream, length).await {
                        Ok(buf) => {
                            // NOTE: Our implementation writes 2 messages when sending something over the wire, the first
                            // is exactly 10 bytes which constitute the length of the payload message, followed by the payload.
                            // This tells our TCP reader how much data to read off the wire

                            // [buf] here should contain the exact amount of data to decode an object properly.
                            let bytes = Bytes::from(buf);
                            match crate::protocol::NetworkMessage::decode(bytes) {
                                Ok(msg) => {
                                    // we decoded a message, pass it up the chain
                                    let _ = self.session.cast(SessionMessage::ObjectAvailable(msg));
                                }
                                Err(decode_err) => {
                                    log::error!(
                                        "Error decoding network message: '{}'. Discarding",
                                        decode_err
                                    );
                                }
                            }
                        }
                        Err(err) if err.kind() == ErrorKind::UnexpectedEof => {
                            // EOF, close the stream by dropping the stream
                            drop(state.reader.take());
                            myself.stop(Some("channel_closed".to_string()));
                            return;
                        }
                        Err(_other_err) => {
                            // TODO: some other TCP error, more handling necessary
                        }
                    }
                }

                // we've read the object, now wait for next object
                let _ = myself.cast(SessionReaderMessage::WaitForObject);
            }
            _ => {
                // no stream is available, keep looping until one is available
                let _ = myself.cast(SessionReaderMessage::WaitForObject);
            }
        }
    }
}
