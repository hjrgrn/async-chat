use crate::globals::{HANDSHAKE_TIMEOUT, LIST, TIMEOUT};
use crate::server_lib::structs::{CommandFromIdRecord, IdRecordConnHandler};
use crate::server_lib::OutputMsg;
use anyhow::anyhow;
use std::error::Error;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::net::tcp::WriteHalf;
use tokio::net::TcpStream;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, mpsc};
use tokio::time;

use crate::server_lib::structs::{ConnHandlerIdRecordMsg, Message};

use crate::server_lib::connection_handling::handshaking::handshake;

/// # connection has dropped
///
/// Helper function of `connection_handler`, this function is invoked when a client drops,
/// if the drop incurred becouse of an error then the error will be displayed on the server.
///
///
/// ## Parameters
///
/// - `id_tx`: sends message to `id_record`
/// - `err`: eventual error that needs to be communicated
/// - `addr`: address of the client that dropped
/// - `output_tx`: channel to output on the server side
/// TODO: graceful shutdown
async fn connection_dropped<T: Error>(
    id_tx: &mpsc::Sender<ConnHandlerIdRecordMsg>,
    err: Option<T>,
    addr: SocketAddr,
    output_tx: &mpsc::Sender<OutputMsg>,
) {
    match err {
        Some(e) => output_tx
            .send(OutputMsg::new_error(&e.to_string()))
            .await
            .unwrap(),
        None => {}
    }
    // update id_record
    id_tx
        .send(ConnHandlerIdRecordMsg::ClientLeft(addr))
        .await
        .unwrap();
}

/// # send message
///
/// `connection_handler`'s helper, sends the formatted message to the client.
///
/// ## Parameters
///
/// - `nick`: nickname of the creator of the message
/// - `content`: content of the message
/// - `addr`: address of the creator of the message
/// - `writer`: buffer of the tcp write half
/// - `id_tx`: channel that sends messages to `id_record`
/// - `output_tx`: communicates eventual outputs with third parties
///
/// ## Returns
///
/// `bool`: if `true` the caller needs to keep going with the loop, if `false` it needs to stop the
/// loop.
/// TODO: Result instead of bool, error handling/propagation, graceful shutdonw
async fn send_messages(
    content: &str,
    addr: &SocketAddr,
    writer: &mut BufWriter<&mut WriteHalf<'_>>,
    id_tx: &mpsc::Sender<ConnHandlerIdRecordMsg>,
    output_tx: &mpsc::Sender<OutputMsg>,
) -> bool {
    let mut keep_going = true;

    match writer.write_all(content.as_bytes()).await {
        Ok(_) => {}
        Err(err) => {
            // connection has dropped
            connection_dropped(id_tx, Some(err), addr.clone(), output_tx).await;
            keep_going = false;
        }
    }
    // without this procedure not all bytes of the buffer may be transmitted
    match writer.flush().await {
        Ok(_) => {}
        Err(err) => {
            // connection has dropped
            connection_dropped(id_tx, Some(err), addr.clone(), output_tx).await;
            keep_going = false;
        }
    }

    keep_going
}

/// # `connection_handler`'s helper
///
/// Simplyfies the code
///
/// ## Parameters
///
/// - `stream`: communicates with the client
/// - `id_tx`: sends informations to `id_record`
/// - `addr`: address of the client
///
/// ## Returns
///
/// - `Option<(String, mpsc::Receiver<IdRecordConnHandler>, mpsc::Receiver<CommandFromIdRecord>)>`: the nickname of the client, the
/// receiver that will be used to receive messages from `id_record`, the channel that will be used
/// to receive commands from `id_record`
/// TODO: graceful shutdown
pub async fn handshake_wrapper(
    stream: &mut TcpStream,
    id_tx: &mpsc::Sender<ConnHandlerIdRecordMsg>,
    addr: &SocketAddr,
    output_tx: &mpsc::Sender<OutputMsg>,
) -> Result<
    (
        String,
        mpsc::Receiver<IdRecordConnHandler>,
        mpsc::Receiver<CommandFromIdRecord>,
    ),
    anyhow::Error,
> {
    // Handshake
    tokio::select! {
        // getting the nickname
        res = handshake(stream, addr.clone(), &id_tx, output_tx) => {
            return res;
        }
        // timer
        _ = time::sleep(Duration::from_secs(HANDSHAKE_TIMEOUT)) => {
            let mut buffer = BufWriter::new(stream);
            let _ = buffer.write_all(TIMEOUT.as_bytes()).await;
            let _ = buffer.flush();
            return Err(anyhow!("Handshake failed becouse timeout has been reached."));
        }
    }
}

