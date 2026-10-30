//! Request builders for the endpoints that take several optional fields.

use serde::Serialize;

use crate::types::CandleInterval;

/// Most fills `userFillsByTime` returns in one page (also the default).
pub const MAX_FILLS_PER_PAGE: u32 = 2000;
/// Most events `liquidationHistoryByTime` returns in one request.
pub const MAX_LIQUIDATIONS_PER_REQUEST: u32 = 1000;
/// Most candles `candleSnapshot` returns in one request (also the default).
pub const MAX_CANDLES_PER_REQUEST: u32 = 5000;

/// A `userFillsByTime` request: fills for one user, oldest first.
///
/// Use it with [`Client::user_fills_by_time`](crate::Client::user_fills_by_time)
/// for one page, or [`Client::user_fills_pages`](crate::Client::user_fills_pages)
/// to follow the cursor.
///
/// ```
/// use hydromancer::UserFillsByTime;
///
/// let req = UserFillsByTime::new("0xdfc24b077bc1425ad1dea75bcb6f8158e10df303", 1_793_000_000_000)
///     .end_time(1_793_086_400_000)
///     .limit(500);
/// # let _ = req;
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserFillsByTime {
    #[serde(rename = "type")]
    kind: &'static str,
    pub(crate) user: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) start_time: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) end_time: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    builder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    venue: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    aggregate_by_time: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) limit: Option<u32>,
}

impl UserFillsByTime {
    /// Fills for `user` (a 0x address) from `start_time` (Unix ms, inclusive).
    pub fn new(user: impl Into<String>, start_time: u64) -> Self {
        Self {
            kind: "userFillsByTime",
            user: user.into(),
            start_time: Some(start_time),
            end_time: None,
            cursor: None,
            builder: None,
            dex: None,
            venue: None,
            aggregate_by_time: None,
            limit: None,
        }
    }

    /// Latest fill time to include (Unix ms, inclusive). Defaults to now.
    pub fn end_time(mut self, end_time: u64) -> Self {
        self.end_time = Some(end_time);
        self
    }

    /// Continue after a previous page: `"<time>_<txIndex>"` of its last fill
    /// (see [`Fill::cursor`](crate::Fill::cursor)). The server ignores
    /// `start_time` while a cursor is set.
    pub fn cursor(mut self, cursor: impl Into<String>) -> Self {
        self.cursor = Some(cursor.into());
        self
    }

    /// Only fills routed through this builder address.
    pub fn builder(mut self, builder: impl Into<String>) -> Self {
        self.builder = Some(builder.into());
        self
    }

    /// Only fills on this perp DEX, for example `"xyz"`.
    pub fn dex(mut self, dex: impl Into<String>) -> Self {
        self.dex = Some(dex.into());
        self
    }

    /// Only fills on outcome markets deployed by this HIP-4 venue.
    pub fn venue(mut self, venue: impl Into<String>) -> Self {
        self.venue = Some(venue.into());
        self
    }

    /// Merge partial fills of the same order at the same time.
    pub fn aggregate_by_time(mut self, aggregate: bool) -> Self {
        self.aggregate_by_time = Some(aggregate);
        self
    }

    /// Page size, clamped to `1..=2000` ([`MAX_FILLS_PER_PAGE`]). Defaults to 2000.
    pub fn limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit.clamp(1, MAX_FILLS_PER_PAGE));
        self
    }

    pub(crate) fn page_size(&self) -> u32 {
        self.limit.unwrap_or(MAX_FILLS_PER_PAGE)
    }
}

/// A `liquidationHistoryByTime` request: liquidation events, newest first.
///
/// ```
/// use hydromancer::LiquidationHistoryByTime;
///
/// // Market-wide, last 1,000 events.
/// let all = LiquidationHistoryByTime::new().limit(1000);
/// // One coin in a time window.
/// let btc = LiquidationHistoryByTime::new()
///     .coin("BTC")
///     .start_time(1_793_000_000_000)
///     .end_time(1_793_086_400_000);
/// # let _ = (all, btc);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiquidationHistoryByTime {
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    coin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_time: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    end_time: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
}

