//! News: headlines from RSS feeds, de-duplicated and tagged with the listed
//! companies they mention.
//!
//! Only headline, source, time and link are kept. Feed bodies and summaries are
//! never read, so there is nothing to render but text, and links open only if
//! they are http or https.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use feed_rs::parser;
use futures::future::join_all;
use nivesh_core::{FeedKind, Provenance, ProviderId, Sourced};

use crate::http::HttpClient;
use crate::{Capabilities, DataError, DataProvider, NewsItem, NewsRequest};

/// A feed to read: a display name and its URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedSource {
    pub name: String,
    pub url: String,
}

impl FeedSource {
    pub fn new(name: &str, url: &str) -> Self {
        Self {
            name: name.to_owned(),
            url: url.to_owned(),
        }
    }
}

/// The feeds read by default: free, public market-news RSS from Indian
/// publishers. The list is configuration, not code: pass another to
/// [`RssProvider::new`].
pub fn default_feeds() -> Vec<FeedSource> {
    vec![
        FeedSource::new(
            "ET Markets",
            "https://economictimes.indiatimes.com/markets/rssfeeds/1977021501.cms",
        ),
        FeedSource::new("Mint", "https://www.livemint.com/rss/markets"),
        FeedSource::new(
            "Business Standard",
            "https://www.business-standard.com/rss/markets-106.rss",
        ),
        FeedSource::new(
            "BusinessLine",
            "https://www.thehindubusinessline.com/markets/feeder/default.rss",
        ),
        FeedSource::new(
            "Moneycontrol",
            "https://www.moneycontrol.com/rss/latestnews.xml",
        ),
    ]
}

/// Items kept from each feed.
const PER_FEED: usize = 60;

/// Feeds are fetched together; one slow publisher must not hold the rest up.
const FEED_TIMEOUT: Duration = Duration::from_secs(20);

pub struct RssProvider {
    http: HttpClient,
    feeds: Vec<FeedSource>,
}

impl RssProvider {
    pub fn new(http: HttpClient, feeds: Vec<FeedSource>) -> Self {
        Self { http, feeds }
    }

    async fn fetch_one(&self, feed: &FeedSource) -> Result<Vec<NewsItem>, DataError> {
        let response = self
            .http
            .get(
                &feed.url,
                &[(
                    "Accept",
                    "application/rss+xml, application/xml, text/xml, */*",
                )],
            )
            .await?;
        parse_feed(&response.body, &feed.name)
    }
}

#[async_trait]
impl DataProvider for RssProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Rss
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::NEWS
    }

    fn feed_kind(&self) -> FeedKind {
        FeedKind::Delayed
    }

    /// Every feed, merged. A feed that fails is skipped (and logged); only if
    /// all of them fail is that an error.
    async fn news(&self, req: &NewsRequest) -> Result<Vec<NewsItem>, DataError> {
        let results = join_all(self.feeds.iter().map(|feed| async move {
            let result = tokio::time::timeout(FEED_TIMEOUT, self.fetch_one(feed))
                .await
                .unwrap_or(Err(DataError::Timeout));
            (feed, result)
        }))
        .await;
        let mut lists = Vec::new();
        let mut first_error = None;
        for (feed, result) in results {
            match result {
                Ok(items) => lists.push(items),
                Err(error) => {
                    tracing::debug!(feed = %feed.name, %error, "news feed failed");
                    first_error = first_error.or(Some(error));
                }
            }
        }
        if lists.is_empty() {
            return Err(first_error.unwrap_or(DataError::NotFound));
        }
        let mut merged = merge(lists);
        merged.truncate(req.limit);
        Ok(merged)
    }
}

// ---- parsing ---------------------------------------------------------------

/// Plain headline text: tags removed, the common entities decoded, whitespace
/// collapsed. Headlines are rendered as text only, so this is for readability,
/// not for safety.
pub fn clean_title(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut in_tag = false;
    for c in raw.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if in_tag => {}
            _ => out.push(c),
        }
    }
    let decoded = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace('\u{a0}', " ");
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether a link may be opened: http or https only, with no whitespace or
/// control characters. Everything else (`javascript:`, `file:`, ...) is dropped.
pub fn is_openable(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://"))
        && lower.len() > "https://".len()
        && !url.chars().any(|c| c.is_control() || c.is_whitespace())
}

