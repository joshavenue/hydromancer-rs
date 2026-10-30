#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod client;
mod error;
mod requests;
mod retry;
pub mod types;

pub use client::{
    Client, ClientBuilder, UserFillsPages, ALL_DEXES, API_KEY_ENV, DEFAULT_USER_AGENT, MAINNET_URL, MAX_BATCH_USERS,
    MAX_BATCH_USERS_ALL_DEXES, TESTNET_URL,
};
pub use error::{Error, Result};
pub use requests::{
    CandleSnapshot, LiquidationHistoryByTime, UserFillsByTime, MAX_CANDLES_PER_REQUEST, MAX_FILLS_PER_PAGE,
    MAX_LIQUIDATIONS_PER_REQUEST,
};
pub use retry::RetryPolicy;
pub use types::{
    AllDexesState, ApiUsage, AssetPosition, BatchClearinghouseStates, Candle, CandleInterval, ClearinghouseState,
    CumFunding, Fill, Leverage, Liquidation, LiquidationDetail, MarginSummary, Position,
};

/// Re-export of the `serde_json` version used for [`Client::info`] values.
pub use serde_json;
