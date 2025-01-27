//! # Connection Handling
//!
//! Set of functions relative to the handling of the incoming connections

use auxiliaries::{ReadBranchError, WriteBranchError};
use secrecy::SecretString;
use std::net::SocketAddr;
use tokio::io::{BufReader, BufWriter};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use crate::server_lib::connection_handling::auxiliaries::{read_branch, write_branch};
use crate::server_lib::structs::CommandFromIdRecord;
use crate::shared_lib::socket_handling::{RecvHandler, WriteHandler};

use self::auxiliaries::handshake_wrapper;

use super::structs::{ConnHandlerIdRecordMsg, Message};
use super::OutputMsg;

mod auxiliaries;
mod handshaking;

/// `connection_handler`'s wrapper
pub async fn connection_handler_wrapper(
    stream: TcpStream,
    addr: SocketAddr,
    int_com_tx: broadcast::Sender<Message>, // internal communication
    int_com_rx: broadcast::Receiver<Message>, // internal communication
    id_tx: mpsc::Sender<ConnHandlerIdRecordMsg>, // sending to id record
    output_tx: mpsc::Sender<OutputMsg>,     // Output channel
    ctoken: CancellationToken,
    shared_secret: SecretString, // Secret needed for authenticate the users during handshake.
) {
    tokio::select! {
        _ = ctoken.cancelled() => {}
        res = connection_handler(
                stream,
                addr,
                int_com_tx,
                int_com_rx,
                id_tx,
                output_tx.clone(),
                shared_secret,
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
/// - mut stream: stream between the client and the server
/// - addr: address of the client
/// - int_com_tx: channel for communication internale to the handler, transmitter
/// - mut int_com_rx: channel for communication internale to the handler, receiver
/// - id_tx: channel for communication with id_record, transmitter
/// - output_tx: output channel
/// - `shared_secret` -> Secret needed for authenticate the users during handshake.
#[tracing::instrument(
    name = "Handling connection.",
    skip(stream, addr, int_com_tx, int_com_rx, id_tx, output_tx, shared_secret),
    fields(
        username = tracing::field::Empty,
        address = %addr
    )
)]
async fn connection_handler(
    mut stream: TcpStream,
    addr: SocketAddr,
    int_com_tx: broadcast::Sender<Message>,
    mut int_com_rx: broadcast::Receiver<Message>,
    id_tx: mpsc::Sender<ConnHandlerIdRecordMsg>,
    output_tx: mpsc::Sender<OutputMsg>,
    shared_secret: SecretString
) -> Result<(), anyhow::Error> {
    // buffers
    let mut line = String::new();
    let mut id_hand_rx;
    let mut command_rx;
    let nick;

    let (read, write) = stream.split();
    let reader = BufReader::new(read);
    let writer = BufWriter::new(write);
    let mut write_handler = WriteHandler::new(writer);
    let mut read_handler = RecvHandler::new(reader);

    match handshake_wrapper(
        &mut write_handler,
        &mut read_handler,
        &id_tx,
        &addr,
        &output_tx,
        shared_secret
    )
    .await
    {
        Ok((n, i, c)) => {
            nick = n;
            tracing::Span::current().record("username", &nick);
            id_hand_rx = i;
            command_rx = c;
        }
        Err(e) => match e {
            handshaking::HandshakeError::NonFatal(e) => {
                tracing::info!(
                    "Failed to complete handshake with:\naddr: {}\nBecouse of:\n{}",
                    addr,
                    e
                );
                return Ok(());
            }
            handshaking::HandshakeError::Fatal(e) => {
                return Err(e);
            }
        },
    }

    loop {
        tokio::select! {
            // commands form `id_record`
            opt = command_rx.recv() => {
                match opt {
                    Some(command) => {
                        match command {
                            CommandFromIdRecord::Kick => {
                                let msg = ConnHandlerIdRecordMsg::ClientLeft(addr.clone());
                                match id_tx.send(msg).await{
                                    Ok(_) => {}
                                    Err(e) => {
                                        let _ = output_tx.send(OutputMsg::new_error(e.to_string())).await;
                                        return Err(e.into());
                                    }
                                };
                                let content = String::from("Master: You have been kicked.\n");
                                let personal = Message::Personal {
                                    content,
                                    address: addr.clone()
                                };
                                match int_com_tx.send(personal) {
                                    Ok(_) => {}
                                    Err(e) => {
                                        let _ = output_tx.send(OutputMsg::new_error(e.to_string())).await;
                                        return Err(e.into());
                                    }
                                };
                                break;
                            }
                        }
                    }
                    None => {
                        break;
                    }
                }
            }
            // read from the client
            bytes = read_handler.recv_str(&mut line) => {
                match read_branch(
                    bytes,
                    &mut line,
                    &id_tx,
                    &addr,
                    &mut id_hand_rx,
                    &int_com_tx,
                    &nick,
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
                match write_branch(res, &addr, &mut write_handler, &id_tx, output_tx.clone()).await {
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
