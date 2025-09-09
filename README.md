## Arbitrage Detector (Rust / DeFi)

Detects arbitrage opportunities between a DEX (Uniswap V3) and a CEX (Binance WS). It compares DEX and CEX prices for the ETH/USDC pair, accounting for CEX fee, DEX LP fee, DEX slippage (via Uniswap math), and gas cost. This project is created for proof of concept not intended for production use.

### Features
- DEX pricing via on‑chain `slot0` and Uniswap V3 math (sqrtPriceX96 → price)
- CEX top‑of‑book via Binance WebSocket depth stream
- Arbitrage evaluation in both directions with fee and gas adjustments
- Structured logging of detected opportunities
- Unit tests for core pricing and evaluation

### Requirements
- Rust (stable)
- An Ethereum RPC URL (Infura/Alchemy free tier is fine)

### Instructions
1) Create a `.env` file in the project root:

```env
RPC_URL="https://eth-mainnet.alchemyapi.io/v2/{YOUR_ALCHEMY_API_KEY}"
POOL_ADDRESS="0x88E6A0c2dDD26FEEb64F039a2c41296FcB3f5640" # USDC/WETH 0.05% pool
CEX_WS_URL="wss://stream.binance.com:9443/ws"
MIN_PNL_USDC="0"
CEX_FEE_BPS="1.0"
DEX_FEE_BPS="1.0"
GAS_UNITS="200000"
GAS_MULTIPLIER="1"
```

2) Run with Docker:

```bash
docker compose up --build
```

Stop:

```bash
docker compose down
```

3) Run locally (without Docker):

```bash
cargo run --release
```

Tests:

```bash
cargo test
```



### How it works
1) CEX: Subscribes to Binance depth; extracts best bid/ask.
2) With the CEX bid/ask, I calculate the DEX price target. So basically I calculate how would I need to buy/sell to match the CEX price.
3) For example, if the CEX bid is 1000 USDC and the CEX ask is 1001 USDC, I would check the spot price of the DEX, lets say its 1000.5 USDC.
4) Then I need to calculate how much ETH I should sell to the dex to get 1000 USDC. With accounting for the slippage, fees, etc.
5) I keep the order books depth in mind and calculate the max amount of ETH I can buy/sell from CEX(Mostly It will more than enough)

### Troubleshooting
- If you see no opportunities, try setting `MIN_PNL_USDC=0` and/or decreasing `DEX_FEE_BPS`, `CEX_FEE_BPS` and `GAS_MULTIPLIER`.
- Ensure `RPC_URL` is reachable and `POOL_ADDRESS` is a live USDC/WETH pool.

### Extension ideas
- Reconnect/backoff logic for CEX WS
- Event‑driven evaluator on state change instead of fixed interval
- Better gas estimation and smoothing
- Multi‑pool and multi‑CEX support
- Should work for every pool and token pair, plug and play with proper config for CEX and DEX integration.
- To increase the speed, I can use a websocket connection to the DEX and subscribe to the pool state changes.
- To increase opps I could have written a mempool watcher and check for the best price in the mempool.
- I could have written multi-tick calculation for the DEX, so we can get the best price in the next few ticks. (Also assuming if we had mempool watcher we could predict the next tick price)


