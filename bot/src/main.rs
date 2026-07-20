// bot/src/main.rs
mod abi;
mod config;
mod executor;
mod graph;
mod mobius;
mod monitor;
mod pool;
mod strategy;

use config::Config;
use executor::Executor;
use pool::{batch_update_reserves, discover_pools};
use strategy::Strategy;

use ethers::prelude::*;
use ethers::signers::LocalWallet;
use eyre::Result;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("mobius_arb=info".parse()?),
        )
        .init();

    tracing::info!("🔷 Möbius Arbitrage Bot Starting...");
    tracing::info!("   Based on: The Geometry of Arbitrage (Möbius Transformations)");

    // Load configuration
    let config = Config::from_env()?;
    tracing::info!("Configuration loaded");
    tracing::info!("  Executor: {:?}", config.executor_address);
    tracing::info!("  WETH: {:?}", config.weth_address);
    tracing::info!("  Max hops: {}", config.max_hops);
    tracing::info!("  Min profit: {} wei", config.min_profit_wei);
    tracing::info!("  Factories: {}", config.factories.len());

    // Setup provider and wallet
    let provider = Provider::<Http>::try_from(&config.rpc_http_url)?;
    let chain_id = provider.get_chainid().await?.as_u64();
    tracing::info!("  Chain ID: {}", chain_id);

    let wallet: LocalWallet = config
        .private_key
        .parse::<LocalWallet>()?
        .with_chain_id(chain_id);
    let wallet_address = wallet.address();
    tracing::info!("  Wallet: {:?}", wallet_address);

    let provider = Arc::new(SignerMiddleware::new(provider, wallet));

    // Check wallet balance
    let balance = provider.get_balance(wallet_address, None).await?;
    tracing::info!(
        "  Wallet ETH balance: {} ETH",
        ethers::utils::format_ether(balance)
    );

    // ─── Phase 1: Pool Discovery ───────────────────────────────────────
    tracing::info!("\n📡 Phase 1: Discovering pools...");
    let mut all_pools: Vec<pool::Pool> = Vec::new();

    for factory_config in &config.factories {
        tracing::info!("  Scanning {} factory: {:?}", factory_config.name, factory_config.address);
        match discover_pools(
            factory_config.address,
            &factory_config.name,
            factory_config.fee_numerator,
            factory_config.fee_denominator,
            provider.clone(),
            2000, // Max pools per factory
        )
        .await
        {
            Ok(pools) => {
                tracing::info!(
                    "  Found {} active pools from {}",
                    pools.len(),
                    factory_config.name
                );
                all_pools.extend(pools);
            }
            Err(e) => {
                tracing::warn!(
                    "  Failed to scan {}: {}",
                    factory_config.name,
                    e
                );
            }
        }
    }

    tracing::info!("Total pools discovered: {}", all_pools.len());

    if all_pools.is_empty() {
        tracing::error!("No pools found. Check factory addresses and RPC connection.");
        return Ok(());
    }

    // ─── Phase 2: Setup Strategy & Executor ────────────────────────────
    // Estimate gas cost in ETH terms
    let gas_price = provider.get_gas_price().await?;
    let gas_cost = gas_price * U256::from(config.gas_limit);
    tracing::info!(
        "  Estimated gas cost: {} ETH",
        ethers::utils::format_ether(gas_cost)
    );

    let strategy = Strategy::new(
        config.weth_address,
        config.max_hops,
        config.min_profit_wei,
        gas_cost,
    );

    let executor = Executor::new(
        config.executor_address,
        provider.clone(),
        wallet_address,
        config.gas_limit,
        config.max_gas_price_gwei,
    );

    // ─── Phase 3: Main Loop ────────────────────────────────────────────
    tracing::info!("\n🔄 Phase 3: Starting main arbitrage loop...");

    let (block_tx, mut block_rx) = mpsc::channel::<u64>(10);

    // Spawn block watcher
    let ws_url = config.rpc_ws_url.clone();
    tokio::spawn(async move {
        loop {
            match monitor::watch_blocks(&ws_url, block_tx.clone()).await {
                Ok(_) => {
                    tracing::warn!("Block stream ended, reconnecting...");
                }
                Err(e) => {
                    tracing::error!("Block stream error: {}, reconnecting in 5s...", e);
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                }
            }
        }
    });

    // Process each new block
    let mut total_opportunities = 0u64;
    let mut total_executed = 0u64;
    let mut total_profit = U256::zero();

    while let Some(block_number) = block_rx.recv().await {
        let start = Instant::now();

        // Update reserves for all pools
        tracing::info!("Updating reserves for {} pools...", all_pools.len());
        if let Err(e) = batch_update_reserves(&mut all_pools, provider.clone()).await {
            tracing::warn!("Failed to update some reserves: {}", e);
        }

        // Find opportunities using Möbius framework
        let opportunities = strategy.find_opportunities(&all_pools);

        if !opportunities.is_empty() {
            total_opportunities += opportunities.len() as u64;
            tracing::info!(
                "🎯 Found {} opportunities in {:?}",
                opportunities.len(),
                start.elapsed()
            );

            // Execute the best opportunity
            let best = &opportunities[0];
            tracing::info!(
                "Best opportunity: {} | Profit: {} wei ({} ETH)",
                best.path_description,
                best.expected_profit,
                ethers::utils::format_ether(best.expected_profit)
            );

            match executor.execute(best).await {
                Ok(Some(receipt)) => {
                    if receipt.status == Some(U64::from(1)) {
                        total_executed += 1;
                        total_profit += best.expected_profit;
                        tracing::info!(
                            "💰 Cumulative stats: {} opportunities, {} executed, {} ETH profit",
                            total_opportunities,
                            total_executed,
                            ethers::utils::format_ether(total_profit)
                        );
                    }
                }
                Ok(None) => {
                    tracing::info!("Skipped execution (simulation failed)");
                }
                Err(e) => {
                    tracing::error!("Execution error: {}", e);
                }
            }
        } else {
            tracing::debug!(
                "No profitable opportunities at block {} ({:?})",
                block_number,
                start.elapsed()
            );
        }
    }

    Ok(())
}
