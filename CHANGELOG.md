# Changelog

## 0.1.0

First release.

- Async `Client` for `POST /info` with Bearer auth, a `hydromancer-rs/<version>` User-Agent, mainnet and testnet base URLs.
- Typed helpers: `clearinghouseState`, `batchClearinghouseStates`, `userFillsByTime` (with a cursor paginator), `liquidationHistoryByTime`, `candleSnapshot`, `apiUsage`.
- `info` / `info_as` for any other JSON endpoint.
- HTTP 429 retries using `Retry-After`, then `retryAfterSecs`, then exponential backoff with jitter.
