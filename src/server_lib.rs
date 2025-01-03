use std::net::SocketAddr;

use connection_handling::connection_handler_wrapper;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use crate::server_lib::{settings::Settings, structs::Message};
use crate::shared_lib::{OutputMsg, StdinRequest};

use self::id_record::id_record;
pub use self::structs::{ConnHandlerIdRecordMsg, IdRecordRunMsg, RunIdRecordMsg};

pub mod administration;
mod connection_handling;
mod id_record;
pub mod settings;
mod structs;

pub async fn run_wrapper(
    settings: Settings,
    con_hand_id_tx: mpsc::Sender<ConnHandlerIdRecordMsg>,
    con_hand_id_rx: mpsc::Receiver<ConnHandlerIdRecordMsg>,
    output_tx: mpsc::Sender<OutputMsg>,
    stdin_req_tx: mpsc::Sender<StdinRequest>,
    ctoken: CancellationToken,
) {
    tokio::select! {
        _ = ctoken.cancelled() => {}
        _ = run(
            settings,
            con_hand_id_tx,
            con_hand_id_rx,
            output_tx,
            stdin_req_tx,
            ctoken.clone(),
        ) => {
            ctoken.cancel();
        }
    }
}

/// # Run
///
/// Runs the server, listens from incoming connection, if there is space for a connection spawns a
/// `connection_handler` specific for the connection.
/// A Sender of the type `mpsc::Sender<OutputMsg>` is used to communicate with the
/// function that displays the content.
/// Spawns the task `id_record`, that handles the clients connected.
///
///
/// ## Parameters
///
/// - `con_hand_id_tx` -> sender channel used to communicate with `id_record`, the user manager: connection_handler to
/// id_record.
/// - `con_hand_id_rx` -> receiver channel used to communicate with id_record: connection_handler
/// to id_record
/// - `output_tx` -> this channel is used to send the output of the server to a third entity.
/// - `stdin_req_tx` -> channel used to request information from stdin through `StdinRequest`.
/// - `ctoken` -> Cancellation token used to communicate the shutdown
/// TODO: telemetry, error handling
#[tracing::instrument(
    name = "Server is running",
    skip(settings, con_hand_id_tx, con_hand_id_rx, output_tx)
)]
async fn run(
    settings: Settings,
    con_hand_id_tx: mpsc::Sender<ConnHandlerIdRecordMsg>,
    con_hand_id_rx: mpsc::Receiver<ConnHandlerIdRecordMsg>,
    output_tx: mpsc::Sender<OutputMsg>,
    stdin_req_tx: mpsc::Sender<StdinRequest>,
    ctoken: CancellationToken,
) {
    if output_tx
        .send(OutputMsg::new("Listening..."))
        .await
        .is_err()
    {
        return;
    }
    let listener = match TcpListener::bind(&settings.get_full_address()).await {
        Ok(l) => l,
        Err(e) => {
            let _ = output_tx.send(OutputMsg::new_error(e.to_string())).await;
            return;
        }
    };

    // IdRecord
    // channels
    // run to id_record
    let (run_id_com_tx, run_id_com_rx) = mpsc::channel::<RunIdRecordMsg>(10);
    // id_record to run
    let (id_run_com_tx, mut id_run_com_rx) = mpsc::channel::<IdRecordRunMsg>(10);

    // Server channel
    // internal communication between `connection_handler`s
    let (int_com_tx, _) = broadcast::channel::<Message>(10);
    let id_msg_tx1 = int_com_tx.clone();

    let addr: SocketAddr = match settings.get_full_address().parse() {
        Ok(a) => a,
        Err(e) => {
            let _ = output_tx.send(OutputMsg::new_error(e.to_string())).await;
            return;
        }
    };

    tokio::spawn(id_record(
        settings.get_max_connections(),
        run_id_com_rx,
        id_run_com_tx,
        con_hand_id_rx,
        id_msg_tx1,
        output_tx.clone(),
        addr,
        stdin_req_tx.clone(),
        ctoken.clone(),
    ));

    loop {
        // internal communication between `connection_handler`s subfunctions
        let int_com_tx1 = int_com_tx.clone();
        let int_com_rx = int_com_tx.subscribe();
        // communication with id_record
        let con_hand_id_tx1 = con_hand_id_tx.clone();

        let (stream, addr) = match listener.accept().await {
            Ok((s, a)) => (s, a),
            Err(_) => {
                // TODO: log error
                continue;
            }
        };

        // Ask if there is space to `id_record`
        match run_id_com_tx.send(RunIdRecordMsg::IsThereSpace).await {
            Ok(_) => {}
            Err(e) => {
                // NOTE: cancel should be called in `id_record`
                let _ = output_tx.send(OutputMsg::new_error(e.to_string())).await;
                ctoken.cancel();
                return;
            }
        }
        let is_there_space = match id_run_com_rx.recv().await {
            Some(i) => i,
            None => {
                let _ = output_tx
                    .send(OutputMsg::new_error(
                        "Failed to reciver from `id_record` in `run`",
                    ))
                    .await;
                ctoken.cancel();
                return;
            }
        };
        match is_there_space {
            IdRecordRunMsg::IsThereSpace(true) => {
                tokio::spawn(connection_handler_wrapper(
                    stream,
                    addr,
                    int_com_tx1,
                    int_com_rx,
                    con_hand_id_tx1,
                    output_tx.clone(),
                    ctoken.clone(),
                ));
            }
            _others => {
                // TODO: maybe too much noise
                let s = format!("Connection refused from: {}\n", addr);
                match output_tx.send(OutputMsg::new(&s)).await {
                    Ok(()) => {}
                    Err(_) => {
                        ctoken.cancel();
                        return;
                    }
                };
            }
        }
    }
}
