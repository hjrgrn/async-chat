use std::collections::VecDeque;

use globals::CLIENT_COM;
use handshaking::handshake;
use secrecy::SecretString;
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

/// # `run`'s wrapper
///
/// Allows graceful shutdown to be porformed
pub async fn run_wrapper(
    settings: Settings,
    output_tx: mpsc::Sender<OutputMsg>,
    input_rx: mpsc::Receiver<InputMsg>,
    stdin_req_tx: mpsc::Sender<StdinRequest>,
    ctoken: CancellationToken,
    shared_secret: SecretString,
) {
    tokio::select! {
        _ = ctoken.cancelled() => {}
        res = run(settings, output_tx, input_rx, stdin_req_tx, ctoken.clone(), shared_secret) => {
                match res {
                    Ok(_) => {}
                    Err(e) => {
                        tracing::error!("`run` can't work anymore:\n{:?}", e);
                    }
                }
                ctoken.cancel();
            }
    }
}

/// # `run`
///
/// Runs the main task of the client application.
///
///
/// ## Parameters
///
/// settings: application settings
/// output_tx: channel for comunicating output to be displayed
/// input_rx: channel that receives input from the user
/// stdin_req_tx: channel for requesting informations from stdin
/// ctoken: cancellation token
/// shared_secret: Secret needed during authentication
#[tracing::instrument(
    name = "Client main task is running",
    skip(settings, output_tx, input_rx, stdin_req_tx, ctoken, shared_secret)
)]
async fn run(
    settings: Settings,
    mut output_tx: mpsc::Sender<OutputMsg>,
    input_rx: mpsc::Receiver<InputMsg>,
    mut stdin_req_tx: mpsc::Sender<StdinRequest>,
    ctoken: CancellationToken,
    shared_secret: SecretString,
) -> Result<(), anyhow::Error> {
    let stream = match TcpStream::connect(settings.get_full_address()).await {
        Ok(s) => s,
        Err(e) => {
            let err_msg = OutputMsg::new_error(&e);
            let _ = output_tx.send(err_msg).await;
            return Err(e.into());
        }
    };

    let (read, write) = stream.into_split();
    let reader = BufReader::new(read);
    let writer = BufWriter::new(write);
    let mut write_handler = WriteHandler::new(writer);
    let mut read_handler = RecvHandler::new(reader);

    match handshake(
        &mut write_handler,
        &mut read_handler,
        &mut stdin_req_tx,
        &mut output_tx,
        shared_secret
    )
    .await
    {
        Ok(_) => {}
        Err(e) => {
            let _ = output_tx.send(OutputMsg::new_error(&e)).await;
            return Err(e.into());
        }
    }

    let input_handle = tokio::spawn(handling_stdin_input_wrapper(
        write_handler,
        input_rx,
        output_tx.clone(),
        ctoken.clone(),
    ));
    let recv_handle = tokio::spawn(recv_msg_wrapper(read_handler, output_tx, ctoken));

    let _ = input_handle.await;
    let _ = recv_handle.await;
    Ok(())
}

/// TODO: move this somewhere
async fn recv_msg_wrapper(
    mut read_handler: RecvHandler<BufReader<OwnedReadHalf>>,
    output_tx: mpsc::Sender<OutputMsg>,
    ctoken: CancellationToken,
) {
    tokio::select! {
        _ = ctoken.cancelled() => {}
        res = recv_msg(&mut read_handler, output_tx) => {
                match res {
                    Ok(_) => {}
                    Err(e) => {
                        tracing::error!("`recv_msg` can't work anymore:\n{:?}", e);
                    }
                }
                ctoken.cancel();
            }
    }
}

