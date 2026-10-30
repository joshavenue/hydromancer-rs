//! Live tests against mainnet. They are ignored by default; run them with
//! `HYDROMANCER_API_KEY=... cargo test --test live -- --ignored`.

use std::time::{SystemTime, UNIX_EPOCH};

use hydromancer::{CandleInterval, CandleSnapshot, Client, Error, LiquidationHistoryByTime, UserFillsByTime};
use serde_json::json;

const HOUR_MS: u64 = 3_600_000;

fn client() -> Client {
    Client::from_env().expect("set HYDROMANCER_API_KEY to run the live tests")
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

/// A wallet that was liquidated recently, so it is likely to have fills and positions.
async fn active_user(c: &Client) -> String {
    let now = now_ms();
    let liqs = c
        .liquidation_history_by_time(
            &LiquidationHistoryByTime::new()
                .start_time(now - 24 * HOUR_MS)
                .end_time(now)
                .limit(20),
        )
        .await
        .unwrap();
    assert!(!liqs.is_empty(), "no liquidations in the last 24 h");
    liqs[0].user.clone()
}

#[tokio::test]
#[ignore = "needs HYDROMANCER_API_KEY and network"]
async fn live_liquidations_are_newest_first() {
    let c = client();
    let now = now_ms();
    let liqs = c
        .liquidation_history_by_time(
            &LiquidationHistoryByTime::new()
                .start_time(now - 24 * HOUR_MS)
                .end_time(now)
                .limit(50),
        )
        .await
        .unwrap();
    assert!(!liqs.is_empty());
    assert!(liqs.windows(2).all(|w| w[0].fill.time >= w[1].fill.time));
    assert!(liqs.iter().all(|l| l.user == l.liquidation.liquidated_user));
    assert!(liqs
        .iter()
        .all(|l| l.fill.time >= now - 24 * HOUR_MS && l.fill.time <= now));
}

#[tokio::test]
#[ignore = "needs HYDROMANCER_API_KEY and network"]
async fn live_clearinghouse_and_batch() {
    let c = client();
    let user = active_user(&c).await;
    let single = c.clearinghouse_state(&user).await.unwrap();
    assert!(single.time > 0);

    let all = c.clearinghouse_state_all_dexes(&user).await.unwrap();
    assert!(all.contains_key("native"), "{:?}", all.keys().collect::<Vec<_>>());

    let batch = c
        .batch_clearinghouse_states(&[user.as_str(), "0xdfc24b077bc1425ad1dea75bcb6f8158e10df303"])
        .await
        .unwrap();
    assert_eq!(batch.successful_states.len() + batch.failed_wallets.len(), 2);
    assert!(batch
        .successful_states
        .iter()
        .any(|(a, _)| a.eq_ignore_ascii_case(&user)));

    let batch_all = c.batch_clearinghouse_states_all_dexes(&[user.as_str()]).await.unwrap();
    assert!(batch_all.successful_states[0].1.contains_key("native"));
}

#[tokio::test]
#[ignore = "needs HYDROMANCER_API_KEY and network"]
async fn live_fills_paginate_without_duplicates() {
    let c = client();
    let user = active_user(&c).await;
    let now = now_ms();
    let start = now - 7 * 24 * HOUR_MS;

    // Small pages so the paginator has to follow the cursor.
    let mut pages = c.user_fills_pages(UserFillsByTime::new(&user, start).end_time(now).limit(25));
    let paged = pages.collect_pages(20).await.unwrap();
    assert!(!paged.is_empty(), "no fills for {user} in 7 days");
    assert!(paged
        .windows(2)
        .all(|w| (w[0].time, w[0].tx_index) <= (w[1].time, w[1].tx_index)));
    let mut keys: Vec<(u64, u64)> = paged.iter().map(|f| (f.tid, f.oid)).collect();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), paged.len());

    // The same window in one big page must start with the same fills.
    let one = c
        .user_fills_by_time(&UserFillsByTime::new(&user, start).end_time(now))
        .await
        .unwrap();
    let n = paged.len().min(one.len());
    let a: Vec<u64> = paged[..n].iter().map(|f| f.tid).collect();
    let b: Vec<u64> = one[..n].iter().map(|f| f.tid).collect();
    assert_eq!(a, b);
    if one.len() > 25 {
        assert!(pages.pages_fetched() > 1, "paginator did not follow the cursor");
    }
    if pages.is_done() {
        assert_eq!(paged.len(), one.len());
    }
}

#[tokio::test]
#[ignore = "needs HYDROMANCER_API_KEY and network"]
async fn live_candles() {
    let c = client();
    let now = now_ms();
    let candles = c
        .candle_snapshot(&CandleSnapshot::new("BTC", CandleInterval::H1, now - 6 * HOUR_MS, now))
        .await
        .unwrap();
    assert!((5..=7).contains(&candles.len()), "{}", candles.len());
    assert!(candles.windows(2).all(|w| w[0].open_time < w[1].open_time));
    assert!(candles.iter().all(|k| k.coin == "BTC" && k.interval == "1h"));
}

#[tokio::test]
#[ignore = "needs HYDROMANCER_API_KEY and network"]
async fn live_api_usage() {
    let rows = client().api_usage(Some(2)).await.unwrap();
    assert!(!rows.is_empty());
    assert!(rows.iter().all(|r| r.day.len() == 10));
}

#[tokio::test]
#[ignore = "needs HYDROMANCER_API_KEY and network"]
async fn live_info_escape_hatch_and_errors() {
    let c = client();
    let mids = c.info(&json!({"type": "allMids"})).await.unwrap();
    assert!(mids.get("BTC").is_some());

    let err = c.info(&json!({"type": "noSuchRequestType"})).await.unwrap_err();
    assert!(matches!(err, Error::Api { status: 400, .. }), "{err:?}");

    let bad = Client::new("not-a-real-key").unwrap();
    let err = bad.info(&json!({"type": "allMids"})).await.unwrap_err();
    assert!(matches!(err.status(), Some(401 | 403)), "{err:?}");
    assert!(!err.to_string().contains("not-a-real-key"));
}
