use aes_gcm::{Aes256Gcm, KeyInit};
use anyhow::Context;
use hmac::{Hmac, Mac};
use rand::{distributions::Alphanumeric, rngs::OsRng, Rng};
use rsa::{pkcs1::DecodeRsaPublicKey, Pkcs1v15Encrypt, RsaPublicKey};
use secrecy::{ExposeSecret, SecretString};
use sha2::Sha256;
use tokio::{
    io::{BufReader, BufWriter},
    net::tcp::{OwnedReadHalf, OwnedWriteHalf},
    sync::{mpsc, oneshot},
};

use crate::shared_lib::socket_handling::{
    RecvHandler, RecvHandlerError, WriteHandler, HMAC_KEY_SIZE,
};
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
    shared_secret: SecretString,
) -> Result<(), anyhow::Error> {
    let mut response = String::new();

    key_exchange(read_handler, write_handler, shared_secret).await?;

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
                let msg = "Connection refused due to: too many tries.";
                output_tx.send(OutputMsg::new(msg)).await?;
                return Err(anyhow::anyhow!(msg));
            } else if response == TOO_SHORT {
                output_tx
                    .send(OutputMsg::new("The nickname you chose is to short, retry."))
                    .await?;
            } else if response == TOO_LONG {
                output_tx
                    .send(OutputMsg::new("The nickname you chose is to long, retry."))
                    .await?;
            } else if response == TIMEOUT {
                let msg = "Timeout reached, try and reconnect.";
                output_tx.send(OutputMsg::new(msg)).await?;
                return Err(anyhow::anyhow!(msg));
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
                    let msg = "Connection reset by server.";
                    output_tx
                        .send(OutputMsg::new_error("Connection reset by server."))
                        .await?;
                    return Err(anyhow::anyhow!(msg));
                }
                _others => {}
            }

            output_tx
                .send(OutputMsg::new_error(&format!("{}", e)))
                .await?;

            return Err(anyhow::anyhow!(format!("{}", e)));
        }
    }
    Ok(true)
}

async fn key_exchange(
    read_handler: &mut RecvHandler<BufReader<OwnedReadHalf>>,
    write_handler: &mut WriteHandler<BufWriter<OwnedWriteHalf>>,
    shared_secret: SecretString,
) -> Result<(), anyhow::Error> {
    let mut hmac = <Hmac<Sha256> as Mac>::new_from_slice(shared_secret.expose_secret().as_bytes())?;

    let mut response = String::new();
    read_handler.recv_str(&mut response).await?;
    // sizes: 24, 251
    let (received_nonce, pub_rsa_key_str) = response
        .split_at_checked(24)
        .context("Received invalid nonce/pub rsa")?;

    let mut rng = OsRng::default();
    let pub_rsa_key = RsaPublicKey::from_pkcs1_pem(pub_rsa_key_str)?;

    // nonce + pub_rsa_key_str hash using secret
    hmac.update(received_nonce.as_bytes());
    hmac.update(pub_rsa_key_str.as_bytes());
    // len 32
    let mut n_pr_hash = hmac.finalize().into_bytes().to_vec();
    // building nonce
    let mut body: Vec<u8> = rng.sample_iter(Alphanumeric).take(24).collect();
    let sent_nonce = body.clone();

    let aes_key = Aes256Gcm::generate_key(&mut rng);
    let key_bytes: [u8; 32] = aes_key.try_into()?;

    let mut seq_num_packet: [u8; 10] = [0; 10];
    let seq_num = rng.gen::<u32>();
    let seq_number_string = seq_num.to_string();
    let seq_number_bytes = seq_number_string.as_bytes();
    let mut index = 0;
    for i in 10 - seq_number_bytes.len()..10 {
        seq_num_packet[i] = seq_number_bytes[index];
        index = index + 1;
    }

    let hmac_secret_key: [u8; HMAC_KEY_SIZE] = rng.gen();

    let mut payload: [u8; 74] = [0; 74];
    payload[..32].copy_from_slice(&key_bytes);
    payload[32..64].copy_from_slice(&hmac_secret_key);
    payload[64..].copy_from_slice(&seq_num_packet);

    let mut rsa_enc_aes_hamc_seq = pub_rsa_key.encrypt(&mut rng, Pkcs1v15Encrypt, &payload[..])?;
    body.append(&mut n_pr_hash);
    body.append(&mut rsa_enc_aes_hamc_seq);
    write_handler.write_bytes(&body).await?;

    let res = read_handler.recv_bytes().await?;
    let mut hmac = <Hmac<Sha256> as Mac>::new_from_slice(shared_secret.expose_secret().as_bytes())?;
    hmac.update(&sent_nonce);
    hmac.update(&aes_key);
    hmac.update(&hmac_secret_key);
    hmac.update(&seq_num_packet);
    hmac.update(received_nonce.as_bytes());
    hmac.update(pub_rsa_key_str.as_bytes());
    hmac.verify_slice(&res)?;

    write_handler.import_safety_tools(&aes_key, &hmac_secret_key, seq_num)?;
    read_handler.import_safety_tools(&aes_key, &hmac_secret_key, seq_num)?;

    Ok(())
}
