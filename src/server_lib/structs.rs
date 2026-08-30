//! # Structs
//!
//! Structs relatives to the server library.

use std::net::SocketAddr;
use tokio::{net::TcpStream, sync::mpsc};

use crate::globals::{KICK, LIST};

/// # `RunIdRecordMsg`
///
/// Message that is sent from `crate::server_lib::run` to
/// `crate::server_lib::id_record::id_record`
pub enum RunIdRecordMsg {
    NewConnection { stream: TcpStream, addr: SocketAddr },
}
/// # `RunIdRecordMsg`
///
/// Message that is sent from `crate::server_lib::id_record::id_record` to
/// `crate::server_lib::run`
// XXX: this is probably not needed anymore
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
    List(SocketAddr),
    ServerCommand(Command),
}

/// XXX: domain.
#[derive(Debug)]
pub enum Command {
    ServerList,
    Kick,
    Msg(String),
}

// TODO: proper from
impl Command {
    pub fn from_str(s: &str) -> Command {
        if s == KICK {
            return Command::Kick;
        } else if s == LIST {
            return Command::ServerList;
        } else {
            return Command::Msg(s.into());
        }
    }
}

/// # `ConnHandlerIdRecordMsg`
///
/// Message sent from a `crate::server_lib::id_record::id_record` to
/// `crate::server_lib::connection_handling::connection_handler`
#[derive(Debug)]
pub enum IdRecordConnHandler {
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
