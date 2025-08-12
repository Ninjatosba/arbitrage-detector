use super::types::{ArbitrageConfig, ArbitrageOpportunity};
use crate::dex::{PoolState, calculate_swap_with_library};
use crate::models::{BookDepth, SwapDirection};

/// Evaluate arbitrage opportunities in both directions
pub fn evaluate_opportunities(
    pool_state: &PoolState,
    book: &BookDepth,
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
    if let Some(opp) = evaluate_direction_b(pool_state, book, config, gas_cost_usdc) {
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

    if token0_out <= 0.0 {
        return None;
    }

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

    if token1_out <= 0.0 {
        return None;
    }

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
    price_usdc_per_eth: f64,
) -> f64 {
    gas_gwei * 1e-9 * gas_units * gas_multiplier * price_usdc_per_eth
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dex::calc::calculate_sqrt_price_with_precision_per_eth;

    fn make_pool(price_usdc_per_eth: f64, liquidity: u128) -> PoolState {
        let token0_decimals = 6;
        let token1_decimals = 18;
        let sqrt_q96 = calculate_sqrt_price_with_precision_per_eth(
            price_usdc_per_eth,
            token0_decimals,
            token1_decimals,
        )
        .unwrap();
        PoolState {
            sqrt_price_x96: sqrt_q96,
            liquidity,
            tick: 0,
            token0_decimals,
            token1_decimals,
            limit_lower_sqrt_price_x96: None,
            limit_upper_sqrt_price_x96: None,
            price_usdc_per_eth,
        }
    }

    #[test]
    fn gas_cost_basic_calculation() {
        let cost = calculate_gas_cost_usdc(30.0, 300000.0, 1.2, 4000.0);
        assert!(cost > 0.0);
        assert_eq!(cost, 43.2);
    }

    #[test]
    fn direction_a_smoke_profitability() {
        let pool = make_pool(4200.0, 1_800_000_000_000_000_000);
        let book = BookDepth {
            timestamp: 0,
            bids: vec![(4225.0, 5.0)],
            asks: vec![(4230.0, 5.0)],
        };
        let cfg = ArbitrageConfig {
            min_pnl_usdc: 0.0,
            dex_fee_bps: 30.0,
            cex_fee_bps: 10.0,
        };
        let opps = evaluate_opportunities(&pool, &book, &cfg, 0.0);
        assert!(!opps.is_empty());
    }

    #[test]
    fn empty_order_book_returns_no_opportunities() {
        let pool = make_pool(4200.0, 1_800_000_000_000_000_000);
        let empty_bids = BookDepth {
            timestamp: 0,
            bids: vec![],
            asks: vec![(4210.0, 1.0)],
        };
        let empty_asks = BookDepth {
            timestamp: 0,
            bids: vec![(4210.0, 1.0)],
            asks: vec![],
        };
        let cfg = ArbitrageConfig {
            min_pnl_usdc: 0.0,
            dex_fee_bps: 30.0,
            cex_fee_bps: 10.0,
        };

        let opps_a = evaluate_opportunities(&pool, &empty_bids, &cfg, 0.0);
        let opps_b = evaluate_opportunities(&pool, &empty_asks, &cfg, 0.0);

        assert!(opps_a.is_empty());
        assert!(opps_b.is_empty());
    }

    #[test]
    fn direction_b_smoke_profitability() {
        // DEX price higher than CEX ask makes B direction attractive
        let pool = make_pool(4250.0, 1_800_000_000_000_000_000);
        let book = BookDepth {
            timestamp: 0,
            bids: vec![(4240.0, 5.0)],
            asks: vec![(4223.0, 5.0)],
        };
        let cfg = ArbitrageConfig {
            min_pnl_usdc: 0.0,
            dex_fee_bps: 30.0,
            cex_fee_bps: 10.0,
        };
        let opps = evaluate_opportunities(&pool, &book, &cfg, 0.0);
        assert!(opps.iter().any(|o| o.direction == "B"));
    }

    #[test]
    fn min_pnl_threshold_filters_out_opportunities() {
        let pool = make_pool(4200.0, 1_800_000_000_000_000_000);
        let book = BookDepth {
            timestamp: 0,
            bids: vec![(4225.0, 5.0)],
            asks: vec![(4230.0, 5.0)],
        };
        // Set very high minimum profit to filter out any result
        let cfg = ArbitrageConfig {
            min_pnl_usdc: 1.0,
            dex_fee_bps: 30.0,
            cex_fee_bps: 10.0,
        };
        let opps = evaluate_opportunities(&pool, &book, &cfg, 0.0);
        assert!(opps.is_empty());

        let cfg = ArbitrageConfig {
            min_pnl_usdc: 0.001,
            dex_fee_bps: 30.0,
            cex_fee_bps: 10.0,
        };
        let opps = evaluate_opportunities(&pool, &book, &cfg, 0.0);
        assert!(!opps.is_empty());
    }

    #[test]
    fn gas_cost_can_turn_a_profitable_trade_unprofitable() {
        let pool = make_pool(4200.0, 1_800_000_000_000_000_000);
        let book = BookDepth {
            timestamp: 0,
            bids: vec![(4225.0, 5.0)],
            asks: vec![(4230.0, 5.0)],
        };
        let cfg = ArbitrageConfig {
            min_pnl_usdc: 0.0,
            dex_fee_bps: 30.0,
            cex_fee_bps: 10.0,
        };

        // With zero gas, expect at least one opportunity
        let opps_no_gas = evaluate_opportunities(&pool, &book, &cfg, 0.0);
        assert!(!opps_no_gas.is_empty());

        // With large gas, opportunities should disappear under a modest min_pnl
        let cfg_with_min = ArbitrageConfig {
            min_pnl_usdc: 0.0,
            ..cfg.clone()
        };
        let opps_high_gas = evaluate_opportunities(&pool, &book, &cfg_with_min, 0.3);
        assert!(opps_high_gas.is_empty());
    }

    #[test]
    fn description_contains_expected_phrasing_and_values() {
        let pool = make_pool(4200.0, 1_800_000_000_000_000_000);
        let book = BookDepth {
            timestamp: 0,
            bids: vec![(4225.0, 5.0)],
            asks: vec![(4300.0, 5.0)], // make B unlikely so we focus on A
        };
        let cfg = ArbitrageConfig {
            min_pnl_usdc: 0.0,
            dex_fee_bps: 30.0,
            cex_fee_bps: 10.0,
        };
        let opps = evaluate_opportunities(&pool, &book, &cfg, 0.0);
        if let Some(opp) = opps.iter().find(|o| o.direction == "A") {
            assert!(opp.description.contains("A:"));
            assert!(opp.description.contains("Earn $"));
            assert!(opp.pnl >= 0.0);
        } else {
            // If A did not appear, ensure at least B has the expected format
            let opp_b = opps
                .iter()
                .find(|o| o.direction == "B")
                .expect("expected at least one opportunity");
            assert!(opp_b.description.contains("B:"));
            assert!(opp_b.description.contains("Earn $"));
        }
    }

    #[test]
    fn high_cex_fee_can_eliminate_opportunities() {
        let pool = make_pool(4200.0, 1_800_000_000_000_000_000);
        // Prices that would normally allow A and B, but crank CEX fee very high
        let book = BookDepth {
            timestamp: 0,
            bids: vec![(4250.0, 5.0)],
            asks: vec![(4150.0, 5.0)],
        };
        let cfg = ArbitrageConfig {
            min_pnl_usdc: 0.0,
            dex_fee_bps: 30.0,
            cex_fee_bps: 1000.0,
        }; // 10%
        let opps = evaluate_opportunities(&pool, &book, &cfg, 0.0);
        // With such a large CEX fee, adjusted prices likely remove profitability
        assert!(opps.is_empty());
    }

    #[test]
    fn gas_cost_formula_matches_expected_math() {
        let gas_gwei = 35.0;
        let gas_units = 250_000.0;
        let multiplier = 1.3;
        let price = 3800.0;
        let expected = gas_gwei * 1e-9 * gas_units * multiplier * price;
        let got = calculate_gas_cost_usdc(gas_gwei, gas_units, multiplier, price);
        let tol = 1e-12;
        assert!((got - expected).abs() < tol, "{} vs {}", got, expected);
    }
}
