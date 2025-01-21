use std::fmt::{Debug, Display};

use aes_gcm::{Aes256Gcm, KeyInit};
use rand::rngs::OsRng;
use rsa::{pkcs1::DecodeRsaPublicKey, Pkcs1v15Encrypt, RsaPublicKey};
use tokio::{
    io::{BufReader, BufWriter},
    net::tcp::{OwnedReadHalf, OwnedWriteHalf},
    sync::{mpsc, oneshot},
};

use crate::shared_lib::socket_handling::{RecvHandler, RecvHandlerError, WriteHandler};
use crate::{
    client_lib::globals::CLIENT_COM,
    globals::{CONNECTION_ACCEPTED, TAKEN, TIMEOUT, TOO_LONG, TOO_MANY_TRIES, TOO_SHORT},
    shared_lib::{OutputMsg, StdinRequest},
};

/// TODO: comment, custom error, error handling
pub async fn handshake(
    write_handler: &mut WriteHandler<BufWriter<OwnedWriteHalf>>,
    read_handler: &mut RecvHandler<BufReader<OwnedReadHalf>>,
    stdin_req_tx: &mut mpsc::Sender<StdinRequest>,
    output_tx: &mut mpsc::Sender<OutputMsg>,
) -> Result<(), anyhow::Error> {
    let mut response = String::new();

    key_exchange(read_handler, write_handler).await?;

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
                match nick {
                    Ok(n) => {
                        write_handler.write_str(&n).await?;
                    }
                    Err(e) => {
                        output_tx.send(OutputMsg::new_error(&format!("{}, Retry.", e))).await?;
                    }
                }
            }
            // reading the response from the server
            r = read_handler.recv_str(&mut response) => {
                match handles_response(output_tx, &r, &response).await {
                        Ok(keep_trying) => {
                            if keep_trying {
                                continue;
                            } else {
                                break;
                            }
                        },
                        Err(e) => {
                            return Err(e.into());
                        }
                    }
            }
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
async fn handles_response(
    output_tx: &mut mpsc::Sender<OutputMsg>,
    outcome: &Result<(), RecvHandlerError>,
    response: &str,
) -> Result<bool, anyhow::Error> {
    match outcome {
        Ok(_) => {
            if response == CONNECTION_ACCEPTED {
                output_tx.send(OutputMsg::new(&format!("You have been accepted, type \"{}\" for displaying all the avaible commands", CLIENT_COM.trim()))).await?;
                return Ok(false);
            } else if response == TOO_MANY_TRIES {
                output_tx
                    .send(OutputMsg::new("Connection refused due to: too many tries."))
                    .await?;
                return Err(HandshakeError.into());
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
                return Err(HandshakeError.into());
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
            match e {
                RecvHandlerError::ConnectionInterrupted => {
                    output_tx
                        .send(OutputMsg::new_error("Connection reset by server."))
                        .await?;
                    return Err(HandshakeError.into());
                }
                _others => {}
            }

            output_tx
                .send(OutputMsg::new_error(&format!("{}", e)))
                .await?;

            return Err(HandshakeError.into());
        }
    }
    Ok(true)
}

async fn key_exchange(
    read_handler: &mut RecvHandler<BufReader<OwnedReadHalf>>,
    write_handler: &mut WriteHandler<BufWriter<OwnedWriteHalf>>,
) -> Result<(), anyhow::Error> {
    let mut response = String::new();
    read_handler.recv_str(&mut response).await?;
    let mut rng = OsRng::default();
    let pub_rsa_key = RsaPublicKey::from_pkcs1_pem(&response)?;
    let aes_key = Aes256Gcm::generate_key(&mut rng);
    let key_bytes: [u8; 32] = aes_key.try_into()?;
    let rsa_enc_aes_key = pub_rsa_key.encrypt(&mut rng, Pkcs1v15Encrypt, &key_bytes[..])?;
    write_handler.write_bytes(&rsa_enc_aes_key).await?;
    let cipher = Aes256Gcm::new(&aes_key);
    write_handler.import_cipher(cipher.clone());
    read_handler.import_cipher(cipher);
    Ok(())
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
