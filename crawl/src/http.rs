//! A polite HTTP client: one request at a time, a delay between requests,
//! retries with exponential backoff, and size limits.

use std::cell::Cell;
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime};

use ureq::Agent;

use crate::Error;

const ATTEMPTS: u32 = 4;

/// A polite HTTP client: one request at a time, at least `delay` apart, with
/// retries and size limits.
///
/// # Examples
///
/// ```no_run
/// use std::time::Duration;
/// use pelicode_crawl::Client;
///
/// let client = Client::new(Some("https://example.org/about"), Duration::from_secs(1));
/// let response = client.get("https://pypi.org/pypi/six/json", 1024 * 1024)?;
/// println!("{} bytes, modified {:?}", response.body.len(), response.last_modified);
/// # Ok::<(), pelicode_crawl::Error>(())
/// ```
pub struct Client {
    agent: Agent,
    delay: Duration,
    last_request: Cell<Option<Instant>>,
}

/// A successful response.
pub struct Response {
    /// Body of the response.
    pub body: Vec<u8>,
    /// `Last-Modified` as RFC 3339, when present and valid.
    pub last_modified: Option<String>,
}

impl Client {
    /// `contact` (a URL or e-mail address) is added to the User-Agent so that
    /// repository operators can reach the crawler's operator.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use pelicode_crawl::Client;
    ///
    /// let client = Client::new(Some("https://example.org/about"), Duration::from_millis(1000));
    /// ```
    pub fn new(contact: Option<&str>, delay: Duration) -> Client {
        let user_agent = match contact {
            Some(c) => format!(
                "pelicode/{} (research crawler; +{c})",
                env!("CARGO_PKG_VERSION")
            ),
            None => format!("pelicode/{} (research crawler)", env!("CARGO_PKG_VERSION")),
        };
        let config = Agent::config_builder()
            .user_agent(user_agent)
            .timeout_global(Some(Duration::from_secs(300)))
            .build();
        Client {
            agent: Agent::new_with_config(config),
            delay,
            last_request: Cell::new(None),
        }
    }

    /// GETs `url`, reading at most `limit` bytes. 404 becomes [`Error::NotFound`].
    ///
    /// Waits until `delay` has passed since the previous request. On 429, 5xx
    /// and network errors, the request is tried up to four times in all, with
    /// exponential backoff.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] for 404, and [`Error::Http`] for other failures, including
    /// a body larger than `limit`.
    ///
    /// # Examples
    ///
    /// See [`Client`].
    pub fn get(&self, url: &str, limit: u64) -> Result<Response, Error> {
        let mut backoff = Duration::from_secs(1);
        let mut attempt = 1;
        loop {
            self.wait();
            match self.try_get(url, limit) {
                Err(Error::Http(_, ref e)) if attempt < ATTEMPTS && is_transient(e) => {
                    sleep(backoff);
                    backoff *= 2;
                    attempt += 1;
                }
                result => return result,
            }
        }
    }

    fn try_get(&self, url: &str, limit: u64) -> Result<Response, Error> {
        let http_error = |e: ureq::Error| match e {
            ureq::Error::StatusCode(404) => Error::NotFound(url.to_string()),
            e => Error::Http(url.to_string(), e),
        };
        let mut response = self.agent.get(url).call().map_err(http_error)?;
        let last_modified = response
            .headers()
            .get("last-modified")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| httpdate::parse_http_date(v).ok())
            .map(rfc3339);
        let body = response
            .body_mut()
            .with_config()
            .limit(limit)
            .read_to_vec()
            .map_err(http_error)?;
        Ok(Response {
            body,
            last_modified,
        })
    }

    fn wait(&self) {
        if let Some(last) = self.last_request.get() {
            let elapsed = last.elapsed();
            if elapsed < self.delay {
                sleep(self.delay - elapsed);
            }
        }
        self.last_request.set(Some(Instant::now()));
    }
}

fn is_transient(e: &ureq::Error) -> bool {
    match e {
        ureq::Error::StatusCode(code) => *code == 429 || *code >= 500,
        ureq::Error::Io(_) | ureq::Error::Timeout(_) | ureq::Error::ConnectionFailed => true,
        _ => false,
    }
}

/// `time` as an RFC 3339 timestamp in UTC, to the second.
///
/// # Examples
///
/// ```
/// use std::time::{Duration, UNIX_EPOCH};
/// use pelicode_crawl::http::rfc3339;
///
/// assert_eq!(rfc3339(UNIX_EPOCH + Duration::from_secs(86_400)), "1970-01-02T00:00:00Z");
/// ```
pub fn rfc3339(time: SystemTime) -> String {
    humantime::format_rfc3339_seconds(time).to_string()
}
