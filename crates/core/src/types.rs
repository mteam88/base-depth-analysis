use alloy_primitives::{Address, U256};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Bid,
    Ask,
}

impl Side {
    pub fn as_str(&self) -> &'static str {
        match self {
            Side::Bid => "bid",
            Side::Ask => "ask",
        }
    }
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Venue {
    UniswapV3,
    PancakeV3,
    Slipstream,
    PancakeV2,
    AerodromeV2,
    Kyber,
    Unified,
    UnifiedKyber,
    #[serde(rename = "tessera")]
    TesseraSwap,
}

impl Venue {
    pub fn as_str(&self) -> &'static str {
        match self {
            Venue::UniswapV3 => "uniswap_v3",
            Venue::PancakeV3 => "pancake_v3",
            Venue::Slipstream => "slipstream",
            Venue::PancakeV2 => "pancake_v2",
            Venue::AerodromeV2 => "aerodrome_v2",
            Venue::Kyber => "kyber",
            Venue::Unified => "unified",
            Venue::UnifiedKyber => "unified_kyber",
            Venue::TesseraSwap => "tessera",
        }
    }
}

impl fmt::Display for Venue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Venue {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let value = s.trim().to_lowercase();
        match value.as_str() {
            "uniswap_v3" | "uniswap" => Ok(Venue::UniswapV3),
            "pancake_v3" | "pancakev3" => Ok(Venue::PancakeV3),
            "slipstream" => Ok(Venue::Slipstream),
            "pancake_v2" | "pancakev2" => Ok(Venue::PancakeV2),
            "aerodrome_v2" | "aerodrome" => Ok(Venue::AerodromeV2),
            "kyber" | "kyberswap" => Ok(Venue::Kyber),
            "unified" => Ok(Venue::Unified),
            "unified_kyber" | "kyber_unified" => Ok(Venue::UnifiedKyber),
            "tessera" | "tessera_swap" | "tesseraswap" => Ok(Venue::TesseraSwap),
            other => Err(format!("unknown venue: {other}")),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenInfo {
    pub address: Address,
    pub symbol: String,
    pub decimals: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PairInfo {
    pub base: TokenInfo,
    pub quote: TokenInfo,
}

#[derive(Clone, Debug)]
pub struct QuoteExactInRequest {
    pub token_in: Address,
    pub token_out: Address,
    pub amount_in: U256,
}

#[derive(Clone, Debug, Default)]
pub struct QuoteContext {
    pub chain_id: u64,
    pub block_number: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct Quote {
    pub amount_in: U256,
    pub amount_out: U256,
    pub venue: Venue,
}

#[async_trait]
pub trait QuoteEngine: Send + Sync {
    async fn quote_exact_in(
        &self,
        request: QuoteExactInRequest,
        ctx: &QuoteContext,
    ) -> anyhow::Result<Quote>;
}

#[async_trait]
pub trait StatefulSimulator: Send + Sync {
    fn clone_box(&self) -> Box<dyn StatefulSimulator>;
    fn venue(&self) -> Venue;
    fn pool_label(&self) -> &str;
    fn base(&self) -> &TokenInfo;
    fn quote(&self) -> &TokenInfo;
    fn mid_price(&self) -> anyhow::Result<f64>;
    fn liquidity_tag(&self) -> Option<String> {
        None
    }

    async fn swap_exact_in(&mut self, token_in: Address, amount_in: U256) -> anyhow::Result<U256>;

    async fn quote_exact_in(&self, token_in: Address, amount_in: U256) -> anyhow::Result<U256> {
        let mut cloned = self.clone_box();
        cloned.swap_exact_in(token_in, amount_in).await
    }
}

impl Clone for Box<dyn StatefulSimulator> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}
