use lib::{
    server_lib::{
        self, administration::server_commands_wrapper, settings::get_settings,
        ConnHandlerIdRecordMsg,
    },
    shared_lib::{display_output, graceful_shutdown::handling_sigint, OutputMsg, StdinRequest},
    telemetry::{get_subscriber, init_subscriber},
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[tokio::main]
pub async fn main() {
    let sub = get_subscriber("TcpChatServer".into(), "warn".into(), std::io::stdout);
    init_subscriber(sub);
    let settings = match get_settings() {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to read settings:\n{:?}", e);
            return;
        }
    };

    // Cancellation token for graceful shutdown
    let ctoken = CancellationToken::new();

    // spawn the function that allow the output of the server to be displayed
    let (output_tx, output_rx) = mpsc::channel::<OutputMsg>(10);
    tokio::spawn(display_output(output_rx, ctoken.clone()));

    // spawn the function that handles graceful shutdown
    tokio::spawn(handling_sigint(ctoken.clone(), output_tx.clone()));

    // spawn the function that allow the admin to communicate with the server
    let (con_hand_id_tx, con_hand_id_rx) = mpsc::channel::<ConnHandlerIdRecordMsg>(10);
    let (stdin_req_tx, stdin_req_rx) = mpsc::channel::<StdinRequest>(10);
    tokio::spawn(server_commands_wrapper(
        con_hand_id_tx.clone(),
        stdin_req_rx,
        output_tx.clone(),
        ctoken.clone(),
    ));

    server_lib::run_wrapper(
        settings,
        con_hand_id_tx,
        con_hand_id_rx,
        output_tx,
        stdin_req_tx,
        ctoken,
    )
    .await
}
