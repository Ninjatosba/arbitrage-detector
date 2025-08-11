use thiserror::Error;

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Environment variable error: {0}")]
    Env(#[from] std::env::VarError),

    #[error("Parse float error: {0}")]
    ParseFloat(#[from] std::num::ParseFloatError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("URL parse error: {0}")]
    UrlParse(#[from] url::ParseError),

    #[error("WebSocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("Provider error: {0}")]
    Provider(#[from] ethers::providers::ProviderError),

    #[error("Contract error: {0}")]
    Contract(
        #[from]
        ethers::contract::ContractError<ethers::providers::Provider<ethers::providers::Http>>,
    ),

    #[error("Serialization error: {0}")]
    SerdeJson(#[from] serde_json::Error),

    #[error("Math error: {0}")]
    Math(#[from] uniswap_v3_math::error::UniswapV3MathError),

    #[error("Other: {0}")]
    Other(String),
}