/// Headlines of one feed, newest first. Entries without a title, an openable
/// link or a date are skipped.
pub fn parse_feed(body: &[u8], source: &str) -> Result<Vec<NewsItem>, DataError> {
    let feed =
        parser::parse(body).map_err(|e| DataError::Parse(format!("feed `{source}`: {e}")))?;
    let mut items: Vec<NewsItem> = feed
        .entries
        .iter()
        .filter_map(|entry| {
            let title = clean_title(&entry.title.as_ref()?.content);
            let url = entry.links.first()?.href.trim().to_owned();
            let published = entry.published.or(entry.updated)?;
            (!title.is_empty() && is_openable(&url)).then_some(NewsItem {
                title,
                source: source.to_owned(),
                url,
                published,
                tags: Vec::new(),
            })
        })
        .collect();
    items.sort_by_key(|i| std::cmp::Reverse(i.published));
    items.truncate(PER_FEED);
    Ok(items)
}

// ---- merging ---------------------------------------------------------------

/// Letters and digits only, lower-cased: two headlines that differ in
/// punctuation or case are the same story.
fn title_key(title: &str) -> String {
    title
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// A link without its query string, fragment and trailing slash (trackers
/// differ between feeds; the article doesn't).
fn url_key(url: &str) -> String {
    let end = url.find(['?', '#']).unwrap_or(url.len());
    url[..end].trim_end_matches('/').to_ascii_lowercase()
}

/// Merges feeds into one list, newest first, with each story once: items
/// sharing a link or a headline collapse, keeping the earliest-listed source.
pub fn merge(lists: Vec<Vec<NewsItem>>) -> Vec<NewsItem> {
    let mut all: Vec<NewsItem> = lists.into_iter().flatten().collect();
    // Stable sort: of equal times, the feed listed first keeps its place.
    all.sort_by_key(|i| std::cmp::Reverse(i.published));
    let (mut titles, mut urls) = (HashSet::new(), HashSet::new());
    all.retain(|item| {
        let fresh_title = titles.insert(title_key(&item.title));
        let fresh_url = urls.insert(url_key(&item.url));
        fresh_title && fresh_url
    });
    all
}

// ---- tagging -----------------------------------------------------------------

/// Words that end a company's name without telling companies apart.
const NAME_SUFFIXES: [&str; 6] = ["LIMITED", "LTD", "THE", "INDIA", "CORPORATION", "CO"];

/// First words too generic to stand for a company on their own.
const GENERIC_FIRST_WORDS: [&str; 12] = [
    "INDIAN",
    "NATIONAL",
    "STATE",
    "GENERAL",
    "GLOBAL",
    "UNITED",
    "BHARAT",
    "GUJARAT",
    "MAHARASHTRA",
    "SOUTH",
    "WESTERN",
    "FIRST",
];

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '&')
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_uppercase)
        .collect()
}

/// Finds the listed companies a headline mentions.
///
/// It leans towards precision: a headline is tagged with a company when it
/// contains the company's name (without "Limited" and the like), the first word
/// of the name for well-known companies when no other company shares it, or the
/// trading symbol written in capitals (`TCS`, `HDFCBANK`). A miss is better
/// than tagging unrelated news.
#[derive(Debug, Clone, Default)]
pub struct TagIndex {
    /// Upper-case name phrases (1 to 4 words, space-joined) to symbols.
    phrases: HashMap<String, Vec<String>>,
    symbols: HashSet<String>,
}

const MAX_PHRASE_WORDS: usize = 4;
const MAX_TAGS: usize = 4;

impl TagIndex {
    /// `entries` are `(symbol, company name, popularity tier)`; see
    /// `InstrumentIndex::tag_entries`.
    pub fn new(entries: &[(String, String, u32)]) -> Self {
        let mut phrases: HashMap<String, Vec<String>> = HashMap::new();
        let mut first_words: HashMap<String, Vec<String>> = HashMap::new();
        let mut symbols = HashSet::new();
        for (symbol, name, tier) in entries {
            if symbol.len() >= 3 {
                symbols.insert(symbol.clone());
            }
            let mut name_words = words(name);
            while name_words
                .last()
                .is_some_and(|w| NAME_SUFFIXES.contains(&w.as_str()))
            {
                name_words.pop();
            }
            if name_words.is_empty() || name_words.len() > MAX_PHRASE_WORDS {
                continue;
            }
            // Names of two or more words are specific enough to be a key. A
            // one-word name is a key only for a well-known company.
            if name_words.len() >= 2 || *tier <= 2 {
                phrases
                    .entry(name_words.join(" "))
                    .or_default()
                    .push(symbol.clone());
            }
            if *tier <= 1 {
                let first = &name_words[0];
                if first.len() >= 5 && !GENERIC_FIRST_WORDS.contains(&first.as_str()) {
                    first_words
                        .entry(first.clone())
                        .or_default()
                        .push(symbol.clone());
                }
            }
        }
        // A first word shared by several companies (Bajaj, Adani) names none of them.
        for (word, syms) in first_words {
            if syms.len() == 1 && !phrases.contains_key(&word) {
                phrases.insert(word, syms);
            }
        }
        // A phrase naming several listings (a company's two share classes) is ambiguous.
        phrases.retain(|_, syms| syms.len() == 1);
        Self { phrases, symbols }
    }

