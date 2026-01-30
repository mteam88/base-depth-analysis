use crate::contracts::{IAerodromePool, IAerodromePoolFactory};
use crate::provider::{block_id, RpcProvider};
use alloy_primitives::aliases::U24;
use alloy_primitives::{Address, U256};
use async_trait::async_trait;
use base_depth_core::{StatefulSimulator, TokenInfo, Venue};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct AerodromePoolSimulator {
    pub pool: Address,
    pub pool_label: String,
    pub token0: Address,
    pub token1: Address,
    pub reserve0: U256,
    pub reserve1: U256,
    pub decimals0: U256,
    pub decimals1: U256,
    pub stable: bool,
    pub fee_bps: u32,
    pub base: TokenInfo,
    pub quote: TokenInfo,
}

impl AerodromePoolSimulator {
    pub async fn from_factory(
        provider: Arc<RpcProvider>,
        factory: Address,
        token_a: Address,
        token_b: Address,
        stable: bool,
        block: Option<u64>,
        base: TokenInfo,
        quote: TokenInfo,
    ) -> anyhow::Result<Option<Self>> {
        let fee_flag = if stable {
            U24::from(1u8)
        } else {
            U24::from(0u8)
        };
        let pool = match block {
            Some(block_number) => {
                IAerodromePoolFactory::new(factory, provider.clone())
                    .getPool(token_a, token_b, fee_flag)
                    .block(block_id(block_number))
                    .call()
                    .await?
            }
            None => {
                IAerodromePoolFactory::new(factory, provider.clone())
                    .getPool(token_a, token_b, fee_flag)
                    .call()
                    .await?
            }
        };
        if pool.is_zero() {
            return Ok(None);
        }

        let (metadata, fee_bps) = match block {
            Some(block_number) => {
                let metadata = IAerodromePool::new(pool, provider.clone())
                    .metadata()
                    .block(block_id(block_number))
                    .call()
                    .await?;
                let fee_bps = IAerodromePoolFactory::new(factory, provider.clone())
                    .getFee(pool, stable)
                    .block(block_id(block_number))
                    .call()
                    .await?;
                (metadata, fee_bps)
            }
            None => {
                let metadata = IAerodromePool::new(pool, provider.clone())
                    .metadata()
                    .call()
                    .await?;
                let fee_bps = IAerodromePoolFactory::new(factory, provider.clone())
                    .getFee(pool, stable)
                    .call()
                    .await?;
                (metadata, fee_bps)
            }
        };

        Ok(Some(Self {
            pool,
            pool_label: format!("{pool:?}"),
            token0: metadata.t0,
            token1: metadata.t1,
            reserve0: metadata.r0,
            reserve1: metadata.r1,
            decimals0: metadata.dec0,
            decimals1: metadata.dec1,
            stable: metadata.st,
            fee_bps: fee_bps.to::<u32>(),
            base,
            quote,
        }))
    }

    fn amount_out(&self, token_in: Address, amount_in: U256) -> anyhow::Result<U256> {
        if amount_in.is_zero() {
            return Ok(U256::ZERO);
        }

        let fee_numerator = U256::from(10_000u64 - self.fee_bps as u64);
        let amount_in = amount_in * fee_numerator / U256::from(10_000u64);

        if self.stable {
            let xy = self.k(self.reserve0, self.reserve1)?;
            let reserve0 =
                self.reserve0 * U256::from(1_000_000_000_000_000_000u128) / self.decimals0;
            let reserve1 =
                self.reserve1 * U256::from(1_000_000_000_000_000_000u128) / self.decimals1;
            let (reserve_a, reserve_b, decimals_out) = if token_in == self.token0 {
                (reserve0, reserve1, self.decimals1)
            } else {
                (reserve1, reserve0, self.decimals0)
            };
            let amount_in_scaled = if token_in == self.token0 {
                amount_in * U256::from(1_000_000_000_000_000_000u128) / self.decimals0
            } else {
                amount_in * U256::from(1_000_000_000_000_000_000u128) / self.decimals1
            };
            let y = reserve_b.saturating_sub(self.get_y(
                amount_in_scaled + reserve_a,
                xy,
                reserve_b,
            )?);
            Ok(y * decimals_out / U256::from(1_000_000_000_000_000_000u128))
        } else {
            let (reserve_in, reserve_out) = if token_in == self.token0 {
                (self.reserve0, self.reserve1)
            } else {
                (self.reserve1, self.reserve0)
            };
            Ok(amount_in * reserve_out / (reserve_in + amount_in))
        }
    }

