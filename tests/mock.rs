//! Tests against a local mock server: request shape, headers, pagination and 429 handling.

use std::time::{Duration, Instant};

use hydromancer::{
    CandleInterval, CandleSnapshot, Client, Error, LiquidationHistoryByTime, RetryPolicy, UserFillsByTime,
    DEFAULT_USER_AGENT,
};
use serde_json::{json, Value};
use wiremock::matchers::{body_json, body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const KEY: &str = "test-key-123";
const USER: &str = "0xdfc24b077bc1425ad1dea75bcb6f8158e10df303";

fn client(server: &MockServer) -> Client {
    let mut retry = RetryPolicy::default();
    retry.max_jitter = Duration::ZERO;
    retry.initial_backoff = Duration::from_millis(10);
    Client::builder()
        .api_key(KEY)
        .base_url(server.uri())
        .retry_policy(retry)
        .build()
        .unwrap()
}

fn state(account_value: &str) -> Value {
    json!({
        "marginSummary": {"accountValue": account_value, "totalNtlPos": "6246.6", "totalRawUsd": "-6085.1", "totalMarginUsed": "161.4"},
        "crossMarginSummary": {"accountValue": "0.0", "totalNtlPos": "0.0", "totalRawUsd": "0.0", "totalMarginUsed": "0.0"},
        "crossMaintenanceMarginUsed": "0.0",
        "withdrawable": "0.0",
        "assetPositions": [{
            "type": "oneWay",
            "position": {
                "coin": "BTC", "szi": "0.08111",
                "leverage": {"type": "isolated", "value": 20, "rawUsd": "-6085.146055"},
                "entryPx": "78914.3", "positionValue": "6246.60554", "unrealizedPnl": "-154.14101",
                "returnOnEquity": "-0.48", "liquidationPx": null, "marginUsed": "161.459485",
                "maxLeverage": 40, "cumFunding": {"allTime": "-0.38", "sinceOpen": "0.17", "sinceChange": "0.0"},
                "newField": 1
            }
        }],
        "time": 1793324733912u64
    })
}

fn fill(time: u64, tx_index: u64, tid: u64) -> Value {
    json!({
        "coin": "BTC", "px": "79000.0", "sz": "0.01", "side": "B", "time": time,
        "startPosition": "0.0", "dir": "Open Long", "closedPnl": "0.0",
        "hash": "0xabc", "oid": tid + 1, "crossed": true, "fee": "0.3", "tid": tid,
        "cloid": null, "builderFee": "0.1", "feeToken": "USDC", "builder": "0xb84168cf3be63c6b8dad05",
        "twapId": null, "txIndex": tx_index
    })
}

#[tokio::test]
async fn sends_auth_user_agent_and_json_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/info"))
        .and(header("authorization", "Bearer test-key-123"))
        .and(header("user-agent", DEFAULT_USER_AGENT))
        .and(header("content-type", "application/json"))
        .and(body_json(json!({"type": "clearinghouseState", "user": USER})))
        .respond_with(ResponseTemplate::new(200).set_body_json(state("161.45")))
        .expect(1)
        .mount(&server)
        .await;

    let s = client(&server).clearinghouse_state(USER).await.unwrap();
    assert_eq!(s.margin_summary.account_value, "161.45");
    assert_eq!(s.withdrawable.as_deref(), Some("0.0"));
    let p = &s.asset_positions[0].position;
    assert_eq!(p.leverage.kind, "isolated");
    assert_eq!(p.leverage.raw_usd.as_deref(), Some("-6085.146055"));
    assert_eq!(p.liquidation_px, None);
    assert_eq!(p.extra["newField"], 1);
}

