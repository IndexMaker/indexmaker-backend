use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Query parameters for GET /api/market-cap/history
#[derive(Debug, Clone, Deserialize)]
pub struct MarketCapHistoryQuery {
    pub coin_id: String,
    pub start_date: Option<String>, // YYYY-MM-DD format
    pub end_date: Option<String>,   // YYYY-MM-DD format
}

/// Query parameters for GET /api/market-cap/top-category
#[derive(Debug, Clone, Deserialize)]
pub struct TopCategoryQuery {
    pub category_id: String,
    pub top: Option<u32>,        // Default: 10, Max: 250
    pub date: Option<String>,    // YYYY-MM-DD format, Default: today
}

/// Single coin in top category response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopCategoryCoin {
    pub rank: u32,
    pub coin_id: String,
    pub symbol: String,
    pub name: String,
    pub market_cap: f64,
    pub price: f64,
    pub volume_24h: f64,
}

/// Response structure for top category endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopCategoryResponse {
    pub category_id: String,
    pub category_name: String,
    pub date: String,
    pub top: u32,
    pub coins: Vec<TopCategoryCoin>,
}

/// Single data point in market cap history
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketCapDataPoint {
    pub date: DateTime<Utc>,
    pub market_cap: f64,
    pub price: f64,
    pub volume_24h: f64,
}

/// Response structure for market cap history endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketCapHistoryResponse {
    pub coin_id: String,
    pub symbol: String,
    pub data: Vec<MarketCapDataPoint>,
}

impl MarketCapHistoryQuery {
    /// Validates query parameters
    pub fn validate(&self) -> Result<(), String> {
        // Validate coin_id is not empty
        if self.coin_id.trim().is_empty() {
            return Err("coin_id cannot be empty".to_string());
        }

        // Validate start_date format if provided
        if let Some(ref start) = self.start_date {
            if chrono::NaiveDate::parse_from_str(start, "%Y-%m-%d").is_err() {
                return Err(format!(
                    "Invalid start_date format: '{}'. Expected YYYY-MM-DD",
                    start
                ));
            }
        }

        // Validate end_date format if provided
        if let Some(ref end) = self.end_date {
            if chrono::NaiveDate::parse_from_str(end, "%Y-%m-%d").is_err() {
                return Err(format!(
                    "Invalid end_date format: '{}'. Expected YYYY-MM-DD",
                    end
                ));
            }
        }

        Ok(())
    }
}

impl TopCategoryQuery {
    /// Validates query parameters
    pub fn validate(&self) -> Result<(), String> {
        // Validate category_id is not empty
        if self.category_id.trim().is_empty() {
            return Err("category_id cannot be empty".to_string());
        }

        // Validate top parameter is within range (1-250)
        if let Some(top_val) = self.top {
            if top_val < 1 || top_val > 250 {
                return Err(format!(
                    "top parameter must be between 1 and 250, got: {}",
                    top_val
                ));
            }
        }

        // Validate date format if provided
        if let Some(ref date_str) = self.date {
            if chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d").is_err() {
                return Err(format!(
                    "Invalid date format: '{}'. Expected YYYY-MM-DD",
                    date_str
                ));
            }
        }

