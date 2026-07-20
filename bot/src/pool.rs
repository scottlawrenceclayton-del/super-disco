// bot/src/pool.rs
use ethers::prelude::*;
use ethers::types::{Address, U256};
use eyre::Result;
use std::sync::Arc;

abigen!(
    IUniswapV2Factory,
    r#"[
        function getPair(address tokenA, address tokenB) external view returns (address pair)
        function allPairs(uint256) external view returns (address pair)
        function allPairsLength() external view returns (uint256)
    ]"#
);

abigen!(
    IUniswapV2Pair,
    r#"[
        function getReserves() external view returns (uint112 reserve0, uint112 reserve1, uint32 blockTimestampLast)
        function token0() external view returns (address)
        function token1() external view returns (address)
        function factory() external view returns (address)
    ]"#
);

#[derive(Debug, Clone)]
pub struct Pool {
    pub address: Address,
    pub token0: Address,
    pub token1: Address,
    pub reserve0: U256,
    pub reserve1: U256,
    pub factory: Address,
    pub dex_name: String,
    pub fee_numerator: u64,
    pub fee_denominator: u64,
}

impl Pool {
    pub fn reserve_in_out(&self, token_in: Address) -> (f64, f64) {
        if token_in == self.token0 {
            (self.reserve0.as_u128() as f64, self.reserve1.as_u128() as f64)
        } else {
            (self.reserve1.as_u128() as f64, self.reserve0.as_u128() as f64)
        }
    }

    /// Returns which token you get out when you put token_in in
    pub fn other_token(&self, token_in: Address) -> Address {
        if token_in == self.token0 {
            self.token1
        } else {
            self.token0
        }
    }

    /// Is this a token0 → token1 swap?
    pub fn is_zero_for_one(&self, token_in: Address) -> bool {
        token_in == self.token0
    }

    /// Compute the getAmountOut in U256 precision
    pub fn get_amount_out(&self, amount_in: U256, token_in: Address) -> U256 {
        let (reserve_in, reserve_out) = if token_in == self.token0 {
            (self.reserve0, self.reserve1)
        } else {
            (self.reserve1, self.reserve0)
        };

        if reserve_in.is_zero() || reserve_out.is_zero() || amount_in.is_zero() {
            return U256::zero();
        }

        let amount_in_with_fee = amount_in * U256::from(self.fee_numerator);
        let numerator = amount_in_with_fee * reserve_out;
        let denominator = reserve_in * U256::from(self.fee_denominator) + amount_in_with_fee;

        numerator / denominator
    }
}

/// Fetch pool data from on-chain
pub async fn fetch_pool<M: Middleware>(
    pair_address: Address,
    dex_name: &str,
    fee_numerator: u64,
    fee_denominator: u64,
    provider: Arc<M>,
) -> Result<Pool> {
    let pair = IUniswapV2Pair::new(pair_address, provider.clone());

    let token0 = pair.token_0().call().await?;
    let token1 = pair.token_1().call().await?;
    let (reserve0, reserve1, _) = pair.get_reserves().call().await?;

    Ok(Pool {
        address: pair_address,
        token0,
        token1,
        reserve0: U256::from(reserve0),
        reserve1: U256::from(reserve1),
        factory: Address::zero(), // Can be fetched if needed
        dex_name: dex_name.to_string(),
        fee_numerator,
        fee_denominator,
    })
}

/// Batch-fetch reserves for multiple pools using multicall
pub async fn batch_update_reserves<M: Middleware + 'static>(
    pools: &mut [Pool],
    provider: Arc<M>,
) -> Result<()> {
    // For efficiency, we use individual calls with tokio::join
    // In production, use a multicall contract for batching
    let futures: Vec<_> = pools
        .iter()
        .map(|pool| {
            let pair = IUniswapV2Pair::new(pool.address, provider.clone());
            async move { pair.get_reserves().call().await }
        })
        .collect();

    let results = futures::future::join_all(futures).await;

    for (pool, result) in pools.iter_mut().zip(results) {
        if let Ok((r0, r1, _)) = result {
            pool.reserve0 = U256::from(r0);
            pool.reserve1 = U256::from(r1);
        }
    }

    Ok(())
}

/// Discover all pairs from a factory (with pagination)
pub async fn discover_pools<M: Middleware + 'static>(
    factory_address: Address,
    dex_name: &str,
    fee_numerator: u64,
    fee_denominator: u64,
    provider: Arc<M>,
    max_pools: usize,
) -> Result<Vec<Pool>> {
    let factory = IUniswapV2Factory::new(factory_address, provider.clone());
    let total_pairs: U256 = factory.all_pairs_length().call().await?;
    let count = std::cmp::min(total_pairs.as_usize(), max_pools);

    tracing::info!(
        "Discovering {} pools from {} (total: {})",
        count,
        dex_name,
        total_pairs
    );

    let mut pools = Vec::new();
    let batch_size = 100;

    for start in (0..count).step_by(batch_size) {
        let end = std::cmp::min(start + batch_size, count);
        let mut futures = Vec::new();

        for i in start..end {
            let factory = factory.clone();
            let provider = provider.clone();
            let dex_name = dex_name.to_string();
            futures.push(async move {
                let pair_address = factory.all_pairs(U256::from(i)).call().await?;
                fetch_pool(pair_address, &dex_name, fee_numerator, fee_denominator, provider).await
            });
        }

        let results = futures::future::join_all(futures).await;
        for result in results {
            match result {
                Ok(pool) => {
                    // Skip pools with zero liquidity
                    if !pool.reserve0.is_zero() && !pool.reserve1.is_zero() {
                        pools.push(pool);
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to fetch pool: {}", e);
                }
            }
        }

        tracing::info!("Fetched {}/{} pools", pools.len(), count);
    }

    Ok(pools)
}