#[tokio::test]
async fn custom_user_agent_is_sent() {
    let server = MockServer::start().await;
    Mock::given(header("user-agent", "my-bot/2.0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;
    let c = Client::builder()
        .api_key(KEY)
        .base_url(server.uri())
        .user_agent("my-bot/2.0")
        .build()
        .unwrap();
    assert_eq!(c.info(&json!({"type": "x"})).await.unwrap(), json!({"ok": true}));
}

#[tokio::test]
async fn user_agent_is_sent_with_a_custom_http_client() {
    let server = MockServer::start().await;
    Mock::given(header("user-agent", DEFAULT_USER_AGENT))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(1)
        .mount(&server)
        .await;
    let c = Client::builder()
        .api_key(KEY)
        .base_url(server.uri())
        .http_client(reqwest::Client::new())
        .build()
        .unwrap();
    c.info(&json!({"type": "x"})).await.unwrap();
}

#[tokio::test]
async fn batch_request_and_response_shape() {
    let server = MockServer::start().await;
    Mock::given(body_json(
        json!({"type": "batchClearinghouseStates", "users": [USER, "0x01"]}),
    ))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
        "successful_states": [[USER, state("1.0")]],
        "failed_wallets": ["0x01"]
    })))
    .expect(1)
    .mount(&server)
    .await;
    let b = client(&server)
        .batch_clearinghouse_states(&[USER, "0x01"])
        .await
        .unwrap();
    assert_eq!(b.successful_states.len(), 1);
    assert_eq!(b.successful_states[0].0, USER);
    assert_eq!(b.failed_wallets, vec!["0x01".to_string()]);
}

#[tokio::test]
async fn batch_all_dexes_shape_and_limit() {
    let server = MockServer::start().await;
    Mock::given(body_partial_json(
        json!({"type": "batchClearinghouseStates", "dex": "ALL_DEXES"}),
    ))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
        "successful_states": [[USER, {"native": state("1.0"), "xyz": state("2.0")}]],
        "failed_wallets": []
    })))
    .expect(1)
    .mount(&server)
    .await;
    let c = client(&server);
    let b = c.batch_clearinghouse_states_all_dexes(&[USER]).await.unwrap();
    assert_eq!(b.successful_states[0].1["xyz"].margin_summary.account_value, "2.0");

    let too_many = vec![USER; 101];
    assert!(matches!(
        c.batch_clearinghouse_states_all_dexes(&too_many).await,
        Err(Error::InvalidRequest(_))
    ));
    let too_many = vec![USER; 1001];
    assert!(matches!(
        c.batch_clearinghouse_states(&too_many).await,
        Err(Error::InvalidRequest(_))
    ));
}

/// Responds to userFillsByTime like the server: cursor is exclusive, pages of `limit`.
struct FillsServer {
    fills: Vec<Value>,
    /// `true`: the next page starts after the cursor (what the live API does).
    /// `false`: the page repeats the cursor fill, to exercise de-duplication.
    exclusive: bool,
}

impl Respond for FillsServer {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&req.body).unwrap();
        let limit = body["limit"].as_u64().unwrap_or(2000) as usize;
        let start = match body["cursor"].as_str() {
            Some(c) => {
                assert!(body.get("startTime").is_none(), "startTime sent with cursor");
                let (t, i) = c.split_once('_').unwrap();
                let key = (t.parse::<u64>().unwrap(), i.parse::<u64>().unwrap());
                self.fills
                    .iter()
                    .position(|f| {
                        let k = (f["time"].as_u64().unwrap(), f["txIndex"].as_u64().unwrap());
                        if self.exclusive {
                            k > key
                        } else {
                            k >= key
                        }
                    })
                    .unwrap_or(self.fills.len())
            }
            None => 0,
        };
        let page: Vec<Value> = self.fills.iter().skip(start).take(limit).cloned().collect();
        ResponseTemplate::new(200).set_body_json(page)
    }
}

