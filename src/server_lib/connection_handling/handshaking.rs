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
use crate::shared_lib::auxiliaries::error_chain_fmt;
use crate::shared_lib::socket_handling::{RecvHandler, RecvHandlerError, WriteHandler};
use aes_gcm::{Aes256Gcm, Key, KeyInit};
use rand::rngs::OsRng;
use rsa::pkcs1::EncodeRsaPublicKey;
use rsa::pkcs8::LineEnding;
use rsa::{Pkcs1v15Encrypt, RsaPrivateKey, RsaPublicKey};
use std::fmt::Debug;
use std::net::SocketAddr;
use tokio::io::{BufReader, BufWriter};
use tokio::net::tcp::{ReadHalf, WriteHalf};
use tokio::sync::mpsc;

use super::super::structs::{Client, ConnHandlerIdRecordMsg, IdRecordConnHandler};

/// # `handshake`
///
/// Procedure for accepting or refusing a connection,
/// will return a `Result` containig the nickname of
/// the new client, the channels that `id_record`
/// will use to send messages to `connection_handler` or an error;
/// the error may be fatal, if that is the case the application needs to shutdown.
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
/// client name, channel for receiving messages from `id_record` and channle
/// for receiving commands from `id_record`
/// TODO: comment
pub async fn handshake(
    write_handler: &mut WriteHandler<BufWriter<WriteHalf<'_>>>,
    read_handler: &mut RecvHandler<BufReader<ReadHalf<'_>>>,
    addr: SocketAddr,
    int_com_tx: &mpsc::Sender<ConnHandlerIdRecordMsg>,
    output_tx: &mpsc::Sender<OutputMsg>,
) -> Result<
    (
        String,
        mpsc::Receiver<IdRecordConnHandler>,
        mpsc::Receiver<CommandFromIdRecord>,
    ),
    HandshakeError,
> {
    let mut nick = String::new();
    let mut counter: u8 = 0;
    let (req_tx, mut req_rx) = mpsc::channel(10);
    let (mut command_tx, mut command_rx);

    // TODO: refactor here
    let mut rng = OsRng::default();
    let bits = 1024;
    let priv_key =
        RsaPrivateKey::new(&mut rng, bits).map_err(|e| HandshakeError::NonFatal(e.into()))?;
    let pub_key = RsaPublicKey::from(&priv_key);
    let pk_str = pub_key.to_pkcs1_pem(LineEnding::default()).unwrap();
    write_handler
        .write_str(&pk_str)
        .await
        .map_err(|e| HandshakeError::NonFatal(e.into()))?;
    let enc_key_bytes = read_handler
        .recv_bytes()
        .await
        .map_err(|e| HandshakeError::NonFatal(e.into()))?;
    let key_bytes = &priv_key
        .decrypt(Pkcs1v15Encrypt, &enc_key_bytes)
        .map_err(|e| HandshakeError::NonFatal(e.into()))?[..32];
    let aes_key = Key::<Aes256Gcm>::from_slice(key_bytes);
    let cipher = Aes256Gcm::new(&aes_key);
    write_handler.import_cipher(cipher.clone());
    read_handler.import_cipher(cipher);

    loop {
        counter += 1;

        if counter > MAX_TRIES {
            write_handler
                .write_str(TOO_MANY_TRIES)
                .await
                .map_err(|e| HandshakeError::NonFatal(e.into()))?;
            return Err(HandshakeError::NonFatal(anyhow::anyhow!(
                "User have tried to register too many times without success"
            )));
        }

        match read_handler.recv_str(&mut nick).await {
            Ok(_) => {
                nick = String::from(nick.trim());
                let len = nick.len();
                if len > MAX_LEN {
                    write_handler
                        .write_str(TOO_LONG)
                        .await
                        .map_err(|e| HandshakeError::NonFatal(e.into()))?;
                    continue;
                } else if len < 3 {
                    write_handler
                        .write_str(TOO_SHORT)
                        .await
                        .map_err(|e| HandshakeError::NonFatal(e.into()))?;
                    continue;
                } else {
                    (command_tx, command_rx) = mpsc::channel::<CommandFromIdRecord>(10);
                    let new_client =
                        Client::new(nick.clone(), addr.clone(), req_tx.clone(), command_tx);
                    let req = ConnHandlerIdRecordMsg::AcceptanceRequest(new_client);
                    int_com_tx
                        .send(req)
                        .await
                        .map_err(|e| HandshakeError::Fatal(e.into()))?;
                    let accepted = match req_rx.recv().await {
                        Some(a) => a,
                        None => {
                            let msg = "Unable to communicate with id_record properly from handshake, this shouldn't have happened";
                            let _ = output_tx.send(OutputMsg::new_error(&msg)).await;
                            return Err(HandshakeError::Fatal(anyhow::anyhow!(msg)));
                        }
                    };
                    match accepted {
                        IdRecordConnHandler::Acceptance(res) => {
                            if res {
                                write_handler
                                    .write_str(CONNECTION_ACCEPTED)
                                    .await
                                    .map_err(|e| HandshakeError::NonFatal(e.into()))?;
                                let p = format!("{} has been accepted as {}\n", addr, nick);
                                output_tx
                                    .send(OutputMsg::new(&p))
                                    .await
                                    .map_err(|e| HandshakeError::Fatal(e.into()))?;
                                break;
                            } else {
                                write_handler
                                    .write_str(TAKEN)
                                    .await
                                    .map_err(|e| HandshakeError::NonFatal(e.into()))?;
                                continue;
                            }
                        }
                        _ => {
                            let msg = "Unexpected response from `id_record` in `handshake`, this shouldn't have happend.";
                            let _ = output_tx.send(OutputMsg::new_error(&msg)).await;
                            return Err(HandshakeError::Fatal(anyhow::anyhow!(msg)));
                        }
                    }
                }
            }
            Err(e) => {
                match e {
                    RecvHandlerError::MalformedPacket(_) => {
                        write_handler
                            .write_str(MALFORMED_PACKET)
                            .await
                            .map_err(|e| HandshakeError::NonFatal(e.into()))?;
                    }
                    _ => {}
                }
                return Err(HandshakeError::NonFatal(e.into()));
            }
        }
    }

    Ok((nick, req_rx, command_rx))
}

#[derive(thiserror::Error)]
pub enum HandshakeError {
    #[error(transparent)]
    Fatal(anyhow::Error),
    #[error(transparent)]
    NonFatal(anyhow::Error),
}

impl Debug for HandshakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        error_chain_fmt(self, f)
    }
}
