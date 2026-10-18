//! Rankings of packages from [ecosyste.ms](https://ecosyste.ms/), for the
//! repositories that publish none themselves.
//!
//! ecosyste.ms collects the metadata of the public package repositories and
//! serves it through a free API (5,000 requests an hour without a key). Its
//! data is licensed under CC BY-SA 4.0; pelicode uses only the names.

use serde_json::Value;

use crate::Error;
use crate::http::Client;

const API: &str = "https://packages.ecosyste.ms/api/v1/registries";
const PER_PAGE: usize = 100;
const LIMIT: u64 = 1024 * 1024;

/// How ecosyste.ms ranks the packages of a repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ranking {
    /// By downloads, for repositories that publish them.
    Downloads,
    /// By the number of repositories depending on the package.
    DependentRepositories,
}

impl Ranking {
    fn sort(self) -> &'static str {
        match self {
            Ranking::Downloads => "downloads",
            Ranking::DependentRepositories => "dependent_repos_count",
        }
    }
}

/// The names of the `n` best ranked packages of `registry` (a registry name of
/// ecosyste.ms, such as `rubygems.org`).
///
/// # Errors
///
/// [`Error::Metadata`] when the response cannot be understood, and [`Error::Http`]
/// for network failures.
///
/// # Examples
///
/// ```no_run
/// use std::time::Duration;
/// use pelicode_crawl::Client;
/// use pelicode_crawl::ecosystems::{Ranking, top_packages};
///
/// let client = Client::new(Some("https://example.org/about"), Duration::from_secs(1));
/// let gems = top_packages(&client, "rubygems.org", Ranking::Downloads, 100)?;
/// # Ok::<(), pelicode_crawl::Error>(())
/// ```
pub fn top_packages(
    client: &Client,
    registry: &str,
    ranking: Ranking,
    n: usize,
) -> Result<Vec<String>, Error> {
    let mut names: Vec<String> = Vec::with_capacity(n);
    let mut page = 1;
    while names.len() < n {
        let url = format!(
            "{API}/{registry}/package_names?sort={}&order=desc&per_page={PER_PAGE}&page={page}",
            ranking.sort()
        );
        let json: Value = serde_json::from_slice(&client.get(&url, LIMIT)?.body)
            .map_err(|e| Error::Metadata(format!("{url}: {e}")))?;
        let found = parse_names(&json)
            .ok_or_else(|| Error::Metadata(format!("{url}: not a list of names")))?;
        if found.is_empty() {
            break;
        }
        for name in found {
            if !names.contains(&name) {
                names.push(name);
            }
        }
        page += 1;
    }
    names.truncate(n);
    Ok(names)
}

/// The names of a `package_names` response: a JSON array of strings.
///
/// # Examples
///
/// ```
/// use pelicode_crawl::ecosystems::parse_names;
///
/// let json = serde_json::json!(["junit:junit", "org.projectlombok:lombok"]);
/// assert_eq!(parse_names(&json).unwrap(), ["junit:junit", "org.projectlombok:lombok"]);
/// assert!(parse_names(&serde_json::json!({"error": "x"})).is_none());
/// ```
pub fn parse_names(json: &Value) -> Option<Vec<String>> {
    Some(
        json.as_array()?
            .iter()
            .filter_map(|n| n.as_str())
            .filter(|n| !n.is_empty())
            .map(String::from)
            .collect(),
    )
}