impl LiquidationHistoryByTime {
    /// Market-wide liquidations (all coins).
    pub fn new() -> Self {
        Self {
            kind: "liquidationHistoryByTime",
            coin: None,
            start_time: None,
            end_time: None,
            limit: None,
        }
    }

    /// Only this coin, for example `"BTC"` or a HIP-3 market such as `"xyz:JPY"`.
    pub fn coin(mut self, coin: impl Into<String>) -> Self {
        self.coin = Some(coin.into());
        self
    }

    /// Earliest event time (Unix ms, inclusive).
    pub fn start_time(mut self, start_time: u64) -> Self {
        self.start_time = Some(start_time);
        self
    }

    /// Latest event time (Unix ms, inclusive).
    pub fn end_time(mut self, end_time: u64) -> Self {
        self.end_time = Some(end_time);
        self
    }

    /// Maximum events, clamped to `1..=1000` ([`MAX_LIQUIDATIONS_PER_REQUEST`]). The server default is 500.
    pub fn limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit.clamp(1, MAX_LIQUIDATIONS_PER_REQUEST));
        self
    }
}

impl Default for LiquidationHistoryByTime {
    fn default() -> Self {
        Self::new()
    }
}

/// A `candleSnapshot` request: OHLCV candles, oldest first.
///
/// ```
/// use hydromancer::{CandleInterval, CandleSnapshot};
///
/// let req = CandleSnapshot::new("BTC", CandleInterval::H1, 1_793_000_000_000, 1_793_086_400_000);
/// # let _ = req;
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CandleSnapshot {
    #[serde(rename = "type")]
    kind: &'static str,
    pub(crate) req: CandleReq,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CandleReq {
    coin: String,
    interval: CandleInterval,
    pub(crate) start_time: u64,
    pub(crate) end_time: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
}

impl CandleSnapshot {
    /// Candles for `coin` from `start_time` (inclusive) to `end_time` (exclusive), both Unix ms.
    /// HIP-3 coins are prefixed with the DEX, for example `"xyz:JPY"`.
    pub fn new(coin: impl Into<String>, interval: CandleInterval, start_time: u64, end_time: u64) -> Self {
        Self {
            kind: "candleSnapshot",
            req: CandleReq {
                coin: coin.into(),
                interval,
                start_time,
                end_time,
                limit: None,
            },
        }
    }

    /// Maximum candles, clamped to `1..=5000` ([`MAX_CANDLES_PER_REQUEST`]). Defaults to 5000.
    pub fn limit(mut self, limit: u32) -> Self {
        self.req.limit = Some(limit.clamp(1, MAX_CANDLES_PER_REQUEST));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn user_fills_serializes_only_set_fields() {
        let req = UserFillsByTime::new("0xabc", 10).limit(5000).aggregate_by_time(true);
        assert_eq!(
            serde_json::to_value(&req).unwrap(),
            json!({"type": "userFillsByTime", "user": "0xabc", "startTime": 10, "limit": 2000, "aggregateByTime": true})
        );
    }

    #[test]
    fn liquidations_clamp_limit() {
        let req = LiquidationHistoryByTime::new().coin("BTC").limit(0);
        assert_eq!(
            serde_json::to_value(&req).unwrap(),
            json!({"type": "liquidationHistoryByTime", "coin": "BTC", "limit": 1})
        );
    }

    #[test]
    fn candles_nest_under_req() {
        let req = CandleSnapshot::new("BTC", CandleInterval::Month1, 1, 2).limit(10);
        assert_eq!(
            serde_json::to_value(&req).unwrap(),
            json!({"type": "candleSnapshot", "req": {"coin": "BTC", "interval": "1M", "startTime": 1, "endTime": 2, "limit": 10}})
        );
    }
}
