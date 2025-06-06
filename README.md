# vote-inclusion-analyzer

Checks for vote transactions by slot/account, with leader schedule and retry logic.

## Usage

```sh
cargo run --release -- \
  --url <RPC_URL> \
  --account <VOTE_ACCOUNT_PUBKEY> \
  --slot <START_SLOT> \
  --distance <N>
```

- `--url`      RPC endpoint (e.g. https://api.mainnet-beta.solana.com)
- `--account`  Vote account pubkey to filter for
- `--slot`     Starting slot number
- `--distance` How many slots back to check (inclusive)

## Example

```sh
cargo run --release -- \
  --url https://api.mainnet-beta.solana.com \
  --account 5Y5Q5... \
  --slot 344883706 \
  --distance 10
```

## Dependencies

- Rust (2021 edition)
- [anyhow](https://crates.io/crates/anyhow)
- [clap](https://crates.io/crates/clap)
- [colored](https://crates.io/crates/colored)
- [reqwest](https://crates.io/crates/reqwest)
- [rand](https://crates.io/crates/rand)
- [serde](https://crates.io/crates/serde)
- [serde_json](https://crates.io/crates/serde_json)
- [solana-client](https://crates.io/crates/solana-client)
- [solana-program](https://crates.io/crates/solana-program)
- [indicatif](https://crates.io/crates/indicatif)
- [bs58](https://crates.io/crates/bs58)
- [bincode](https://crates.io/crates/bincode)

Build:

```sh
cargo build --release
```

## License

MIT

---
**No support. For professional/engineering use only. No responsibility for any damage =Q.Q=**
