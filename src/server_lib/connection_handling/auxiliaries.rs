use crate::globals::{HANDSHAKE_TIMEOUT, LIST, TIMEOUT};
use crate::server_lib::structs::{CommandFromIdRecord, IdRecordConnHandler};
use crate::server_lib::OutputMsg;
use crate::shared_lib::auxiliaries::error_chain_fmt;
use crate::shared_lib::socket_handling::{RecvHandler, RecvHandlerError, WriteHandler};
use anyhow::anyhow;
use secrecy::SecretString;
use std::fmt::{Debug, Display};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{BufReader, BufWriter};
use tokio::net::tcp::{ReadHalf, WriteHalf};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, mpsc};
use tokio::time;

use crate::server_lib::structs::{ConnHandlerIdRecordMsg, Message};

use crate::server_lib::connection_handling::handshaking::handshake;

use super::handshaking::HandshakeError;

/// # connection has dropped
///
/// Helper function of `connection_handler`, this function is invoked when a client drops,
/// if the drop incurred becouse of an error then the error will be displayed on the server.
/// If an error is returned it is fatal and the application needs to be shutdown.
///
/// ## Parameters
///
/// - `id_tx`: sends message to `id_record`
/// - `err`: eventual error that needs to be communicated
/// - `addr`: address of the client that dropped
/// - `output_tx`: channel to output on the server side
async fn connection_dropped<T: Display>(
    id_tx: &mpsc::Sender<ConnHandlerIdRecordMsg>,
    err: Option<T>,
    addr: SocketAddr,
    output_tx: &mpsc::Sender<OutputMsg>,
) -> Result<(), anyhow::Error> {
    match err {
        Some(e) => output_tx.send(OutputMsg::new_error(&e.to_string())).await?,
        None => {}
    }
    // update id_record
    id_tx.send(ConnHandlerIdRecordMsg::ClientLeft(addr)).await?;
    Ok(())
}

/// # `connection_handler`'s helper
///
/// Simplyfies the code.
/// Returns an error that may be fatal, if that is the case the application needs to shutdown.
///
/// ## Parameters
///
/// - `stream`: communicates with the client
/// - `id_tx`: sends informations to `id_record`
/// - `addr`: address of the client
///
/// ## Returns
///
///  the nickname of the client, the receiver that will be used to receive messages from `id_record`, the channel that will be used
/// to receive commands from `id_record`
/// TODO: comment
pub async fn handshake_wrapper(
    write_handler: &mut WriteHandler<BufWriter<WriteHalf<'_>>>,
    read_handler: &mut RecvHandler<BufReader<ReadHalf<'_>>>,
    id_tx: &mpsc::Sender<ConnHandlerIdRecordMsg>,
    addr: &SocketAddr,
    output_tx: &mpsc::Sender<OutputMsg>,
    shared_secret: SecretString,
) -> Result<
    (
        String,
        mpsc::Receiver<IdRecordConnHandler>,
        mpsc::Receiver<CommandFromIdRecord>,
    ),
    HandshakeError,
> {
    // Handshake
    tokio::select! {
        // getting the nickname
        res = handshake(write_handler, read_handler, addr.clone(), &id_tx, output_tx, shared_secret) => {
            return res;
        }
        // timer
        _ = time::sleep(Duration::from_secs(HANDSHAKE_TIMEOUT)) => {
            let _ = write_handler.write_str(TIMEOUT).await;
            return Err(HandshakeError::NonFatal(anyhow!("Handshake failed becouse timeout has been reached.")));
        }
    }
}

