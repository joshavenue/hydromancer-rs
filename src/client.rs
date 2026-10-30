use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use reqwest::header::{HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::requests::{CandleSnapshot, LiquidationHistoryByTime, UserFillsByTime};
use crate::retry::{header_u64, retry_after_header, RateLimitBody, RetryPolicy};
use crate::types::{AllDexesState, ApiUsage, BatchClearinghouseStates, Candle, ClearinghouseState, Fill, Liquidation};

/// Mainnet API base URL.
pub const MAINNET_URL: &str = "https://api.hydromancer.xyz";
/// Testnet API base URL. Testnet access must be enabled for your key.
pub const TESTNET_URL: &str = "https://api-testnet.hydromancer.xyz";
/// Environment variable read by [`Client::from_env`].
pub const API_KEY_ENV: &str = "HYDROMANCER_API_KEY";
/// `User-Agent` sent with every request unless overridden.
pub const DEFAULT_USER_AGENT: &str = concat!("hydromancer-rs/", env!("CARGO_PKG_VERSION"));
/// The `dex` value that asks for every perp DEX (native plus HIP-3) at once.
pub const ALL_DEXES: &str = "ALL_DEXES";
/// Most users per `batchClearinghouseStates` request.
pub const MAX_BATCH_USERS: usize = 1000;
/// Most users per `batchClearinghouseStates` request with `"dex": "ALL_DEXES"`.
pub const MAX_BATCH_USERS_ALL_DEXES: usize = 100;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
const BODY_SNIPPET: usize = 4096;

/// Client for the Hydromancer REST API.
///
/// Every query is a `POST /info` with a JSON body whose `type` picks the
/// operation. Typed helpers cover the common ones; [`Client::info`] sends
/// anything else.
///
/// The client is cheap to clone and safe to share between tasks. Its `Debug`
/// output never contains the API key.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    base_url: String,
    info_url: reqwest::Url,
    key: String,
    auth: HeaderValue,
    user_agent: HeaderValue,
    retry: RetryPolicy,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("base_url", &self.inner.base_url)
            .field("user_agent", &self.inner.user_agent)
            .field("retry", &self.inner.retry)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

/// Builder for [`Client`].
///
/// ```
/// use std::time::Duration;
/// use hydromancer::Client;
///
/// let client = Client::builder()
///     .api_key("your-key")
///     .testnet()
///     .timeout(Duration::from_secs(30))
///     .max_retries(5)
///     .build()?;
/// # let _ = client;
/// # Ok::<(), hydromancer::Error>(())
/// ```
#[derive(Clone)]
pub struct ClientBuilder {
    api_key: Option<String>,
    base_url: String,
    user_agent: String,
    timeout: Duration,
    http: Option<reqwest::Client>,
    retry: RetryPolicy,
}

impl fmt::Debug for ClientBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientBuilder")
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("base_url", &self.base_url)
            .field("user_agent", &self.user_agent)
            .field("timeout", &self.timeout)
            .field("retry", &self.retry)
            .finish_non_exhaustive()
    }
}

impl Default for ClientBuilder {
    fn default() -> Self {
        Self {
            api_key: None,
            base_url: MAINNET_URL.to_owned(),
            user_agent: DEFAULT_USER_AGENT.to_owned(),
            timeout: DEFAULT_TIMEOUT,
            http: None,
            retry: RetryPolicy::default(),
        }
    }
}

impl ClientBuilder {
    /// Uses this API key. Without it, [`build`](Self::build) reads `HYDROMANCER_API_KEY`.
    pub fn api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    /// Sends requests to another base URL, for example a proxy. Defaults to [`MAINNET_URL`].
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Shorthand for `base_url(TESTNET_URL)`.
    pub fn testnet(self) -> Self {
        self.base_url(TESTNET_URL)
    }

