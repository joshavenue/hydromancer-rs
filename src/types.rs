//! Response types for the typed helpers.
//!
//! Prices, sizes, PnL and fees are kept as the decimal strings the API sends,
//! so no precision is lost; parse them with the decimal type of your choice.
//! Every struct keeps fields it does not know about in `extra`, so new API
//! fields never break deserialization.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Perp account state for one user on one DEX (`clearinghouseState`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ClearinghouseState {
    /// Totals across all positions.
    pub margin_summary: MarginSummary,
    /// Totals across cross-margin positions only.
    pub cross_margin_summary: MarginSummary,
    /// Maintenance margin used by cross positions.
    #[serde(default)]
    pub cross_maintenance_margin_used: Option<String>,
    /// Amount the user can withdraw.
    #[serde(default)]
    pub withdrawable: Option<String>,
    /// Open positions.
    #[serde(default)]
    pub asset_positions: Vec<AssetPosition>,
    /// Snapshot time in Unix milliseconds.
    pub time: u64,
    /// Fields not modelled above.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Margin totals inside a [`ClearinghouseState`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct MarginSummary {
    /// Account value in USD.
    pub account_value: String,
    /// Total notional position size in USD.
    pub total_ntl_pos: String,
    /// Raw USD balance.
    pub total_raw_usd: String,
    /// Margin in use.
    pub total_margin_used: String,
    /// Fields not modelled above.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One entry of [`ClearinghouseState::asset_positions`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AssetPosition {
    /// Position mode, for example `"oneWay"`.
    #[serde(rename = "type")]
    pub kind: String,
    /// The position itself.
    pub position: Position,
    /// Fields not modelled above.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// An open perp position.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Position {
    /// Coin, for example `"BTC"`; HIP-3 coins are prefixed with the DEX, for example `"xyz:JPY"`.
    pub coin: String,
    /// Signed size: positive for long, negative for short.
    pub szi: String,
    /// Leverage settings.
    pub leverage: Leverage,
    /// Average entry price.
    #[serde(default)]
    pub entry_px: Option<String>,
    /// Position value in USD.
    pub position_value: String,
    /// Unrealized PnL in USD.
    pub unrealized_pnl: String,
    /// Return on equity as a fraction.
    pub return_on_equity: String,
    /// Estimated liquidation price, `None` when there is none.
    #[serde(default)]
    pub liquidation_px: Option<String>,
    /// Margin allocated to this position.
    pub margin_used: String,
    /// Maximum leverage allowed for this coin.
    #[serde(default)]
    pub max_leverage: Option<u32>,
    /// Funding paid or received.
    #[serde(default)]
    pub cum_funding: Option<CumFunding>,
    /// Fields not modelled above.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Leverage of a [`Position`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Leverage {
    /// `"cross"` or `"isolated"`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Leverage multiple.
    pub value: u32,
    /// Raw USD for isolated positions.
    #[serde(default)]
    pub raw_usd: Option<String>,
    /// Fields not modelled above.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Cumulative funding of a [`Position`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct CumFunding {
    /// Since the account first held this coin.
    pub all_time: String,
    /// Since the position was opened.
    pub since_open: String,
    /// Since the position size last changed.
    pub since_change: String,
    /// Fields not modelled above.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Response of `batchClearinghouseStates`.
///
/// `S` is [`ClearinghouseState`] for one DEX, or a map of DEX name to state
/// ([`AllDexesState`]) when the request used `"dex": "ALL_DEXES"`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BatchClearinghouseStates<S = ClearinghouseState> {
    /// `(address, state)` pairs for the wallets that were fetched.
    pub successful_states: Vec<(String, S)>,
    /// Addresses that could not be fetched.
    #[serde(default)]
    pub failed_wallets: Vec<String>,
    /// Fields not modelled above.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Clearinghouse state per DEX (`"native"` plus each HIP-3 DEX), as returned with `"dex": "ALL_DEXES"`.
pub type AllDexesState = BTreeMap<String, ClearinghouseState>;

/// A trade fill (`userFillsByTime`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Fill {
    /// Coin, for example `"BTC"`, `"xyz:JPY"` (HIP-3) or `"@107"` (spot).
    pub coin: String,
    /// Fill price.
    pub px: String,
    /// Fill size.
    pub sz: String,
    /// `"B"` for buy, `"A"` for sell.
    pub side: String,
    /// Fill time in Unix milliseconds.
    pub time: u64,
    /// Position size before the fill.
    #[serde(default)]
    pub start_position: Option<String>,
    /// Direction, for example `"Open Long"` or `"Close Short"`.
    #[serde(default)]
    pub dir: Option<String>,
    /// PnL realised by this fill.
    #[serde(default)]
    pub closed_pnl: Option<String>,
    /// Transaction hash.
    pub hash: String,
    /// Order id.
    pub oid: u64,
    /// Whether the order crossed the spread (taker).
    #[serde(default)]
    pub crossed: Option<bool>,
    /// Fee paid (negative for a rebate).
    #[serde(default)]
    pub fee: Option<String>,
    /// Trade id. The paginator de-duplicates on trade id plus order id.
    pub tid: u64,
    /// Client order id.
    #[serde(default)]
    pub cloid: Option<String>,
    /// Builder fee.
    #[serde(default)]
    pub builder_fee: Option<String>,
    /// Builder address the order was routed through.
    #[serde(default)]
    pub builder: Option<String>,
    /// HIP-3 deployer fee.
    #[serde(default)]
    pub deployer_fee: Option<String>,
    /// Priority gas fee in HYPE.
    #[serde(default)]
    pub priority_gas: Option<String>,
    /// Token the fee was paid in, for example `"USDC"`.
    #[serde(default)]
    pub fee_token: Option<String>,
    /// TWAP order id for TWAP fills.
    #[serde(default)]
    pub twap_id: Option<u64>,
    /// Index of the transaction in its block; with `time` it forms the page cursor.
    pub tx_index: u64,
    /// HIP-4 venue of an outcome market.
    #[serde(default)]
    pub venue: Option<String>,
    /// Fields not modelled above.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Fill {
    /// The `userFillsByTime` cursor that points just after this fill: `"<time>_<txIndex>"`.
    pub fn cursor(&self) -> String {
        format!("{}_{}", self.time, self.tx_index)
    }
}