    fn apply_swap(&mut self, token_in: Address, amount_in: U256, amount_out: U256) {
        if token_in == self.token0 {
            self.reserve0 = self.reserve0 + amount_in;
            self.reserve1 = self.reserve1.saturating_sub(amount_out);
        } else {
            self.reserve1 = self.reserve1 + amount_in;
            self.reserve0 = self.reserve0.saturating_sub(amount_out);
        }
    }

    fn k(&self, x: U256, y: U256) -> anyhow::Result<U256> {
        if self.stable {
            let one = U256::from(1_000_000_000_000_000_000u128);
            let _x = x * one / self.decimals0;
            let _y = y * one / self.decimals1;
            let a = _x * _y / one;
            let b = (_x * _x / one) + (_y * _y / one);
            Ok(a * b / one)
        } else {
            Ok(x * y)
        }
    }

    fn f(&self, x0: U256, y: U256) -> U256 {
        let one = U256::from(1_000_000_000_000_000_000u128);
        let a = x0 * y / one;
        let b = (x0 * x0 / one) + (y * y / one);
        a * b / one
    }

    fn d(&self, x0: U256, y: U256) -> U256 {
        let one = U256::from(1_000_000_000_000_000_000u128);
        (U256::from(3u64) * x0 * (y * y / one) / one) + (x0 * x0 / one * x0 / one)
    }

    fn get_y(&self, x0: U256, xy: U256, mut y: U256) -> anyhow::Result<U256> {
        let one = U256::from(1_000_000_000_000_000_000u128);
        for _ in 0..255 {
            let k = self.f(x0, y);
            if k < xy {
                let mut dy = (xy - k) * one / self.d(x0, y);
                if dy.is_zero() {
                    if k == xy {
                        return Ok(y);
                    }
                    if self.k(x0, y + U256::from(1u64))? > xy {
                        return Ok(y + U256::from(1u64));
                    }
                    dy = U256::from(1u64);
                }
                y = y + dy;
            } else {
                let mut dy = (k - xy) * one / self.d(x0, y);
                if dy.is_zero() {
                    if k == xy {
                        return Ok(y);
                    }
                    if y.is_zero() {
                        return Ok(y);
                    }
                    if self.f(x0, y - U256::from(1u64)) < xy {
                        return Ok(y);
                    }
                    dy = U256::from(1u64);
                }
                y = y - dy;
            }
        }
        Ok(y)
    }
}

#[async_trait]
impl StatefulSimulator for AerodromePoolSimulator {
    fn clone_box(&self) -> Box<dyn StatefulSimulator> {
        Box::new(self.clone())
    }

    fn venue(&self) -> Venue {
        Venue::AerodromeV2
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
        let base_scale = 10f64.powi(self.base.decimals as i32);
        let quote_scale = 10f64.powi(self.quote.decimals as i32);
        let (base_reserve, quote_reserve) = if self.base.address == self.token0 {
            (self.reserve0, self.reserve1)
        } else {
            (self.reserve1, self.reserve0)
        };
        let base_reserve: u128 = base_reserve.try_into().unwrap_or(u128::MAX);
        let quote_reserve: u128 = quote_reserve.try_into().unwrap_or(u128::MAX);
        if base_reserve == 0 {
            return Ok(0.0);
        }
        Ok((quote_reserve as f64 / quote_scale) / (base_reserve as f64 / base_scale))
    }

    async fn swap_exact_in(&mut self, token_in: Address, amount_in: U256) -> anyhow::Result<U256> {
        let amount_out = self.amount_out(token_in, amount_in)?;
        self.apply_swap(token_in, amount_in, amount_out);
        Ok(amount_out)
    }
}
