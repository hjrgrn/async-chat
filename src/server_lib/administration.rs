use core::panic;
use std::collections::VecDeque;

use tokio::io::{stdin, AsyncBufReadExt, BufReader};
use tokio::select;
use tokio::sync::mpsc;

use crate::globals::{COMMANDS, SERVER_COM};
use crate::shared_lib::{OutputMsg, StdinRequest};

use super::ConnHandlerIdRecordMsg;

/// # `server_commands`
///
/// Handles inputs from stdin.
/// Receives request for reading from stdin through `req_rx`, at the same time allows the user to
/// type.
/// After the user finished typing, if there is a pending request to stdin the content written by
/// the user will be sent to the requester through the oneshot channel inside `StdinRequest`; if
/// there are no requests pending the content will be sent to `id_record` through `comm_tx`,
/// becouse it is assumed to be a command issued by the admin.
/// The command `SERVER_COM` will be sent directly to the function that displays the output through
/// `output_tx`.
///
/// - `comm_tx` -> sends messages to connection handlers so that messages can be sent to the clients
/// and visualized by them
/// - `req_rx` -> receives requests about reading from stdin, when a part of the program needs an
/// input from stdin it sends said input through this channel and `server_commands` will respont to
/// it.
/// - `output_tx` -> this channel is used to send the output of the server to a third entity.
/// TODO: refactor, telemetry, error handling
pub async fn server_commands(
    comm_tx: mpsc::Sender<ConnHandlerIdRecordMsg>,
    mut req_rx: mpsc::Receiver<StdinRequest>,
    output_tx: mpsc::Sender<OutputMsg>,
) {
    let mut typer = BufReader::new(stdin());
    let mut content = String::new();

    let mut requests: VecDeque<StdinRequest> = VecDeque::new();

    output_tx
        .send(OutputMsg::new("You can start writing commands(type \"\x1b[33;1m&COMM\x1b[0m\" to list all the commands)."))
        .await
        .unwrap();

    loop {
        select! {
            res = typer.read_line(&mut content) => {
                res.unwrap();
            }
            res = req_rx.recv() => {
                requests.push_back(res.unwrap());
            }
        }

        // Eliminates empty strings from stdin
        if content.trim().len() < 1 {
            content.clear();
        }

        if content.len() > 0 {
            if content == SERVER_COM {
                output_tx.send(OutputMsg::new(COMMANDS)).await.unwrap();
            } else {
                loop {
                    match requests.pop_front() {
                        Some(req) => match req {
                            StdinRequest::Plain(channel) => {
                                match channel.send(content.clone()) {
                                    Ok(_) => {}
                                    Err(_) => {
                                        // The channle of the stdin request has been closed, meaning
                                        // the input is not required anymore, so we either display
                                        // it or, if there is another request pending, we satisfy
                                        // the other request
                                        continue;
                                    }
                                }
                            }
                        },
                        None => {
                            // send commands
                            let msg = ConnHandlerIdRecordMsg::ServerCommand(content.clone());
                            comm_tx.send(msg).await.unwrap();
                        }
                    }
                    break;
                }
            }
            content.clear();
        }
    }
}
