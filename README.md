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


Also, `/create-index` is useful to create new indexes
```
curl -X POST http://localhost:3002/create-index \
  -H "Content-Type: application/json" \
  -d '{
    "indexId": 21,
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
```

Note: The above endpoint, only adds index data related to rebalancing, historical data. About 
deployment data and index constituents, one should use below binary to correctly set all the necessary information:

```

# IndexMaker API Documentation

Base URL: `https://api2.indexmaker.global/`

## Table of Contents
- [GET Endpoints](#get-endpoints)
- [POST Endpoints](#post-endpoints)
- [Data Structures](#data-structures)

---

## GET Endpoints

### 1. Health Check
**Endpoint:** `/`  
**Method:** GET  
**Description:** Returns a welcome message to verify the API is running.

**Response:**
```
Hello from IndexMaker Backend! 🚀
```

---

### 2. Get Index List
**Endpoint:** `/indexes`  
**Method:** GET  
**Description:** Retrieves a list of all available indexes with their details.

**Response Structure:**
```json
{
  "indexes": [
    {
      "indexId": 21,
      "name": "Top 100 Market-Cap Tokens",
      "address": "0x1a64a446e31f19172c6eb3197a1e85ff664af380",
      "ticker": "SY100",
      "curator": "0xF7F7d5C0d394f75307B4D981E8DE2Bab9639f90F",
      "totalSupply": 0.0,
      "totalSupplyUSD": 0.0,
      "ytdReturn": 9.24,
      "collateral": [
        {
          "name": "BTC",
          "logo": ""
        }
      ],
      "managementFee": 2,
      "assetClass": "Cryptocurrencies",
      "inceptionDate": "2024-01-02",
      "category": "Top 100 Market-Cap Tokens",
      "ratings": {
        "overallRating": "A+",
        "expenseRating": "B",
        "riskRating": "C+"
      },
      "performance": {
        "ytdReturn": 9.24,
        "oneYearReturn": -59.412583899937374,
        "threeYearReturn": 0.0,
        "fiveYearReturn": 0.0,
        "tenYearReturn": 0.0
      },
      "indexPrice": 117899.79481497542
    }
  ]
}
```

**Fields:**
- `indexId`: Unique identifier for the index
- `name`: Full name of the index
- `address`: Smart contract address
- `ticker`: Trading symbol
- `curator`: Address of the index curator
- `totalSupply`: Total supply of index tokens
- `totalSupplyUSD`: Total supply value in USD
- `ytdReturn`: Year-to-date return percentage
- `collateral`: Array of assets in the index
- `managementFee`: Management fee percentage
- `assetClass`: Asset classification
- `inceptionDate`: Date when index was created
- `category`: Index category
- `ratings`: Rating object with overall, expense, and risk ratings
- `performance`: Performance metrics over different time periods
- `indexPrice`: Current price of the index

---

### 3. Get Index Maker Info
**Endpoint:** `/get-index-maker-info`  
**Method:** GET  
**Description:** Returns global information about the IndexMaker platform.

**Response:**
```json
{
  "totalVolume": "0",
  "totalManaged": "0"
}
```

**Fields:**
- `totalVolume`: Total trading volume across all indexes
- `totalManaged`: Total assets under management

---

### 4. Get CoinGecko Categories
**Endpoint:** `/coingecko-categories`  
**Method:** GET  
**Description:** Returns all available cryptocurrency categories from CoinGecko (728 categories).

**Response Structure:**
```json
[
  {
    "categoryId": "dn-404",
    "name": " DN-404"
  },
  {
    "categoryId": "puzzle-games",
    "name": " Puzzle Games"
  }
]
```

**Fields:**
- `categoryId`: Unique identifier for the category
- `name`: Display name of the category

---

### 5. Fetch All Assets
**Endpoint:** `/fetch-all-assets`  
**Method:** GET  
**Description:** Retrieves all available cryptocurrency assets (98 assets).

**Response Structure:**
```json
[
  {
    "id": "bitcoin",
    "symbol": "BTC",
    "name": "Bitcoin",
    "totalSupply": 19971437.0,
    "circulatingSupply": 19971437.0,
    "priceUsd": 91233.0,
    "marketCap": 1821746282317.0,
    "expectedInventory": 0.01275424752817658,
    "thumb": "https://coin-images.coingecko.com/coins/images/1/large/bitcoin.png?1696501400"
  }
]
```

**Fields:**
- `id`: CoinGecko asset identifier
- `symbol`: Trading symbol
- `name`: Full asset name
- `totalSupply`: Total supply of the asset
- `circulatingSupply`: Circulating supply
- `priceUsd`: Current price in USD
- `marketCap`: Market capitalization
- `expectedInventory`: Expected inventory allocation
- `thumb`: URL to asset thumbnail image

---

### 6. Get Current Index Weight
**Endpoint:** `/current-index-weight/{index_id}`  
**Method:** GET  
**Description:** Returns the current weight distribution and constituents of a specific index.

**URL Parameters:**
- `index_id`: The ID of the index (e.g., 21)

**Response Structure:**
```json
{
  "indexId": 21,
  "indexName": "Top 100 Market-Cap Tokens",
  "indexSymbol": "SY100",
  "lastRebalanceDate": "2025-12-21",
  "portfolioValue": "110408.09057211776534645660289",
  "totalWeight": "98",
  "constituents": [
    {
      "coinId": "bitcoin",
      "symbol": "BTC",
      "weight": "1",
      "weightPercentage": 1.0204081632653061,
      "quantity": "0.0127542475281765803362509269",
      "price": 88347.94310567998,
      "value": 1126.811534975104,
      "exchange": "binance",
      "tradingPair": "usdc"
    }
  ]
}
```

**Fields:**
- `indexId`: Index identifier
- `indexName`: Full index name
- `indexSymbol`: Trading symbol
- `lastRebalanceDate`: Date of last rebalance
- `portfolioValue`: Total portfolio value
- `totalWeight`: Sum of all weights
- `constituents`: Array of assets in the index with:
  - `coinId`: Asset identifier
  - `symbol`: Trading symbol
  - `weight`: Weight value
  - `weightPercentage`: Weight as percentage
  - `quantity`: Quantity held
  - `price`: Current price
  - `value`: Total value of holding
  - `exchange`: Exchange where traded
  - `tradingPair`: Trading pair

---

### 7. Get Index Configuration
**Endpoint:** `/get-index-config/{index_id}`  
**Method:** GET  
**Description:** Returns configuration details for a specific index.

**URL Parameters:**
- `index_id`: The ID of the index (e.g., 21)

**Response:**
```json
{
  "indexId": 21,
  "symbol": "SY100",
  "name": "Top 100 Market-Cap Tokens",
  "address": "0x1a64a446e31f19172c6eb3197a1e85ff664af380",
  "initialDate": "2024-01-01",
  "initialPrice": "200000",
  "exchangesAllowed": ["binance", "bitget"],
  "exchangeTradingFees": "0.001",
  "exchangeAvgSpread": "0.0005",
  "rebalancePeriod": 30
}
```

**Fields:**
- `indexId`: Index identifier
- `symbol`: Trading symbol
- `name`: Full index name
- `address`: Smart contract address
- `initialDate`: Creation date
- `initialPrice`: Initial price
- `exchangesAllowed`: List of allowed exchanges
- `exchangeTradingFees`: Trading fee percentage
- `exchangeAvgSpread`: Average spread percentage
- `rebalancePeriod`: Rebalancing period in days

---

### 8. Fetch Vault Assets
**Endpoint:** `/fetch-vault-assets/{index_id}`  
**Method:** GET  
**Description:** Returns assets held in the vault for a specific index.

**URL Parameters:**
- `index_id`: The ID of the index (e.g., 21)

**Response Structure:**
```json
[
  {
    "id": 1,
    "ticker": "BTC",
    "pair": "btcusdc",
    "listing": "bi",
    "assetname": "Bitcoin",
    "sector": "Bitcoin Ecosystem",
    "marketCap": 1821746282317.0,
    "weights": "1.00",
    "quantity": 0.01275424752817658
  }
]
```

**Fields:**
- `id`: Asset identifier in vault
- `ticker`: Trading symbol
- `pair`: Trading pair
- `listing`: Exchange listing code
- `assetname`: Full asset name
- `sector`: Asset sector/category
- `marketCap`: Market capitalization
- `weights`: Asset weight in index
- `quantity`: Quantity held in vault

---

### 9. Get Index Transactions
**Endpoint:** `/indexes/{index_id}/transactions`  
**Method:** GET  
**Description:** Returns transaction history for a specific index.

**URL Parameters:**
- `index_id`: The ID of the index (e.g., 21)

**Response:**
```json
[]
```
**Note:** Returns an array of transactions. Empty if no transactions exist.

---

### 10. Get Index Last Price
**Endpoint:** `/indexes/{index_id}/last-price`  
**Method:** GET  
**Description:** Returns the most recent price data for an index including all constituents.

**URL Parameters:**
- `index_id`: The ID of the index (e.g., 21)

**Response Structure:**
```json
{
  "indexId": 21,
  "timestamp": 1766275200,
  "lastPrice": 120548.73542006688,
  "lastBid": null,
  "lastAsk": null,
  "constituents": [
    {
      "coinId": "bitcoin",
      "symbol": "BTC",
      "quantity": "0.0127542475281765803362509269",
      "weight": "1",
      "price": 90593.85443180415,
      "value": 1155.4564439548271
    }
  ]
}
```

**Fields:**
- `indexId`: Index identifier
- `timestamp`: Unix timestamp
- `lastPrice`: Latest index price
- `lastBid`: Last bid price (may be null)
- `lastAsk`: Last ask price (may be null)
- `constituents`: Array of constituent assets with quantities, prices, and values

---

### 11. Get Index Price at Date
**Endpoint:** `/indexes/{index_id}/price-at-date`  
**Method:** GET  
**Description:** Returns the index price and constituents at a specific date.

**URL Parameters:**
- `index_id`: The ID of the index (e.g., 21)

**Query Parameters:**
- `date`: Date in YYYY-MM-DD format (e.g., "2025-01-01")

**Example:**
```
GET /indexes/21/price-at-date?date=2025-01-01
```

**Response Structure:**
```json
{
  "indexId": 21,
  "date": "2025-01-01",
  "price": 243457.83963272453,
  "constituents": [
    {
      "coinId": "bitcoin",
      "symbol": "BTC",
      "quantity": "0.0268247406715615630479768689",
      "weight": "1",
      "price": 93507.85874741492,
      "value": 2508.324061652415
    }
  ]
}
```

**Fields:**
- `indexId`: Index identifier
- `date`: Requested date
- `price`: Index price on that date
- `constituents`: Array of constituent holdings on that date

---

### 12. Get Deposit Transaction Data
**Endpoint:** `/get-deposit-transaction-data/{index_id}/{address}`  
**Method:** GET  
**Description:** Returns deposit transaction data for a specific address and index.

**URL Parameters:**
- `index_id`: The ID of the index (e.g., 21)
- `address`: Wallet address

**Response:**
```json
[]
```
**Note:** Returns an array of deposit transactions. Empty if no deposits exist.

---

### 13. Fetch Coin Historical Data
**Endpoint:** `/fetch-coin-historical-data/{coin_id}`  
**Method:** GET  
**Description:** Returns historical price data for a specific cryptocurrency.

**URL Parameters:**
- `coin_id`: CoinGecko coin identifier (e.g., "bitcoin")

**Response Structure:**
```json
{
  "data": [
    {
      "name": "bitcoin",
      "date": "2019-01-01T00:00:00.000Z",
      "price": 3692.531565524698,
      "value": 10000.0
    },
    {
      "name": "bitcoin",
      "date": "2019-01-02T00:00:00.000Z",
      "price": 3794.264253739717,
      "value": 10275.50932581009
    }
  ]
}
```

**Fields:**
- `data`: Array of historical data points
  - `name`: Coin identifier
  - `date`: ISO date string
  - `price`: Price on that date
  - `value`: Normalized value (starting at 10000)

---

### 14. Download Daily Price Data
**Endpoint:** `/download-daily-price-data/{index_id}`  
**Method:** GET  
**Description:** Returns historical daily price data in CSV format.

**URL Parameters:**
- `index_id`: The ID of the index (e.g., 21)

**Response Format:** CSV  
**Content-Type:** text/csv

**CSV Structure:**
```csv
Index,IndexId,Date,Price,Asset Quantities,Asset Prices
Top 100 Market-Cap Tokens,21,2024-01-02,209728.0111118932,"{bitcoin:0.04738...}","{bitcoin:44168.68...}"
```

**Columns:**
- `Index`: Index name
- `IndexId`: Index identifier
- `Date`: Date of data point
- `Price`: Index price on that date
- `Asset Quantities`: JSON object with asset quantities
- `Asset Prices`: JSON object with asset prices

---

### 15. Fetch Index Historical Data
**Endpoint:** `/fetch-index-historical-data/{index_id}`  
**Method:** GET  
**Description:** Returns complete historical data for an index including chart data and transactions.

**URL Parameters:**
- `index_id`: The ID of the index (e.g., 21)

**Response Structure:**
```json
{
  "indexId": 21,
  "name": "Top 100 Market-Cap Tokens",
  "chartData": [
    {
      "name": "Top 100 Market-Cap Tokens",
      "date": "2024-01-02T00:00:00.000Z",
      "price": 209728.0111118932,
      "value": 10000.0
    }
  ],
  "formattedTransactions": []
}
```

**Fields:**
- `indexId`: Index identifier
- `name`: Index name
- `chartData`: Array of historical price points
  - `name`: Index name
  - `date`: ISO date string
  - `price`: Index price
  - `value`: Normalized value (starting at 10000)
- `formattedTransactions`: Array of formatted transactions (empty if none)

---

## POST Endpoints

### 16. Create Index
**Endpoint:** `/create-index`  
**Method:** POST  
**Description:** Creates a new index with specified configuration.

**Expected Request Body:**
```json
{
  "name": "string",
  "symbol": "string",
  "curator": "string",
  "assetClass": "string",
  "category": "string",
  "managementFee": "number",
  "initialPrice": "number",
  "exchangesAllowed": ["string"],
  "constituents": [
    {
      "coinId": "string",
      "weight": "number"
    }
  ]
}
```

**Response:** TBD (requires authentication/authorization)

---

### 17. Remove Index
**Endpoint:** `/remove-index`  
**Method:** POST  
**Description:** Removes an existing index from the platform.

**Expected Request Body:**
```json
{
  "indexId": "number"
}
```

**Response:** TBD (requires authentication/authorization)

---

### 18. Save Blockchain Event
**Endpoint:** `/save-blockchain-event`  
**Method:** POST  
**Description:** Records blockchain events (e.g., deposits, withdrawals, rebalances).

**Expected Request Body:**
```json
{
  "eventType": "string",
  "indexId": "number",
  "transactionHash": "string",
  "blockNumber": "number",
  "address": "string",
  "amount": "string",
  "timestamp": "number"
}
```

**Response:** TBD (requires authentication/authorization)

---

### 19. Subscribe
**Endpoint:** `/subscribe`  
**Method:** POST  
**Description:** Subscribe to IndexMaker updates or notifications.

**Expected Request Body:**
```json
{
  "email": "string",
  "subscriptionType": "string"
}
```

**Response:** TBD

---

## Data Structures

### Index Object
```typescript
interface Index {
  indexId: number;
  name: string;
  address: string;
  ticker: string;
  curator: string;
  totalSupply: number;
  totalSupplyUSD: number;
  ytdReturn: number;
  collateral: Asset[];
  managementFee: number;
  assetClass: string;
  inceptionDate: string;
  category: string;
  ratings: Ratings;
  performance: Performance;
  indexPrice: number;
}
```

### Asset Object
```typescript
interface Asset {
  id?: string;
  name: string;
  symbol?: string;
  logo?: string;
  totalSupply?: number;
  circulatingSupply?: number;
  priceUsd?: number;
  marketCap?: number;
  expectedInventory?: number;
  thumb?: string;
}
```

### Constituent Object
```typescript
interface Constituent {
  coinId: string;
  symbol: string;
  weight: string;
  weightPercentage: number;
  quantity: string;
  price: number;
  value: number;
  exchange: string;
  tradingPair: string;
}
```

### Ratings Object
```typescript
interface Ratings {
  overallRating: string;
  expenseRating: string;
  riskRating: string;
}
```

### Performance Object
```typescript
interface Performance {
  ytdReturn: number;
  oneYearReturn: number;
  threeYearReturn: number;
  fiveYearReturn: number;
  tenYearReturn: number;
}
```

### Historical Data Point
```typescript
interface HistoricalDataPoint {
  name: string;
  date: string;
  price: number;
  value: number;
}
```

## Notes

- All monetary values are returned as numbers or strings for precision
- Dates are returned in ISO 8601 format
- The API uses CoinGecko identifiers for cryptocurrency assets
- Index prices are calculated based on constituent weights and prices
- Historical data starts from the inception date of each index

---

**Documentation Generated:** January 4, 2026  