#[tokio::test]
async fn paginator_follows_cursor_and_dedupes() {
    let server = MockServer::start().await;
    let fills: Vec<Value> = (0..7).map(|i| fill(1000 + i, i % 3, 500 + i)).collect();
    Mock::given(body_partial_json(
        json!({"type": "userFillsByTime", "user": USER, "limit": 3}),
    ))
    .respond_with(FillsServer {
        fills,
        exclusive: false,
    })
    .mount(&server)
    .await;

    let c = client(&server);
    let mut pages = c.user_fills_pages(UserFillsByTime::new(USER, 1).limit(3));
    let all = pages.collect_pages(100).await.unwrap();
    let tids: Vec<u64> = all.iter().map(|f| f.tid).collect();
    assert_eq!(tids, vec![500, 501, 502, 503, 504, 505, 506]);
    assert!(pages.is_done());
    assert_eq!(all[0].builder.as_deref(), Some("0xb84168cf3be63c6b8dad05"));

    let requests = server.received_requests().await.unwrap();
    let bodies: Vec<Value> = requests
        .iter()
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect();
    assert_eq!(bodies[0]["startTime"], 1);
    assert!(bodies[0].get("cursor").is_none());
    assert_eq!(bodies[1]["cursor"], "1002_2");
    // Pages: [0,1,2] [2,3,4] [4,5,6] [6] -> the short last page ends paging.
    assert_eq!(bodies.len(), 4);
}

#[tokio::test]
async fn paginator_stops_on_empty_page_and_respects_max_pages() {
    let server = MockServer::start().await;
    Mock::given(path("/info"))
        .respond_with(FillsServer {
            fills: (0..10).map(|i| fill(2000 + i, 0, 900 + i)).collect(),
            exclusive: false,
        })
        .mount(&server)
        .await;
    let c = client(&server);
    let mut pages = c.user_fills_pages(UserFillsByTime::new(USER, 1).limit(2));
    let first = pages.collect_pages(2).await.unwrap();
    assert_eq!(first.len(), 3); // 2 + 1 new (one overlap)
    assert!(!pages.is_done());
    assert_eq!(pages.cursor(), Some("2002_0"));
    let rest = pages.collect_pages(100).await.unwrap();
    assert_eq!(first.len() + rest.len(), 10);
    assert!(pages.is_done());
    assert_eq!(pages.next_page().await.unwrap(), None);

    let server = MockServer::start().await;
    Mock::given(path("/info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;
    let mut pages = client(&server).user_fills_pages(UserFillsByTime::new(USER, 1));
    assert_eq!(pages.next_page().await.unwrap(), None);
    assert!(pages.is_done());
}

#[tokio::test]
async fn liquidations_candles_and_usage_decode() {
    let server = MockServer::start().await;
    let mut liq = fill(1793324664987, 21, 184551171610461);
    liq["user"] = json!("0xe56d");
    liq["liquidation"] = json!({"liquidatedUser": "0xe56d", "markPx": "0.0655", "method": "market"});
    Mock::given(body_json(
        json!({"type": "liquidationHistoryByTime", "coin": "BTC", "limit": 1000}),
    ))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!([liq])))
    .mount(&server)
    .await;
    Mock::given(body_json(json!({
        "type": "candleSnapshot",
        "req": {"coin": "BTC", "interval": "1h", "startTime": 1, "endTime": 2}
    })))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
        "s": "BTC", "i": "1h", "t": 1793311200000u64, "T": 1793314799999u64,
        "o": "79020.00", "h": "79253.00", "l": "78603.00", "c": "79003.00",
        "v": "4734.07", "q": null, "n": 33056, "x": true
    }])))
    .mount(&server)
    .await;
    Mock::given(body_json(json!({"type": "apiUsage", "days": 7})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
            "day": "2026-10-30", "request_type": "candleSnapshot", "total_requests": 1,
            "success_requests": 1, "failure_requests": 0, "avg_processing_time_ms": 6.0, "tokens_consumed": 0
        }])))
        .mount(&server)
        .await;

    let c = client(&server);
    let l = c
        .liquidation_history_by_time(&LiquidationHistoryByTime::new().coin("BTC").limit(5000))
        .await
        .unwrap();
    assert_eq!(l[0].user, "0xe56d");
    assert_eq!(l[0].liquidation.method, "market");
    assert_eq!(l[0].fill.tx_index, 21);
    assert!(l[0].fill.extra.is_empty(), "{:?}", l[0].fill.extra);

    let candles = c
        .candle_snapshot(&CandleSnapshot::new("BTC", CandleInterval::H1, 1, 2))
        .await
        .unwrap();
    assert_eq!(candles[0].close_time, 1793314799999);
    assert_eq!(candles[0].quote_volume, None);
    assert_eq!(candles[0].closed, Some(true));

    let usage = c.api_usage(Some(7)).await.unwrap();
    assert_eq!(usage[0].request_type, "candleSnapshot");

    assert!(matches!(
        c.candle_snapshot(&CandleSnapshot::new("BTC", CandleInterval::H1, 5, 5))
            .await,
        Err(Error::InvalidRequest(_))
    ));
}

