use std::time::Duration;

/// Result alias used throughout this crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Errors returned by [`Client`](crate::Client).
///
/// No variant ever contains the API key.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// No API key was given and `HYDROMANCER_API_KEY` is unset or empty.
    #[error("no API key: set HYDROMANCER_API_KEY or pass a key to Client::new (get one at https://docs.hydromancer.xyz/readme/get-api-keys)")]
    MissingApiKey,

    /// The API key contains characters that cannot be sent in an HTTP header.
    #[error("the API key contains characters that are not valid in an HTTP header")]
    InvalidApiKey,

    /// The base URL could not be parsed.
    #[error("invalid base URL: {0}")]
    InvalidBaseUrl(String),

    /// The user agent cannot be sent in an HTTP header.
    #[error("invalid User-Agent: {0}")]
    InvalidUserAgent(String),

    /// The request was rejected on the client side before it was sent,
    /// for example a batch with too many users.
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    /// The HTTP request failed before a response arrived (DNS, TLS, timeout, connection reset).
    #[error("HTTP transport error: {0}")]
    Transport(#[from] reqwest::Error),

    /// The API answered with a non-success status other than 429.
    #[error("Hydromancer API returned HTTP {status}: {message}")]
    Api {
        /// HTTP status code.
        status: u16,
        /// The `error` field of the JSON body when present, otherwise the start of the body.
        message: String,
        /// The raw response body (truncated to 4 KiB).
        body: String,
    },

    /// HTTP 429 that was still returned after the configured retries, or
    /// whose wait would not fit in the remaining retry budget.
    #[error("rate limited by the Hydromancer API after {attempts} attempt(s){}", fmt_retry(.retry_after))]
    RateLimited {
        /// How many requests were sent in total.
        attempts: u32,
        /// How long the server asked us to wait before the next attempt.
        retry_after: Option<Duration>,
        /// Weight points remaining in the key's window (`remaining` in the body).
        remaining: Option<u64>,
        /// Weight points this request needs (`required` in the body).
        required: Option<u64>,
        /// Bucket capacity from `X-RateLimit-Limit`.
        limit: Option<u64>,
        /// Server correlation id, useful when contacting support.
        correlation_id: Option<String>,
    },

    /// The response body was not the JSON shape this method expects.
    #[error("could not decode the Hydromancer API response: {source}")]
    Decode {
        /// The underlying JSON error.
        #[source]
        source: serde_json::Error,
        /// The start of the response body (truncated to 4 KiB).
        body: String,
    },
}

fn fmt_retry(retry_after: &Option<Duration>) -> String {
    match retry_after {
        Some(d) => format!("; server asked to retry after {:.1}s", d.as_secs_f64()),
        None => String::new(),
    }
}

impl Error {
    /// The HTTP status code, when the error came from an HTTP response.
    pub fn status(&self) -> Option<u16> {
        match self {
            Error::Api { status, .. } => Some(*status),
            Error::RateLimited { .. } => Some(429),
            Error::Transport(e) => e.status().map(|s| s.as_u16()),
            _ => None,
        }
    }
}
