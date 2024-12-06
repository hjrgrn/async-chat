use lib::{
    server_lib::{self, administration::server_commands, ConnHandlerIdRecordMsg, settings::get_settings},
    shared_lib::{display_output, OutputMsg, StdinRequest},
    telemetry::{get_subscriber, init_subscriber},
};
use tokio::sync::mpsc;

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

    // spawn the function that allow the output of the server to be displayed
    let (output_tx, output_rx) = mpsc::channel::<OutputMsg>(10);
    tokio::spawn(display_output(output_rx));

    // spawn the function that allow the admin to communicate with the server
    let (con_hand_id_tx, con_hand_id_rx) = mpsc::channel::<ConnHandlerIdRecordMsg>(10);
    let (stdin_req_tx, stdin_req_rx) = mpsc::channel::<StdinRequest>(10);
    tokio::spawn(server_commands(
        con_hand_id_tx.clone(),
        stdin_req_rx,
        output_tx.clone(),
    ));

    server_lib::run(
        settings,
        con_hand_id_tx,
        con_hand_id_rx,
        output_tx,
        stdin_req_tx,
    )
    .await
}
