pub const ADDRESS: &'static str = "127.0.0.1:9999";
pub const MAX_CONNECTIONS: usize = 10;

// handshake limitations
pub const MAX_LEN: usize = 20;
pub const MAX_TRIES: u8 = 4;
pub const HANDSHAKE_TIMEOUT: u64 = 20;
// handshake protocols tokens sent from the server to the client
pub const INVALID_UTF8: &'static str = "$INVALID_UTF8\n";
pub const CONNECTION_ACCEPTED: &'static str = "$OK\n";
pub const MALFORMED_PACKET: &'static str = "$MALFORMED_PACKET\n";
pub const TOO_MANY_TRIES: &'static str = "$TOO_MANY_TRIES\n";
pub const TOO_LONG: &'static str = "$TOO_LONG\n";
pub const TOO_SHORT: &'static str = "$TOO_SHORT\n";
pub const TIMEOUT: &'static str = "$TIMEOUT\n";
pub const TAKEN: &'static str = "$NICK_ALREADY_TAKEN\n";

// commands for connection handler
pub const COMM: &'static str = "&COMM\n";
pub const LIST: &'static str = "&LIST\n";

// commands for id record from server
pub const SERVER_COM: &'static str = "&COMM\n";
pub const SERVER_LIST: &'static str = "&LIST\n";
pub const KICK: &'static str = "&KICK\n";
// messages displayed on the server side
pub const COMMANDS: &'static str = "
\x1b[32;1mCommands:\x1b[0m
\t\x1b[33;1m&COMM\x1b[0m -> list all the avaible commands.
\t\x1b[33;1m&LIST\x1b[0m -> list all clients currently connected to the chat.
\t\x1b[33;1m&KICK\x1b[0m -> kick a user.

or press \x1b[33;1mCTRL + C\x1b[0m to shutdown the server.
";
// general messages
pub const EXIT_MSG: &'static str = "\nShutting down, press <enter> to leave,\nSee you space cowboy...";