/// # `connection_handler`'s helper
///
/// Envelops the logic of the read branch of `connection_handler`'s main loop, reads from the
/// client corresponding to the specific `connection_handler`.
/// If a fatal error is returned the application needs to be shutdown, if a non fatal error is
/// returned the outer loop beeds to be broken
///
///
/// # Parameters
///
/// - `bytes`: result from reading the line that arrives from the client from the tcp stream
/// - `line`: string received and about to be send to the internal communication channel
/// - `id_tx`: channel used to send messages to `id_record`
/// - `addr`: address of the client
/// - `id_hand_rx`: channel that receives messaged from `id_record`
/// - `int_com_tx`: internal communication channel used to send
/// - `nick`: nickname of the client
/// - `output_tx`: communicates eventual outputs
/// - `ctoken`: cancellation token
#[tracing::instrument(
    name = "Reading from connection",
    skip(bytes, line, id_tx, id_hand_rx, int_com_tx, nick, output_tx)
)]
pub async fn read_branch(
    bytes: Result<(), RecvHandlerError>,
    line: &mut String,
    id_tx: &mpsc::Sender<ConnHandlerIdRecordMsg>,
    addr: &SocketAddr,
    id_hand_rx: &mut mpsc::Receiver<IdRecordConnHandler>,
    int_com_tx: &broadcast::Sender<Message>,
    nick: &str,
    output_tx: mpsc::Sender<OutputMsg>,
) -> Result<(), ReadBranchError> {
    let res;
    match bytes {
        Ok(n) => {
            read_branch_n(line, id_tx, id_hand_rx, int_com_tx, addr, nick, &output_tx).await?;
            res = Ok(n);
        }
        Err(err) => {
            match err {
                RecvHandlerError::ConnectionInterrupted => {
                    // connection has been closed by the client
                    let e: Option<RecvHandlerError> = None;
                    connection_dropped(&id_tx, e, addr.clone(), &output_tx)
                        .await
                        .map_err(|e| ReadBranchError::Fatal(e))?;
                    res = Err(ReadBranchError::NonFatal(err.into()));
                }
                RecvHandlerError::MalformedPacket(_)
                | RecvHandlerError::IoError(_)
                | RecvHandlerError::EncryptionError(_) => {
                    connection_dropped(&id_tx, Some(&err), addr.clone(), &output_tx)
                        .await
                        .map_err(|e| ReadBranchError::Fatal(e))?;
                    res = Err(ReadBranchError::NonFatal(err.into()));
                }
                RecvHandlerError::HmacError(ref e) => {
                    let _ = connection_dropped(&id_tx, Some(&err), addr.clone(), &output_tx).await;
                    res = Err(ReadBranchError::Fatal(anyhow::anyhow!(
                        "This is a fatal error that shouldn't have happended:\n{}",
                        e
                    )));
                }
            }
        }
    }
    line.clear();
    res
}

/// # `read_branch`'s helper
///
/// Branch of code for the `Ok(n)` variant in the match of `read_branch`.
/// If an error is returned the error is fatal and the application needs to be shut down.
///
///
/// ## Parameters
///
/// - `line`: line to be sent to the `connection_handler` branch that sends stuff to the client
/// - `id_tx`: sends informations to `id_record`
/// - `id_hand_rx`: receives informations from `id_record`
/// - `int_com_tx`: sends `line` to `connection_handler`
/// - `addr`: address of the client
/// - `nick`: nickname of the client
async fn read_branch_n(
    line: &str,
    id_tx: &mpsc::Sender<ConnHandlerIdRecordMsg>,
    id_hand_rx: &mut mpsc::Receiver<IdRecordConnHandler>,
    int_com_tx: &broadcast::Sender<Message>,
    addr: &SocketAddr,
    nick: &str,
    output_tx: &mpsc::Sender<OutputMsg>,
) -> Result<(), ReadBranchError> {
    match line.chars().nth(0) {
        Some(n) => {
            if n == '&' {
                if line == LIST {
                    let req = ConnHandlerIdRecordMsg::List(addr.clone());
                    match id_tx.send(req).await {
                        Ok(_) => {}
                        Err(e) => {
                            return Err(ReadBranchError::Fatal(anyhow::anyhow!(e.to_string())));
                        }
                    };
                    let list = match id_hand_rx.recv().await {
                        Some(l) => l,
                        None => {
                            let e = "Failed to receive from `id_record` in `read_branch_n`";
                            let _ = output_tx.send(OutputMsg::new_error(&e)).await;
                            return Err(ReadBranchError::Fatal(anyhow::anyhow!(e)));
                        }
                    };
                    let content;
                    match list {
                        IdRecordConnHandler::List(s) => {
                            content = s;
                        }
                        _other => {
                            // `id_record` should always respond with a `List` variant here.
                            return Err(ReadBranchError::Fatal(anyhow::anyhow!("Unexpecte behaviour from `id_record`:\nIt responded with a non `IdRecordConnHandler::List` to a request for a list.")));
                        }
                    };
                    let msg = Message::Personal {
                        content,
                        address: addr.clone(),
                    };
                    match int_com_tx.send(msg) {
                        Ok(_) => {}
                        Err(e) => {
                            let _ = output_tx.send(OutputMsg::new_error(e.to_string())).await;
                            return Err(ReadBranchError::Fatal(anyhow::anyhow!(e)));
                        }
                    };
                }
            } else {
                // regular message
                let msg = format!("{}: {}", nick, line);
                match output_tx.send(OutputMsg::new(&msg)).await {
                    Ok(()) => {}
                    Err(e) => {
                        return Err(ReadBranchError::Fatal(anyhow::anyhow!(e)));
                    }
                };
                // send the line to the branch that communicates
                // with the clinet
                let msg = Message::Broadcast {
                    content: msg,
                    address: addr.clone(),
                };
                match int_com_tx.send(msg) {
                    Ok(_) => {}
                    Err(e) => {
                        return Err(ReadBranchError::Fatal(anyhow::anyhow!(e)));
                    }
                }
            }
        }
        None => {}
    }

    Ok(())
}

