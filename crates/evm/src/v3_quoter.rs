use crate::contracts::IUniswapV3QuoterV2;
use crate::provider::{block_id, RpcProvider};
use alloy_primitives::aliases::{U160, U24};
use alloy_primitives::Address;
use base_depth_core::{Quote, QuoteContext, QuoteEngine, QuoteExactInRequest, Venue};
use std::sync::Arc;

#[derive(Clone)]
pub struct V3QuoterEngine {
    provider: Arc<RpcProvider>,
    quoter: Address,
    fee: u32,
    venue: Venue,
}

impl V3QuoterEngine {
    pub fn new(provider: Arc<RpcProvider>, quoter: Address, fee: u32, venue: Venue) -> Self {
        Self {
            provider,
            quoter,
            fee,
            venue,
        }
    }
}

#[async_trait::async_trait]
impl QuoteEngine for V3QuoterEngine {
    async fn quote_exact_in(
        &self,
        request: QuoteExactInRequest,
        ctx: &QuoteContext,
    ) -> anyhow::Result<Quote> {
        let contract = IUniswapV3QuoterV2::new(self.quoter, self.provider.clone());
        let params = IUniswapV3QuoterV2::QuoteExactInputSingleParams {
            tokenIn: request.token_in,
            tokenOut: request.token_out,
            amountIn: request.amount_in,
            fee: U24::from(self.fee),
            sqrtPriceLimitX96: U160::ZERO,
        };

        let mut call = contract.quoteExactInputSingle(params);
        if let Some(block_number) = ctx.block_number {
            call = call.block(block_id(block_number));
        }

        let result = call.call().await?;
        Ok(Quote {
            amount_in: request.amount_in,
            amount_out: result.amountOut,
            venue: self.venue,
        })
    }
}
