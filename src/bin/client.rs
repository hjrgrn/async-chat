use lib::{
    client_lib::{self, client_commands, settings::get_settings, InputMsg},
    shared_lib::{display_output, OutputMsg, StdinRequest},
    telemetry::{get_subscriber, init_subscriber},
};
use tokio::sync::mpsc;

#[tokio::main]
pub async fn main() {
    let sub = get_subscriber("TcpChatClient".into(), "warn".into(), std::io::stdout);
    init_subscriber(sub);
    let settings = get_settings().expect("Failed to obtain settings.");

    // spawn the function that allow the output of the server to be displayed
    let (output_tx, output_rx) = mpsc::channel::<OutputMsg>(10);
    tokio::spawn(display_output(output_rx));

    // spawns the function that allow the user to communicate with the application
    let (input_tx, input_rx) = mpsc::channel::<InputMsg>(10);
    let (stdin_req_tx, stdin_req_rx) = mpsc::channel::<StdinRequest>(10);
    tokio::spawn(client_commands(
        input_tx.clone(),
        stdin_req_rx,
        output_tx.clone(),
    ));

    client_lib::run(settings, output_tx, input_rx, stdin_req_tx).await
}
