//! # Structs
//!
//! Structs relatives to the server library.

use std::net::SocketAddr;
use tokio::sync::mpsc;

/// # `RunIdRecordMsg`
///
/// Message that is sent from `crate::server_lib::run` to
/// `crate::server_lib::id_record::id_record`
pub enum RunIdRecordMsg {
    IsThereSpace,
}
/// # `RunIdRecordMsg`
///
/// Message that is sent from `crate::server_lib::id_record::id_record` to
/// `crate::server_lib::run`
pub enum IdRecordRunMsg {
    IsThereSpace(bool),
}
/// # `ConnHandlerIdRecordMsg`
///
/// Message sent from a `crate::server_lib::connection_handling::connection_handler` to
/// `crate::server_lib::id_record::id_record`
#[derive(Debug)]
pub enum ConnHandlerIdRecordMsg {
    ClientLeft(SocketAddr),
    AcceptanceRequest(Client),
    List(SocketAddr),
    ServerCommand(String),
}

/// # `ConnHandlerIdRecordMsg`
///
/// Message sent from a `crate::server_lib::id_record::id_record` to
/// `crate::server_lib::connection_handling::connection_handler`
#[derive(Debug)]
pub enum IdRecordConnHandler {
    Acceptance(bool),
    List(String),
}

/// TODO: Description
#[derive(Debug)]
pub struct Client {
    pub nick: String,
    pub addr: SocketAddr,
    pub channel: mpsc::Sender<IdRecordConnHandler>,
    pub command: mpsc::Sender<CommandFromIdRecord>,
}
impl Client {
    pub fn new(
        nick: String,
        addr: SocketAddr,
        channel: mpsc::Sender<IdRecordConnHandler>,
        command: mpsc::Sender<CommandFromIdRecord>,
    ) -> Self {
        Self {
            nick,
            addr,
            channel,
            command,
        }
    }
}

/// TODO: Description
pub enum CommandFromIdRecord {
    Kick,
}

/// TODO: Description
#[derive(Debug, Clone)]
pub enum Message {
    Personal {
        content: String,
        address: SocketAddr,
    },
    Broadcast {
        content: String,
        address: SocketAddr,
    },
}