    /// Replaces the `User-Agent` (default [`DEFAULT_USER_AGENT`]). It must not be empty:
    /// the API rejects some default HTTP-library agents with 403 "error code: 1010".
    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    /// Per-request timeout for the built-in HTTP client. Default 60 s.
    /// Ignored when you pass your own client with [`http_client`](Self::http_client).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Uses your own `reqwest::Client` (proxies, pools, TLS settings). The API key
    /// and `User-Agent` are still added to every request.
    pub fn http_client(mut self, client: reqwest::Client) -> Self {
        self.http = Some(client);
        self
    }

    /// Sets the whole 429 retry policy.
    pub fn retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry = policy;
        self
    }

    /// Retries after the first attempt on HTTP 429. `0` disables retries. Default 3.
    pub fn max_retries(mut self, retries: u32) -> Self {
        self.retry.max_retries = retries;
        self
    }

    /// Builds the client.
    ///
    /// # Errors
    ///
    /// [`Error::MissingApiKey`] when no key was given and `HYDROMANCER_API_KEY`
    /// is unset or empty; [`Error::InvalidApiKey`], [`Error::InvalidBaseUrl`] or
    /// [`Error::InvalidUserAgent`] for values that cannot be used.
    pub fn build(self) -> Result<Client> {
        let key = match self.api_key {
            Some(k) => k,
            None => std::env::var(API_KEY_ENV).unwrap_or_default(),
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(Error::MissingApiKey);
        }
        let mut auth = HeaderValue::from_str(&format!("Bearer {key}")).map_err(|_| Error::InvalidApiKey)?;
        auth.set_sensitive(true);

        let raw = self.base_url.trim();
        let mut info_url = reqwest::Url::parse(raw).map_err(|e| Error::InvalidBaseUrl(format!("{raw}: {e}")))?;
        match info_url.scheme() {
            "https" => {}
            "http" if is_loopback(&info_url) => {}
            "http" => {
                return Err(Error::InvalidBaseUrl(format!(
                    "{raw}: plain http is only allowed for localhost; use https"
                )))
            }
            _ => return Err(Error::InvalidBaseUrl(format!("{raw}: scheme must be https"))),
        }
        if !info_url.username().is_empty()
            || info_url.password().is_some()
            || info_url.query().is_some()
            || info_url.fragment().is_some()
        {
            return Err(Error::InvalidBaseUrl(format!(
                "{raw}: must not contain credentials, a query or a fragment"
            )));
        }
        let path = info_url.path().trim_end_matches('/').to_owned();
        info_url.set_path(&path);
        let base_url = info_url.as_str().trim_end_matches('/').to_owned();
        info_url.set_path(&format!("{path}/info"));

        let ua = self.user_agent.trim();
        if ua.is_empty() {
            return Err(Error::InvalidUserAgent("must not be empty".into()));
        }
        let user_agent = HeaderValue::from_str(ua).map_err(|e| Error::InvalidUserAgent(e.to_string()))?;

        let http = match self.http {
            Some(c) => c,
            None => reqwest::Client::builder()
                .user_agent(ua)
                .timeout(self.timeout)
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
        };

        Ok(Client {
            inner: Arc::new(Inner {
                http,
                info_url,
                base_url,
                key: key.to_owned(),
                auth,
                user_agent,
                retry: self.retry,
            }),
        })
    }
}

impl Client {
    /// A mainnet client using `api_key`.
    ///
    /// ```
    /// let client = hydromancer::Client::new("your-key")?;
    /// # let _ = client;
    /// # Ok::<(), hydromancer::Error>(())
    /// ```
    pub fn new(api_key: impl Into<String>) -> Result<Self> {
        ClientBuilder::default().api_key(api_key).build()
    }

    /// A mainnet client using the key in `HYDROMANCER_API_KEY`.
    ///
    /// # Errors
    ///
    /// [`Error::MissingApiKey`] when the variable is unset or empty.
    pub fn from_env() -> Result<Self> {
        ClientBuilder::default().build()
    }

    /// A builder for testnet, a custom base URL, timeouts or retry settings.
    pub fn builder() -> ClientBuilder {
        ClientBuilder::default()
    }

    /// The base URL requests go to.
    pub fn base_url(&self) -> &str {
        &self.inner.base_url
    }

