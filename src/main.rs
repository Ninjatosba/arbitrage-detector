use anyhow::Result;
use ethers::types::Address;
use arbitrage_detector::dex::Dex;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    let rpc_url = std::env::var("RPC_URL")
        .expect("Set RPC_URL env var to your Ethereum node HTTP endpoint");

    // Default pool address: Uniswap V3 ETH/USDC 0.3% fee tier on Ethereum mainnet.
    let default_pool = "0x8ad599c3a0ff1de082011efddc58f1908eb6e6d8";
    let pool_addr_raw = std::env::var("POOL_ADDRESS").unwrap_or_else(|_| default_pool.to_string());
    let pool_addr: Address = pool_addr_raw.parse()?;

    println!("Connecting to pool {} via {}", pool_addr, rpc_url);

    let dex = Dex::new(&rpc_url, pool_addr).await?;

    // Stream slot0 every 5 seconds.
    dex.stream_slot0(5).await?;

    Ok(())
}
