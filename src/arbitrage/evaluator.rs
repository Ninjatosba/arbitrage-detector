use super::types::{ArbitrageConfig, ArbitrageOpportunity};
use crate::dex::{PoolState, calculate_swap_with_library};
use crate::models::{BookDepth, SwapDirection};

/// Evaluate arbitrage opportunities in both directions
pub fn evaluate_opportunities(
    pool_state: &PoolState,
    book: &BookDepth,
    dex_price: f64,
    config: &ArbitrageConfig,
    gas_cost_usdc: f64,
) -> Vec<ArbitrageOpportunity> {
    let mut opportunities = Vec::new();

    if book.bids.is_empty() || book.asks.is_empty() {
        return opportunities;
    }

    // Direction A: buy on DEX -> sell on CEX (use CEX bid)
    if let Some(opp) = evaluate_direction_a(pool_state, book, config, gas_cost_usdc) {
        opportunities.push(opp);
    }

    // Direction B: buy on CEX -> sell on DEX (use CEX ask)
    if let Some(opp) = evaluate_direction_b(pool_state, book, dex_price, config, gas_cost_usdc) {
        opportunities.push(opp);
    }

    opportunities
}

/// Evaluate Direction A: buy on DEX -> sell on CEX
fn evaluate_direction_a(
    pool_state: &PoolState,
    book: &BookDepth,
    config: &ArbitrageConfig,
    gas_cost_usdc: f64,
) -> Option<ArbitrageOpportunity> {
    let (bid_price, bid_qty_cex) = book.bids[0];
    // I am seeling on Cex so we should decrease price by the fee to adjust our target
    let adjusted_bid_price = bid_price * (1.0 - config.cex_fee_bps / 10_000.0);

    let res = calculate_swap_with_library(
        pool_state,
        adjusted_bid_price,
        SwapDirection::Token0ToToken1,
        config.dex_fee_bps,
        bid_qty_cex,
    )
    .ok()?;

    let token1_in = res.amount_in; // USDC we will spend on DEX
    let token0_out = res.amount_out; // ETH we obtain from DEX

    // Calculate profit and loss: revenue on CEX minus cost on DEX minus gas.
    let revenue_total = bid_price * token0_out;
    let cost_total = token1_in; // USDC spent already includes DEX LP fee
    let pnl = revenue_total - cost_total - gas_cost_usdc;

    if pnl >= config.min_pnl_usdc {
        let description = format!(
            "A: Buy {:.6} ETH on DEX → Sell on CEX @ ${:.2} | Earn ${:.2}",
            token0_out, bid_price, pnl
        );

        Some(ArbitrageOpportunity {
            direction: "A".to_string(),
            description,
            pnl,
        })
    } else {
        None
    }
}

/// Evaluate Direction B: buy on CEX -> sell on DEX
fn evaluate_direction_b(
    pool_state: &PoolState,
    book: &BookDepth,
    _dex_price: f64,
    config: &ArbitrageConfig,
    gas_cost_usdc: f64,
) -> Option<ArbitrageOpportunity> {
    let (ask_price, ask_qty_cex) = book.asks[0];
    // I am buying on Cex so we should increase price by the fee to adjust our target
    let adjusted_ask_price = ask_price * (1.0 + config.cex_fee_bps / 10_000.0);

    let res = calculate_swap_with_library(
        pool_state,
        adjusted_ask_price,
        SwapDirection::Token1ToToken0,
        config.dex_fee_bps,
        ask_qty_cex,
    )
    .ok()?;

    let token0_in = res.amount_in; // ETH to sell on DEX
    let token1_out = res.amount_out; // USDC received from DEX
    // Library will include dex fees on input so we don't need to adjust

    // Calculate profit and loss: revenue on DEX minus cost on CEX minus gas
    let revenue_total = token1_out;
    let cost_total = adjusted_ask_price * token0_in;
    let pnl = revenue_total - cost_total - gas_cost_usdc;

    if pnl >= config.min_pnl_usdc {
        let description = format!(
            "B: Buy {:.6} ETH on CEX  → Sell on DEX @ ${:.2} | Earn ${:.2}",
            token0_in, ask_price, pnl
        );

        Some(ArbitrageOpportunity {
            direction: "B".to_string(),
            description,
            pnl,
        })
    } else {
        None
    }
}

/// Calculate gas cost in USDC
pub fn calculate_gas_cost_usdc(
    gas_gwei: f64,
    gas_units: f64,
    gas_multiplier: f64,
    dex_price: f64,
) -> f64 {
    gas_gwei * 1e-9 * gas_units * gas_multiplier * dex_price
}

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use ethers::types::U256 as EthersU256;

//     fn sqrt_price_x96_from_price_usdc_per_eth(
//         price_usdc_per_eth: f64,
//         token0_decimals: u8,
//         token1_decimals: u8,
//     ) -> EthersU256 {
//         let dec_factor = 10f64.powi(token1_decimals as i32 - token0_decimals as i32);
//         let ratio_raw = dec_factor / price_usdc_per_eth;
//         let sqrt_ratio = ratio_raw.sqrt();
//         let q96 = (sqrt_ratio * 2f64.powi(96)) as u128;
//         EthersU256::from(q96)
//     }

//     fn make_pool(price_usdc_per_eth: f64, liquidity: u128) -> PoolState {
//         let token0_decimals = 6;
//         let token1_decimals = 18;
//         let sqrt_q96 = sqrt_price_x96_from_price_usdc_per_eth(
//             price_usdc_per_eth,
//             token0_decimals,
//             token1_decimals,
//         );
//         PoolState {
//             sqrt_price_x96: sqrt_q96,
//             liquidity,
//             tick: 0,
//             token0_decimals,
//             token1_decimals,
//             limit_lower_sqrt_price_x96: None,
//             limit_upper_sqrt_price_x96: None,
//             price_usdc_per_eth,
//         }
//     }

//     #[test]
//     fn gas_cost_basic_calculation() {
//         let cost = calculate_gas_cost_usdc(30.0, 300000.0, 1.2, 4000.0);
//         assert!(cost > 0.0);
//     }

//     #[test]
//     #[ignore]
//     fn direction_a_smoke_profitability() {
//         let pool = make_pool(4200.0, 1_800_000_000_000_000_000);
//         let book = BookDepth {
//             timestamp: 0,
//             bids: vec![(4225.0, 5.0)],
//             asks: vec![(4230.0, 5.0)],
//         };
//         let cfg = ArbitrageConfig {
//             min_pnl_usdc: 0.0,
//             dex_fee_bps: 30.0,
//             cex_fee_bps: 10.0,
//         };
//         let opps = evaluate_opportunities(&pool, &book, pool.price_usdc_per_eth, &cfg, 0.0);
//         assert!(!opps.is_empty());
//     }
// }
