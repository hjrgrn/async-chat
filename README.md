# Async Chat

Asynchronous encrypted chat written using [tokio](https://tokio.rs/) for practicing purpose. Don't use it into production.


## Dependencies

[Rust](https://www.rust-lang.org/learn/get-started)


## Usage

```bash
git clone 'https://github.com/hjrgrn/async-chat.git'
cd async-chat
```

The chat uses a shared secret for end point authentication that needs to be provided as an environment variable.

```bash
# server
ASYNC_CHAT_SECRET="my secret" cargo run
```

```bash
# client
ASYNC_CHAT_SECRET="my secret" cargo run --bin client
```

Exemplar configuration can be found in `<ROOT_DIRECTORY>/configuration/{ClientSettings.toml,ServerSettings.toml}`.

