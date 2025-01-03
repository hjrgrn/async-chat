use std::collections::VecDeque;

use globals::CLIENT_COM;
use handshaking::handshake;
use sending_messages::handling_stdin_input_wrapper;
use tokio::{
    io::{stdin, AsyncBufReadExt, BufReader, BufWriter},
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpStream,
    },
    select,
    sync::mpsc,
};
use tokio_util::sync::CancellationToken;

use crate::{
    client_lib::{globals::COMMANDS, settings::Settings},
    globals::LIST,
    shared_lib::{
        socket_handling::{RecvHandler, WriteHandler, WriteHandlerError},
        OutputMsg, StdinRequest,
    },
};

mod globals;
mod handshaking;
mod sending_messages;
pub mod settings;

pub async fn run_wrapper(
    settings: Settings,
    output_tx: mpsc::Sender<OutputMsg>,
    input_rx: mpsc::Receiver<InputMsg>,
    stdin_req_tx: mpsc::Sender<StdinRequest>,
    ctoken: CancellationToken,
) {
    tokio::select! {
        _ = ctoken.cancelled() => {}
        _ = run(settings, output_tx, input_rx, stdin_req_tx, ctoken.clone()) => {
                ctoken.cancel();
            }
    }
}

/// TODO: comment, telemetry, error handling
async fn run(
    settings: Settings,
    mut output_tx: mpsc::Sender<OutputMsg>,
    input_rx: mpsc::Receiver<InputMsg>,
    mut stdin_req_tx: mpsc::Sender<StdinRequest>,
    ctoken: CancellationToken,
) {
    let mut stream = match TcpStream::connect(settings.get_full_address()).await {
        Ok(s) => s,
        Err(e) => {
            let err_msg = OutputMsg::new_error(e);
            let _ = output_tx.send(err_msg).await;
            return;
        }
    };

    match handshake(&mut stream, &mut stdin_req_tx, &mut output_tx).await {
        Ok(_) => {}
        Err(e) => {
            let _ = output_tx.send(OutputMsg::new_error(e)).await;
            return;
        }
    }

    let (read_half, write_half) = stream.into_split();
    let input_handle = tokio::spawn(handling_stdin_input_wrapper(
        write_half,
        input_rx,
        output_tx.clone(),
        ctoken.clone(),
    ));
    let recv_handle = tokio::spawn(recv_msg_wrapper(read_half, output_tx, ctoken));

    let _ = input_handle.await;
    let _ = recv_handle.await;
}

/// TODO: move this somewhere
async fn recv_msg_wrapper(
    reader: OwnedReadHalf,
    output_tx: mpsc::Sender<OutputMsg>,
    ctoken: CancellationToken,
) {
    tokio::select! {
        _ = ctoken.cancelled() => {}
        _ = recv_msg(reader, output_tx) => {
                ctoken.cancel();
            }
    }
}

/// TODO: Description
async fn recv_msg(reader: OwnedReadHalf, output_tx: mpsc::Sender<OutputMsg>) {
    let mut reader = BufReader::new(reader);
    let mut recv_handler = RecvHandler::new();

    let mut response = String::new();

    loop {
        match recv_handler.recv(&mut response, &mut reader).await {
            Ok(_) => {
                match output_tx.send(OutputMsg::new(&response)).await {
                    Ok(_) => {}
                    Err(_) => {
                        break;
                    }
                };
            }
            Err(e) => {
                let _ = output_tx.send(OutputMsg::new_error(e)).await;
                break;
            }
        }
    }
}

/// `client_commands`' wrapper
///
///
/// ## Parameters
///
/// - `input_tx` -> sends user's messages and commands from stdin to the functionality that handles
/// them.
/// - `req_rx` -> receives requests about reading from stdin, when a part of the program needs an
/// input from stdin it sends said input through this channel and `client_command` will respond to
/// it.
/// - `output_tx` -> this channel is used to send the output of the server to a third entity.
/// - `ctoken` -> CancellationToken for graceful shutdown
pub async fn client_commands_wrapper(
    input_tx: mpsc::Sender<InputMsg>,
    req_rx: mpsc::Receiver<StdinRequest>,
    output_tx: mpsc::Sender<OutputMsg>,
    ctoken: CancellationToken,
) {
    tokio::select! {
        _ = ctoken.cancelled() => {}
        _ = client_commands(
                input_tx,
                req_rx,
                output_tx,
            ) => {
                ctoken.cancel();
            }
    }
}

