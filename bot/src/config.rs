// bot/src/config.rs
use ethers::types::{Address, U256};
use eyre::Result;
use std::env;
use std::str::FromStr;

#[derive(Debug, Clone)]
pub struct Config {
    pub rpc_ws_url: String,
    pub rpc_http_url: String,
    pub private_key: String,
    pub executor_address: Address,
    pub weth_address: Address,
    pub factories: Vec<FactoryConfig>,
    pub min_profit_wei: U256,
    pub max_hops: usize,
    pub max_gas_price_gwei: f64,
    pub gas_limit: u64,
}

#[derive(Debug, Clone)]
pub struct FactoryConfig {
    pub name: String,
    pub address: Address,
    pub init_code_hash: [u8; 32],
    pub fee_numerator: u64,   // e.g., 997 for 0.3% fee
    pub fee_denominator: u64, // e.g., 1000
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenv::dotenv().ok();

        let mut factories = Vec::new();

        // SushiSwap on Arbitrum
        if let Ok(addr) = env::var("SUSHI_FACTORY") {
            let hash_str = env::var("SUSHI_INIT_CODE_HASH")
                .unwrap_or_else(|_| "0xe18a34eb0e04b04f7a0ac29a6e80748dca96319b42c54d679cb821dca90c6303".into());
            let hash_bytes = hex::decode(hash_str.trim_start_matches("0x"))?;
            let mut hash = [0u8; 32];
            hash.copy_from_slice(&hash_bytes);
            factories.push(FactoryConfig {
                name: "SushiSwap".into(),
                address: Address::from_str(&addr)?,
                init_code_hash: hash,
                fee_numerator: 997,
                fee_denominator: 1000,
            });
        }

        // Camelot on Arbitrum (also uses UniV2 model but some pairs have variable fees)
        if let Ok(addr) = env::var("CAMELOT_FACTORY") {
            // Camelot uses a different init code hash
            let hash_str = env::var("CAMELOT_INIT_CODE_HASH").unwrap_or_else(|_| {
                "0xa856464ae65f7619571a63dd tried3880d1e4a16d15e2a8a5e26c2f4e0e6aa5e".into()
            });
            let hash_bytes = hex::decode(hash_str.trim_start_matches("0x")).unwrap_or_else(|_| vec![0u8; 32]);
            let mut hash = [0u8; 32];
            if hash_bytes.len() == 32 {
                hash.copy_from_slice(&hash_bytes);
            }
            factories.push(FactoryConfig {
                name: "Camelot".into(),
                address: Address::from_str(&addr)?,
                init_code_hash: hash,
                fee_numerator: 997,
                fee_denominator: 1000,
            });
        }

        Ok(Config {
            rpc_ws_url: env::var("RPC_WS_URL")?,
            rpc_http_url: env::var("RPC_HTTP_URL")?,
            private_key: env::var("PRIVATE_KEY")?,
            executor_address: Address::from_str(&env::var("EXECUTOR_ADDRESS")?)?,
            weth_address: Address::from_str(&env::var("WETH_ADDRESS")?)?,
            factories,
            min_profit_wei: U256::from_dec_str(
                &env::var("MIN_PROFIT_WEI").unwrap_or_else(|_| "500000000000000".into()),
            )?,
            max_hops: env::var("MAX_HOPS")
                .unwrap_or_else(|_| "4".into())
                .parse()?,
            max_gas_price_gwei: env::var("MAX_GAS_PRICE_GWEI")
                .unwrap_or_else(|_| "0.5".into())
                .parse()?,
            gas_limit: env::var("GAS_LIMIT")
                .unwrap_or_else(|_| "500000".into())
                .parse()?,
        })
    }
}
