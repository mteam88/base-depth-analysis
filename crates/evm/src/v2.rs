use crate::contracts::{IUniswapV2Factory, IUniswapV2Pair};
use crate::provider::{block_id, RpcProvider};
use alloy_primitives::{Address, U256};
use async_trait::async_trait;
use base_depth_core::{StatefulSimulator, TokenInfo, Venue};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct V2PoolSimulator {
    pub venue: Venue,
    pub pool: Address,
    pub pool_label: String,
    pub token0: Address,
    pub token1: Address,
    pub reserve0: u128,
    pub reserve1: u128,
    pub fee_bps: u32,
    pub base: TokenInfo,
    pub quote: TokenInfo,
}

impl V2PoolSimulator {
    pub fn new(
        venue: Venue,
        pool: Address,
        token0: Address,
        token1: Address,
        reserve0: u128,
        reserve1: u128,
        fee_bps: u32,
        base: TokenInfo,
        quote: TokenInfo,
    ) -> Self {
        Self {
            venue,
            pool,
            pool_label: format!("{pool:?}"),
            token0,
            token1,
            reserve0,
            reserve1,
            fee_bps,
            base,
            quote,
        }
    }

    pub async fn from_factory(
        provider: Arc<RpcProvider>,
        factory: Address,
        token_a: Address,
        token_b: Address,
        block: Option<u64>,
        venue: Venue,
        fee_bps: u32,
        base: TokenInfo,
        quote: TokenInfo,
    ) -> anyhow::Result<Option<Self>> {
        let pool = match block {
            Some(block_number) => {
                IUniswapV2Factory::new(factory, provider.clone())
                    .getPair(token_a, token_b)
                    .block(block_id(block_number))
                    .call()
                    .await?
            }
            None => {
                IUniswapV2Factory::new(factory, provider.clone())
                    .getPair(token_a, token_b)
                    .call()
                    .await?
            }
        };
        if pool.is_zero() {
            return Ok(None);
        }

        let pair = IUniswapV2Pair::new(pool, provider.clone());
        let (token0, token1, reserves) = match block {
            Some(block_number) => {
                let token0 = pair.token0().block(block_id(block_number)).call().await?;
                let token1 = pair.token1().block(block_id(block_number)).call().await?;
                let reserves = pair
                    .getReserves()
                    .block(block_id(block_number))
                    .call()
                    .await?;
                (token0, token1, reserves)
            }
            None => {
                let token0 = pair.token0().call().await?;
                let token1 = pair.token1().call().await?;
                let reserves = pair.getReserves().call().await?;
                (token0, token1, reserves)
            }
        };
        let token0 = token0;
        let token1 = token1;
        let reserve0: u128 = reserves.reserve0.to::<u128>();
        let reserve1: u128 = reserves.reserve1.to::<u128>();

        Ok(Some(Self::new(
            venue, pool, token0, token1, reserve0, reserve1, fee_bps, base, quote,
        )))
    }

    fn amount_out(&self, token_in: Address, amount_in: U256) -> anyhow::Result<U256> {
        if amount_in.is_zero() {
            return Ok(U256::ZERO);
        }
        let (reserve_in, reserve_out) = if token_in == self.token0 {
            (self.reserve0, self.reserve1)
        } else {
            (self.reserve1, self.reserve0)
        };

        let fee_numerator = 10_000u128.saturating_sub(self.fee_bps as u128);
        let amount_in_u128: u128 = amount_in.try_into().unwrap_or(u128::MAX);
        let amount_in_with_fee = amount_in_u128.saturating_mul(fee_numerator) / 10_000u128;
        let numerator = (amount_in_with_fee as u128).saturating_mul(reserve_out);
        let denominator = reserve_in.saturating_add(amount_in_with_fee);
        if denominator == 0 {
            return Ok(U256::ZERO);
        }
        Ok(U256::from(numerator / denominator))
    }

    fn apply_swap(&mut self, token_in: Address, amount_in: U256, amount_out: U256) {
        let amount_in_u128: u128 = amount_in.try_into().unwrap_or(u128::MAX);
        let amount_out_u128: u128 = amount_out.try_into().unwrap_or(u128::MAX);
        if token_in == self.token0 {
            self.reserve0 = self.reserve0.saturating_add(amount_in_u128);
            self.reserve1 = self.reserve1.saturating_sub(amount_out_u128);
        } else {
            self.reserve1 = self.reserve1.saturating_add(amount_in_u128);
            self.reserve0 = self.reserve0.saturating_sub(amount_out_u128);
        }
    }
}

#[async_trait]
impl StatefulSimulator for V2PoolSimulator {
    fn clone_box(&self) -> Box<dyn StatefulSimulator> {
        Box::new(self.clone())
    }

    fn venue(&self) -> Venue {
        self.venue
    }

    fn pool_label(&self) -> &str {
        self.pool_label.as_str()
    }

    fn base(&self) -> &TokenInfo {
        &self.base
    }

    fn quote(&self) -> &TokenInfo {
        &self.quote
    }

    fn mid_price(&self) -> anyhow::Result<f64> {
        let (base_reserve, quote_reserve) = if self.base.address == self.token0 {
            (self.reserve0 as f64, self.reserve1 as f64)
        } else {
            (self.reserve1 as f64, self.reserve0 as f64)
        };
        if base_reserve <= 0.0 {
            return Ok(0.0);
        }
        let base_scale = 10f64.powi(self.base.decimals as i32);
        let quote_scale = 10f64.powi(self.quote.decimals as i32);
        let base_reserve = base_reserve / base_scale;
        let quote_reserve = quote_reserve / quote_scale;
        Ok(quote_reserve / base_reserve)
    }

    async fn swap_exact_in(&mut self, token_in: Address, amount_in: U256) -> anyhow::Result<U256> {
        let amount_out = self.amount_out(token_in, amount_in)?;
        self.apply_swap(token_in, amount_in, amount_out);
        Ok(amount_out)
    }
}
