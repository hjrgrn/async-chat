/// # auxiliaries
///
/// Internal functions used by the function `id_record`
use std::net::SocketAddr;

use tokio::sync::{
    broadcast,
    mpsc::{self, error::SendError, Sender},
    oneshot,
};

use crate::{
    globals::{KICK, SERVER_COM, SERVER_LIST},
    server_lib::{
        structs::{CommandFromIdRecord, IdRecordConnHandler},
        OutputMsg, StdinRequest,
    },
};

use crate::server_lib::structs::{
    Client, ConnHandlerIdRecordMsg, IdRecordRunMsg, Message, RunIdRecordMsg,
};

/// # `id_record`'s helper `receiving_from_run`
///
/// Responds to messages sent by `run`.
///
/// Returns nothing if successful; otherwise, returns a fatal error requiring
/// the application to shutdown.
///
/// **NOTE**: At the moment it only checks if there is enough space to accept a
/// new connection and communicates that to `run`.
///
/// ## Parameters
///
/// - `channel`: Sends messages to `run`.
/// - `msg`: Message received from `run` in the caller (`id_record`).
/// - `actual_connections`: Number of currently connected clients.
/// - `max_connections`: Maximum number of connections allowed.
pub async fn receiving_from_run(
    channel: &mut Sender<IdRecordRunMsg>,
    msg: RunIdRecordMsg,
    actual_connections: usize,
    max_connections: usize,
) -> Result<(), SendError<IdRecordRunMsg>> {
    match msg {
        // NOTE: for now we only have `IsThereSpace`
        RunIdRecordMsg::IsThereSpace => {
            if actual_connections < max_connections {
                channel.send(IdRecordRunMsg::IsThereSpace(true)).await?;
            } else {
                channel.send(IdRecordRunMsg::IsThereSpace(false)).await?;
            }
        }
    }
    Ok(())
}

/// # `id_record`'s helper `print_list`
///
/// Formats the list of active clients and gives it back as a `String`.
///
/// ## Parameters
///
/// - clients -> reference to the vector of active clients.
///
/// ## Returns
/// `String` -> formatted list of active clients.
/// IDEA: keep the state of `clients` and update it every time a client arrives/leaves
/// istead of running this every time
pub fn print_list(clients: &[Client]) -> String {
    let mut content = String::from("\n\x1b[32;1mClients:\x1b[0m\n");
    for client in clients.iter() {
        let name = format!("\x1b[33;1m{}\x1b[0m\n", client.nick);
        content.push_str(&name);
    }
    content
}

/// # `id_record`'s helper
///
/// Parses messages received from a `connection_handler` and acts accordingly.
/// The message may be a command from `server_commands`.
/// If an error is returned, it is fatal and requires the application to shut down.
///
/// ## Parameters
///
/// - `msg`: Message received from the `connection_handler`.
/// - `clients`: List of currently connected clients.
/// - `address`: Address of the application.
/// - `con_hand_tx`: Channel to send messages to the `connection_handler`.
/// - `output_tx`: Channel to send output to the displayer.
/// - `stdin_req_tx`: Channel to send requests to the stdin handler.
// TODO: refactor, telemetry
// IDEA: this function doesn't check if there is space, that is handled by `run`: `run` asks
// if there is space, after that it launches the connection_handler_wrapper. connection_handler_wrapper
// ask for acceptance using an `ConnHandlerIdRecordMsg::AcceptanceRequest`, but it doesn't check
// if there is space, in the meantime run may have asked if there is space again for another
// connection, and another client may have asked for acceptance. I think there is a race condition.
// A possible solution could be to keep track of the actual number of connections instead of relying
// on `clients.len()` in `id_record` but this seems weak. I think the best solution would be to
// id_records to handle the connection handling sequentially, instead of run.
//
// there may be a
// race condition.
pub async fn receiving_from_hand(
    msg: ConnHandlerIdRecordMsg,
    clients: &mut Vec<Client>,
    address: &SocketAddr,
    con_hand_tx: &broadcast::Sender<Message>,
    output_tx: &mpsc::Sender<OutputMsg>,
    stdin_req_tx: &mpsc::Sender<StdinRequest>,
) -> Result<(), anyhow::Error> {
    match msg {
        // Remove the record from `clients`.
        ConnHandlerIdRecordMsg::ClientLeft(addr) => {
            for i in 0..clients.len() {
                if clients[i].addr == addr {
                    output_tx
                        .send(OutputMsg::new(format!(
                            "{} {} has left the chat.\n",
                            clients[i].nick, clients[i].addr
                        )))
                        .await?;
                    let content = format!("Master: {} has left the chat.\n", clients[i].nick);
                    con_hand_tx.send(Message::Broadcast {
                        content,
                        address: addr,
                    })?;
                    clients.remove(i);
                    break;
                }
            }
        }
        // client acceptance
        ConnHandlerIdRecordMsg::AcceptanceRequest(new_client) => {
            let mut accepted = true;
            for client in clients.iter() {
                if new_client.nick == client.nick {
                    accepted = false;
                    break;
                }
                if new_client.addr == client.addr {
                    accepted = false;
                    break;
                }
            }
            new_client
                .channel
                .send(IdRecordConnHandler::Acceptance(accepted))
                .await?;

            if accepted {
                clients.push(new_client);
            }
        }
        // a client has requested a list clients
        ConnHandlerIdRecordMsg::List(addr) => {
            let content = print_list(clients);
            let msg = IdRecordConnHandler::List(content);
            for client in clients.iter() {
                if client.addr == addr {
                    client.channel.send(msg).await?;
                    break;
                }
            }
        }
        // commands received from `server_commands`
        ConnHandlerIdRecordMsg::ServerCommand(msg) => {
            parse_command(&msg, clients, address, con_hand_tx, output_tx, stdin_req_tx).await?;
        }
    }
    Ok(())
}

