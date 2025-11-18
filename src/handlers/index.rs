use axum::{extract::State, Json};
use crate::models::index::{
    CollateralToken, IndexListEntry, IndexListResponse, Performance, Ratings,
};
use crate::AppState;

pub async fn get_index_list(State(_state): State<AppState>) -> Json<IndexListResponse> {
    // TODO: Replace with actual database query
    let mock_data = vec![IndexListEntry {
        index_id: 21,
        name: "SY100_V2".to_string(),
        address: "0x9080dd35d88b7de97afd0498fc309784ef7ebc49".to_string(),
        ticker: "SY100".to_string(),
        curator: "0xF7F7d5C0d394f75307B4D981E8DE2Bab9639f90F".to_string(),
        total_supply: 0.00002010588139611647,
        total_supply_usd: 6.195548738217032,
        ytd_return: -11.49,
        collateral: vec![
            CollateralToken {
                name: "BTC".to_string(),
                logo: "https://coin-images.coingecko.com/coins/images/1/thumb/bitcoin.png?1696501400".to_string(),
            },
            CollateralToken {
                name: "ETH".to_string(),
                logo: "https://coin-images.coingecko.com/coins/images/279/thumb/ethereum.png?1696501628".to_string(),
            },
            CollateralToken {
                name: "XRP".to_string(),
                logo: "https://coin-images.coingecko.com/coins/images/44/thumb/xrp-symbol-white-128.png?1696501442".to_string(),
            },
            CollateralToken {
                name: "SOL".to_string(),
                logo: "https://coin-images.coingecko.com/coins/images/4128/thumb/solana.png?1718769756".to_string(),
            },
            CollateralToken {
                name: "BNB".to_string(),
                logo: "https://coin-images.coingecko.com/coins/images/825/thumb/bnb-icon2_2x.png?1696501970".to_string(),
            },
            CollateralToken {
                name: "DOGE".to_string(),
                logo: "".to_string(),
            },
        ],
        management_fee: 2,
        asset_class: Some("Cryptocurrencies".to_string()),
        category: Some("Top 100 Market-Cap Tokens".to_string()),
        inception_date: Some("2019-01-01".to_string()),
        performance: Some(Performance {
            ytd_return: -11.49,
            one_year_return: 76.38137132434154,
            three_year_return: 237.1885256621526,
            five_year_return: 1738.3370284019127,
            ten_year_return: 0.0,
        }),
        ratings: Some(Ratings {
            overall_rating: "A+".to_string(),
            expense_rating: "B".to_string(),
            risk_rating: "C+".to_string(),
        }),
        index_price: Some(308146.09),
    }];

    Json(IndexListResponse {
        indexes: mock_data,
    })
}
