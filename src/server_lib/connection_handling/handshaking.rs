//! Handshaking
//!
//! Module that contains code relative to the Handshaking procedure, submodule of
//! `connection_handling`.

use crate::globals::{
    CONNECTION_ACCEPTED, INVALID_UTF8, MAX_LEN, MAX_TRIES, TAKEN, TOO_LONG, TOO_MANY_TRIES,
    TOO_SHORT,
};
use crate::server_lib::structs::CommandFromIdRecord;
use crate::server_lib::OutputMsg;
use std::net::SocketAddr;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::tcp::WriteHalf;
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use super::super::structs::{Client, ConnHandlerIdRecordMsg, IdRecordConnHandler};

/// # `handshake`
///
/// Procedure for accepting or refusing a connection,
/// will return an `Option` containig the nickname of
/// the new client, the channels that `id_record`
/// will use to send messages to `connection_handler`.
///
/// ## Parameters
///
/// - `stream: &mut TcpStream`: `TcpStream` that communicates with the client.
/// - `addr: SocketAddr`: ip address and port used by the client.
/// - `int_com_tx: &mpsc::Sender<ConnHandlerIdRecordMsg>`: transmitter used to send messages to the
/// `id_record`.
/// - `output_tx`: Channel used to communicate the output with third parties.
///
/// ## Returns
///
/// `Option<(String, mpsc::Receiver<IdRecordConnHandler>, mpsc::Receiver<CommandFromIdRecord>)>`: client name, channel for receiving
/// messages from `id_record` and channle for receiving commands from `id_record`
/// TODO: Result instead of Option, error handling, custom error, error propagation
pub async fn handshake(
    stream: &mut TcpStream,
    addr: SocketAddr,
    int_com_tx: &mpsc::Sender<ConnHandlerIdRecordMsg>,
    output_tx: &mpsc::Sender<OutputMsg>,
) -> Option<(
    String,
    mpsc::Receiver<IdRecordConnHandler>,
    mpsc::Receiver<CommandFromIdRecord>,
)> {
    let (mut read, mut write) = stream.split();
    let mut reader = BufReader::new(&mut read);
    let mut writer = BufWriter::new(&mut write);
    let mut nick = String::new();
    let mut counter: u8 = 0;
    let (req_tx, mut req_rx) = mpsc::channel(10);
    let (mut command_tx, mut command_rx);

    loop {
        nick.clear();
        counter += 1;

        if counter > MAX_TRIES {
            let _ = notification(TOO_MANY_TRIES, &mut writer).await;
            return None;
        }

        match reader.read_line(&mut nick).await {
            Ok(0) => {
                return None;
            }
            Ok(_) => {
                nick = String::from(nick.trim());
                let len = nick.len();
                if len > MAX_LEN {
                    let success = notification(TOO_LONG, &mut writer).await;
                    if !success {
                        return None;
                    }
                    continue;
                } else if len < 3 {
                    let success = notification(TOO_SHORT, &mut writer).await;
                    if !success {
                        return None;
                    }
                    continue;
                } else {
                    (command_tx, command_rx) = mpsc::channel::<CommandFromIdRecord>(10);
                    let new_client =
                        Client::new(nick.clone(), addr.clone(), req_tx.clone(), command_tx);
                    let req = ConnHandlerIdRecordMsg::AcceptanceRequest(new_client);
                    int_com_tx.send(req).await.expect("This shouldn't happen.");
                    let accepted = req_rx.recv().await.expect("This shouldn't happen.");
                    match accepted {
                        IdRecordConnHandler::Acceptance(res) => {
                            if res {
                                let success = notification(CONNECTION_ACCEPTED, &mut writer).await;
                                if !success {
                                    return None;
                                }
                                let p = format!("{} has been accepted as {}\n", addr, nick);
                                output_tx.send(OutputMsg::new(&p)).await.unwrap();
                                break;
                            } else {
                                let success = notification(TAKEN, &mut writer).await;
                                if !success {
                                    return None;
                                }
                                continue;
                            }
                        }
                        _ => { /* TODO: problems with `id_record` */ }
                    }
                }
            }
            Err(_) => {
                let success = notification(INVALID_UTF8, &mut writer).await;
                if !success {
                    return None;
                }
                continue;
            }
        }
    }

    Some((nick, req_rx, command_rx))
}

/// # connection_handler's helper notification
///
/// Write a message to the client through the tcp stream.
///
/// Returns `false` if the connection dropped
/// TODO: Result instead of bool
async fn notification(msg: &str, writer: &mut BufWriter<&mut WriteHalf<'_>>) -> bool {
    match writer.write_all(msg.as_bytes()).await {
        Ok(_) => {}
        Err(_) => {
            return false;
        }
    }
    match writer.flush().await {
        Ok(_) => {}
        Err(_) => {
            return false;
        }
    }

    return true;
}