/// # `recv_msg`
///
/// Receiving messages from the server
///
///
/// ## Patameters
///
/// reader: half of the sockets that receives from the server
/// output_tx: channel for displaying output
/// TODO: comment
#[tracing::instrument(name = "Receiving from server", skip(read_handler, output_tx))]
async fn recv_msg(
    read_handler: &mut RecvHandler<BufReader<OwnedReadHalf>>,
    output_tx: mpsc::Sender<OutputMsg>,
) -> Result<(), anyhow::Error> {
    let mut response = String::new();

    loop {
        match read_handler.recv_str(&mut response).await {
            Ok(_) => {
                match output_tx.send(OutputMsg::new(&response)).await {
                    Ok(_) => {}
                    Err(e) => {
                        return Err(e.into());
                    }
                };
            }
            Err(e) => {
                let _ = output_tx.send(OutputMsg::new_error(&e)).await;
                return Err(e.into());
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
        res = client_commands(
                input_tx,
                req_rx,
                output_tx,
            ) => {
                match res {
                    Ok(_) => {}
                    Err(e) => {
                        tracing::error!("`client_commands` can't work anymore:\n{:?}", e)
                    }
                }
                ctoken.cancel();
            }
    }
}

/// TODO: move the code somewhere else, duplication in `server_lib::server_commands`
/// #`client_commands`
///
/// Handles inputs from stdin.
/// Receives request for reading from stdin through `req_rx`, at the same time allows the user to
/// type.
/// After the user finished typing, if there is a pending request to stdin the content written by
/// the user will be sent to the requester through the oneshot channel inside `StdinRequest`; if
/// there are no requests pending the content will be sent to `id_record` through `comm_tx`,
/// becouse it is assumed to be a command issued by the admin.
/// The command `CLIENT_COM` will be sent directly to the function that displays the output through
/// `output_tx`.
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
#[tracing::instrument(
    name = "Receiving commands from user",
    skip(input_tx, req_rx, output_tx)
)]
async fn client_commands(
    input_tx: mpsc::Sender<InputMsg>,
    mut req_rx: mpsc::Receiver<StdinRequest>,
    output_tx: mpsc::Sender<OutputMsg>,
) -> Result<(), anyhow::Error> {
    let mut typer = BufReader::new(stdin());
    let mut content = String::new();

    let mut requests: VecDeque<StdinRequest> = VecDeque::new();

    loop {
        select! {
            res = typer.read_line(&mut content) => {
                match res {
                    Ok(_) => {},
                    Err(e) => {
                        let _ = output_tx.send(OutputMsg::new_error(&e)).await;
                        return Err(e.into());
                    }
                }
            }
            res = req_rx.recv() => {
                let r = match res {
                    Some(r) => {r},
                    None => {
                        let msg = "Failed to send a stdin response in `client_commands`";
                        let _ = output_tx.send(OutputMsg::new_error(&msg)).await;
                        return Err(anyhow::anyhow!(msg));
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
                output_tx.send(OutputMsg::new(COMMANDS)).await?;
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
                                            let _ = output_tx.send(OutputMsg::new_error(&e)).await;
                                            return Err(e.into());
                                        }
                                    };
                                }
                                Err(e) => output_tx.send(OutputMsg::new_error(&e)).await?,
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

impl InputMsg {
    pub fn build(payload: &str) -> Result<Self, anyhow::Error> {
        match payload.chars().nth(0) {
            Some(c) => {
                if c == '&' {
                    if payload == LIST {
                        return Ok(InputMsg::Command {
                            payload: ClientCommand::ListUsers,
                        });
                    } else {
                        return Err(anyhow::anyhow!("Invalid command"));
                    }
                } else {
                    return Ok(InputMsg::Plain {
                        payload: payload.into(),
                    });
                }
            }
            None => {
                return Err(anyhow::anyhow!("Empty string has been provided."));
            }
        }
    }

    /// `action`
    ///
    /// This method call the action appropriate for a specific variant.
    /// Usually communicates with the server.
    ///
    /// - writer: socket that writes to the server
    /// - write_handler: handler that uses `writer`
    pub async fn action(
        &self,
        write_handler: &mut WriteHandler<BufWriter<OwnedWriteHalf>>,
    ) -> Result<(), WriteHandlerError> {
        match self {
            InputMsg::Plain { payload } => {
                tracing::info!("Writing plain string to server");
                write_handler.write_str(&payload).await?;
            }
            InputMsg::Command { payload } => match payload {
                ClientCommand::ListUsers => {
                    tracing::info!("Writing LIST command to server");
                    write_handler.write_str(LIST).await?;
                }
            },
        }
        Ok(())
    }
}

// TODO: comment
pub enum ClientCommand {
    ListUsers,
}