/// # `connection_handler`'s helper
///
/// Envelops the logic of the read branch of `connection_handler`'s main loop, reads from the
/// clinet corresponding to the specific `connection_handler`, if false is returned
/// the outer loop needs to be broken.
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
/// TODO: result instead of a bool, error handling, error propagation, graceful shutdonw
pub async fn read_branch(
    bytes: Result<usize, std::io::Error>,
    line: &mut String,
    id_tx: &mpsc::Sender<ConnHandlerIdRecordMsg>,
    addr: &SocketAddr,
    id_hand_rx: &mut mpsc::Receiver<IdRecordConnHandler>,
    int_com_tx: &broadcast::Sender<Message>,
    nick: &str,
    output_tx: mpsc::Sender<OutputMsg>,
) -> bool {
    match bytes {
        Ok(0) => {
            // connection has been closed by the client
            // NOTE: it's a generic error, used so that I can use `None`, waiting for `anyhow`
            let err: Option<RecvError> = None;
            connection_dropped(&id_tx, err, addr.clone(), &output_tx).await;

            return false;
        }
        Ok(_) => {
            read_branch_n(line, id_tx, id_hand_rx, int_com_tx, addr, nick, &output_tx).await;
        }
        Err(err) => {
            // send the line to the branch that communicates
            // with the clinet
            let msg = Message::Broadcast {
                content: line.clone(),
                address: addr.clone(),
            };
            int_com_tx.send(msg).unwrap();
            connection_dropped(&id_tx, Some(err), addr.clone(), &output_tx).await;
            return false;
        }
    }

    true
}

/// # `read_branch`'s helper
///
/// branch of code for the `Ok(n)` variant in the match of `read_branch`
///
/// ## Parameters
///
/// - `line`: line to be sent to the `connection_handler` branch that sends stuff to the client
/// - `id_tx`: sends informations to `id_record`
/// - `id_hand_rx`: receives informations from `id_record`
/// - `int_com_tx`: sends `line` to `connection_handler`
/// - `addr`: address of the client
/// - `nick`: nickname of the client
/// TODO: graceful shutdown
async fn read_branch_n(
    line: &mut String,
    id_tx: &mpsc::Sender<ConnHandlerIdRecordMsg>,
    id_hand_rx: &mut mpsc::Receiver<IdRecordConnHandler>,
    int_com_tx: &broadcast::Sender<Message>,
    addr: &SocketAddr,
    nick: &str,
    output_tx: &mpsc::Sender<OutputMsg>,
) {
    match line.chars().nth(0) {
        Some(n) => {
            if n == '&' {
                if line == LIST {
                    let req = ConnHandlerIdRecordMsg::List(addr.clone());
                    id_tx.send(req).await.unwrap();
                    let list = id_hand_rx.recv().await.unwrap();
                    let mut content = String::new();
                    match list {
                        IdRecordConnHandler::List(s) => {
                            content = s;
                        }
                        _ => {
                            // TODO: logging the unexpected behaviour
                        } // `id_record` should always respond with a `List` variant here.
                    };
                    let msg = Message::Personal {
                        content,
                        address: addr.clone(),
                    };
                    int_com_tx.send(msg).unwrap();
                }
            } else {
                // regular message
                *line = format!("{}: {}", nick, line);
                output_tx.send(OutputMsg::new(&line)).await.unwrap();
                // send the line to the branch that communicates
                // with the clinet
                let msg = Message::Broadcast {
                    content: line.clone(),
                    address: addr.clone(),
                };
                int_com_tx.send(msg).unwrap();
                line.clear();
            }
        }
        None => {}
    }
}

/// # `connection_handler`'s helper
///
/// Envelops the logic of the write barnch of `connection_handler`'s main loop, if `false` is
/// returned the outer loop have to be ended.
///
/// ## Parameters
/// - `res`: `Option` returned from the internal communication channel
/// - `addr`: address of the client
/// - `nick`: nickname of the client
/// - `write`: tcp connection used to send messages to the client
/// - `id_tx`: channel used to send messages to `id_record`
/// - `line`: line about to be transmitted to the client
/// - `output_tx`: communicates eventual outputs with third parties
///
/// ## Returns
/// `bool`: keep going or not
///
/// TODO: Result instead of bool, error handling/propagation, graceful shutdonw
pub async fn write_branch(
    res: Result<Message, RecvError>,
    addr: &SocketAddr,
    writer: &mut BufWriter<&mut WriteHalf<'_>>,
    id_tx: &mpsc::Sender<ConnHandlerIdRecordMsg>,
    output_tx: mpsc::Sender<OutputMsg>,
) -> bool {
    let mut keep_going = true;

    match res {
        Ok(msg) => match msg {
            Message::Broadcast { content, address } => {
                if address != *addr {
                    keep_going = send_messages(&content, addr, writer, id_tx, &output_tx).await;
                }
            }
            Message::Personal { content, address } => {
                if address == *addr {
                    keep_going = send_messages(&content, &addr, writer, id_tx, &output_tx).await;
                }
            }
        },
        // `int_com_rx` and `int_com_tx` have dropped, so it means the future
        // `connection_handler` has returned, so this should not happen
        Err(err) => {
            connection_dropped(&id_tx, Some(err), addr.clone(), &output_tx).await;
            keep_going = false;
        }
    }
    keep_going
}
