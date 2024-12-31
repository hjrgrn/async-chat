//! Handshaking
//!
//! Module that contains code relative to the Handshaking procedure, submodule of
//! `connection_handling`.

use crate::globals::{
    CONNECTION_ACCEPTED, MALFORMED_PACKET, MAX_LEN, MAX_TRIES, TAKEN, TOO_LONG, TOO_MANY_TRIES,
    TOO_SHORT,
};
use crate::server_lib::structs::CommandFromIdRecord;
use crate::server_lib::OutputMsg;
use crate::shared_lib::socket_handling::{RecvHandler, RecvHandlerError, WriteHandler};
use anyhow::anyhow;
use std::net::SocketAddr;
use tokio::io::{BufReader, BufWriter};
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
/// TODO: error handling, custom error, error propagation
pub async fn handshake(
    stream: &mut TcpStream,
    addr: SocketAddr,
    int_com_tx: &mpsc::Sender<ConnHandlerIdRecordMsg>,
    output_tx: &mpsc::Sender<OutputMsg>,
) -> Result<
    (
        String,
        mpsc::Receiver<IdRecordConnHandler>,
        mpsc::Receiver<CommandFromIdRecord>,
    ),
    anyhow::Error,
> {
    let (mut read, mut write) = stream.split();
    let mut reader = BufReader::new(&mut read);
    let mut writer = BufWriter::new(&mut write);
    let mut nick = String::new();
    let mut counter: u8 = 0;
    let (req_tx, mut req_rx) = mpsc::channel(10);
    let (mut command_tx, mut command_rx);
    let mut write_handler = WriteHandler::new();
    let mut read_handler = RecvHandler::new();

    loop {
        counter += 1;

        if counter > MAX_TRIES {
            write_handler.write(TOO_MANY_TRIES, &mut writer).await?;
            return Err(anyhow!(
                "User have tried to register too many times without success"
            ));
        }

        match read_handler.recv(&mut nick, &mut reader).await {
            Ok(_) => {
                nick = String::from(nick.trim());
                let len = nick.len();
                if len > MAX_LEN {
                    write_handler.write(TOO_LONG, &mut writer).await?;
                    continue;
                } else if len < 3 {
                    write_handler.write(TOO_SHORT, &mut writer).await?;
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
                                write_handler
                                    .write(CONNECTION_ACCEPTED, &mut writer)
                                    .await?;
                                let p = format!("{} has been accepted as {}\n", addr, nick);
                                output_tx.send(OutputMsg::new(&p)).await.unwrap();
                                break;
                            } else {
                                write_handler.write(TAKEN, &mut writer).await?;
                                continue;
                            }
                        }
                        _ => { /* TODO: problems with `id_record` */ }
                    }
                }
            }
            Err(e) => {
                match e {
                    RecvHandlerError::MalformedPacket(_) => {
                        let _ = write_handler.write(MALFORMED_PACKET, &mut writer).await;
                    }
                    _ => {}
                }
                return Err(e.into());
            }
        }
    }

    Ok((nick, req_rx, command_rx))
}
