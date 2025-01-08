use tokio::{signal, sync::mpsc};
use tokio_util::sync::CancellationToken;

use crate::shared_lib::OutputMsg;

/// # `handling_sigint`
///
/// Task that handles an eventual SIGINT.
///
/// ## Params
///
/// - `ctoken` -> Cancellation token used to communicate the shutdown
/// - `output_tx` -> this channel is used to send the output of the server to a third entity.
#[tracing::instrument(name = "Handling SIGING", skip(ctoken, output_tx))]
pub async fn handling_sigint(ctoken: CancellationToken, output_tx: mpsc::Sender<OutputMsg>) {
    tokio::select! {
        res = signal::ctrl_c() => {
            match res {
                Ok(()) => {}
                Err(e) => {
                    tracing::error!("`handling_sigint` can't work anymore: {:?}", e);
                    let _ = output_tx.send(OutputMsg::new_error(e)).await;
                }
            }
        }
        _ = ctoken.cancelled() => { return; }
    }
    ctoken.cancel();
}
