# Arbitrage-Detector – Detailed Task List

## 1. Project Setup
- [ ] **Cargo Init & Modules**  
      `src/` sub-modules:  
      • `cex` – centralized-exchange WebSocket client  
      • `dex` – Uniswap V3 interaction & math  
      • `models` – shared structs/enums  
      • `arbitrage` – detection engine  
      • `config` – env + CLI loader  
      • `utils` – numeric helpers, retry logic  
      • `cli` – command-line interface entry points
- [ ] `.gitignore`, license header

## 2. Dependencies
- [ ] Add to `Cargo.toml`  
      `tokio`, `ethers`, `tokio-tungstenite`, `serde`, `serde_json`,  
      `tracing`, `tracing-subscriber`, `dotenvy`, `bigdecimal`, `anyhow`, `thiserror`, `async-stream`

## 3. Configuration
- [ ] `.env.example` (RPC_URL, WS_URL, PAIR, AMOUNT_ETH, GAS_UNITS, CEX_FEE, DEX_FEE)  
- [ ] `config::load()` → `AppConfig`  
      • merge `.env` and CLI flags (using `clap` or `argh`)  
      • validate required fields

## 4. Models
- [ ] `PricePoint { timestamp, bid, ask }`  
- [ ] `TradeQuote { amount_in, amount_out, fee_bps, slippage_bps }`  
- [ ] `Opportunity { direction, gross_pnl, net_pnl, details }`

## 5. CEX Integration
- [ ] **WebSocket client**
  1. Connect to public feed (`wss://...`)  
  2. Send subscribe message for `ETH/USDC` (or user-selected pair)  
  3. Parse JSON into Rust structs (`serde`)  
  4. Maintain best bid / ask in an `Arc<RwLock<OrderBookTop>>`
- [ ] **Reconnection**
  • Exponential backoff on drop / error  
  • Resubscribe automatically

## 6. DEX Integration
- [ ] **Contract Bindings**
  • `ethers::abigen!` for `IUniswapV3Pool` and `QuoterV2`  
- [ ] **slot0 Price Path**  
  1. `fetch_slot0()` → `sqrtPriceX96`  
  2. `sqrtPriceX96_to_price()` helper (high-precision math, unit tests)
- [ ] **Quoter Path (optional)**  
  • `quote_exact_input_single(amount_in)` → `amount_out`, slippage
- [ ] **Slippage Estimator**  
  • given trade size, simulate price impact or use Quoter output

## 7. Gas & Cost Modeling
- [ ] `gas::estimate_cost(provider)`  
  • `eth_gasPrice` * constant gas units  
  • Return USD & ETH value using CEX mid-price

## 8. Arbitrage Engine
- [ ] `arbitrage::detect()`  
  • Combine latest CEX bid/ask with DEX quote  
  • Apply: CEX fee, DEX LP fee, slippage, gas cost  
  • Emit `Opportunity` if `net_pnl > cfg.min_profit`
- [ ] Unit tests covering:  
  • Fee edge cases, 0-profit scenarios, negative spreads

## 9. Logging & Alerts
- [ ] Initialize `tracing_subscriber` (env filter, pretty/JSON)  
- [ ] Log on: connect/disconnect, price updates, detected opportunities  
- [ ] Optional: send Telegram/Slack webhook

## 10. CLI Interface
- [ ] Flags: `--pair`, `--amount`, `--min-profit`, `--output json|pretty`, `--log-level`  
- [ ] Sub-commands: `run`, `test-math`, `dump-config`

## 11. Orchestration (`main.rs`)
- [ ] Start async runtime (`tokio::main`)  
- [ ] Spawn tasks: CEX stream, DEX poller, gas updater  
- [ ] Channel fan-in to arbitrage detector  
- [ ] Graceful shutdown on Ctrl-C

## 12. Docker & Packaging
- [ ] Multi-stage Dockerfile  
  • Stage 1: build in Rust official image  
  • Stage 2: copy binary into distroless/alpine  
- [ ] `README.md` with:  
  • Setup (`cargo run`, Docker run)  
  • Environment variables  
  • Example command and sample log

## 13. Testing & CI
- [ ] Unit tests: math helpers, arbitrage calc  
- [ ] Integration test: mock CEX & DEX feeds  
- [ ] GitHub Actions (build, test)

## 14. Extension Ideas (future work)
- Simulate on-chain execution & MEV risk  
- Multi-pair & multi-pool support  
- Triangular arbitrage inside Uniswap  
- Smart-contract executor for automatic on-chain execution