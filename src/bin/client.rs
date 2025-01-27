use std::env;

use lib::{
    client_lib::{self, client_commands_wrapper, settings::get_settings, InputMsg},
    shared_lib::{display_output, graceful_shutdown::handling_sigint, OutputMsg, StdinRequest},
    telemetry::{get_subscriber, init_subscriber},
};
use secrecy::SecretString;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[tokio::main]
pub async fn main() {
    // NOTE: for the time being it will be possible to abtain the shared secret only from an
    // environment variable `ASYNC_CHAT_SECRET`
    let shared_secret = SecretString::from(env::var("ASYNC_CHAT_SECRET").expect("Failed to obtain the shared secret, write that into the environment variable \"ASYNC_CHAT_SECRET\""));

    let sub = get_subscriber("TcpChatClient".into(), "warn".into(), std::io::stdout);
    init_subscriber(sub);
    let settings = get_settings().expect("Failed to obtain settings.");

    // Cancellation token for graceful shutdown
    let ctoken = CancellationToken::new();

    // spawn the function that allow the output of the server to be displayed
    let (output_tx, output_rx) = mpsc::channel::<OutputMsg>(10);
    tokio::spawn(display_output(output_rx, ctoken.clone()));

    // spawn the function that handles graceful shutdown
    tokio::spawn(handling_sigint(ctoken.clone(), output_tx.clone()));

    // spawns the function that allow the user to communicate with the application
    let (input_tx, input_rx) = mpsc::channel::<InputMsg>(10);
    let (stdin_req_tx, stdin_req_rx) = mpsc::channel::<StdinRequest>(10);
    tokio::spawn(client_commands_wrapper(
        input_tx.clone(),
        stdin_req_rx,
        output_tx.clone(),
        ctoken.clone(),
    ));

    client_lib::run_wrapper(
        settings,
        output_tx,
        input_rx,
        stdin_req_tx,
        ctoken,
        shared_secret,
    )
    .await
}
