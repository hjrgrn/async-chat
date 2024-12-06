use std::fmt::{Debug, Display};

use std::error::Error;
use std::{io, usize};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter},
    net::{tcp::WriteHalf, TcpStream},
    sync::{mpsc, oneshot, oneshot::error::RecvError},
};

use crate::{
    client_lib::globals::CLIENT_COM,
    globals::{CONNECTION_ACCEPTED, TAKEN, TIMEOUT, TOO_LONG, TOO_MANY_TRIES, TOO_SHORT},
    shared_lib::{OutputMsg, StdinRequest},
};

/// TODO: desctiption, custom error, error handling
pub async fn handshake(
    stream: &mut TcpStream,
    stdin_req_tx: &mut mpsc::Sender<StdinRequest>,
    output_tx: &mut mpsc::Sender<OutputMsg>,
) -> Result<(), Box<dyn Error>> {
    let (mut reader, mut writer) = stream.split();
    let mut reader = BufReader::new(&mut reader);
    let mut writer = BufWriter::new(&mut writer);
    let mut response = String::new();

    output_tx
        .send(OutputMsg::new(
            "Welcome to rust async chat!\nType your nickname:",
        ))
        .await?;

    loop {
        response.clear();

        let (tx, rx) = oneshot::channel::<String>();
        let req = StdinRequest::Plain(tx);
        stdin_req_tx.send(req).await?;

        tokio::select! {
            // reading from stdin
            nick = rx => {
                send_nick(&mut writer, &nick, output_tx).await?;
            }
            // reading the response from the server
            r = reader.read_line(&mut response) => {
                match handles_response(output_tx, &r, &response).await {
                        Ok(keep_trying) => {
                            if keep_trying {
                                continue;
                            } else {
                                break;
                            }
                        },
                        Err(e) => {
                            return Err(e);
                        }
                    }
            }
        }
    }

    Ok(())
}

/// # `handshake`'s helper `send_nick`
///
/// Sends the nickname to the server, if an error is returned
/// the application cannot continue.
/// TODO: error handling, gracefull shutdown
async fn send_nick(
    writer: &mut BufWriter<&mut WriteHalf<'_>>,
    nick: &Result<String, RecvError>,
    output_tx: &mut mpsc::Sender<OutputMsg>,
) -> Result<(), Box<dyn Error>> {
    match nick {
        Ok(n) => {
            writer.write_all(n.as_bytes()).await?;
            writer.flush().await?;
        }
        Err(e) => {
            // invalid UTF-8
            output_tx
                .send(OutputMsg::new_error(&format!("{}, Retry", e)))
                .await?;
        }
    }
    Ok(())
}

/// # `handshake`'s helper `handles_response`
///
/// After reading sending the nickname to the server a response will be issued, outcome is the
/// result of this interation, response is the value that resulted from the interation.
/// The function returns an error if the connection has been interrupted, a bool otherwise,
/// it the bool is true we can keep trying and send another nickname, if the bool is false
/// it means the handshake has been successfull and the user can start using the chat.
///
/// ## Params
///
/// - output_tx: channel that displays eventual outputs
/// - outcome: result of the communication between client and server
/// - response: values returned by the server.
/// TODO: gracefull shutdown
async fn handles_response(
    output_tx: &mut mpsc::Sender<OutputMsg>,
    outcome: &io::Result<usize>,
    response: &str,
) -> Result<bool, Box<dyn Error>> {
    match outcome {
        Ok(0) => {
            output_tx
                .send(OutputMsg::new_error("Connection reset by server."))
                .await?;
            return Err(Box::new(HandshakeError));
        }
        Ok(_) => {
            if response == CONNECTION_ACCEPTED {
                output_tx.send(OutputMsg::new(&format!("You have been accepted, type \"{}\" for displaying all the avaible commands", CLIENT_COM.trim()))).await?;
                return Ok(false);
            } else if response == TOO_MANY_TRIES {
                output_tx
                    .send(OutputMsg::new("Connection refused due to: too many tries."))
                    .await?;
                return Err(Box::new(HandshakeError));
            } else if response == TOO_SHORT {
                output_tx
                    .send(OutputMsg::new("The nickname you chose is to short, retry."))
                    .await?;
            } else if response == TOO_LONG {
                output_tx
                    .send(OutputMsg::new("The nickname you chose is to long, retry."))
                    .await?;
            } else if response == TIMEOUT {
                output_tx
                    .send(OutputMsg::new("Timeout reached, try and reconnect."))
                    .await?;
                return Err(Box::new(HandshakeError));
            } else if response == TAKEN {
                output_tx
                    .send(OutputMsg::new(&format!(
                        "The nickname has already been taken, please choose another one.",
                    )))
                    .await?;
            } else {
                output_tx.send(OutputMsg::new("Retry")).await?;
            }
        }
        Err(e) => {
            output_tx
                .send(OutputMsg::new_error(&format!("{}", e)))
                .await?;

            return Err(Box::new(HandshakeError));
        }
    }
    Ok(true)
}

/// TODO: everything, waiting for anyhow
pub struct HandshakeError;
impl Display for HandshakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Failed to perform the handshake")
    }
}
impl Debug for HandshakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Failed to perform the handshake")
    }
}
impl std::error::Error for HandshakeError {}