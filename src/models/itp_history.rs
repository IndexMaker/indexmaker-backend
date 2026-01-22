//! ITP Price History request/response models
//!
//! Models for the GET /api/itp/{id}/history endpoint that returns
//! historical price data for ITPs.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Valid period values for price history queries
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Day1,
    Day7,
    Day30,
    All,
}

impl Period {
    pub fn as_str(&self) -> &'static str {
        match self {
            Period::Day1 => "1d",
            Period::Day7 => "7d",
            Period::Day30 => "30d",
            Period::All => "all",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "1d" => Some(Period::Day1),
            "7d" => Some(Period::Day7),
            "30d" => Some(Period::Day30),
            "all" => Some(Period::All),
            _ => None,
        }
    }

    /// Returns the granularity to use for this period
    #[allow(dead_code)]
    pub fn granularity(&self) -> &'static str {
        match self {
            Period::Day1 => "5min",
            Period::Day7 => "5min",
            Period::Day30 => "hourly",
            Period::All => "daily",
        }
    }
}

/// Query parameters for price history endpoint
#[derive(Debug, Clone, Deserialize)]
pub struct PriceHistoryQuery {
    /// Period: 1d, 7d, 30d, all (defaults to 7d)
    #[serde(default = "default_period")]
    pub period: String,
}

fn default_period() -> String {
    "7d".to_string()
}

impl PriceHistoryQuery {
    /// Validate the period parameter
    pub fn validate(&self) -> Result<Period, String> {
        Period::from_str(&self.period).ok_or_else(|| {
            format!(
                "Invalid period: '{}'. Must be one of: 1d, 7d, 30d, all",
                self.period
            )
        })
    }
}

/// Single price history entry
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceHistoryEntry {
    /// UTC timestamp
    pub timestamp: DateTime<Utc>,
    /// Price at this timestamp
    pub price: f64,
    /// Optional trading volume
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<f64>,
}

/// Response for price history endpoint
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceHistoryResponse {
    /// Array of price history entries
    pub data: Vec<PriceHistoryEntry>,
    /// ITP vault address
    pub itp_id: String,
    /// Requested period
    pub period: String,
}

/// Generic error response (reuse from ITP module)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryErrorResponse {
    /// Error message
    pub error: String,
    /// Optional error code
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_period_validation_valid() {
        let query = PriceHistoryQuery {
            period: "7d".to_string(),
        };
        assert!(query.validate().is_ok());
        assert_eq!(query.validate().unwrap(), Period::Day7);
    }

    #[test]
    fn test_period_validation_all_valid() {
        for period in ["1d", "7d", "30d", "all"] {
            let query = PriceHistoryQuery {
                period: period.to_string(),
            };
            assert!(query.validate().is_ok());
        }
    }

    #[test]
    fn test_period_validation_invalid() {
        let query = PriceHistoryQuery {
            period: "2d".to_string(),
        };
        let result = query.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid period"));
    }

    #[test]
    fn test_period_granularity() {
        assert_eq!(Period::Day1.granularity(), "5min");
        assert_eq!(Period::Day7.granularity(), "5min");
        assert_eq!(Period::Day30.granularity(), "hourly");
        assert_eq!(Period::All.granularity(), "daily");
    }

    #[test]
    fn test_default_period() {
        assert_eq!(default_period(), "7d");
    }
}
