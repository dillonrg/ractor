// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Support for remote nodes in a distributed cluster.
//!
//! A node is the same as [Erlang's definition](https://www.erlang.org/doc/reference_manual/distributed.html)
//! for distributed Erlang, in that it's a remote "hosting" process in the distributed pool of processes.
//!
//! In this realization, nodes are simply actors which handle an external connection to the other nodes in the pool.
//! When nodes connect, they identify all of the nodes the remote node is also connected to and additionally connect
//! to them as well. They merge registries and pg groups together in order to create larger clusters of services.
//!
//! We have chosen protobuf for our inter-node defined protocol, however you can chose whatever medium you like
//! for binary serialization + deserialization. The "remote" actor will simply encode your message type and send it
//! over the wire for you

// #![deny(warnings)]
#![warn(unused_imports)]
#![warn(unsafe_code)]
#![warn(missing_docs)]
#![warn(unused_crate_dependencies)]
#![cfg_attr(docsrs, feature(doc_cfg))]

mod hash;
pub mod net;
pub mod node;
pub mod protocol;

/// Node's are representing by an integer id
pub type NodeId = u64;

/// Represents messages that can cross the node boundary which can be serialized and sent over the wire
pub trait SerializableMessage {
    /// Serialize the message to binary
    fn serialize(&self) -> Vec<u8>;

    /// Deserialize from binary back into the message type
    fn deserialize(&self, data: &[u8]) -> Self;
}
