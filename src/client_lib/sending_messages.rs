//! # sending messages
//!
//! TODO: description, new name for the module

use tokio::{io::BufWriter, net::tcp::OwnedWriteHalf, sync::mpsc};
use tokio_util::sync::CancellationToken;

use crate::shared_lib::{socket_handling::WriteHandler, OutputMsg};

use super::InputMsg;

pub async fn handling_stdin_input_wrapper(
    mut write_handler: WriteHandler<BufWriter<OwnedWriteHalf>>,
    input_rx: mpsc::Receiver<InputMsg>,
    output_tx: mpsc::Sender<OutputMsg>,
    ctoken: CancellationToken,
) {
    tokio::select! {
        _ = ctoken.cancelled() => {}
        res = handling_stdin_input(&mut write_handler, input_rx, output_tx) => {
                match res {
                    Ok(_) => {}
                    Err(e) => {
                        tracing::error!("`handling_stdin_input` can't work anymore:\n{:?}", e);
                    }
                }
                ctoken.cancel();
            }
    }
}

/// # `handling_stdin_input`
///
/// Handles inputs from stdin.
///
///
/// ## Parameters
///
/// - write_hadler: write socket handler
/// - input_rx: channel that receives input from the user
/// - output_tx: channel for displaying output
#[tracing::instrument(
    name = "Handling input from stdin",
    skip(write_handler, input_rx, output_tx)
)]
async fn handling_stdin_input(
    write_handler: &mut WriteHandler<BufWriter<OwnedWriteHalf>>,
    mut input_rx: mpsc::Receiver<InputMsg>,
    output_tx: mpsc::Sender<OutputMsg>,
) -> Result<(), anyhow::Error> {
    loop {
        let res = &mut input_rx.recv().await;
        match res {
            Some(inp) => match inp.action(write_handler).await {
                Ok(_) => {}
                Err(e) => {
                    let _ = output_tx.send(OutputMsg::new_error(&e)).await;
                    return Err(e.into());
                }
            },
            None => {}
        }
    }
}
