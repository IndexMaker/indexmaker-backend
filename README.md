# backend
A repo to develop a web backend system in order to be responsible for index data, reports, invoice, etc

For inserting tokens, `/add-tokens` endpoint can be used.

```curl -X POST http://localhost:3002/add-tokens \
  -H "Content-Type: application/json" \
  -d '{
    "tokens": [
      { "symbol": "BTC", "logo_address": "https://coin-images.coingecko.com/coins/images/1/thumb/bitcoin.png?1696501400" },
       { "symbol": "ETH", "logo_address": "https://coin-images.coingecko.com/coins/images/279/thumb/ethereum.png?1696501628" },
       { "symbol": "XRP", "logo_address": "https://coin-images.coingecko.com/coins/images/44/thumb/xrp-symbol-white-128.png?1696501442" },
       { "symbol": "SOL", "logo_address": "https://coin-images.coingecko.com/coins/images/4128/thumb/solana.png?1718769756" },
       { "symbol": "BNB", "logo_address": "https://coin-images.coingecko.com/coins/images/825/thumb/bnb-icon2_2x.png?1696501970" },
       { "symbol": "DOGE", "logo_address": "" }
    ]
  }'
  ```


Also, `/add-index` is useful to create new indexes
```
curl -X POST http://localhost:3002/add-index \
  -H "Content-Type: application/json" \
  -d '{
    "index_id": 21,
    "name": "SY100",
    "symbol": "SY100",
    "address": "0x9080dd35d88b7de97afd0498fc309784ef7ebc49",
    "category": "Top 100 Market-Cap Tokens",
    "asset_class": "Cryptocurrencies",
    "tokens": ["BTC", "ETH", "XRP", "SOL", "BNB", "DOGE"]
  }'
```




curl -X POST http://localhost:3002/create-index \
  -H "Content-Type: application/json" \
  -d '{
    "indexId": 100,
    "name": "Top 100 Market-Cap Tokens",
    "symbol": "SY100",
    "address": "0x9080dd35d88b7de97afd0498fc309784ef7ebc49",
    "category": "Top 100 Market-Cap Tokens",
    "assetClass": "Cryptocurrencies",
    "tokens": [],
    "initialDate": "2024-01-01",
    "initialPrice": "10.0",
    "coingeckoCategory": "null",
    "exchangesAllowed": ["binance", "bitget"],
    "exchangeTradingFees": "0.001",
    "exchangeAvgSpread": "0.0005",
    "rebalancePeriod": 14
  }'