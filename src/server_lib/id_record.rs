//! # id_record
//!
//! Functions relative to handling the record that keeps track of the clients connected.
use std::net::SocketAddr;

use tokio::sync::{
    broadcast,
    mpsc::{self, Receiver, Sender},
};
use tokio_util::sync::CancellationToken;

use crate::server_lib::id_record::utils::{receiving_from_hand, receiving_from_run};

use super::{
    structs::{Client, ConnHandlerIdRecordMsg, IdRecordRunMsg, Message, RunIdRecordMsg},
    OutputMsg, StdinRequest,
};

mod utils;

/// # id_record
///
/// This function keeps track of the number of clients connected at a given time. It communicates,
/// via appropriate channels, with the main task and with
/// `crate::lib::server_lib::connection_handling::connection_handler`.
/// It accepts requests and responds with data regarding currently connected clients.
/// Through this, the administrator can send messages to other clients or commands to the server.
///
/// ## Notes
///
/// - Since the actual state could be represented simply by a `usize` (requiring no I/O operations),
///   I could have used a `Mutex`. However, for practice and to allow for future expansion of this
///   feature, I decided to spawn a dedicated task for this role and use message passing.
///
/// ## Parameters
///
/// - `max_connections`: Maximum number of connections allowed.
/// - `run_com_rx`: Receiving channel from `run`; used by `run` to query the record.
/// - `run_com_tx`: Sending channel to `run`; used to respond to queries from `run`.
/// - `con_hand_rx`: Receiving channel from the connection handlers.
/// - `con_hand_tx`: Sends messages from the server to the clients.
/// - `output_tx`: Channel used to send server output to a third entity.
/// - `stdin_req_tx`: Channel used to request information from stdin via `StdinRequest`.
/// - `ctoken`: Cancellation token used to signal shutdown.
#[allow(clippy::too_many_arguments)] // TODO: solve this
#[tracing::instrument(name = "Id record thread is running", skip_all)]
pub async fn id_record(
    max_connections: usize,
    mut run_com_rx: Receiver<RunIdRecordMsg>,
    mut run_com_tx: Sender<IdRecordRunMsg>,
    mut con_hand_rx: Receiver<ConnHandlerIdRecordMsg>,
    con_hand_tx: broadcast::Sender<Message>,
    output_tx: mpsc::Sender<OutputMsg>,
    address: SocketAddr,
    stdin_req_tx: mpsc::Sender<StdinRequest>,
    ctoken: CancellationToken,
) {
    // TODO: this should probably be a map
    let mut clients: Vec<Client> = Vec::new();

    loop {
        tokio::select! {
            // Receiving from run task.
            opt = run_com_rx.recv() => {
                let msg = match opt {
                    Some(m) => {m},
                    None => {
                        let msg = "id_record is unable to communicate with `run`";
                        let _ = output_tx.send(OutputMsg::new_error(msg)).await;
                        // TODO: tracing should probably live in OutputMsg::print()?
                        tracing::error!("{}", msg);
                        break;
                    }
                };
                match receiving_from_run(&mut run_com_tx, msg, clients.len(), max_connections).await {
                    Ok(()) => {}
                    Err(e) => {
                        let _ = output_tx.send(OutputMsg::new_error(e.to_string())).await;
                        tracing::error!("id_record can't work anymore:\n{:?}", e);
                        break;
                    }
                };
            }
            // Receiving from a connection handler.
            opt = con_hand_rx.recv() => {
                let msg = match opt {
                    Some(m) => {m}
                    None => {
                        let msg = "id_record is unable to communicate with a connection handler";
                        let _ = output_tx.send(OutputMsg::new_error(msg)).await;
                        tracing::error!("{}", msg);
                        break;
                    }
                };
                // FROMHERE:
                match receiving_from_hand(msg, &mut clients, &address, &con_hand_tx, &output_tx, &stdin_req_tx).await {
                    Ok(()) => {}
                    Err(e) => {
                        let _ = output_tx.send(OutputMsg::new_error(e.to_string())).await;
                        tracing::error!("id_record can't work anymore:\n{:?}", e);
                        break;
                    }
                }
            }
        }
    }
    ctoken.cancel();
}
