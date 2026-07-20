// bot/src/graph.rs
//
// Token graph construction and cycle detection.
// Each edge is a pool connecting two tokens.
// We find cycles of length 2..=MAX_HOPS that start and end at a base token (WETH).

use crate::mobius::{compose_cycle, ArbitrageOpportunity, MobiusMatrix};
use crate::pool::Pool;
use ethers::types::Address;
use std::collections::{HashMap, HashSet};

/// An edge in the token graph
#[derive(Debug, Clone)]
pub struct Edge {
    pub pool_index: usize,
    pub token_in: Address,
    pub token_out: Address,
}

/// A detected arbitrage cycle
#[derive(Debug, Clone)]
pub struct Cycle {
    pub edges: Vec<Edge>,
    pub pool_indices: Vec<usize>,
    pub opportunity: ArbitrageOpportunity,
}

/// Token graph for finding arbitrage cycles
pub struct TokenGraph {
    /// token_address → list of (neighbor_token, pool_index, direction)
    pub adjacency: HashMap<Address, Vec<(Address, usize)>>,
    pub tokens: HashSet<Address>,
}

impl TokenGraph {
    pub fn new() -> Self {
        TokenGraph {
            adjacency: HashMap::new(),
            tokens: HashSet::new(),
        }
    }

    /// Build the token graph from a list of pools
    pub fn build(pools: &[Pool]) -> Self {
        let mut graph = TokenGraph::new();

        for (i, pool) in pools.iter().enumerate() {
            // Skip zero-liquidity pools
            if pool.reserve0.is_zero() || pool.reserve1.is_zero() {
                continue;
            }

            graph.tokens.insert(pool.token0);
            graph.tokens.insert(pool.token1);

            // Add edges in both directions
            graph
                .adjacency
                .entry(pool.token0)
                .or_default()
                .push((pool.token1, i));
            graph
                .adjacency
                .entry(pool.token1)
                .or_default()
                .push((pool.token0, i));
        }

        graph
    }

    /// Find all profitable cycles starting and ending at `base_token`
    /// with at most `max_hops` steps.
    ///
    /// Uses DFS with pruning.
    pub fn find_profitable_cycles(
        &self,
        pools: &[Pool],
        base_token: Address,
        max_hops: usize,
    ) -> Vec<Cycle> {
        let mut results = Vec::new();
        let mut path: Vec<Edge> = Vec::new();
        let mut visited_pools: HashSet<usize> = HashSet::new();

        self.dfs_find_cycles(
            pools,
            base_token,
            base_token,
            max_hops,
            &mut path,
            &mut visited_pools,
            &mut results,
        );

        // Sort by expected profit descending
        results.sort_by(|a, b| {
            b.opportunity
                .expected_profit
                .partial_cmp(&a.opportunity.expected_profit)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        results
    }

    fn dfs_find_cycles(
        &self,
        pools: &[Pool],
        current_token: Address,
        base_token: Address,
        max_hops: usize,
        path: &mut Vec<Edge>,
        visited_pools: &mut HashSet<usize>,
        results: &mut Vec<Cycle>,
    ) {
        if path.len() > max_hops {
            return;
        }

        // If we've taken at least 2 steps, check if we can close the cycle
        if path.len() >= 2 && current_token == base_token {
            // Compose the Möbius matrices for this cycle
            let matrices: Vec<MobiusMatrix> = path
                .iter()
                .map(|edge| {
                    let pool = &pools[edge.pool_index];
                    let (r_in, r_out) = pool.reserve_in_out(edge.token_in);
                    MobiusMatrix::from_pool(
                        r_in,
                        r_out,
                        pool.fee_numerator as f64,
                        pool.fee_denominator as f64,
                    )
                })
                .collect();

            let composed = compose_cycle(&matrices);

            if let Some(opp) = composed.optimal_input() {
                if opp.expected_profit > 0.0 {
                    results.push(Cycle {
                        edges: path.clone(),
                        pool_indices: path.iter().map(|e| e.pool_index).collect(),
                        opportunity: opp,
                    });
                }
            }
            // Don't return — there might be longer profitable cycles too
            // But we do return to avoid exploring further from base with this path
            if path.len() >= max_hops {
                return;
            }
        }

        // Explore neighbors
        if let Some(neighbors) = self.adjacency.get(&current_token) {
            for (next_token, pool_idx) in neighbors {
                if visited_pools.contains(pool_idx) {
                    continue;
                }

                // Pruning: only revisit base_token to close the cycle
                if *next_token != base_token && path.iter().any(|e| e.token_out == *next_token) {
                    continue;
                }

                visited_pools.insert(*pool_idx);
                path.push(Edge {
                    pool_index: *pool_idx,
                    token_in: current_token,
                    token_out: *next_token,
                });

                self.dfs_find_cycles(
                    pools,
                    *next_token,
                    base_token,
                    max_hops,
                    path,
                    visited_pools,
                    results,
                );

                path.pop();
                visited_pools.remove(pool_idx);
            }
        }
    }
}
