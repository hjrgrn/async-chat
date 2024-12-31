//! # sending messages
//!
//! TODO: description, new name for the module

use tokio::{io::BufWriter, net::tcp::OwnedWriteHalf, sync::mpsc};

use crate::shared_lib::socket_handling::WriteHandler;

use super::InputMsg;

/// # `handling_stdin_input`
///
///
/// ## Parameters
///
/// - writer: socket that writes to the server
/// - mut input_rx: channel that receives input from the user
/// TODO: Description, error handling/propagation, graceful shutodown
pub async fn handling_stdin_input(writer: OwnedWriteHalf, mut input_rx: mpsc::Receiver<InputMsg>) {
    let mut writer = BufWriter::new(writer);
    let mut write_handler = WriteHandler::new();

    loop {
        let res = &mut input_rx.recv().await;
        match res {
            Some(inp) => inp.action(&mut writer, &mut write_handler).await.unwrap(),
            None => {}
        }
    }
}
