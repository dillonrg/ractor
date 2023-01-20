// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use crate::concurrency as mpsc;

use crate::actor::messages::{BoxedMessage, StopMessage};
use crate::actor::supervision::SupervisionTree;
use crate::concurrency::{
    MpscReceiver as BoundedInputPortReceiver, MpscSender as BoundedInputPort,
    MpscUnboundedReceiver as InputPortReceiver, MpscUnboundedSender as InputPort,
};
use crate::{Actor, ActorId, ActorName, ActorStatus, MessagingErr, Signal, SupervisionEvent};

// The inner-properties of an Actor
pub(crate) struct ActorProperties {
    pub(crate) id: ActorId,
    pub(crate) name: Option<ActorName>,
    status: Arc<AtomicU8>,
    pub(crate) signal: BoundedInputPort<Signal>,
    pub(crate) stop: BoundedInputPort<StopMessage>,
    pub(crate) supervision: InputPort<SupervisionEvent>,
    pub(crate) message: InputPort<BoxedMessage>,
    pub(crate) tree: SupervisionTree,
    pub(crate) type_id: std::any::TypeId,
}

impl ActorProperties {
    pub fn new<TActor>(
        name: Option<ActorName>,
    ) -> (
        Self,
        BoundedInputPortReceiver<Signal>,
        BoundedInputPortReceiver<StopMessage>,
        InputPortReceiver<SupervisionEvent>,
        InputPortReceiver<BoxedMessage>,
    )
    where
        TActor: Actor,
    {
        let (tx_signal, rx_signal) = mpsc::mpsc_bounded(2);
        let (tx_stop, rx_stop) = mpsc::mpsc_bounded(2);
        let (tx_supervision, rx_supervision) = mpsc::mpsc_unbounded();
        let (tx_message, rx_message) = mpsc::mpsc_unbounded();
        (
            Self {
                id: crate::actor_id::get_new_local_id(),
                name,
                status: Arc::new(AtomicU8::new(ActorStatus::Unstarted as u8)),
                signal: tx_signal,
                stop: tx_stop,
                supervision: tx_supervision,
                message: tx_message,
                tree: SupervisionTree::default(),
                type_id: std::any::TypeId::of::<TActor>(),
            },
            rx_signal,
            rx_stop,
            rx_supervision,
            rx_message,
        )
    }

    pub fn get_status(&self) -> ActorStatus {
        match self.status.load(Ordering::Relaxed) {
            0u8 => ActorStatus::Unstarted,
            1u8 => ActorStatus::Starting,
            2u8 => ActorStatus::Running,
            3u8 => ActorStatus::Upgrading,
            4u8 => ActorStatus::Stopping,
            _ => ActorStatus::Stopped,
        }
    }

    pub fn set_status(&self, status: ActorStatus) {
        self.status.store(status as u8, Ordering::Relaxed);
    }

    pub fn send_signal(&self, signal: Signal) -> Result<(), MessagingErr> {
        self.signal.try_send(signal).map_err(|e| e.into())
    }

    pub fn send_supervisor_evt(&self, message: SupervisionEvent) -> Result<(), MessagingErr> {
        self.supervision.send(message).map_err(|e| e.into())
    }

    pub fn send_message<TActor>(&self, message: TActor::Msg) -> Result<(), MessagingErr>
    where
        TActor: Actor,
    {
        if self.type_id != std::any::TypeId::of::<TActor>() {
            return Err(MessagingErr::InvalidActorType);
        }

        let boxed = BoxedMessage::new(message);
        self.message.send(boxed).map_err(|e| e.into())
    }

    pub fn send_stop(&self, reason: Option<String>) -> Result<(), MessagingErr> {
        let msg = reason.map(StopMessage::Reason).unwrap_or(StopMessage::Stop);
        self.stop.try_send(msg).map_err(|e| e.into())
    }
}
