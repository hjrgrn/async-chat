use std::env;

use lib::{
    server_lib::{self, settings::get_settings},
    telemetry::{get_subscriber, init_subscriber},
};
use secrecy::SecretString;

#[tokio::main]
pub async fn main() {
    // NOTE: for the time being it will be possible to abtain the shared secret only from an
    // environment variable `ASYNC_CHAT_SECRET`
    let shared_secret = SecretString::from(env::var("ASYNC_CHAT_SECRET").expect("Failed to obtain the shared secret, write that into the environment variable \"ASYNC_CHAT_SECRET\""));

    let sub = get_subscriber("TcpChatServer".into(), "warn".into(), std::io::stdout);
    init_subscriber(sub);
    let settings = match get_settings() {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to read settings:\n{:?}", e);
            return;
        }
    };

    server_lib::run_wrapper(settings, shared_secret).await
}
