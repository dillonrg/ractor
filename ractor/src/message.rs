// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Message types

use std::any::Any;

/// Message type for an actor. Generally an enum
/// which muxes the various types of inner-messages the actor
/// supports
///
/// ## Example
///
/// ```rust
/// pub enum MyMessage {
///     /// Record the name to the actor state
///     RecordName(String),
///     /// Print the recorded name from the state to command line
///     PrintName,
/// }
/// ```
pub trait BaseMessage: Any + Send + 'static {}
impl<T: Any + Send + 'static> BaseMessage for T {}

// ============= Basic Message ============= //
/// Message type for an actor. Generally an enum
/// which muxes the various types of inner-messages the actor
/// supports
///
/// ## Example
///
/// ```rust
/// pub enum MyMessage {
///     /// Record the name to the actor state
///     RecordName(String),
///     /// Print the recorded name from the state to command line
///     PrintName,
/// }
/// ```
#[cfg(not(feature = "cluster"))]
pub trait Message: BaseMessage {}
#[cfg(not(feature = "cluster"))]
impl<T: BaseMessage> Message for T {}

// ============= Distributed Message ============= //

/// Distributed message type for an actor which supports
/// serialization.
///
/// ## Example
///
/// ```rust
/// pub enum MyMessage {
///     /// Record the name to the actor state
///     RecordName(String),
///     /// Print the recorded name from the state to command line
///     PrintName,
/// }
/// ```
#[cfg(feature = "cluster")]
pub trait Message: crate::BaseMessage {} //+ crate::distributed::SerializableMessage
#[cfg(feature = "cluster")]
impl<T: crate::BaseMessage> Message for T {} //+ crate::distributed::SerializableMessage

/*
/// impl ractor::distributed::SerializableMessage for MyMessage {
///
///     fn serialize(&self) -> Vec<u8> {
///         &[]
///     }
///     fn deserialize(&self, data: &[u8]) -> Self {
///         MyMessage::PrintName
///     }
/// }
*/
