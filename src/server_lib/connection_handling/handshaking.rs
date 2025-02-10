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
use core::str;
use hmac::{Hmac, Mac};
use rand::distributions::Alphanumeric;
use rand::rngs::OsRng;
use rand::Rng;
use rsa::pkcs1::EncodeRsaPublicKey;
use rsa::pkcs8::LineEnding;
use rsa::{Pkcs1v15Encrypt, RsaPrivateKey, RsaPublicKey};
use secrecy::{ExposeSecret, SecretString};
use sha2::Sha256;
use std::char;
use std::fmt::Debug;
use std::net::SocketAddr;
use tokio::io::{BufReader, BufWriter};
use tokio::net::tcp::{ReadHalf, WriteHalf};
use tokio::sync::mpsc;

use super::super::structs::{Client, ConnHandlerIdRecordMsg, IdRecordConnHandler};

/// # `handshake`
///
/// Procedure for accepting or refusing a connection.
/// During this procedure all the information necessary for the
/// communication will be exchanged and set.
/// In this fase `write_handler` and `read_handler` will be provided with the
/// cipher for encrypted communication, and endpoints are authenticated.
/// It will return a `Result` containig the nickname of
/// the new client, the channels that `id_record`
/// will use to send messages to `connection_handler` or an error;
/// the error may be fatal, if that is the case the application needs to shutdown.
///
/// ## Parameters
///
/// - `write_handler`: handles the write part of the socket.
/// - `read_handler`: handles the read part of the socket.
/// - `addr: SocketAddr`: ip address and port used by the client.
/// - `int_com_tx: &mpsc::Sender<ConnHandlerIdRecordMsg>`: transmitter used to send messages to the
/// `id_record`.
/// - `output_tx`: Channel used to communicate the output with third parties.
/// - `shared_secret`: Secret needed for authenticate the users during handshake.
///
/// ## Returns
///
/// client name, channel for receiving messages from `id_record` and channle
/// for receiving commands from `id_record`
pub async fn handshake(
    write_handler: &mut WriteHandler<BufWriter<WriteHalf<'_>>>,
    read_handler: &mut RecvHandler<BufReader<ReadHalf<'_>>>,
    addr: SocketAddr,
    int_com_tx: &mpsc::Sender<ConnHandlerIdRecordMsg>,
    output_tx: &mpsc::Sender<OutputMsg>,
    shared_secret: SecretString,
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

    key_exchange(write_handler, read_handler, shared_secret).await?;

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

/// # `handshake`'s helper `key_exchange`
///
/// This function uses the socket handlers and RSA for exchanging a AES key
/// that will be used to produce a cipher, which will be assigned to both
/// the handlers.
/// This function may return an error that can be fatal, in this case the
/// application needs to shutdown.
async fn key_exchange(
    write_handler: &mut WriteHandler<BufWriter<WriteHalf<'_>>>,
    read_handler: &mut RecvHandler<BufReader<ReadHalf<'_>>>,
    shared_secret: SecretString,
) -> Result<(), HandshakeError> {
    let mut hmac = Hmac::<Sha256>::new_from_slice(shared_secret.expose_secret().as_bytes())
        .map_err(|e| HandshakeError::NonFatal(e.into()))?;

    let mut rng = OsRng::default();
    // generate nonce
    let mut body: String = rng
        .sample_iter(Alphanumeric)
        .take(24)
        .map(char::from)
        .collect();
    let sent_nonce = body.clone();

    let bits = 1024;
    let priv_key =
        RsaPrivateKey::new(&mut rng, bits).map_err(|e| HandshakeError::NonFatal(e.into()))?;
    let pub_key = RsaPublicKey::from(&priv_key);
    let pk_str = pub_key
        .to_pkcs1_pem(LineEnding::default())
        .map_err(|e| HandshakeError::Fatal(e.into()))?;
    // nonce + pem, size 24 + 251
    body.push_str(&pk_str);

    write_handler
        .write_str(&body)
        .await
        .map_err(|e| HandshakeError::NonFatal(e.into()))?;

    let res = read_handler
        .recv_bytes()
        .await
        .map_err(|e| HandshakeError::NonFatal(e.into()))?;
    if res.len() != 184 {
        return Err(HandshakeError::NonFatal(anyhow::anyhow!(
            "Malformed packet received during key_exchange."
        )));
    }
    let (received_nonce, hash_key_seq) =
        res.split_at_checked(24)
            .ok_or(HandshakeError::NonFatal(anyhow::anyhow!(
                "Failed to receive nonce/hash/key"
            )))?;
    let (received_rsa_hash, enc_key_hmac_seq) =
        hash_key_seq
            .split_at_checked(32)
            .ok_or(HandshakeError::NonFatal(anyhow::anyhow!(
                "Failed to receive nonce/hash/key"
            )))?;

    Mac::update(&mut hmac, &sent_nonce.as_bytes());
    Mac::update(&mut hmac, &pk_str.as_bytes());
    Mac::verify_slice(hmac, received_rsa_hash).map_err(|e| HandshakeError::NonFatal(e.into()))?;

    let key_hmac_seq = &priv_key
        .decrypt(Pkcs1v15Encrypt, &enc_key_hmac_seq)
        .map_err(|e| HandshakeError::NonFatal(e.into()))?;
    let (key_bytes, hmac_seq) =
        key_hmac_seq
            .split_at_checked(32)
            .ok_or(HandshakeError::NonFatal(anyhow::anyhow!(
                "Failed to receive nonce/hash/key"
            )))?;
    let (hmac_secret_key, seq_num_bytes) =
        hmac_seq
            .split_at_checked(32)
            .ok_or(HandshakeError::NonFatal(anyhow::anyhow!(
                "Failed to receive nonce/hash/key"
            )))?;

    let seq_num_str =
        str::from_utf8(seq_num_bytes).map_err(|e| HandshakeError::NonFatal(e.into()))?;
    let seq_num = seq_num_str
        .parse::<u32>()
        .map_err(|e| HandshakeError::NonFatal(e.into()))?;

    let aes_key = aes_gcm::Key::<aes_gcm::Aes256Gcm>::from_slice(key_bytes);

    let mut hmac = Hmac::<Sha256>::new_from_slice(shared_secret.expose_secret().as_bytes())
        .map_err(|e| HandshakeError::NonFatal(e.into()))?;
    Mac::update(&mut hmac, &received_nonce);
    Mac::update(&mut hmac, &aes_key);
    Mac::update(&mut hmac, &hmac_secret_key);
    Mac::update(&mut hmac, &seq_num_bytes);
    Mac::update(&mut hmac, &sent_nonce.as_bytes());
    Mac::update(&mut hmac, &pk_str.as_bytes());
    let hash = Mac::finalize(hmac).into_bytes();

    write_handler
        .write_bytes(&hash)
        .await
        .map_err(|e| HandshakeError::NonFatal(e.into()))?;

    write_handler
        .import_safety_tools(&aes_key, &hmac_secret_key, seq_num)
        .map_err(HandshakeError::Fatal)?;
    read_handler
        .import_safety_tools(&aes_key, &hmac_secret_key, seq_num)
        .map_err(HandshakeError::Fatal)?;

    Ok(())
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