#[tokio::test]
async fn api_errors_are_reported_and_not_retried() {
    let server = MockServer::start().await;
    Mock::given(path("/info"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": "Unknown API route: nope"})))
        .expect(1)
        .mount(&server)
        .await;
    let err = client(&server).info(&json!({"type": "nope"})).await.unwrap_err();
    match err {
        Error::Api { status, message, .. } => {
            assert_eq!(status, 400);
            assert_eq!(message, "Unknown API route: nope");
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn decode_errors_keep_a_body_snippet() {
    let server = MockServer::start().await;
    Mock::given(path("/info"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .mount(&server)
        .await;
    let err = client(&server).clearinghouse_state(USER).await.unwrap_err();
    assert!(matches!(err, Error::Decode { ref body, .. } if body == "not json"));

    // A decode error message must not carry an echoed key either.
    let server = MockServer::start().await;
    Mock::given(path("/info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"marginSummary": KEY})))
        .mount(&server)
        .await;
    let err = client(&server).clearinghouse_state(USER).await.unwrap_err();
    assert!(!format!("{err} {err:?}").contains(KEY), "{err:?}");
}

/// Returns 429 for the first `fail` requests, then 200.
struct FlakyServer {
    fail: usize,
    template_429: ResponseTemplate,
    calls: std::sync::atomic::AtomicUsize,
}

impl Respond for FlakyServer {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n < self.fail {
            self.template_429.clone()
        } else {
            ResponseTemplate::new(200).set_body_json(json!({"ok": true}))
        }
    }
}

fn flaky(fail: usize, template_429: ResponseTemplate) -> FlakyServer {
    FlakyServer {
        fail,
        template_429,
        calls: Default::default(),
    }
}

#[tokio::test]
async fn retry_after_header_seconds_is_honoured() {
    let server = MockServer::start().await;
    let r429 = ResponseTemplate::new(429)
        .insert_header("retry-after", "1")
        .set_body_json(json!({"error": "Rate limit exceeded", "retryAfterSecs": 5}));
    Mock::given(path("/info"))
        .respond_with(flaky(1, r429))
        .expect(2)
        .mount(&server)
        .await;
    let started = Instant::now();
    let v = client(&server).info(&json!({"type": "x"})).await.unwrap();
    let elapsed = started.elapsed();
    assert_eq!(v, json!({"ok": true}));
    // Waited for the header (1 s), not the body (5 s).
    assert!(elapsed >= Duration::from_secs(1), "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(4), "{elapsed:?}");
}

#[tokio::test]
async fn retry_after_http_date_is_honoured() {
    let server = MockServer::start().await;
    let at = std::time::SystemTime::now() + Duration::from_secs(2);
    let r429 = ResponseTemplate::new(429).insert_header("retry-after", httpdate_fmt(at));
    Mock::given(path("/info"))
        .respond_with(flaky(1, r429))
        .expect(2)
        .mount(&server)
        .await;
    let started = Instant::now();
    client(&server).info(&json!({"type": "x"})).await.unwrap();
    // HTTP dates have 1 s resolution, so the wait is between ~1 and ~2 s.
    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_millis(900), "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(4), "{elapsed:?}");
}

fn httpdate_fmt(t: std::time::SystemTime) -> String {
    // Minimal RFC 7231 IMF-fixdate formatter so the test does not need extra crates.
    let secs = t.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let days = secs / 86_400;
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    let weekday = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][(days % 7) as usize];
    // Civil-from-days (Howard Hinnant).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(mo <= 2);
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ][(mo - 1) as usize];
    format!("{weekday}, {d:02} {month} {y} {h:02}:{m:02}:{s:02} GMT")
}

#[tokio::test]
async fn body_retry_after_secs_is_used_without_header() {
    let server = MockServer::start().await;
    let r429 = ResponseTemplate::new(429).set_body_json(json!({"error": "Rate limit exceeded", "retryAfterSecs": 1}));
    Mock::given(path("/info"))
        .respond_with(flaky(1, r429))
        .expect(2)
        .mount(&server)
        .await;
    let started = Instant::now();
    client(&server).info(&json!({"type": "x"})).await.unwrap();
    assert!(started.elapsed() >= Duration::from_secs(1));
}

#[tokio::test]
async fn exponential_backoff_without_hints() {
    let server = MockServer::start().await;
    Mock::given(path("/info"))
        .respond_with(flaky(2, ResponseTemplate::new(429)))
        .expect(3)
        .mount(&server)
        .await;
    // initial_backoff is 10 ms in tests: 10 ms + 20 ms.
    let started = Instant::now();
    client(&server).info(&json!({"type": "x"})).await.unwrap();
    assert!(started.elapsed() >= Duration::from_millis(30));
}

#[tokio::test]
async fn exhausted_retries_return_rate_limited_with_details() {
    let server = MockServer::start().await;
    let r429 = ResponseTemplate::new(429)
        .insert_header("retry-after", "0")
        .insert_header("x-ratelimit-limit", "20000")
        .set_body_json(json!({
            "error": "Rate limit exceeded. You have 8 points remaining. This request requires more points.",
            "retryAfterSecs": 0, "remaining": 8, "required": 20, "correlationId": "AB12CD34"
        }));
    Mock::given(path("/info"))
        .respond_with(r429)
        .expect(4) // first attempt + 3 retries
        .mount(&server)
        .await;
    let err = client(&server).info(&json!({"type": "x"})).await.unwrap_err();
    assert_eq!(err.status(), Some(429));
    match err {
        Error::RateLimited {
            attempts,
            retry_after,
            remaining,
            required,
            limit,
            correlation_id,
            ..
        } => {
            assert_eq!(attempts, 4);
            assert_eq!(retry_after, Some(Duration::ZERO));
            assert_eq!((remaining, required, limit), (Some(8), Some(20), Some(20000)));
            assert_eq!(correlation_id.as_deref(), Some("AB12CD34"));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn wait_longer_than_budget_fails_fast() {
    let server = MockServer::start().await;
    Mock::given(path("/info"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "120"))
        .expect(1)
        .mount(&server)
        .await;
    let started = Instant::now();
    let err = client(&server).info(&json!({"type": "x"})).await.unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(
        matches!(err, Error::RateLimited { attempts: 1, retry_after: Some(d), .. } if d == Duration::from_secs(120))
    );
}

#[tokio::test]
async fn retries_can_be_disabled() {
    let server = MockServer::start().await;
    Mock::given(path("/info"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "0"))
        .expect(1)
        .mount(&server)
        .await;
    let c = Client::builder()
        .api_key(KEY)
        .base_url(server.uri())
        .max_retries(0)
        .build()
        .unwrap();
    assert!(matches!(
        c.info(&json!({"type": "x"})).await,
        Err(Error::RateLimited { attempts: 1, .. })
    ));
}

#[tokio::test]
async fn errors_never_contain_the_key() {
    let server = MockServer::start().await;
    Mock::given(path("/info"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({"error": "Invalid API key"})))
        .mount(&server)
        .await;
    let err = client(&server).info(&json!({"type": "x"})).await.unwrap_err();
    assert!(!err.to_string().contains(KEY));
    assert!(!format!("{err:?}").contains(KEY));
}

#[tokio::test]
async fn paginator_with_exclusive_cursor_and_limit_one() {
    let server = MockServer::start().await;
    Mock::given(path("/info"))
        .respond_with(FillsServer {
            fills: (0..4).map(|i| fill(3000 + i, 0, 700 + i)).collect(),
            exclusive: true,
        })
        .mount(&server)
        .await;
    let mut pages = client(&server).user_fills_pages(UserFillsByTime::new(USER, 1).limit(1));
    let all = pages.collect_pages(100).await.unwrap();
    assert_eq!(all.iter().map(|f| f.tid).collect::<Vec<_>>(), vec![700, 701, 702, 703]);
    assert!(pages.is_done());
}

#[tokio::test]
async fn paginator_reports_a_stalled_cursor() {
    let server = MockServer::start().await;
    // Inclusive cursor with one fill per page: the cursor can never move.
    Mock::given(path("/info"))
        .respond_with(FillsServer {
            fills: (0..4).map(|i| fill(4000 + i, 0, 800 + i)).collect(),
            exclusive: false,
        })
        .mount(&server)
        .await;
    let mut pages = client(&server).user_fills_pages(UserFillsByTime::new(USER, 1).limit(1));
    assert_eq!(pages.next_page().await.unwrap().unwrap().len(), 1);
    let err = pages.next_page().await.unwrap_err();
    assert!(
        matches!(err, Error::PaginationStalled { ref cursor, .. } if cursor == "4000_0"),
        "{err:?}"
    );
    assert!(pages.is_done());
}

#[tokio::test]
async fn budget_bounds_slow_responses() {
    let server = MockServer::start().await;
    Mock::given(path("/info"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(5)))
        .mount(&server)
        .await;
    let mut retry = RetryPolicy::default();
    retry.budget = Duration::from_millis(300);
    let c = Client::builder()
        .api_key(KEY)
        .base_url(server.uri())
        .http_client(reqwest::Client::new()) // no timeout of its own
        .retry_policy(retry)
        .build()
        .unwrap();
    let started = Instant::now();
    let err = c.info(&json!({"type": "x"})).await.unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(matches!(err, Error::Timeout { .. }), "{err:?}");
}

#[tokio::test]
async fn slow_retry_after_429_times_out() {
    let server = MockServer::start().await;
    Mock::given(path("/info"))
        .respond_with(SlowAfter429 {
            calls: Default::default(),
        })
        .mount(&server)
        .await;
    let mut retry = RetryPolicy::default();
    retry.budget = Duration::from_millis(500);
    retry.max_jitter = Duration::ZERO;
    let c = Client::builder()
        .api_key(KEY)
        .base_url(server.uri())
        .retry_policy(retry)
        .build()
        .unwrap();
    let err = c.info(&json!({"type": "x"})).await.unwrap_err();
    assert!(matches!(err, Error::Timeout { .. }), "{err:?}");
}

struct SlowAfter429 {
    calls: std::sync::atomic::AtomicUsize,
}

impl Respond for SlowAfter429 {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            ResponseTemplate::new(429)
                .insert_header("retry-after", "0")
                .set_body_json(json!({"correlationId": "C1"}))
        } else {
            ResponseTemplate::new(200).set_delay(Duration::from_secs(5))
        }
    }
}

#[tokio::test]
async fn huge_retry_after_does_not_panic() {
    let server = MockServer::start().await;
    Mock::given(path("/info"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "18446744073709551615")
                .set_body_json(json!({"retryAfterSecs": 1e300})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let err = client(&server).info(&json!({"type": "x"})).await.unwrap_err();
    assert!(matches!(err, Error::RateLimited { attempts: 1, .. }), "{err:?}");
}

#[tokio::test]
async fn redirects_are_not_followed() {
    let server = MockServer::start().await;
    Mock::given(path("/info"))
        .respond_with(ResponseTemplate::new(307).insert_header("location", "https://elsewhere.example/info"))
        .expect(1)
        .mount(&server)
        .await;
    let err = client(&server).info(&json!({"type": "x"})).await.unwrap_err();
    assert_eq!(err.status(), Some(307));
}

#[tokio::test]
async fn echoed_key_is_redacted_from_errors() {
    let server = MockServer::start().await;
    Mock::given(path("/info"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({"error": format!("bad token {KEY}")})))
        .mount(&server)
        .await;
    let err = client(&server).info(&json!({"type": "x"})).await.unwrap_err();
    let text = format!("{err} {err:?}");
    assert!(!text.contains(KEY), "{text}");
    assert!(text.contains("<redacted>"));
}
