//! # Connection Handling
//!
//! Set of functions relative to the handling of the incoming connections

use std::net::SocketAddr;

use anyhow::Result as AnyResult;
use tokio::io::{BufReader, BufWriter};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;
use utils::{ReadBranchError, WriteBranchError};

use crate::server_lib::connection_handling::utils::{
    handle_id_record_command, read_branch, write_branch,
};
use crate::server_lib::structs::{CommandFromIdRecord, IdRecordConnHandler};
use crate::shared_lib::socket_handling::{RecvHandler, WriteHandler};

use super::structs::{ConnHandlerIdRecordMsg, Message};
use super::OutputMsg;

pub mod handshaking;
pub mod utils;

/// `connection_handler`'s wrapper
#[allow(clippy::too_many_arguments)] // TODO: maybe do something about it
pub async fn connection_handler_wrapper(
    nick: String,
    addr: SocketAddr,
    int_com_tx: broadcast::Sender<Message>, // internal communication
    int_com_rx: broadcast::Receiver<Message>, // internal communication
    id_tx: mpsc::Sender<ConnHandlerIdRecordMsg>, // sending to id record
    output_tx: mpsc::Sender<OutputMsg>,     // Output channel
    ctoken: CancellationToken,
    write_handler: WriteHandler<BufWriter<OwnedWriteHalf>>,
    read_handler: RecvHandler<BufReader<OwnedReadHalf>>,
    id_hand_rx: mpsc::Receiver<IdRecordConnHandler>,
    command_rx: mpsc::Receiver<CommandFromIdRecord>,
) {
    tokio::select! {
        _ = ctoken.cancelled() => {}
        res = connection_handler(
                &nick,
                &addr,
                int_com_tx,
                int_com_rx,
                id_tx,
                output_tx.clone(),
                write_handler,
                read_handler,
                id_hand_rx,
                command_rx,
            ) => {
            match res {
                Ok(_) => {}
                Err(e) => {
                    let _ = output_tx.send(OutputMsg::new_error(e.to_string())).await;
                    tracing::error!("`connection_handler` can't work anymore:\n{:?}", e);
                    ctoken.cancel();
                }
            }
        }
    }
}

/// #`connection_handler`
///
/// Handles a single connection.
///
///
/// ## Parameters
///
/// - nick: Nickname of the client.
/// - addr: Address of the client.
/// - int_com_tx: Channel for internal communication with handler, transmitter.
/// - int_com_rx: Channel for internal communication with handler, receiver.
/// - id_tx: Channel for communication with id_record, transmitter.
/// - output_tx: Output channel.
/// - write_handler: Write part of the TCP connection with client.
/// - read_handler: Read part of the TCP connection with client.
/// XXX: id_hand_rx, command_rx, probably one of the is redundant
#[tracing::instrument(
    name = "Handling connection.",
    skip_all,
    fields(
        username = %nick,
        address = %addr
    )
)]
#[allow(clippy::too_many_arguments)] // TODO: maybe do something about it
async fn connection_handler(
    nick: &str,
    addr: &SocketAddr,
    int_com_tx: broadcast::Sender<Message>,
    mut int_com_rx: broadcast::Receiver<Message>,
    id_tx: mpsc::Sender<ConnHandlerIdRecordMsg>,
    output_tx: mpsc::Sender<OutputMsg>,
    mut write_handler: WriteHandler<BufWriter<OwnedWriteHalf>>,
    mut read_handler: RecvHandler<BufReader<OwnedReadHalf>>,
    mut id_hand_rx: mpsc::Receiver<IdRecordConnHandler>,
    mut command_rx: mpsc::Receiver<CommandFromIdRecord>,
) -> AnyResult<()> {
    // buffers
    let mut line = String::new();

    loop {
        tokio::select! {
            // commands form `id_record`
            opt = command_rx.recv() => {
                return handle_id_record_command(*addr, id_tx, int_com_tx, &opt, output_tx).await;
            }
            // read from the client
            bytes = read_handler.recv_str(&mut line) => {
                match read_branch(
                    bytes,
                    &mut line,
                    &id_tx,
                    addr,
                    &mut id_hand_rx,
                    &int_com_tx,
                    nick,
                    output_tx.clone(),
                ).await {
                    Ok(_) => {},
                    Err(e) => {
                        match e {
                            ReadBranchError::Fatal(er) => {
                                return Err(er);
                            }
                            ReadBranchError::NonFatal(e) => {
                                tracing::info!(
                                    "Connection with:\naddr: {}\nuser: {}\nClosed becouse of:\n{}",
                                    addr,
                                    nick,
                                    e
                                );
                                break;
                            }
                        }
                    }
                }
            }

            // sends content to the client
            res = int_com_rx.recv() => {
                match write_branch(res, addr, &mut write_handler, &id_tx, output_tx.clone()).await {
                    Ok(_) => {}
                    Err(e) => {
                        match e {
                            WriteBranchError::Fatal(er) => {
                                return Err(er);
                            }
                            WriteBranchError::NonFatal(e) => {
                                tracing::info!(
                                    "Connection with:\naddr: {}\nuser: {}\nClosed becouse of:\n{}",
                                    addr,
                                    nick,
                                    e
                                );
                                break;
                            }
                        }
                    }
                };
            }
        }
    }
    Ok(())
}
