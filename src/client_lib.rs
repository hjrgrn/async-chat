use std::collections::VecDeque;

use globals::CLIENT_COM;
use handshaking::handshake;
use tokio::{
    io::{stdin, AsyncBufReadExt, BufReader, BufWriter},
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpStream,
    },
    select,
    sync::mpsc,
};

use crate::{
    client_lib::{globals::COMMANDS, sending_messages::handling_stdin_input, settings::Settings},
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

/// TODO: comment, telemetry, error handling
pub async fn run(
    settings: Settings,
    mut output_tx: mpsc::Sender<OutputMsg>,
    input_rx: mpsc::Receiver<InputMsg>,
    mut stdin_req_tx: mpsc::Sender<StdinRequest>,
) {
    let mut stream = match TcpStream::connect(settings.get_full_address()).await {
        Ok(s) => s,
        Err(e) => {
            let err_msg = OutputMsg::new_error(e);
            output_tx.send(err_msg).await.unwrap();
            return;
        }
    };

    if handshake(&mut stream, &mut stdin_req_tx, &mut output_tx)
        .await
        .is_err()
    {
        return;
    };

    let (read_half, write_half) = stream.into_split();
    let input_handle = tokio::spawn(handling_stdin_input(write_half, input_rx));
    let recv_handle = tokio::spawn(recv_msg(read_half, output_tx));

    let _ = input_handle.await;
    let _ = recv_handle.await;
}

/// TODO: Description, error handling and propagation, graceful shutdown
async fn recv_msg(reader: OwnedReadHalf, output_tx: mpsc::Sender<OutputMsg>) {
    let mut reader = BufReader::new(reader);
    let mut recv_handler = RecvHandler::new();

    let mut response = String::new();

    loop {
        match recv_handler.recv(&mut response, &mut reader).await {
            Ok(_) => {
                output_tx.send(OutputMsg::new(&response)).await.unwrap();
            }
            Err(e) => {
                eprintln!("{}", e);
                break;
            }
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
pub async fn client_commands(
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
            if content == CLIENT_COM {
                output_tx.send(OutputMsg::new(COMMANDS)).await.unwrap();
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
                                    input_tx.send(m).await.unwrap();
                                }
                                Err(e) => {
                                    output_tx.send(OutputMsg::new_error(&e)).await.unwrap();
                                }
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
