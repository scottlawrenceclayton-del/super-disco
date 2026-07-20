// bot/src/monitor.rs
use ethers::prelude::*;
use eyre::Result;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Block monitor that triggers re-evaluation on each new block
pub struct BlockMonitor<M: Middleware> {
    provider: Arc<M>,
}

impl<M: Middleware + 'static> BlockMonitor<M> {
    pub fn new(provider: Arc<M>) -> Self {
        BlockMonitor { provider }
    }
}

/// Subscribe to new blocks via WebSocket and send notifications
pub async fn watch_blocks(
    ws_url: &str,
    tx: mpsc::Sender<u64>,
) -> Result<()> {
    let provider = Provider::<Ws>::connect(ws_url).await?;

    let mut stream = provider.subscribe_blocks().await?;

    tracing::info!("🔗 Connected to block stream");

    while let Some(block) = stream.next().await {
        let block_number = block.number.unwrap_or_default().as_u64();
        tracing::info!("📦 New block: {}", block_number);

        if tx.send(block_number).await.is_err() {
            tracing::error!("Block channel closed");
            break;
        }
    }

    Ok(())
}
