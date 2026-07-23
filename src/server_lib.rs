use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::server_lib::administration::server_commands_wrapper;
use crate::server_lib::settings::Settings;
use crate::shared_lib::graceful_shutdown::handling_sigint;
use crate::shared_lib::{display_output, OutputMsg, StdinRequest};

use self::id_record::id_record;
pub use self::structs::{ConnHandlerIdRecordMsg, IdRecordRunMsg, RunIdRecordMsg};

pub mod administration;
mod connection_handling;
mod id_record;
pub mod settings;
mod structs;

/// # `run` wrapper
/// Initializes the application. Spawns threads dedicated to:
/// - displaying server-side output
/// - handling graceful shutdown
/// - administration
/// - executing the main application logic
///
/// ## Parameters
/// - `settings`: The application configuration.
/// - `shared_secret`: The secret required to authenticate users during the handshake.
pub async fn run_wrapper(settings: Settings, shared_secret: SecretString) {
    // Cancellation token for graceful shutdown.
    let ctoken = CancellationToken::new();

    // Spawn the function that allow the output of the server to be displayed.
    let (output_tx, output_rx) = mpsc::channel::<OutputMsg>(10);
    tokio::spawn(display_output(output_rx, ctoken.clone()));

    // Spawn the function that handles graceful shutdown.
    tokio::spawn(handling_sigint(ctoken.clone(), output_tx.clone()));

    // Spawn the function that allow the admin to communicate with the server.
    let (con_hand_id_tx, con_hand_id_rx) = mpsc::channel::<ConnHandlerIdRecordMsg>(10);
    let (stdin_req_tx, stdin_req_rx) = mpsc::channel::<StdinRequest>(10);
    tokio::spawn(server_commands_wrapper(
        con_hand_id_tx.clone(),
        stdin_req_rx,
        output_tx.clone(),
        ctoken.clone(),
    ));

    // IdRecord channel
    //
    // Run to id_record.
    let (run_id_com_tx, run_id_com_rx) = mpsc::channel::<RunIdRecordMsg>(10);

    tokio::spawn(id_record(
        settings.clone(),
        run_id_com_rx,
        con_hand_id_rx,
        con_hand_id_tx,
        output_tx.clone(),
        stdin_req_tx.clone(),
        ctoken.clone(),
        shared_secret,
    ));

    tokio::select! {
        _ = ctoken.cancelled() => {}
        res = run(
            settings,
            run_id_com_tx,
            output_tx,
        ) => {
            match res {
                Ok(()) => {},
                Err(e) => {
                    tracing::error!("`run` can't work anymore:\n{:?}", e);
                }
            }
            ctoken.cancel();
        }
    }
}

/// # Run
///
/// Runs the server, listens from incoming connection. When a connection request is received the
/// function passes the connection handling logic to `id_record`.
///
/// ## Parameters
///
/// - `settings`: application settings
/// - `run_id_com_tx`: sender channel used to communicate with `id_record`.
/// - `output_tx`: this channel is used to send the output of the server to a
///   output handler.
#[tracing::instrument(name = "Server is running", skip_all)]
async fn run(
    settings: Settings,
    run_id_com_tx: mpsc::Sender<RunIdRecordMsg>,
    output_tx: mpsc::Sender<OutputMsg>,
) -> Result<(), anyhow::Error> {
    output_tx.send(OutputMsg::new("Listening...")).await?;
    let listener = match TcpListener::bind(&settings.get_full_address()).await {
        Ok(l) => l,
        Err(e) => {
            let _ = output_tx.send(OutputMsg::new_error(e.to_string())).await;
            return Err(e.into());
        }
    };

    loop {
        let (stream, addr) = match listener.accept().await {
            Ok((s, a)) => (s, a),
            Err(e) => {
                tracing::info!("Error receiving a request:\n{:?}", e);
                continue;
            }
        };

        // Ask if there is space to `id_record`
        match run_id_com_tx
            .send(RunIdRecordMsg::NewConnection { stream, addr })
            .await
        {
            Ok(_) => {}
            Err(e) => {
                let _ = output_tx.send(OutputMsg::new_error(e.to_string())).await;
                return Err(e.into());
            }
        }
    }
}
