use crate::contracts::ITesseraSwap;
use crate::provider::{block_id, RpcProvider};
use alloy_primitives::{Address, I256};
use base_depth_core::{Quote, QuoteContext, QuoteEngine, QuoteExactInRequest, Venue};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct TesseraConfig {
    pub swap: Address,
    pub treasury: Option<Address>,
    pub engine: Option<Address>,
    pub sender: Option<Address>,
}

impl TesseraConfig {
    pub fn from_env() -> Option<Self> {
        let swap = std::env::var("TESSERA_SWAP").ok()?;
        let swap = parse_address(&swap)?;
        let treasury = std::env::var("TESSERA_TREASURY")
            .ok()
            .and_then(|v| parse_address(&v));
        let engine = std::env::var("TESSERA_ENGINE")
            .ok()
            .and_then(|v| parse_address(&v));
        let sender = std::env::var("TESSERA_SENDER")
            .ok()
            .and_then(|v| parse_address(&v));
        Some(Self {
            swap,
            treasury,
            engine,
            sender,
        })
    }
}

#[derive(Clone)]
pub struct TesseraQuoteEngine {
    provider: Arc<RpcProvider>,
    swap: Address,
    sender: Option<Address>,
}

impl TesseraQuoteEngine {
    pub fn new(provider: Arc<RpcProvider>, swap: Address, sender: Option<Address>) -> Self {
        Self {
            provider,
            swap,
            sender,
        }
    }
}

#[async_trait::async_trait]
impl QuoteEngine for TesseraQuoteEngine {
    async fn quote_exact_in(
        &self,
        request: QuoteExactInRequest,
        ctx: &QuoteContext,
    ) -> anyhow::Result<Quote> {
        let amount_specified_i128 = i128::try_from(request.amount_in)
            .map_err(|_| anyhow::anyhow!("amount_in too large for tessera int256"))?;
        let amount_specified = I256::try_from(amount_specified_i128)
            .map_err(|_| anyhow::anyhow!("amount_in too large for tessera int256"))?;

        let contract = ITesseraSwap::new(self.swap, self.provider.clone());
        let mut call = contract.tesseraSwapViewAmounts(
            request.token_in,
            request.token_out,
            amount_specified,
        );
        if let Some(sender) = self.sender {
            call = call.from(sender);
        }

        let result = if let Some(block_number) = ctx.block_number {
            call.block(block_id(block_number)).call().await?
        } else {
            call.call().await?
        };
        let amount_in = result.amountIn;
        let amount_out = result.amountOut;

        Ok(Quote {
            amount_in,
            amount_out,
            venue: Venue::TesseraSwap,
        })
    }
}

fn parse_address(input: &str) -> Option<Address> {
    Address::parse_checksummed(input, None)
        .ok()
        .or_else(|| input.parse().ok())
}
