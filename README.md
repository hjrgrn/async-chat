# Async Chat

Asynchronous encrypted chat written using [tokio](https://tokio.rs/) for practicing purpose. Don't use it into production.


## Dependencies

[Rust](https://www.rust-lang.org/learn/get-started)


## Usage

```bash
git clone 'https://github.com/hjrgrn/async-chat.git'
cd async-chat

# server
cargo run

# client
cargo run --bin client
```

Exemplar configuration can be found in `<ROOT_DIRECTORY>/configuration/{ClientSettings.toml,ServerSettings.toml}`.

