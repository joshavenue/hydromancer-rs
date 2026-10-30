# hydromancer

Async Rust client for the [Hydromancer](https://hydromancer.xyz) REST API, which serves
Hyperliquid (HyperCore) market and account data: positions, fills, liquidations, candles and more.

This crate is maintained by the Hydromancer team. You need a Hydromancer API key
([how to get one](https://docs.hydromancer.xyz/readme/get-api-keys)).

## Install

```toml
[dependencies]
hydromancer = "0.1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

TLS uses rustls; no OpenSSL is needed.

## Quick start

Put your key in the environment:

```sh
export HYDROMANCER_API_KEY=...
```

Then read a wallet's open positions:

```rust no_run
use hydromancer::Client;

#[tokio::main]
async fn main() -> Result<(), hydromancer::Error> {
    let client = Client::from_env()?;

    let state = client
        .clearinghouse_state("0xdfc24b077bc1425ad1dea75bcb6f8158e10df303")
        .await?;
    println!("account value: {}", state.margin_summary.account_value);
    for p in &state.asset_positions {
        println!("{} size {} entry {:?}", p.position.coin, p.position.szi, p.position.entry_px);
    }
    Ok(())
}
```

`Client::new("key")` takes the key directly, and `Client::builder()` sets the base URL,
User-Agent, timeout and retry policy. Base URLs must use `https` (plain `http` is accepted only
for loopback hosts such as `localhost` or `127.0.0.1`, for local testing).

## Endpoints

Every query is a `POST /info` with a JSON body whose `type` picks the operation.
These have typed helpers:

| Method | Request `type` |
|---|---|
| `clearinghouse_state`, `clearinghouse_state_on_dex`, `clearinghouse_state_all_dexes` | `clearinghouseState` |
| `batch_clearinghouse_states`, `batch_clearinghouse_states_on_dex` (up to 1,000 users each), `batch_clearinghouse_states_all_dexes` (up to 100 users) | `batchClearinghouseStates` |
| `user_fills_by_time`, `user_fills_pages` | `userFillsByTime` |
| `liquidation_history_by_time` | `liquidationHistoryByTime` |
| `candle_snapshot` | `candleSnapshot` |
| `api_usage` | `apiUsage` |

Prices, sizes and PnL stay as the decimal strings the API sends, so no precision is lost.
Fields a struct does not model are kept in its `extra` map.

For any other JSON endpoint, `info` sends the body you give it and returns a `serde_json::Value`
(`info_as` deserializes into your own type). The full list of request types is at
<https://docs.hydromancer.xyz/llms.txt>.

```rust no_run
use hydromancer::{serde_json::json, Client};

#[tokio::main]
async fn main() -> Result<(), hydromancer::Error> {
    let client = Client::from_env()?;
    let mids = client.info(&json!({"type": "allMids"})).await?;
    println!("BTC mid: {}", mids["BTC"]);
    Ok(())
}
```

Binary snapshot endpoints such as `perpSnapshots` return MessagePack, not JSON; this crate does not decode them,
and `info` returns `Error::Decode` for them.

## Paging through fills

`userFillsByTime` returns at most 2,000 fills per page, oldest first. `user_fills_pages` follows
the `"<time>_<txIndex>"` cursor of each page's last fill and drops fills it has already returned:

```rust no_run
use std::time::{SystemTime, UNIX_EPOCH};
use hydromancer::{Client, UserFillsByTime};

#[tokio::main]
async fn main() -> Result<(), hydromancer::Error> {
    let client = Client::from_env()?;
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    let week_ago = now_ms - 7 * 24 * 3_600_000;

    let mut pages = client.user_fills_pages(
        UserFillsByTime::new("0xdfc24b077bc1425ad1dea75bcb6f8158e10df303", week_ago).end_time(now_ms),
    );
    let fills = pages.collect_pages(50).await?;
    println!("{} fills, complete: {}", fills.len(), pages.is_done());
    Ok(())
}
```

Call `next_page` instead to handle one page at a time, and `cursor` to save where you stopped.
If a full page fails to move the cursor forward, `next_page` returns `Error::PaginationStalled`
instead of stopping quietly.

## Liquidations and candles

```rust no_run
use hydromancer::{CandleInterval, CandleSnapshot, Client, LiquidationHistoryByTime};

#[tokio::main]
async fn main() -> Result<(), hydromancer::Error> {
    let client = Client::from_env()?;

    // Newest first; omit .coin() for every market.
    let liqs = client
        .liquidation_history_by_time(&LiquidationHistoryByTime::new().coin("BTC").limit(100))
        .await?;
    for l in &liqs {
        println!("{} {} {} @ {} ({})", l.fill.time, l.user, l.fill.sz, l.fill.px, l.liquidation.method);
    }

    // Oldest first; end time is exclusive.
    let end = 1_793_086_400_000;
    let candles = client
        .candle_snapshot(&CandleSnapshot::new("BTC", CandleInterval::H1, end - 24 * 3_600_000, end))
        .await?;
    println!("{} candles", candles.len());
    Ok(())
}
```

## Rate limits and retries

Each key has a per-minute weight budget, and heavier endpoints cost more weight
([details](https://docs.hydromancer.xyz/readme/rest-api/rate-limits-and-user-limits)).
On HTTP 429 the client waits and retries, up to 3 times by default. Each wait is the `Retry-After`
header (seconds or an HTTP date), or the body's `retryAfterSecs`, or exponential backoff from
1 second capped at 30 seconds, plus up to 1 second of random jitter. All attempts and waits share a
90-second budget: if the next wait would not fit, the client stops and returns `Error::RateLimited`, and a request
still running when the budget ends is cancelled. When retries run out you get `Error::RateLimited` with the server's
`remaining`, `required` and `correlationId`; a request cut off by the budget gives `Error::Timeout`.
Other HTTP errors and transport failures are not retried. Change this with `ClientBuilder::retry_policy`
or `max_retries`. Each HTTP attempt also has its own 60-second timeout (`ClientBuilder::timeout`).

## Testnet

```rust
fn main() -> Result<(), hydromancer::Error> {
    let client = hydromancer::Client::builder().api_key("your-key").testnet().build()?;
    assert_eq!(client.base_url(), hydromancer::TESTNET_URL);
    Ok(())
}
```

Your key needs testnet access; a 401 on testnet usually means testnet is not enabled for that key.

## API key handling

The key is read from `HYDROMANCER_API_KEY` or passed in code and sent as an `Authorization: Bearer`
header to `{base URL}/info`, marked sensitive in the header map. The built-in HTTP client does not
follow redirects; if you pass your own `reqwest::Client` with `http_client`, configure it with
`redirect::Policy::none()` as well. `Debug` output and the crate's own error messages never include
the key, and if a server echoes the key back in an error response, it is replaced with `<redacted>`.

Every request carries a `User-Agent` (`hydromancer-rs/<version>` by default), because the API rejects
some default HTTP-library agents with HTTP 403 "error code: 1010".

## Development

```sh
cargo test                                  # unit and mock-server tests, no network
HYDROMANCER_API_KEY=... cargo test -- --ignored   # live tests against mainnet
```

## License

MIT. Copyright (c) 2026 Hydromancer BV.
