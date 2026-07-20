// bot/src/strategy.rs
use crate::graph::{Cycle, TokenGraph};
use crate::mobius::{compose_cycle, MobiusMatrix};
use crate::pool::Pool;
use ethers::types::{Address, U256};
use tracing::{info, warn};

/// A validated, executable arbitrage opportunity
#[derive(Debug, Clone)]
pub struct ExecutableArb {
    pub pool_addresses: Vec<Address>,
    pub zero_for_ones: Vec<bool>,
    pub amount_in: U256,
    pub expected_profit: U256,
    pub min_profit: U256,
    pub input_token: Address,
    /// Human-readable path description
    pub path_description: String,
}

/// The strategy engine: finds and validates arbitrage opportunities
pub struct Strategy {
    pub base_token: Address,
    pub max_hops: usize,
    pub min_profit_wei: U256,
    pub gas_cost_estimate: U256,
}

impl Strategy {
    pub fn new(base_token: Address, max_hops: usize, min_profit_wei: U256, gas_cost_estimate: U256) -> Self {
        Strategy {
            base_token,
            max_hops,
            min_profit_wei,
            gas_cost_estimate,
        }
    }

    /// Scan pools for arbitrage opportunities
    pub fn find_opportunities(&self, pools: &[Pool]) -> Vec<ExecutableArb> {
        let graph = TokenGraph::build(pools);
        let cycles = graph.find_profitable_cycles(pools, self.base_token, self.max_hops);

        info!("Found {} raw profitable cycles", cycles.len());

        let mut executable: Vec<ExecutableArb> = Vec::new();

        for cycle in &cycles {
            if let Some(arb) = self.validate_and_build(cycle, pools) {
                executable.push(arb);
            }
        }

        info!("{} cycles pass validation", executable.len());

        // Sort by profit descending
        executable.sort_by(|a, b| b.expected_profit.cmp(&a.expected_profit));

        executable
    }

    /// Validate a cycle using integer math and build executable parameters
    fn validate_and_build(&self, cycle: &Cycle, pools: &[Pool]) -> Option<ExecutableArb> {
        let opp = &cycle.opportunity;

        // Convert optimal input from f64 to U256 (assuming 18 decimals for WETH)
        // In practice, you'd need to handle different token decimals
        let amount_in_f64 = opp.optimal_input;
        if amount_in_f64 <= 0.0 || amount_in_f64.is_nan() || amount_in_f64.is_infinite() {
            return None;
        }

        // Convert to Wei (assuming base token is 18 decimals)
        let amount_in_wei = float_to_u256(amount_in_f64)?;

        // Simulate the exact swap chain using integer math to verify
        let mut current_amount = amount_in_wei;
        let mut path_parts = Vec::new();

        for edge in &cycle.edges {
            let pool = &pools[edge.pool_index];
            let amount_out = pool.get_amount_out(current_amount, edge.token_in);

            if amount_out.is_zero() {
                return None;
            }

            path_parts.push(format!(
                "{}({:?}→{:?})",
                pool.dex_name,
                short_addr(edge.token_in),
                short_addr(edge.token_out)
            ));

            current_amount = amount_out;
        }

        // Check profit with integer math
        if current_amount <= amount_in_wei {
            return None;
        }

        let profit = current_amount - amount_in_wei;

        // Subtract gas cost
        if profit <= self.gas_cost_estimate {
            return None;
        }

        let net_profit = profit - self.gas_cost_estimate;

        if net_profit < self.min_profit_wei {
            return None;
        }

        // Build execution parameters
        let pool_addresses: Vec<Address> = cycle
            .edges
            .iter()
            .map(|e| pools[e.pool_index].address)
            .collect();

        let zero_for_ones: Vec<bool> = cycle
            .edges
            .iter()
            .map(|e| pools[e.pool_index].is_zero_for_one(e.token_in))
            .collect();

        let path_desc = path_parts.join(" → ");

        info!(
            "✅ Arbitrage: {} | Input: {} wei | Profit: {} wei | Net: {} wei",
            path_desc, amount_in_wei, profit, net_profit
        );

        Some(ExecutableArb {
            pool_addresses,
            zero_for_ones,
            amount_in: amount_in_wei,
            expected_profit: profit,
            min_profit: net_profit / 2, // Use half the expected profit as min to account for slippage
            input_token: self.base_token,
            path_description: path_desc,
        })
    }
}

/// Convert a floating point token amount (in whole units) to U256 Wei
/// Assumes 18 decimals
fn float_to_u256(value: f64) -> Option<U256> {
    if value <= 0.0 || value > 1e15 {
        // Sanity check: > 1 quadrillion tokens is suspicious
        return None;
    }
    // Multiply by 1e18
    let wei = value * 1e18;
    if wei > u128::MAX as f64 {
        return None;
    }
    Some(U256::from(wei as u128))
}

fn short_addr(addr: Address) -> String {
    let s = format!("{:?}", addr);
    format!("{}..{}", &s[..6], &s[s.len() - 4..])
}
