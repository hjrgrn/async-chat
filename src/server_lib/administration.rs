use core::panic;
use std::collections::VecDeque;

use tokio::io::{stdin, AsyncBufReadExt, BufReader};
use tokio::select;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::globals::{COMMANDS, SERVER_COM};
use crate::shared_lib::{OutputMsg, StdinRequest};

use super::ConnHandlerIdRecordMsg;

/// # `server_commands_wrapper`
///
/// Wrapper for `server_commands` that allows graceful shutdown.
///
/// ## Parameters
///
/// - `comm_tx`: Sends messages to connection handlers so that messages can be
///   sent to the clients and visualized by them.
/// - `req_rx`: Receives requests about reading from stdin. When a function needs
///   an input from STDIN, it sends said input through this channel, and
///   `server_commands` will respont to it.
/// - `output_tx`: This channel is used to send the output of the server to the
///   display facility.
/// - `ctoken`: Cancellation token used to communicate the shutdown.
pub async fn server_commands_wrapper(
    comm_tx: mpsc::Sender<ConnHandlerIdRecordMsg>,
    req_rx: mpsc::Receiver<StdinRequest>,
    output_tx: mpsc::Sender<OutputMsg>,
    ctoken: CancellationToken,
) {
    tokio::select! {
        _ = ctoken.cancelled() => {}
        res = server_commands(comm_tx, req_rx, output_tx) => {
            match res {
                Ok(_) => {}
                Err(e) => {
                    tracing::error!("`server_commands` can't work anymore:\n{:?}", e);
                }
            }
            ctoken.cancel();
        }
    }
}

/// # `server_commands`
///
/// Handles inputs from STDIN.
///
/// Receives request for input from STDIN through `req_rx`, at the same time
/// allows the admin to type.
/// After the admin finishes typing, if there is a pending request to stdin the
/// content written by the admin will be sent to the requester through the oneshot
/// channel inside `StdinRequest`; if there are no requests pending the content
/// will be sent to `id_record` through `comm_tx`, because it is assumed to be
/// a command issued by the admin. The command `SERVER_COM` will be sent directly
/// to the function that displays the output through `output_tx`.
///
/// ## Parameters
///
/// - `comm_tx`: Sends messages to connection handlers so that messages can be
///   sent to the clients and visualized by them.
/// - `req_rx`: Receives requests about reading from stdin. When a function needs
///   an input from STDIN, it sends said input through this channel, and
///   `server_commands` will respont to it.
/// - `output_tx`: This channel is used to send the output of the server to the
///   display facility.
#[tracing::instrument(
    name = "Receiving commands from user",
    skip(comm_tx, req_rx, output_tx)
)]
async fn server_commands(
    comm_tx: mpsc::Sender<ConnHandlerIdRecordMsg>,
    mut req_rx: mpsc::Receiver<StdinRequest>,
    output_tx: mpsc::Sender<OutputMsg>,
) -> Result<(), anyhow::Error> {
    let mut typer = BufReader::new(stdin());
    let mut content = String::new();

    let mut requests: VecDeque<StdinRequest> = VecDeque::new();

    output_tx
        .send(OutputMsg::new("You can start writing commands(type \"\x1b[33;1m&COMM\x1b[0m\" to list all the commands)."))
        .await?;

    'outer: loop {
        select! {
            res = typer.read_line(&mut content) => {
                if res.is_err() {
                    let _ = output_tx.send(OutputMsg::new_error("Unable to read from stidin.")).await;
                    break;
                };
            }
            res = req_rx.recv() => {
                let r = match res {
                    Some(r) => {r}
                    None => {
                        // All senders have been dropped.
                        let _ = output_tx
                            .send(OutputMsg::new_error(
                                "All senders for `server_commands` has been dropped.".to_string(),
                            ))
                            .await;
                        break;
                    }
                };
                requests.push_back(r);
            }
        }

        if content.trim().is_empty() {
            content.clear();
        } else {
            if content == SERVER_COM {
                // Display admin commands.
                if output_tx.send(OutputMsg::new(COMMANDS)).await.is_err() {
                    break;
                }
            } else {
                loop {
                    match requests.pop_front() {
                        Some(req) => match req {
                            StdinRequest::Plain(channel) => {
                                match channel.send(content.clone()) {
                                    Ok(_) => {}
                                    Err(_) => {
                                        // The channel of the STDIN request has been closed, meaning
                                        // the input is not required anymore, so we either display
                                        // it or, if there is another request pending, we satisfy
                                        // the other request.
                                        continue;
                                    }
                                }
                            }
                        },
                        None => {
                            // There are no requests pending, so this must be the admin wanting to
                            // send a command.
                            let msg = ConnHandlerIdRecordMsg::ServerCommand(content.clone());
                            if comm_tx.send(msg).await.is_err() {
                                break 'outer;
                            }
                        }
                    }
                    break;
                }
            }
            content.clear();
        }
    }
    Ok(())
}