/// TODO: description, refactor, telemetry, error handling, move the code somewhere else, graceful shutdown, duplication in `server_lib::server_commands`
/// - `input_tx` -> sends user's messages and commands from stdin to the functionality that handles
/// them.
/// - `req_rx` -> receives requests about reading from stdin, when a part of the program needs an
/// input from stdin it sends said input through this channel and `client_command` will respond to
/// it.
/// - `output_tx` -> this channel is used to send the output of the server to a third entity.
async fn client_commands(
    input_tx: mpsc::Sender<InputMsg>,
    mut req_rx: mpsc::Receiver<StdinRequest>,
    output_tx: mpsc::Sender<OutputMsg>,
) {
    let mut typer = BufReader::new(stdin());
    let mut content = String::new();

    let mut requests: VecDeque<StdinRequest> = VecDeque::new();

    loop {
        select! {
            res = typer.read_line(&mut content) => {
                match res {
                    Ok(_) => {},
                    Err(e) => {
                        let _ = output_tx.send(OutputMsg::new_error(e)).await;
                        return;
                    }
                }
            }
            res = req_rx.recv() => {
                let r = match res {
                    Some(r) => {r},
                    None => {
                        let _ = output_tx.send(OutputMsg::new_error("Failed to receive request for stdin in `client_commands`")).await;
                        return;
                    }
                };
                requests.push_back(r);
            }
        }
        // Eliminates empty strings from stdin
        if content.trim().len() < 1 {
            content.clear();
        }

        if content.len() > 0 {
            if content == CLIENT_COM {
                match output_tx.send(OutputMsg::new(COMMANDS)).await {
                    Ok(_) => {}
                    Err(_) => {
                        return;
                    }
                }
            } else {
                loop {
                    // Responding to a request
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
                        // Sending a message to server
                        None => {
                            let msg = InputMsg::build(&content);
                            match msg {
                                Ok(m) => {
                                    match input_tx.send(m).await {
                                        Ok(_) => {}
                                        Err(e) => {
                                            let _ = output_tx.send(OutputMsg::new_error(e)).await;
                                            return;
                                        }
                                    };
                                }
                                Err(e) => match output_tx.send(OutputMsg::new_error(&e)).await {
                                    Ok(_) => {}
                                    Err(_) => {
                                        return;
                                    }
                                },
                            }
                        }
                    }
                    break;
                }
            }
            content.clear();
        }
    }
}

/// TODO: comments, move it somewhere else
pub enum InputMsg {
    Plain { payload: String },
    Command { payload: ClientCommand },
}

/// TODO: custom error
impl InputMsg {
    pub fn build(payload: &str) -> Result<Self, String> {
        match payload.chars().nth(0) {
            Some(c) => {
                if c == '&' {
                    if payload == LIST {
                        return Ok(InputMsg::Command {
                            payload: ClientCommand::ListUsers,
                        });
                    } else {
                        return Err("Invalid command".into());
                    }
                } else {
                    return Ok(InputMsg::Plain {
                        payload: payload.into(),
                    });
                }
            }
            None => {
                return Err("Empty string has been provided.".into());
            }
        }
    }

    /// TODO: Description, refactor
    /// - writer: socket that writes to the server
    /// - output_tx: channel that displays output
    pub async fn action(
        &self,
        writer: &mut BufWriter<OwnedWriteHalf>,
        write_handler: &mut WriteHandler,
    ) -> Result<(), WriteHandlerError> {
        match self {
            InputMsg::Plain { payload } => {
                write_handler.write(&payload, writer).await?;
            }
            InputMsg::Command { payload } => match payload {
                ClientCommand::ListUsers => {
                    write_handler.write(LIST, writer).await?;
                }
            },
        }
        Ok(())
    }
}

pub enum ClientCommand {
    ListUsers,
}