    /// Sends any JSON body to `POST /info` and returns the JSON response.
    ///
    /// Use it for endpoints without a typed helper. The body must contain
    /// `type`; see the [endpoint list](https://docs.hydromancer.xyz/llms.txt).
    /// Binary endpoints (for example `perpSnapshots`) are not JSON and return
    /// [`Error::Decode`].
    ///
    /// ```no_run
    /// # async fn run() -> Result<(), hydromancer::Error> {
    /// use serde_json::json;
    ///
    /// let client = hydromancer::Client::from_env()?;
    /// let mids = client.info(&json!({"type": "allMids"})).await?;
    /// println!("BTC mid: {}", mids["BTC"]);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn info<B: Serialize + ?Sized>(&self, body: &B) -> Result<Value> {
        self.info_as(body).await
    }

    /// Like [`info`](Self::info), but deserializes the response into `T`.
    pub async fn info_as<T: DeserializeOwned, B: Serialize + ?Sized>(&self, body: &B) -> Result<T> {
        let body =
            serde_json::to_vec(body).map_err(|e| Error::InvalidRequest(format!("body is not serializable: {e}")))?;
        let bytes = self.post(body).await?;
        serde_json::from_slice(&bytes).map_err(|e| Error::Decode {
            message: redact(e.to_string(), &self.inner.key),
            body: redact(snippet(&bytes), &self.inner.key),
        })
    }

    /// Perp account state of one user on the native DEX (`clearinghouseState`).
    ///
    /// ```no_run
    /// # async fn run() -> Result<(), hydromancer::Error> {
    /// let client = hydromancer::Client::from_env()?;
    /// let state = client.clearinghouse_state("0xdfc24b077bc1425ad1dea75bcb6f8158e10df303").await?;
    /// println!("account value: {}", state.margin_summary.account_value);
    /// for p in &state.asset_positions {
    ///     println!("{} size {}", p.position.coin, p.position.szi);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn clearinghouse_state(&self, user: &str) -> Result<ClearinghouseState> {
        self.info_as(&json!({"type": "clearinghouseState", "user": user})).await
    }

    /// Perp account state of one user on one HIP-3 DEX, for example `"xyz"`.
    pub async fn clearinghouse_state_on_dex(&self, user: &str, dex: &str) -> Result<ClearinghouseState> {
        self.info_as(&json!({"type": "clearinghouseState", "user": user, "dex": dex}))
            .await
    }

    /// Perp account state of one user on every DEX, keyed by DEX name (`"native"` for the native DEX).
    /// This costs more weight than a single-DEX call.
    pub async fn clearinghouse_state_all_dexes(&self, user: &str) -> Result<AllDexesState> {
        self.info_as(&json!({"type": "clearinghouseState", "user": user, "dex": ALL_DEXES}))
            .await
    }

    /// Account state of up to 1,000 users in one request (`batchClearinghouseStates`).
    ///
    /// Wallets that could not be fetched are listed in `failed_wallets`.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidRequest`] for an empty list or more than [`MAX_BATCH_USERS`] users.
    ///
    /// ```no_run
    /// # async fn run() -> Result<(), hydromancer::Error> {
    /// let client = hydromancer::Client::from_env()?;
    /// let batch = client
    ///     .batch_clearinghouse_states(&["0xdfc24b077bc1425ad1dea75bcb6f8158e10df303"])
    ///     .await?;
    /// for (address, state) in &batch.successful_states {
    ///     println!("{address}: {} positions", state.asset_positions.len());
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn batch_clearinghouse_states<U: AsRef<str>>(&self, users: &[U]) -> Result<BatchClearinghouseStates> {
        let users = check_batch(users, MAX_BATCH_USERS)?;
        self.info_as(&json!({"type": "batchClearinghouseStates", "users": users}))
            .await
    }

    /// Like [`batch_clearinghouse_states`](Self::batch_clearinghouse_states), on one HIP-3 DEX.
    pub async fn batch_clearinghouse_states_on_dex<U: AsRef<str>>(
        &self,
        users: &[U],
        dex: &str,
    ) -> Result<BatchClearinghouseStates> {
        let users = check_batch(users, MAX_BATCH_USERS)?;
        self.info_as(&json!({"type": "batchClearinghouseStates", "users": users, "dex": dex}))
            .await
    }

    /// Account state of up to 100 users on every DEX in one request.
    /// The weight is multiplied by the number of DEXs.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidRequest`] for an empty list or more than [`MAX_BATCH_USERS_ALL_DEXES`] users.
    pub async fn batch_clearinghouse_states_all_dexes<U: AsRef<str>>(
        &self,
        users: &[U],
    ) -> Result<BatchClearinghouseStates<AllDexesState>> {
        let users = check_batch(users, MAX_BATCH_USERS_ALL_DEXES)?;
        self.info_as(&json!({"type": "batchClearinghouseStates", "users": users, "dex": ALL_DEXES}))
            .await
    }

    /// One page of fills for a user, oldest first (`userFillsByTime`).
    ///
    /// A page holds at most 2,000 fills. To read a longer history use
    /// [`user_fills_pages`](Self::user_fills_pages).
    pub async fn user_fills_by_time(&self, request: &UserFillsByTime) -> Result<Vec<Fill>> {
        self.info_as(request).await
    }

    /// Follows the `userFillsByTime` cursor page by page, dropping fills already seen.
    ///
    /// ```no_run
    /// # async fn run() -> Result<(), hydromancer::Error> {
    /// use hydromancer::UserFillsByTime;
    ///
    /// let client = hydromancer::Client::from_env()?;
    /// let day_ago = 1_793_000_000_000; // Unix ms
    /// let mut pages = client.user_fills_pages(UserFillsByTime::new("0xdfc24b077bc1425ad1dea75bcb6f8158e10df303", day_ago));
    /// while let Some(fills) = pages.next_page().await? {
    ///     for fill in fills {
    ///         println!("{} {} {} @ {}", fill.time, fill.coin, fill.sz, fill.px);
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn user_fills_pages(&self, request: UserFillsByTime) -> UserFillsPages {
        UserFillsPages::new(self.clone(), request)
    }

    /// Liquidation events, newest first (`liquidationHistoryByTime`).
    ///
    /// ```no_run
    /// # async fn run() -> Result<(), hydromancer::Error> {
    /// use hydromancer::LiquidationHistoryByTime;
    ///
    /// let client = hydromancer::Client::from_env()?;
    /// let events = client
    ///     .liquidation_history_by_time(&LiquidationHistoryByTime::new().coin("BTC").limit(100))
    ///     .await?;
    /// for e in events {
    ///     println!("{} {} {} @ {} ({})", e.fill.time, e.user, e.fill.sz, e.fill.px, e.liquidation.method);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn liquidation_history_by_time(&self, request: &LiquidationHistoryByTime) -> Result<Vec<Liquidation>> {
        self.info_as(request).await
    }

    /// OHLCV candles, oldest first (`candleSnapshot`).
    ///
    /// # Errors
    ///
    /// [`Error::InvalidRequest`] when `end_time` is not after `start_time`.
    ///
    /// ```no_run
    /// # async fn run() -> Result<(), hydromancer::Error> {
    /// use hydromancer::{CandleInterval, CandleSnapshot};
    ///
    /// let client = hydromancer::Client::from_env()?;
    /// let end = 1_793_086_400_000;
    /// let candles = client
    ///     .candle_snapshot(&CandleSnapshot::new("BTC", CandleInterval::H1, end - 24 * 3_600_000, end))
    ///     .await?;
    /// for c in candles {
    ///     println!("{} o={} h={} l={} c={}", c.open_time, c.open, c.high, c.low, c.close);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn candle_snapshot(&self, request: &CandleSnapshot) -> Result<Vec<Candle>> {
        if request.req.end_time <= request.req.start_time {
            return Err(Error::InvalidRequest(
                "candleSnapshot: end_time must be greater than start_time".into(),
            ));
        }
        self.info_as(request).await
    }

    /// Your key's usage by day and request type (`apiUsage`). `days` defaults to 30 on the server.
    ///
    /// ```no_run
    /// # async fn run() -> Result<(), hydromancer::Error> {
    /// let client = hydromancer::Client::from_env()?;
    /// for row in client.api_usage(Some(7)).await? {
    ///     println!("{} {}: {} requests", row.day, row.request_type, row.total_requests);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn api_usage(&self, days: Option<u32>) -> Result<Vec<ApiUsage>> {
        match days {
            Some(d) => self.info_as(&json!({"type": "apiUsage", "days": d})).await,
            None => self.info_as(&json!({"type": "apiUsage"})).await,
        }
    }

    async fn post(&self, body: Vec<u8>) -> Result<Vec<u8>> {
        let inner = &*self.inner;
        let policy = &inner.retry;
        let started = Instant::now();
        let deadline = started.checked_add(policy.budget).map(tokio::time::Instant::from_std);
        let mut attempts: u32 = 0;
        loop {
            attempts += 1;
            let attempt = async {
                let response = inner
                    .http
                    .post(inner.info_url.clone())
                    .header(AUTHORIZATION, inner.auth.clone())
                    .header(USER_AGENT, inner.user_agent.clone())
                    .header(CONTENT_TYPE, "application/json")
                    .header(ACCEPT, "application/json")
                    .body(body.clone())
                    .send()
                    .await?;
                let status = response.status();
                let headers = response.headers().clone();
                let bytes = response.bytes().await?;
                Ok::<_, reqwest::Error>((status, headers, bytes))
            };
            let outcome = match deadline {
                Some(deadline) => match tokio::time::timeout_at(deadline, attempt).await {
                    Ok(outcome) => outcome,
                    Err(_) => return Err(Error::Timeout { budget: policy.budget }),
                },
                None => attempt.await,
            };
            let (status, headers, bytes) = outcome?;

            if status.is_success() {
                return Ok(bytes.to_vec());
            }
            if status != StatusCode::TOO_MANY_REQUESTS {
                return Err(api_error(status, &bytes, &inner.key));
            }

            let rl = RateLimitBody::parse(&bytes);
            let hint = retry_after_header(&headers, SystemTime::now()).or_else(|| rl.retry_after());
            let retry = attempts - 1;
            let wait = policy.wait(hint, retry).saturating_add(policy.jitter());
            let remaining = policy.budget.saturating_sub(started.elapsed());
            if retry >= policy.max_retries || wait > remaining {
                return Err(Error::RateLimited {
                    attempts,
                    retry_after: hint,
                    remaining: rl.remaining,
                    required: rl.required,
                    limit: header_u64(&headers, "x-ratelimit-limit"),
                    correlation_id: rl.correlation_id.map(|id| redact(id, &inner.key)),
                });
            }
            tokio::time::sleep(wait).await;
        }
    }
}

