use chrono::NaiveDateTime;
use regex::Regex;
use scraper::{Html, Selector};
use lazy_static::lazy_static;

lazy_static! {
    // Original regex for "TOKEN/USDT" format
    static ref PAIR_REGEX: Regex = Regex::new(r"([A-Z0-9]+)/([A-Z0-9]+)").unwrap();
    
    // NEW: Regex for "TOKENUSDT" format (common quote assets)
    static ref PAIR_NO_SLASH_REGEX: Regex = Regex::new(
        r"\b([A-Z0-9]{2,10})(USDT|USDC|BTC|ETH|BNB|BUSD)\b"
    ).unwrap();
    
    static ref DATE_REGEX_1: Regex = Regex::new(
        r"(\d{1,2}\s+(?:January|February|March|April|May|June|July|August|September|October|November|December)\s+\d{4}(?:,\s+\d{2}:\d{2})?\s+\(UTC(?:\+\d+)?\))"
    ).unwrap();
    static ref DATE_REGEX_2: Regex = Regex::new(r"(\d{4}-\d{2}-\d{2}\s+\d{2}:\d{2}\s+\(UTC\))").unwrap();
}

/// Extract trading pairs from HTML content
pub fn extract_pairs_from_html(html: &str) -> Vec<String> {
    let mut pairs = Vec::new();
    let document = Html::parse_document(html);

    // 1. Extract from <a href="/spot/TOKENUSDT">
    if let Ok(selector) = Selector::parse("a[href*='/spot/']") {
        for element in document.select(&selector) {
            if let Some(href) = element.value().attr("href") {
                if let Some(pair) = href.split("/spot/").nth(1) {
                    let clean_pair = pair.split('?').next().unwrap_or(pair);
                    if !clean_pair.is_empty() {
                        pairs.push(clean_pair.to_uppercase());
                    }
                }
            }
        }
    }

    // 2. Extract from text content using regex
    // Try "TOKEN/USDT" format first
    for cap in PAIR_REGEX.captures_iter(html) {
        let pair = format!("{}{}", &cap[1], &cap[2]);
        pairs.push(pair);
    }
    
    // 3. Try "TOKENUSDT" format (no slash)
    for cap in PAIR_NO_SLASH_REGEX.captures_iter(html) {
        let pair = format!("{}{}", &cap[1], &cap[2]);
        pairs.push(pair);
    }

    // Deduplicate
    pairs.sort();
    pairs.dedup();
    pairs
}

/// Extract dates from content
pub fn extract_dates_from_content(content: &str) -> Vec<String> {
    let mut dates = Vec::new();

    for cap in DATE_REGEX_1.captures_iter(content) {
        dates.push(cap[0].to_string());
    }

    for cap in DATE_REGEX_2.captures_iter(content) {
        dates.push(cap[0].to_string());
    }

    dates
}

/// Parse trading pair into token and quote asset
pub fn parse_trading_pair(pair: &str) -> Option<(String, String)> {
    let pair_upper = pair.to_uppercase();

    // Try common quote assets
    for quote in &["USDT", "USDC", "BTC", "ETH", "BNB", "BUSD"] {
        if pair_upper.ends_with(quote) {
            let token = pair_upper.trim_end_matches(quote);
            if !token.is_empty() {
                return Some((token.to_string(), quote.to_lowercase()));
            }
        }
    }

    None
}

/// Validate if a pair is properly formatted (all uppercase, no special chars)
pub fn is_valid_pair(pair: &str) -> bool {
    pair == pair.to_uppercase() && pair.chars().all(|c| c.is_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_pairs() {
        let html = r#"<a href="/spot/BTCUSDT">BTC</a> <a href="/spot/ETHUSDC">ETH</a>"#;
        let pairs = extract_pairs_from_html(html);
        assert_eq!(pairs, vec!["BTCUSDT", "ETHUSDC"]);
    }

    #[test]
    fn test_parse_trading_pair() {
        assert_eq!(
            parse_trading_pair("BTCUSDT"),
            Some(("BTC".to_string(), "usdt".to_string()))
        );
        assert_eq!(
            parse_trading_pair("ETHUSDC"),
            Some(("ETH".to_string(), "usdc".to_string()))
        );
    }

    #[test]
    fn test_is_valid_pair() {
        assert!(is_valid_pair("BTCUSDT"));
        assert!(!is_valid_pair("btcusdt"));
        assert!(!is_valid_pair("BTC/USDT"));
    }
}