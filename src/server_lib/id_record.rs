//! # id_record
//!
//! Functions relative to handling the record that keeps track of the clients connected.
use std::net::SocketAddr;

use tokio::sync::{
    broadcast,
    mpsc::{self, Receiver, Sender},
};

use crate::server_lib::id_record::auxiliaries::{receiving_from_hand, receiving_from_run};

use super::{
    structs::{Client, ConnHandlerIdRecordMsg, IdRecordRunMsg, Message, RunIdRecordMsg},
    OutputMsg, StdinRequest,
};

mod auxiliaries;

/// # id_record
///
/// This functions keeps track of the amount of clients connected at a given time; it communicates,
/// thanks to appropriate channels, with the main task and with
/// `crate::lib::server_lib::connection_handling::connection_handler`.
/// It's capable of accepting request and respond with data involving clients currently connected.
/// The administrator is capable through this of sending messages to other clients or commands to the server.
///
///
/// ## Notes
///
/// - Since the actual state could be represented just by a usize, and
/// so no I/O operation are required for this logic, I could have used a `Mutex`,
/// although, for practice, and also for an eventual expansion of the featurs of this element, I
/// decided to go for spawning a specific task for this specific role and use message passing.
///
///
/// ## Parameters:
///
/// - `max_connections` -> Maximum amount of connections allowed.
/// - `run_com_rx` -> Receiving channel from `run`, is used from run for querying the record.
/// - `run_com_tx` -> Sending channel to run, used to respond to the queries of run.
/// - `con_hand_rx` -> Receiving channel from the connection handlers.
/// - `con_hand_tx` -> Sends messages from the server to the clients.
/// - `output_tx` -> this channel is used to send the output of the server to a third entity.
/// - `stdin_req_tx` -> channel used to request information from stdin through `StdinRequest`.
#[tracing::instrument(
    name = "Id record thread is running",
    skip(
        max_connections,
        run_com_rx,
        run_com_tx,
        con_hand_rx,
        con_hand_tx,
        output_tx
    )
)]
pub async fn id_record(
    max_connections: usize,
    mut run_com_rx: Receiver<RunIdRecordMsg>,
    mut run_com_tx: Sender<IdRecordRunMsg>,
    mut con_hand_rx: Receiver<ConnHandlerIdRecordMsg>,
    con_hand_tx: broadcast::Sender<Message>,
    output_tx: mpsc::Sender<OutputMsg>,
    address: SocketAddr,
    stdin_req_tx: mpsc::Sender<StdinRequest>,
) {
    let mut clients: Vec<Client> = Vec::new();

    loop {
        tokio::select! {
            // receiving from run task
            opt = run_com_rx.recv() => {
                receiving_from_run(&mut run_com_tx, opt, clients.len(), max_connections).await;
            }
            // receiving from a connection handler
            opt = con_hand_rx.recv() => {
                receiving_from_hand(opt, &mut clients, &address, &con_hand_tx, &output_tx, &stdin_req_tx).await;
            }
        }
    }
}