/// A liquidation event (`liquidationHistoryByTime`): the liquidated user's fill plus details.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Liquidation {
    /// Address that was liquidated.
    pub user: String,
    /// How the liquidation happened.
    pub liquidation: LiquidationDetail,
    /// The fill. `side` is `"A"` when a long was liquidated and `"B"` when a short was.
    #[serde(flatten)]
    pub fill: Fill,
}

/// The `liquidation` object of a [`Liquidation`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct LiquidationDetail {
    /// Address that was liquidated.
    pub liquidated_user: String,
    /// Mark price at liquidation.
    pub mark_px: String,
    /// `"market"` (closed against the book) or `"backstop"` (taken by the backstop liquidator).
    pub method: String,
    /// Fields not modelled above.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// An OHLCV candle (`candleSnapshot`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Candle {
    /// Coin.
    #[serde(rename = "s")]
    pub coin: String,
    /// Interval, for example `"1m"`.
    #[serde(rename = "i")]
    pub interval: String,
    /// Open time in Unix milliseconds.
    #[serde(rename = "t")]
    pub open_time: u64,
    /// Close time in Unix milliseconds.
    #[serde(rename = "T")]
    pub close_time: u64,
    /// Open price.
    #[serde(rename = "o")]
    pub open: String,
    /// High price.
    #[serde(rename = "h")]
    pub high: String,
    /// Low price.
    #[serde(rename = "l")]
    pub low: String,
    /// Close (latest) price.
    #[serde(rename = "c")]
    pub close: String,
    /// Volume in the base asset.
    #[serde(rename = "v")]
    pub volume: String,
    /// Quote volume; `None` for older candles where it was not recorded.
    #[serde(rename = "q", default)]
    pub quote_volume: Option<String>,
    /// Number of trades.
    #[serde(rename = "n")]
    pub trades: u64,
    /// `true` when the candle is closed, `false` while it is still open.
    #[serde(rename = "x", default)]
    pub closed: Option<bool>,
    /// Fields not modelled above.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One row of `apiUsage`: usage of one request type on one day.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ApiUsage {
    /// Day as `YYYY-MM-DD`.
    pub day: String,
    /// Request type, for example `"clearinghouseState"`.
    pub request_type: String,
    /// Requests sent.
    pub total_requests: u64,
    /// Requests that succeeded.
    pub success_requests: u64,
    /// Requests that failed.
    pub failure_requests: u64,
    /// Average server processing time in milliseconds.
    pub avg_processing_time_ms: f64,
    /// Billable tokens consumed, rounded down.
    pub tokens_consumed: u64,
    /// Fields not modelled above.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Candle interval for [`CandleSnapshot`](crate::CandleSnapshot).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum CandleInterval {
    /// 1 second.
    #[serde(rename = "1s")]
    S1,
    /// 15 seconds.
    #[serde(rename = "15s")]
    S15,
    /// 30 seconds.
    #[serde(rename = "30s")]
    S30,
    /// 1 minute.
    #[serde(rename = "1m")]
    M1,
    /// 3 minutes.
    #[serde(rename = "3m")]
    M3,
    /// 5 minutes.
    #[serde(rename = "5m")]
    M5,
    /// 15 minutes.
    #[serde(rename = "15m")]
    M15,
    /// 30 minutes.
    #[serde(rename = "30m")]
    M30,
    /// 1 hour.
    #[serde(rename = "1h")]
    H1,
    /// 2 hours.
    #[serde(rename = "2h")]
    H2,
    /// 4 hours.
    #[serde(rename = "4h")]
    H4,
    /// 8 hours.
    #[serde(rename = "8h")]
    H8,
    /// 12 hours.
    #[serde(rename = "12h")]
    H12,
    /// 1 day.
    #[serde(rename = "1d")]
    D1,
    /// 3 days.
    #[serde(rename = "3d")]
    D3,
    /// 1 week, Monday-aligned.
    #[serde(rename = "1w")]
    W1,
    /// 1 calendar month.
    #[serde(rename = "1M")]
    Month1,
}

impl CandleInterval {
    /// The interval string the API uses, for example `"15m"`.
    pub fn as_str(self) -> &'static str {
        match self {
            CandleInterval::S1 => "1s",
            CandleInterval::S15 => "15s",
            CandleInterval::S30 => "30s",
            CandleInterval::M1 => "1m",
            CandleInterval::M3 => "3m",
            CandleInterval::M5 => "5m",
            CandleInterval::M15 => "15m",
            CandleInterval::M30 => "30m",
            CandleInterval::H1 => "1h",
            CandleInterval::H2 => "2h",
            CandleInterval::H4 => "4h",
            CandleInterval::H8 => "8h",
            CandleInterval::H12 => "12h",
            CandleInterval::D1 => "1d",
            CandleInterval::D3 => "3d",
            CandleInterval::W1 => "1w",
            CandleInterval::Month1 => "1M",
        }
    }
}

impl std::fmt::Display for CandleInterval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
