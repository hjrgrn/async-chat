//! # sending messages
//!
//! TODO: description, new name for the module

use tokio::{io::BufWriter, net::tcp::OwnedWriteHalf, sync::mpsc};
use tokio_util::sync::CancellationToken;

use crate::shared_lib::{socket_handling::WriteHandler, OutputMsg};

use super::InputMsg;

pub async fn handling_stdin_input_wrapper(
    writer: OwnedWriteHalf,
    input_rx: mpsc::Receiver<InputMsg>,
    output_tx: mpsc::Sender<OutputMsg>,
    ctoken: CancellationToken,
) {
    tokio::select! {
        _ = ctoken.cancelled() => {}
        _ = handling_stdin_input(writer, input_rx, output_tx) => {
                ctoken.cancel();
            }
    }
}

/// # `handling_stdin_input`
///
///
/// ## Parameters
///
/// - writer: socket that writes to the server
/// - mut input_rx: channel that receives input from the user
/// TODO: Description, telemetry
async fn handling_stdin_input(
    writer: OwnedWriteHalf,
    mut input_rx: mpsc::Receiver<InputMsg>,
    output_tx: mpsc::Sender<OutputMsg>,
) {
    let mut writer = BufWriter::new(writer);
    let mut write_handler = WriteHandler::new();

    loop {
        let res = &mut input_rx.recv().await;
        match res {
            Some(inp) => match inp.action(&mut writer, &mut write_handler).await {
                Ok(_) => {}
                Err(e) => {
                    let _ = output_tx.send(OutputMsg::new_error(e)).await;
                    return;
                }
            },
            None => {}
        }
    }
}
