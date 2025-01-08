//! # `shared_lib`
//!
//! This modules contains utils shared by both server and client.

use std::fmt::{Debug, Display};

use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::globals::EXIT_MSG;
pub mod auxiliaries;
pub mod graceful_shutdown;
pub mod socket_handling;

/// # `display_output`
///
/// This function receives messages and displays them to stdout or stderr.
///
/// ## Params
///
/// - `receiver` -> Cannel used to receive messages to display
/// - `ctoken` -> Cancellation token used to communicate the shutdown
#[tracing::instrument(name = "Displaying output", skip(receiver, ctoken))]
pub async fn display_output(mut receiver: mpsc::Receiver<OutputMsg>, ctoken: CancellationToken) {
    loop {
        tokio::select! {
            _ = ctoken.cancelled() => {
                break;
            }
            res = receiver.recv() => {
                match res {
                    Some(msg) => {
                        match msg.payload {
                            Some(m) => {
                                println!("{}", m);
                            }
                            _ => {}
                        }
                        match msg.error {
                            Some(m) => {
                                eprintln!("{}", m);
                            }
                            _ => {}
                        }
                    },
                    None => {
                        // Channel has been closed, meaning the application can't work anymore
                        tracing::error!("`display_output` can't receive messages anymore.");
                        ctoken.cancel();
                        break;
                    }
                }
            }
        }
    }

    println!("{}", EXIT_MSG);
}

/// # `OutputMsg`
///
/// Object used to communicate a message to `display_output`,
/// it may contain a payload and an error.
pub struct OutputMsg {
    pub payload: Option<String>,
    pub error: Option<String>,
}

impl OutputMsg {
    pub fn new<T: Display + Debug>(payload: T) -> Self {
        Self {
            payload: Some(format!("{}", payload)),
            error: None,
        }
    }

    pub fn new_error<T: Display + Debug>(error: T) -> Self {
        Self {
            payload: None,
            error: Some(format!("{}", error)),
        }
    }
}

/// # `StdinRequest`
///
/// Enum used to communicate a request for input from stdin.
/// After sending one of those to `server_commad` with the appropriate
/// channel, await on the oneshot channel receiver to obtain the input.
/// Before requiring an input from stdin use a `OutputMsg` to display
/// a prompt for the admin.
/// If the receiver of the oneshot drops, the functionality that handles stdin should
/// consider the request not valid anymore and send the content to the
/// functionality handling stdout.
#[derive(Debug)]
pub enum StdinRequest {
    Plain(oneshot::Sender<String>),
}