    /// Symbols mentioned in `title`, in order of first mention, at most four.
    pub fn tag(&self, title: &str) -> Vec<String> {
        let mut found: Vec<(usize, &String)> = Vec::new();
        let tokens: Vec<&str> = title
            .split(|c: char| !c.is_alphanumeric() && c != '&')
            .filter(|w| !w.is_empty())
            .collect();
        let upper: Vec<String> = tokens.iter().map(|t| t.to_ascii_uppercase()).collect();
        for (i, token) in tokens.iter().enumerate() {
            // A symbol counts only when written as one: in capitals.
            if token.len() >= 3
                && **token == *upper[i]
                && let Some(symbol) = self.symbols.get(&upper[i])
            {
                found.push((i, symbol));
            }
            for n in 1..=MAX_PHRASE_WORDS.min(tokens.len() - i) {
                let phrase = upper[i..i + n].join(" ");
                if let Some(syms) = self.phrases.get(&phrase) {
                    found.push((i, &syms[0]));
                }
            }
        }
        found.sort_by_key(|(i, _)| *i);
        let mut out: Vec<String> = Vec::new();
        for (_, symbol) in found {
            if !out.contains(symbol) {
                out.push(symbol.clone());
            }
            if out.len() == MAX_TAGS {
                break;
            }
        }
        out
    }
}