/// # `receiving_from_hand`'s helper, `parse_command`
///
/// Parses a message received from a `connection_handler` that is a command from `server_commands`
/// and acts accordingly.
///
///
/// ## Parameters
///
/// `msg`: message to be parsed
/// `clients`: list of the connected clients
/// `address`: address of the app
/// `con_hand_tx`: channel used to send messages to `connection_handler`
/// `output_tx`: channel used for sending somethign to be displayed to the displayer
/// `stdin_req_tx`: channel used to request input from stdin
/// TODO: telemetry
async fn parse_command(
    msg: &String,
    clients: &mut [Client],
    address: &SocketAddr,
    con_hand_tx: &broadcast::Sender<Message>,
    output_tx: &mpsc::Sender<OutputMsg>,
    stdin_req_tx: &mpsc::Sender<StdinRequest>,
) -> Result<(), anyhow::Error> {
    // All commands start with '&'
    if msg.chars().next().expect("this should not happen") == '&' {
        // command for client
        if msg == SERVER_LIST {
            output_tx.send(OutputMsg::new(print_list(clients))).await?;
        } else if msg == KICK {
            kick_user(clients, output_tx, stdin_req_tx).await?;
        } else {
            output_tx
                .send(OutputMsg::new_error(format!(
                    "Invalid command, type \"{}\" for displaying the avaible commands.",
                    SERVER_COM.trim()
                )))
                .await?;
        }
    } else {
        // message for clients
        let content = format!("master: {}", msg);
        let m = Message::Broadcast {
            content,
            address: *address,
        };
        con_hand_tx.send(m)?;
    }
    Ok(())
}

/// TODO: telemetry
/// # `parse_command`'s helper `kick_user`
///
/// Asks the admin what client he wants to remove, communicates the choice to the connection
/// handler relative to the client, the connection handler will send a
/// `ConnHandlerIdRecordMsg::ClientLeft` msg to `id_record` in return and at this point the client
/// will be kicked by id record.
/// If an error is returned the error is fatal and the application should shutodown.
///
/// - clients -> list of clients that are connected
/// - `output_tx` -> communicates with the application that displays the output
/// - `stdin_req_tx` -> channel for requiring informations from stdin
async fn kick_user(
    clients: &mut [Client],
    output_tx: &mpsc::Sender<OutputMsg>,
    stdin_req_tx: &mpsc::Sender<StdinRequest>,
) -> Result<(), anyhow::Error> {
    let amount = clients.len();
    let mut input = String::from("To pick the client type [id] and press <enter>.\n");
    input.push_str("\n\x1b[32;1mClients:\x1b[0m");
    for (i, client) in clients.iter().enumerate().take(amount) {
        input.push_str(&format!("\n[\x1b[33;1m{}\x1b[0m]{}", i, client.nick));
    }
    input.push_str("\n[\x1b[33;1mq\x1b[0m]abort the procedure\n");
    output_tx.send(OutputMsg::new(&input)).await?;

    let (tx, rx) = oneshot::channel::<String>();
    let req = StdinRequest::Plain(tx);
    stdin_req_tx.send(req).await?;
    input = rx.await?;

    let id: usize;
    match input.trim().parse() {
        Ok(i) => {
            if i < amount {
                id = i;
            } else {
                output_tx
                    .send(OutputMsg::new_error("Index out of bound, retry."))
                    .await?;
                return Ok(());
            }
        }
        Err(e) => {
            if input.trim().to_lowercase() == "q" {
                output_tx
                    .send(OutputMsg::new("Procedure aborted as required."))
                    .await?;
                return Ok(());
            }
            output_tx.send(OutputMsg::new_error(e.to_string())).await?;
            return Ok(());
        }
    }

    let client = match clients.get(id) {
        Some(c) => c,
        None => {
            return Err(anyhow::anyhow!(
                "Unexpected error occurred, called `Vec.get(id)` on an `id` out of range after limiting `id`"
            ));
        }
    };
    client.command.send(CommandFromIdRecord::Kick).await?;
    Ok(())
}