/// # `connection_handler`'s helper
///
/// Envelops the logic of the write barnch of `connection_handler`'s main loop, if a
/// fatal error is returned the application has to be shutdown, if a non fatal error
/// is returned the outer loop needs to be broken.
///
/// ## Parameters
/// - `res`: `Option` returned from the internal communication channel
/// - `addr`: address of the client
/// - `nick`: nickname of the client
/// - `write`: tcp connection used to send messages to the client
/// - `id_tx`: channel used to send messages to `id_record`
/// - `line`: line about to be transmitted to the client
/// - `output_tx`: communicates eventual outputs with third parties
/// TODO: comment
#[tracing::instrument(
    name = "Writing to connection",
    skip(res, write_handler, id_tx, output_tx)
)]
pub async fn write_branch(
    res: Result<Message, RecvError>,
    addr: &SocketAddr,
    write_handler: &mut WriteHandler<BufWriter<WriteHalf<'_>>>,
    id_tx: &mpsc::Sender<ConnHandlerIdRecordMsg>,
    output_tx: mpsc::Sender<OutputMsg>,
) -> Result<(), WriteBranchError> {
    match res {
        Ok(msg) => match msg {
            Message::Broadcast { content, address } => {
                if address != *addr {
                    match write_handler.write_str(&content).await {
                        Ok(_) => return Ok(()),
                        Err(e) => {
                            connection_dropped(id_tx, Some(&e), addr.clone(), &output_tx)
                                .await
                                .map_err(|e| WriteBranchError::Fatal(e))?;
                            return Err(WriteBranchError::NonFatal(e.into()));
                        }
                    }
                }
            }
            Message::Personal { content, address } => {
                if address == *addr {
                    match write_handler.write_str(&content).await {
                        Ok(_) => return Ok(()),
                        Err(e) => {
                            connection_dropped(id_tx, Some(&e), addr.clone(), &output_tx)
                                .await
                                .map_err(|e| WriteBranchError::Fatal(e))?;
                            return Err(WriteBranchError::NonFatal(e.into()));
                        }
                    }
                }
            }
        },
        // `int_com_rx` and `int_com_tx` have dropped, so it means the future
        // `connection_handler` has returned, so this should not happen
        Err(err) => {
            connection_dropped(&id_tx, Some(&err), addr.clone(), &output_tx)
                .await
                .map_err(|e| WriteBranchError::Fatal(e))?;
            return Err(WriteBranchError::NonFatal(err.into()));
        }
    }
    Ok(())
}

#[derive(thiserror::Error)]
pub enum ReadBranchError {
    #[error(transparent)]
    Fatal(anyhow::Error),
    #[error(transparent)]
    NonFatal(anyhow::Error),
}

impl Debug for ReadBranchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        error_chain_fmt(self, f)
    }
}

#[derive(thiserror::Error)]
pub enum WriteBranchError {
    #[error(transparent)]
    Fatal(anyhow::Error),
    #[error(transparent)]
    NonFatal(anyhow::Error),
}

impl Debug for WriteBranchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        error_chain_fmt(self, f)
    }
}
