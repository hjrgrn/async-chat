//! # Connection Handling
//!
//! Set of functions relative to the handling of the incoming connections

use std::net::SocketAddr;
use tokio::io::{AsyncBufReadExt, BufReader, BufWriter};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc};

use crate::server_lib::connection_handling::auxiliaries::{read_branch, write_branch};
use crate::server_lib::structs::CommandFromIdRecord;

use self::auxiliaries::handshake_wrapper;

use super::structs::{ConnHandlerIdRecordMsg, Message};
use super::OutputMsg;

mod auxiliaries;
mod handshaking;

/// TODO: Description, error handling, error propagation
pub async fn connection_handler(
    mut stream: TcpStream,
    addr: SocketAddr,
    int_com_tx: broadcast::Sender<Message>, // internal communication
    mut int_com_rx: broadcast::Receiver<Message>, // internal communication
    id_tx: mpsc::Sender<ConnHandlerIdRecordMsg>, // sending to id record
    output_tx: mpsc::Sender<OutputMsg>,     // Output channel
) {
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
            return;
        }
    }

    let (mut read, mut write) = stream.split();
    reader = BufReader::new(&mut read);
    writer = BufWriter::new(&mut write);

    loop {
        tokio::select! {
            // commands form `id_record`
            opt = command_rx.recv() => {
                match opt {
                    Some(command) => {
                        match command {
                            CommandFromIdRecord::Kick => {
                                let msg = ConnHandlerIdRecordMsg::ClientLeft(addr.clone());
                                id_tx.send(msg).await.unwrap();
                                let content = String::from("Master: You have been kicked.\n");
                                let personal = Message::Personal {
                                    content,
                                    address: addr.clone()
                                };
                                let _ = int_com_tx.send(personal);
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
            bytes = reader.read_line(&mut line) => {
                let keep_going = read_branch(bytes, &mut line, &id_tx, &addr, &mut id_hand_rx, &int_com_tx, &nick, output_tx.clone()).await;
                if !keep_going {
                    break;
                }
            }

            // sends content to the client
            res = int_com_rx.recv() => {
                let keep_going = write_branch(res, &addr, &mut writer, &id_tx, output_tx.clone()).await;
                if !keep_going {
                    break;
                }
            }
        }
    }
}