fn is_loopback(url: &reqwest::Url) -> bool {
    let host = url.host_str().unwrap_or_default();
    host.eq_ignore_ascii_case("localhost")
        || host.to_ascii_lowercase().ends_with(".localhost")
        || host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Removes the API key from text that came from the server, in case a proxy echoes it.
fn redact(text: String, key: &str) -> String {
    // Very short keys would mangle ordinary text; real keys are much longer.
    if key.len() >= 8 && text.contains(key) {
        text.replace(key, "<redacted>")
    } else {
        text
    }
}

fn check_batch<U: AsRef<str>>(users: &[U], max: usize) -> Result<Vec<&str>> {
    if users.is_empty() {
        return Err(Error::InvalidRequest(
            "batchClearinghouseStates: users must not be empty".into(),
        ));
    }
    if users.len() > max {
        return Err(Error::InvalidRequest(format!(
            "batchClearinghouseStates: {} users given, at most {max} allowed; split the list",
            users.len()
        )));
    }
    Ok(users.iter().map(AsRef::as_ref).collect())
}

fn api_error(status: StatusCode, bytes: &[u8], key: &str) -> Error {
    let body = redact(snippet(bytes), key);
    let message = serde_json::from_slice::<Value>(bytes)
        .ok()
        .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_else(|| {
            let mut m: String = body.chars().take(200).collect();
            if m.trim().is_empty() {
                m = status.canonical_reason().unwrap_or("no body").to_owned();
            }
            m
        });
    Error::Api {
        status: status.as_u16(),
        message: redact(message, key),
        body,
    }
}

fn snippet(bytes: &[u8]) -> String {
    let end = bytes.len().min(BODY_SNIPPET);
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Pages through `userFillsByTime`, created by [`Client::user_fills_pages`].
///
/// Each call to [`next_page`](Self::next_page) sends one request with the
/// cursor of the previous page's last fill and returns only fills not seen
/// before (keyed on trade id and order id, so both sides of a self-trade are
/// kept). The server's cursor is exclusive: the next page starts after the
/// cursor fill. Paging stops after a short or empty page. If a full page does
/// not move the cursor forward, [`next_page`](Self::next_page) returns
/// [`Error::PaginationStalled`] rather than stopping silently.
#[derive(Debug)]
pub struct UserFillsPages {
    client: Client,
    request: UserFillsByTime,
    page_size: usize,
    cursor: Option<String>,
    seen: HashSet<(u64, u64)>,
    pages: u32,
    done: bool,
}

impl UserFillsPages {
    fn new(client: Client, request: UserFillsByTime) -> Self {
        Self {
            client,
            page_size: request.page_size() as usize,
            cursor: request.cursor.clone(),
            request,
            seen: HashSet::new(),
            pages: 0,
            done: false,
        }
    }

    /// Fetches the next page. Returns `Ok(None)` when there are no more fills.
    ///
    /// A page can come back empty (`Some(vec![])`) if every fill on it was a duplicate.
    pub async fn next_page(&mut self) -> Result<Option<Vec<Fill>>> {
        if self.done {
            return Ok(None);
        }
        let mut request = self.request.clone();
        if let Some(cursor) = &self.cursor {
            request.cursor = Some(cursor.clone());
            request.start_time = None;
        }
        let page = self.client.user_fills_by_time(&request).await?;
        self.pages += 1;

        let Some(last) = page.last() else {
            self.done = true;
            return Ok(None);
        };
        let next = last.cursor();
        let short = page.len() < self.page_size;
        if !short {
            let stalled = match self.cursor.as_deref() {
                Some(current) => match parse_cursor(current) {
                    Some(current) => (last.time, last.tx_index) <= current,
                    None => current == next,
                },
                None => false,
            };
            if stalled {
                self.done = true;
                return Err(Error::PaginationStalled { cursor: next });
            }
        }
        self.done = short;
        self.cursor = Some(next);

        let fresh = page.into_iter().filter(|f| self.seen.insert((f.tid, f.oid))).collect();
        Ok(Some(fresh))
    }

    /// Fetches up to `max_pages` more pages and returns their fills in order.
    ///
    /// Check [`is_done`](Self::is_done) afterwards: if it is `false` the
    /// history was longer and you can call this again.
    pub async fn collect_pages(&mut self, max_pages: u32) -> Result<Vec<Fill>> {
        let mut out = Vec::new();
        for _ in 0..max_pages {
            match self.next_page().await? {
                Some(fills) => out.extend(fills),
                None => break,
            }
        }
        Ok(out)
    }

    /// `true` once the last page has been read.
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// The cursor the next request will use (`"<time>_<txIndex>"`), for resuming later.
    pub fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    /// Requests sent so far.
    pub fn pages_fetched(&self) -> u32 {
        self.pages
    }
}

fn parse_cursor(cursor: &str) -> Option<(u64, u64)> {
    let (time, tx_index) = cursor.split_once('_')?;
    Some((time.parse().ok()?, tx_index.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_shows_key() {
        let client = Client::new("super-secret-key").unwrap();
        let dbg = format!("{client:?}");
        assert!(!dbg.contains("super-secret-key"), "{dbg}");
        assert!(dbg.contains("<redacted>"));
        let builder = Client::builder().api_key("super-secret-key");
        assert!(!format!("{builder:?}").contains("super-secret-key"));
    }

    #[test]
    fn empty_key_is_rejected() {
        assert!(matches!(Client::new("  "), Err(Error::MissingApiKey)));
    }

    #[test]
    fn invalid_key_error_does_not_echo_key() {
        let err = Client::new("bad\nkey").unwrap_err();
        assert!(matches!(err, Error::InvalidApiKey));
        assert!(!err.to_string().contains("bad"));
    }

    #[test]
    fn base_url_is_normalised() {
        let c = Client::builder()
            .api_key("k")
            .base_url("http://localhost:1234/")
            .build()
            .unwrap();
        assert_eq!(c.base_url(), "http://localhost:1234");
        assert_eq!(c.inner.info_url.as_str(), "http://localhost:1234/info");
        let c = Client::builder()
            .api_key("k")
            .base_url("https://proxy.example/hl")
            .build()
            .unwrap();
        assert_eq!(c.inner.info_url.as_str(), "https://proxy.example/hl/info");
        let c = Client::builder()
            .api_key("k")
            .base_url("https://proxy.example/hl//")
            .build()
            .unwrap();
        assert_eq!(c.inner.info_url.as_str(), "https://proxy.example/hl/info");
        assert_eq!(c.base_url(), "https://proxy.example/hl");
        for bad in [
            "http://api.example.com",
            "https://user:pw@api.example.com",
            "https://api.example.com/?token=x",
            "https://api.example.com/#f",
        ] {
            assert!(
                matches!(
                    Client::builder().api_key("k").base_url(bad).build(),
                    Err(Error::InvalidBaseUrl(_))
                ),
                "{bad}"
            );
        }
        for ok in ["http://127.0.0.1:9", "http://[::1]:9", "http://LOCALHOST:9"] {
            assert!(Client::builder().api_key("k").base_url(ok).build().is_ok(), "{ok}");
        }
        let c = Client::builder().api_key("k").testnet().build().unwrap();
        assert_eq!(c.base_url(), TESTNET_URL);
        assert!(matches!(
            Client::builder().api_key("k").base_url("ftp://x").build(),
            Err(Error::InvalidBaseUrl(_))
        ));
    }

    #[test]
    fn server_text_is_redacted() {
        let e = api_error(
            StatusCode::UNAUTHORIZED,
            br#"{"error":"bad key secret-123"}"#,
            "secret-123",
        );
        assert!(!format!("{e} {e:?}").contains("secret-123"));
        assert_eq!(parse_cursor("1793066136762_10"), Some((1793066136762, 10)));
        assert_eq!(parse_cursor("garbage"), None);
    }

    #[test]
    fn user_agent_must_not_be_empty() {
        assert!(matches!(
            Client::builder().api_key("k").user_agent(" ").build(),
            Err(Error::InvalidUserAgent(_))
        ));
        assert!(DEFAULT_USER_AGENT.starts_with("hydromancer-rs/"));
    }

    #[test]
    fn batch_limits() {
        let none: [&str; 0] = [];
        assert!(check_batch(&none, 10).is_err());
        let many = vec!["0x0"; 101];
        assert!(check_batch(&many, MAX_BATCH_USERS_ALL_DEXES).is_err());
        assert_eq!(check_batch(&many, MAX_BATCH_USERS).unwrap().len(), 101);
    }

    #[test]
    fn api_error_uses_error_field() {
        let e = api_error(
            StatusCode::BAD_REQUEST,
            br#"{"error":"Unknown API route: x"}"#,
            "test-key-123",
        );
        assert!(matches!(e, Error::Api { status: 400, ref message, .. } if message == "Unknown API route: x"));
        let e = api_error(StatusCode::FORBIDDEN, b"error code: 1010", "test-key-123");
        assert!(matches!(e, Error::Api { status: 403, ref message, .. } if message == "error code: 1010"));
    }
}
