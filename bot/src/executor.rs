// bot/src/executor.rs
use crate::strategy::ExecutableArb;
use ethers::prelude::*;
use ethers::types::{Address, Bytes, U256};
use eyre::Result;
use std::sync::Arc;

abigen!(
    MobiusExecutor,
    r#"[
        function executeArbitrage(address[] calldata pairs, bool[] calldata zeroForOnes, uint256 amountIn, uint256 minProfit) external
        function withdraw(address token) external
        function withdrawETH() external
        function owner() external view returns (address)
    ]"#
);

pub struct Executor<M: Middleware> {
    contract: MobiusExecutor<M>,
    wallet_address: Address,
    gas_limit: u64,
    max_gas_price: U256,
}

impl<M: Middleware + 'static> Executor<M> {
    pub fn new(
        executor_address: Address,
        provider: Arc<M>,
        wallet_address: Address,
        gas_limit: u64,
        max_gas_price_gwei: f64,
    ) -> Self {
        let contract = MobiusExecutor::new(executor_address, provider);
        let max_gas_price = U256::from((max_gas_price_gwei * 1e9) as u64);
        Executor {
            contract,
            wallet_address,
            gas_limit,
            max_gas_price,
        }
    }

    /// Execute an arbitrage opportunity
    pub async fn execute(&self, arb: &ExecutableArb) -> Result<Option<TransactionReceipt>> {
        tracing::info!(
            "🚀 Executing arbitrage: {} | Amount: {} | Expected profit: {}",
            arb.path_description,
            arb.amount_in,
            arb.expected_profit
        );

        let tx = self
            .contract
            .execute_arbitrage(
                arb.pool_addresses.clone(),
                arb.zero_for_ones.clone(),
                arb.amount_in,
                arb.min_profit,
            )
            .gas(self.gas_limit)
            .gas_price(self.max_gas_price);

        // First simulate the transaction
        match tx.call().await {
            Ok(_) => {
                tracing::info!("✅ Simulation successful, sending transaction");
            }
            Err(e) => {
                tracing::warn!("❌ Simulation failed: {}. Skipping execution.", e);
                return Ok(None);
            }
        }

        // Send the transaction
        let pending = tx.send().await?;
        tracing::info!("📤 Transaction sent: {:?}", pending.tx_hash());

        // Wait for confirmation
        let receipt = pending.await?;
        match &receipt {
            Some(r) => {
                if r.status == Some(U64::from(1)) {
                    tracing::info!(
                        "✅ Transaction confirmed! Hash: {:?}, Gas used: {:?}",
                        r.transaction_hash,
                        r.gas_used
                    );
                } else {
                    tracing::warn!("❌ Transaction reverted: {:?}", r.transaction_hash);
                }
            }
            None => {
                tracing::warn!("⏳ Transaction not yet confirmed");
            }
        }

        Ok(receipt)
    }

    /// Withdraw profits from the contract
    pub async fn withdraw_token(&self, token: Address) -> Result<Option<TransactionReceipt>> {
        let tx = self.contract.withdraw(token);
        let pending = tx.send().await?;
        Ok(pending.await?)
    }

    pub async fn withdraw_eth(&self) -> Result<Option<TransactionReceipt>> {
        let tx = self.contract.withdraw_eth();
        let pending = tx.send().await?;
        Ok(pending.await?)
    }
}