/// Wraps a merged list as a `Sourced` value: as of the newest headline.
pub fn sourced(items: Vec<NewsItem>, now: DateTime<Utc>) -> Sourced<Vec<NewsItem>> {
    let as_of = items.iter().map(|i| i.published).max().unwrap_or(now);
    Sourced::new(
        items,
        Provenance::new(ProviderId::Rss, FeedKind::Delayed, as_of, now),
    )
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;

    fn item(title: &str, source: &str, url: &str, minute: u32) -> NewsItem {
        NewsItem {
            title: title.into(),
            source: source.into(),
            url: url.into(),
            published: Utc.with_ymd_and_hms(2026, 11, 18, 9, minute, 0).unwrap(),
            tags: vec![],
        }
    }

    #[test]
    fn titles_are_plain_text() {
        assert_eq!(
            clean_title("  <b>Sensex</b> &amp; Nifty:&nbsp;what&#39;s   next?<script>x</script> "),
            "Sensex & Nifty: what's next?x"
        );
        assert_eq!(clean_title("<p></p>"), "");
    }

    #[test]
    fn only_http_and_https_links_are_openable() {
        for ok in ["https://a.com/x", "http://a.com", "HTTPS://A.COM/x?y=1"] {
            assert!(is_openable(ok), "{ok}");
        }
        for bad in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,x",
            "ftp://a.com",
            "//a.com",
            "https://",
            "https://a.com/a b",
            "https://a.com/\n",
            "",
        ] {
            assert!(!is_openable(bad), "{bad:?}");
        }
    }

    #[test]
    fn merge_collapses_a_story_seen_in_two_feeds_and_sorts_newest_first() {
        let a = vec![
            item(
                "Sensex jumps 500 points",
                "ET",
                "https://et.com/a?utm=1",
                10,
            ),
            item("RBI holds rates", "ET", "https://et.com/b", 30),
        ];
        let b = vec![
            // Same headline, different case and punctuation, different link.
            item("SENSEX JUMPS 500 POINTS!", "Mint", "https://mint.com/z", 12),
            // Same link as an ET story once trackers are dropped.
            item("Rates on hold", "Mint", "https://et.com/b/?ref=x#top", 31),
            item("IPO opens today", "Mint", "https://mint.com/ipo", 20),
        ];
        let merged = merge(vec![a, b]);
        let titles: Vec<_> = merged.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Rates on hold",
                "IPO opens today",
                "SENSEX JUMPS 500 POINTS!"
            ],
            "{merged:#?}"
        );
    }

    fn entries() -> Vec<(String, String, u32)> {
        [
            ("RELIANCE", "Reliance Industries Limited", 0),
            ("TCS", "Tata Consultancy Services Limited", 0),
            ("INFY", "Infosys Limited", 0),
            ("HDFCBANK", "HDFC Bank Limited", 0),
            ("BAJFINANCE", "Bajaj Finance Limited", 0),
            ("BAJAJFINSV", "Bajaj Finserv Limited", 0),
            ("ITC", "ITC Limited", 0),
            ("IT", "Obscure IT Services Limited", 3),
            ("SBIN", "State Bank of India", 0),
            ("OBSCURE", "Obscure Limited", 3),
        ]
        .iter()
        .map(|(s, n, t)| ((*s).into(), (*n).into(), *t))
        .collect()
    }

    #[test]
    fn tags_names_symbols_and_first_words() {
        let idx = TagIndex::new(&entries());
        assert_eq!(
            idx.tag("Reliance Industries posts record profit"),
            ["RELIANCE"]
        );
        assert_eq!(idx.tag("Reliance eyes retail IPO"), ["RELIANCE"]);
        assert_eq!(
            idx.tag("Infosys raises guidance; TCS flat"),
            ["INFY", "TCS"]
        );
        assert_eq!(idx.tag("HDFC Bank Q2 results beat estimates"), ["HDFCBANK"]);
        assert_eq!(idx.tag("ITC shares rally after demerger"), ["ITC"]);
        assert_eq!(idx.tag("State Bank of India cuts lending rate"), ["SBIN"]);
    }

    #[test]
    fn leans_towards_precision() {
        let idx = TagIndex::new(&entries());
        // A first word shared by two companies tags neither.
        assert!(idx.tag("Bajaj group plans new plant").is_empty());
        // The full name still works.
        assert_eq!(idx.tag("Bajaj Finance raises funds"), ["BAJFINANCE"]);
        // Symbols count only in capitals, and only from three letters.
        assert!(
            idx.tag("it was a good day for the tcs of the world")
                .is_empty()
        );
        assert!(
            idx.tag("IT stocks fall").is_empty(),
            "two-letter symbols are never tags"
        );
        assert!(
            idx.tag("Obscure in talks").is_empty(),
            "unranked one-word names are not keys"
        );
        assert!(idx.tag("Sensex ends higher").is_empty());
    }

    #[test]
    fn at_most_four_tags_without_repeats() {
        let idx = TagIndex::new(&entries());
        let tags = idx.tag("TCS TCS Infosys Reliance HDFC Bank ITC SBIN rally");
        assert_eq!(tags.len(), 4);
        assert_eq!(tags[0], "TCS");
        assert_eq!(tags.iter().filter(|t| *t == "TCS").count(), 1);
    }

    #[test]
    fn sourced_news_is_as_of_the_newest_headline_and_delayed() {
        let now = Utc.with_ymd_and_hms(2026, 11, 18, 12, 0, 0).unwrap();
        let s = sourced(
            vec![
                item("a", "ET", "https://x.com/a", 5),
                item("b", "ET", "https://x.com/b", 50),
            ],
            now,
        );
        assert_eq!(s.provenance.provider, ProviderId::Rss);
        assert_eq!(s.provenance.feed, FeedKind::Delayed);
        assert_eq!(
            s.provenance.as_of,
            Utc.with_ymd_and_hms(2026, 11, 18, 9, 50, 0).unwrap()
        );
    }

    const ET: &[u8] = include_bytes!("../tests/fixtures/rss/et_markets.synthetic.xml");
    const MC: &[u8] = include_bytes!("../tests/fixtures/rss/moneycontrol_latin1.synthetic.xml");
    const ATOM: &[u8] = include_bytes!("../tests/fixtures/rss/atom.synthetic.xml");

    #[test]
    fn parses_rss_dropping_unsafe_undated_and_untitled_items() {
        let items = parse_feed(ET, "ET Markets").unwrap();
        let titles: Vec<_> = items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Sensex, Nifty end higher; IT stocks lead gains",
                "Infosys raises FY27 guidance after strong deal wins",
                "Reliance Industries posts record quarterly profit & revenue",
            ],
            "newest first, entities decoded, junk dropped"
        );
        assert!(
            items
                .iter()
                .all(|i| i.source == "ET Markets" && i.tags.is_empty())
        );
        assert!(items.iter().all(|i| is_openable(&i.url)));
        // 15:45 IST is 10:15 UTC.
        assert_eq!(
            items[0].published,
            Utc.with_ymd_and_hms(2026, 11, 18, 10, 15, 0).unwrap()
        );
    }

    #[test]
    fn never_exposes_a_description() {
        let items = parse_feed(ET, "ET").unwrap();
        let debug = format!("{items:?}");
        assert!(!debug.contains("never read") && !debug.contains("script"));
    }

    #[test]
    fn reads_latin1_and_atom_feeds() {
        let mc = parse_feed(MC, "Moneycontrol").unwrap();
        assert_eq!(mc.len(), 3);
        let rupee = mc.iter().find(|i| i.title.starts_with("Rupee")).unwrap();
        assert!(
            rupee.title.contains('\u{2013}') && rupee.title.contains("café"),
            "{}",
            rupee.title
        );
        let atom = parse_feed(ATOM, "Atom").unwrap();
        assert_eq!(atom.len(), 1);
        assert_eq!(atom[0].url, "https://example.com/news/tcs-contract");
    }

    #[test]
    fn rejects_non_feeds() {
        assert!(matches!(
            parse_feed(b"<html><body>Access denied</body></html>", "X"),
            Err(DataError::Parse(_))
        ));
        assert!(parse_feed(b"", "X").is_err());
    }

    #[test]
    fn the_same_story_from_two_feeds_appears_once() {
        let merged = merge(vec![
            parse_feed(ET, "ET Markets").unwrap(),
            parse_feed(MC, "Moneycontrol").unwrap(),
        ]);
        let sensex: Vec<_> = merged
            .iter()
            .filter(|i| i.title.to_ascii_lowercase().starts_with("sensex"))
            .collect();
        assert_eq!(sensex.len(), 1);
        // Equal times keep the feed listed first; here Moneycontrol's is 5 minutes later.
        assert_eq!(sensex[0].source, "Moneycontrol");
        assert_eq!(merged.len(), 3 + 3 - 1);
    }

    mod provider {
        use std::sync::Arc;

        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        use super::*;
        use crate::http::{HostLimits, HttpConfig, Limit};

        fn provider(server: &MockServer) -> RssProvider {
            let limits = Arc::new(HostLimits::new(Limit::new(Duration::from_millis(1), 1000)));
            let config = HttpConfig {
                max_attempts: 1,
                ..HttpConfig::default()
            };
            RssProvider::new(
                HttpClient::new(config, limits).unwrap(),
                vec![
                    FeedSource::new("ET Markets", &format!("{}/et.xml", server.uri())),
                    FeedSource::new("Moneycontrol", &format!("{}/mc.xml", server.uri())),
                    FeedSource::new("Down", &format!("{}/down.xml", server.uri())),
                ],
            )
        }

        async fn serve(server: &MockServer, route: &str, status: u16, body: &[u8]) {
            Mock::given(method("GET"))
                .and(path(route))
                .respond_with(ResponseTemplate::new(status).set_body_bytes(body))
                .mount(server)
                .await;
        }

        #[tokio::test]
        async fn merges_the_feeds_that_answer_and_skips_the_one_that_does_not() {
            let server = MockServer::start().await;
            serve(&server, "/et.xml", 200, ET).await;
            serve(&server, "/mc.xml", 200, MC).await;
            serve(&server, "/down.xml", 503, b"").await;
            let items = provider(&server)
                .news(&NewsRequest {
                    query: None,
                    limit: 100,
                })
                .await
                .unwrap();
            assert_eq!(items.len(), 5);
            assert!(items.windows(2).all(|w| w[0].published >= w[1].published));
            let limited = provider(&server)
                .news(&NewsRequest {
                    query: None,
                    limit: 2,
                })
                .await
                .unwrap();
            assert_eq!(limited.len(), 2);
        }

        #[tokio::test]
        async fn fails_only_when_every_feed_fails() {
            let server = MockServer::start().await;
            serve(&server, "/et.xml", 200, b"<html>blocked</html>").await;
            serve(&server, "/mc.xml", 404, b"").await;
            serve(&server, "/down.xml", 503, b"").await;
            let err = provider(&server)
                .news(&NewsRequest {
                    query: None,
                    limit: 10,
                })
                .await
                .unwrap_err();
            assert!(
                matches!(
                    err,
                    DataError::Parse(_) | DataError::NotFound | DataError::Upstream(_)
                ),
                "{err:?}"
            );
        }
    }
}