        Ok(())
    }

    /// Get the top value with default of 10
    pub fn get_top(&self) -> u32 {
        self.top.unwrap_or(10)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // MarketCapHistoryQuery tests
    #[test]
    fn test_validate_empty_coin_id() {
        let query = MarketCapHistoryQuery {
            coin_id: "".to_string(),
            start_date: None,
            end_date: None,
        };
        assert!(query.validate().is_err());
        assert_eq!(
            query.validate().unwrap_err(),
            "coin_id cannot be empty"
        );
    }

    #[test]
    fn test_validate_whitespace_coin_id() {
        let query = MarketCapHistoryQuery {
            coin_id: "   ".to_string(),
            start_date: None,
            end_date: None,
        };
        assert!(query.validate().is_err());
    }

    #[test]
    fn test_validate_valid_coin_id() {
        let query = MarketCapHistoryQuery {
            coin_id: "bitcoin".to_string(),
            start_date: None,
            end_date: None,
        };
        assert!(query.validate().is_ok());
    }

    #[test]
    fn test_validate_invalid_start_date_format() {
        let query = MarketCapHistoryQuery {
            coin_id: "bitcoin".to_string(),
            start_date: Some("2024/01/01".to_string()),
            end_date: None,
        };
        assert!(query.validate().is_err());
        assert!(query
            .validate()
            .unwrap_err()
            .contains("Invalid start_date format"));
    }

    #[test]
    fn test_validate_invalid_end_date_format() {
        let query = MarketCapHistoryQuery {
            coin_id: "bitcoin".to_string(),
            start_date: None,
            end_date: Some("01-01-2024".to_string()),
        };
        assert!(query.validate().is_err());
        assert!(query
            .validate()
            .unwrap_err()
            .contains("Invalid end_date format"));
    }

    #[test]
    fn test_validate_valid_date_formats() {
        let query = MarketCapHistoryQuery {
            coin_id: "ethereum".to_string(),
            start_date: Some("2024-01-01".to_string()),
            end_date: Some("2024-12-31".to_string()),
        };
        assert!(query.validate().is_ok());
    }

    #[test]
    fn test_serialize_market_cap_data_point() {
        use chrono::TimeZone;
        
        let data_point = MarketCapDataPoint {
            date: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            market_cap: 850000000000.50,
            price: 42500.25,
            volume_24h: 25000000000.00,
        };

        let json = serde_json::to_string(&data_point).unwrap();
        assert!(json.contains("market_cap"));
        assert!(json.contains("850000000000.5"));
        assert!(json.contains("42500.25"));
    }

    #[test]
    fn test_serialize_market_cap_history_response() {
        use chrono::TimeZone;
        
        let response = MarketCapHistoryResponse {
            coin_id: "bitcoin".to_string(),
            symbol: "BTC".to_string(),
            data: vec![
                MarketCapDataPoint {
                    date: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
                    market_cap: 850000000000.50,
                    price: 42500.25,
                    volume_24h: 25000000000.00,
                },
            ],
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("bitcoin"));
        assert!(json.contains("BTC"));
        assert!(json.contains("data"));
    }

    // TopCategoryQuery tests
    #[test]
    fn test_top_category_validate_empty_category_id() {
        let query = TopCategoryQuery {
            category_id: "".to_string(),
            top: None,
            date: None,
        };
        assert!(query.validate().is_err());
        assert_eq!(
            query.validate().unwrap_err(),
            "category_id cannot be empty"
        );
    }

    #[test]
    fn test_top_category_validate_whitespace_category_id() {
        let query = TopCategoryQuery {
            category_id: "   ".to_string(),
            top: None,
            date: None,
        };
        assert!(query.validate().is_err());
    }

    #[test]
    fn test_top_category_validate_valid_category_id() {
        let query = TopCategoryQuery {
            category_id: "decentralized-finance-defi".to_string(),
            top: None,
            date: None,
        };
        assert!(query.validate().is_ok());
    }

    #[test]
    fn test_top_category_validate_top_below_range() {
        let query = TopCategoryQuery {
            category_id: "defi".to_string(),
            top: Some(0),
            date: None,
        };
        assert!(query.validate().is_err());
        assert!(query.validate().unwrap_err().contains("between 1 and 250"));
    }

    #[test]
    fn test_top_category_validate_top_above_range() {
        let query = TopCategoryQuery {
            category_id: "defi".to_string(),
            top: Some(251),
            date: None,
        };
        assert!(query.validate().is_err());
        assert!(query.validate().unwrap_err().contains("between 1 and 250"));
    }

    #[test]
    fn test_top_category_validate_top_valid_range() {
        let query = TopCategoryQuery {
            category_id: "defi".to_string(),
            top: Some(50),
            date: None,
        };
        assert!(query.validate().is_ok());
    }

    #[test]
    fn test_top_category_validate_invalid_date_format() {
        let query = TopCategoryQuery {
            category_id: "defi".to_string(),
            top: Some(10),
            date: Some("2024/01/01".to_string()),
        };
        assert!(query.validate().is_err());
        assert!(query.validate().unwrap_err().contains("Invalid date format"));
    }

    #[test]
    fn test_top_category_validate_valid_date_format() {
        let query = TopCategoryQuery {
            category_id: "defi".to_string(),
            top: Some(10),
            date: Some("2024-01-01".to_string()),
        };
        assert!(query.validate().is_ok());
    }

    #[test]
    fn test_top_category_get_top_default() {
        let query = TopCategoryQuery {
            category_id: "defi".to_string(),
            top: None,
            date: None,
        };
        assert_eq!(query.get_top(), 10);
    }

    #[test]
    fn test_top_category_get_top_custom() {
        let query = TopCategoryQuery {
            category_id: "defi".to_string(),
            top: Some(50),
            date: None,
        };
        assert_eq!(query.get_top(), 50);
    }

    #[test]
    fn test_serialize_top_category_coin() {
        let coin = TopCategoryCoin {
            rank: 1,
            coin_id: "uniswap".to_string(),
            symbol: "UNI".to_string(),
            name: "Uniswap".to_string(),
            market_cap: 5200000000.00,
            price: 8.45,
            volume_24h: 150000000.00,
        };

        let json = serde_json::to_string(&coin).unwrap();
        assert!(json.contains("uniswap"));
        assert!(json.contains("UNI"));
        assert!(json.contains("5200000000"));
    }

    #[test]
    fn test_serialize_top_category_response() {
        let response = TopCategoryResponse {
            category_id: "decentralized-finance-defi".to_string(),
            category_name: "Decentralized Finance (DeFi)".to_string(),
            date: "2025-01-12".to_string(),
            top: 10,
            coins: vec![
                TopCategoryCoin {
                    rank: 1,
                    coin_id: "uniswap".to_string(),
                    symbol: "UNI".to_string(),
                    name: "Uniswap".to_string(),
                    market_cap: 5200000000.00,
                    price: 8.45,
                    volume_24h: 150000000.00,
                },
            ],
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("decentralized-finance-defi"));
        assert!(json.contains("Decentralized Finance"));
        assert!(json.contains("2025-01-12"));
        assert!(json.contains("coins"));
    }
}
