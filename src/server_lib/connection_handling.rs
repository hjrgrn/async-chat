//! # Connection Handling
//!
//! Set of functions relative to the handling of the incoming connections

use auxiliaries::{ReadBranchError, WriteBranchError};
use std::net::SocketAddr;
use tokio::io::{BufReader, BufWriter};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use crate::server_lib::connection_handling::auxiliaries::{read_branch, write_branch};
use crate::server_lib::structs::CommandFromIdRecord;
use crate::shared_lib::socket_handling::RecvHandler;

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
            ) => {
            match res {
                Ok(_) => {}
                Err(e) => {
                    let _ = output_tx.send(OutputMsg::new_error(e.to_string())).await;
                    ctoken.cancel();
                }
            }
        }
    }
}

/// TODO: Description , telemetry
pub async fn connection_handler(
    mut stream: TcpStream,
    addr: SocketAddr,
    int_com_tx: broadcast::Sender<Message>, // internal communication
    mut int_com_rx: broadcast::Receiver<Message>, // internal communication
    id_tx: mpsc::Sender<ConnHandlerIdRecordMsg>, // sending to id record
    output_tx: mpsc::Sender<OutputMsg>,     // Output channel
) -> Result<(), anyhow::Error> {
    // buffers
    let mut reader;
    let mut writer;
    let mut line = String::new();
    let mut id_hand_rx;
    let mut command_rx;
    let nick;

    match handshake_wrapper(&mut stream, &id_tx, &addr, &output_tx).await {
        Ok((n, i, c)) => {
            nick = n;
            id_hand_rx = i;
            command_rx = c;
        }
        Err(e) => {
            tracing::info!("{}", e);
            return Ok(());
        }
    }

    let (mut read, mut write) = stream.split();
    reader = BufReader::new(&mut read);
    writer = BufWriter::new(&mut write);

    let mut recv_handler = RecvHandler::new();

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
            bytes = recv_handler.recv(&mut line, &mut reader) => {
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
                            ReadBranchError::NonFatal(er) => {
                                tracing::info!("{}", er);
                                break;
                            }
                        }
                    }
                }
            }

            // sends content to the client
            res = int_com_rx.recv() => {
                match write_branch(res, &addr, &mut writer, &id_tx, output_tx.clone()).await {
                    Ok(_) => {}
                    Err(e) => {
                        match e {
                            WriteBranchError::Fatal(er) => {
                                return Err(er);
                            }
                            WriteBranchError::NonFatal(er) => {
                                tracing::info!("{}", er);
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
