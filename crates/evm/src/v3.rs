use crate::contracts::{IMulticall3, IUniswapV3Pool};
use crate::provider::{block_id, RpcProvider};
use alloy::sol_types::SolCall;
use alloy_primitives::aliases::I24;
use alloy_primitives::{Address, I256, U256};
use async_trait::async_trait;
use base_depth_core::{StatefulSimulator, TokenInfo, Venue};
use std::collections::HashMap;
use std::sync::Arc;
use uniswap_v3_math::swap_math::compute_swap_step;
use uniswap_v3_math::tick_bitmap;
use uniswap_v3_math::tick_math;

#[derive(Clone, Debug)]
pub struct V3PoolSimulator {
    pub venue: Venue,
    pub pool: Address,
    pub pool_label: String,
    pub token0: Address,
    pub token1: Address,
    pub base: TokenInfo,
    pub quote: TokenInfo,
    pub fee: u32,
    pub tick_spacing: i32,
    pub sqrt_price_x96: U256,
    pub tick: i32,
    pub liquidity: u128,
    pub last_tick: i32,
    pub tick_bitmap: HashMap<i16, U256>,
    pub ticks: HashMap<i32, TickInfo>,
    pub provider: Arc<RpcProvider>,
    pub block: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct TickInfo {
    pub liquidity_gross: u128,
    pub liquidity_net: i128,
    pub initialized: bool,
}

impl V3PoolSimulator {
    pub async fn from_pool(
        provider: Arc<crate::provider::RpcProvider>,
        pool: Address,
        base: TokenInfo,
        quote: TokenInfo,
        venue: Venue,
        block: Option<u64>,
        multicall3: Option<Address>,
    ) -> anyhow::Result<Self> {
        let state = fetch_v3_state(provider.clone(), pool, block, multicall3).await?;
        Ok(Self {
            venue,
            pool,
            pool_label: format!("{pool:?}"),
            token0: state.token0,
            token1: state.token1,
            base,
            quote,
            fee: state.fee,
            tick_spacing: state.tick_spacing,
            sqrt_price_x96: state.sqrt_price_x96,
            tick: state.tick,
            last_tick: state.tick,
            liquidity: state.liquidity,
            tick_bitmap: HashMap::new(),
            ticks: HashMap::new(),
            provider,
            block,
        })
    }

    async fn load_tick_bitmap_word(&mut self, word_pos: i16) -> anyhow::Result<U256> {
        if let Some(word) = self.tick_bitmap.get(&word_pos) {
            return Ok(*word);
        }
        let pool = IUniswapV3Pool::new(self.pool, self.provider.clone());
        let word = match self.block {
            Some(block_number) => {
                pool.tickBitmap(word_pos)
                    .block(block_id(block_number))
                    .call()
                    .await?
            }
            None => pool.tickBitmap(word_pos).call().await?,
        };
        self.tick_bitmap.insert(word_pos, word);
        Ok(word)
    }

    async fn load_tick(&mut self, tick_idx: i32) -> anyhow::Result<TickInfo> {
        if let Some(tick) = self.ticks.get(&tick_idx) {
            return Ok(tick.clone());
        }
        let pool = IUniswapV3Pool::new(self.pool, self.provider.clone());
        let tick_i24 = I24::try_from(tick_idx)
            .map_err(|_| anyhow::anyhow!("tick out of range: {tick_idx}"))?;
        // Decoded by word so the same code handles Uniswap v3 (8 fields) and
        // Slipstream (10 fields): liquidityGross and liquidityNet are the first two
        // words in both, and `initialized` is always the last.
        let calldata = pool.ticks(tick_i24).calldata().clone();
        let data = raw_call(&self.provider, self.pool, calldata, self.block).await?;
        let words = words(&data, 8)?;
        let tick = TickInfo {
            liquidity_gross: u128::try_from(U256::from_be_slice(words[0]))
                .map_err(|_| anyhow::anyhow!("liquidityGross overflow"))?,
            liquidity_net: i128::try_from(I256::from_raw(U256::from_be_slice(words[1])))
                .map_err(|_| anyhow::anyhow!("liquidityNet overflow"))?,
            initialized: !U256::from_be_slice(words[words.len() - 1]).is_zero(),
        };
        self.ticks.insert(tick_idx, tick.clone());
        Ok(tick)
    }

    async fn next_initialized_tick(&mut self, tick: i32, lte: bool) -> anyhow::Result<(i32, bool)> {
        let compressed = if tick < 0 && tick % self.tick_spacing != 0 {
            (tick / self.tick_spacing) - 1
        } else {
            tick / self.tick_spacing
        };
        let word_pos = if lte {
            (compressed >> 8) as i16
        } else {
            ((compressed + 1) >> 8) as i16
        };
        self.load_tick_bitmap_word(word_pos).await?;
        let (next, initialized) = tick_bitmap::next_initialized_tick_within_one_word(
            &self.tick_bitmap,
            tick,
            self.tick_spacing,
            lte,
        )?;
        Ok((next, initialized))
    }

    fn apply_liquidity_delta(&self, liquidity: u128, delta: i128) -> anyhow::Result<u128> {
        let current = liquidity as i128;
        let next = current
            .checked_add(delta)
            .ok_or_else(|| anyhow::anyhow!("liquidity overflow"))?;
        if next < 0 {
            return Err(anyhow::anyhow!("liquidity underflow"));
        }
        Ok(next as u128)
    }
}

#[async_trait]
impl StatefulSimulator for V3PoolSimulator {
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
        let base_scale = 10f64.powi(self.base.decimals as i32);
        let quote_scale = 10f64.powi(self.quote.decimals as i32);
        let sqrt_price = self
            .sqrt_price_x96
            .to_string()
            .parse::<f64>()
            .unwrap_or(0.0);
        if sqrt_price <= 0.0 {
            return Ok(0.0);
        }
        let price = (sqrt_price * sqrt_price) / 2f64.powi(192);
        let (base_is_token0, quote_is_token1) = (
            self.base.address == self.token0,
            self.quote.address == self.token1,
        );
        let raw_price = if base_is_token0 && quote_is_token1 {
            price
        } else {
            if price == 0.0 {
                0.0
            } else {
                1.0 / price
            }
        };
        Ok(raw_price * (base_scale / quote_scale))
    }

    fn liquidity_tag(&self) -> Option<String> {
        Some(format!("tick={}", self.last_tick))
    }

    async fn swap_exact_in(&mut self, token_in: Address, amount_in: U256) -> anyhow::Result<U256> {
        if amount_in.is_zero() {
            return Ok(U256::ZERO);
        }
        let zero_for_one = token_in == self.token0;
        let mut amount_remaining = I256::from_raw(amount_in);
        let mut amount_out = U256::ZERO;
        let mut sqrt_price_x96 = self.sqrt_price_x96;
        let mut tick = self.tick;
        let mut liquidity = self.liquidity;

        for _ in 0..10_000 {
            if amount_remaining <= I256::ZERO {
                break;
            }
            if sqrt_price_x96 <= tick_math::MIN_SQRT_RATIO
                || sqrt_price_x96 >= tick_math::MAX_SQRT_RATIO
            {
                break;
            }
            if liquidity == 0 {
                break;
            }

            let (next_tick, initialized) = self.next_initialized_tick(tick, zero_for_one).await?;
            let sqrt_price_next = tick_math::get_sqrt_ratio_at_tick(next_tick)?;
            let target = if zero_for_one {
                if sqrt_price_next < tick_math::MIN_SQRT_RATIO {
                    tick_math::MIN_SQRT_RATIO
                } else {
                    sqrt_price_next
                }
            } else {
                if sqrt_price_next > tick_math::MAX_SQRT_RATIO {
                    tick_math::MAX_SQRT_RATIO
                } else {
                    sqrt_price_next
                }
            };

            let (sqrt_price_new, amount_in_used, amount_out_step, fee_amount) = compute_swap_step(
                sqrt_price_x96,
                target,
                liquidity,
                amount_remaining,
                self.fee,
            )?;

            let consumed = amount_in_used + fee_amount;
            amount_remaining = amount_remaining - I256::from_raw(consumed);
            amount_out = amount_out + amount_out_step;
            sqrt_price_x96 = sqrt_price_new;

            if sqrt_price_x96 == target {
                if initialized {
                    let tick_info = self.load_tick(next_tick).await?;
                    let delta = if zero_for_one {
                        -tick_info.liquidity_net
                    } else {
                        tick_info.liquidity_net
                    };
                    liquidity = self.apply_liquidity_delta(liquidity, delta)?;
                }
                tick = if zero_for_one {
                    next_tick - 1
                } else {
                    next_tick
                };
            } else {
                tick = tick_math::get_tick_at_sqrt_ratio(sqrt_price_x96)?;
            }
        }

        self.sqrt_price_x96 = sqrt_price_x96;
        self.tick = tick;
        self.last_tick = tick;
        self.liquidity = liquidity;
        Ok(amount_out)
    }
}

struct V3State {
    token0: Address,
    token1: Address,
    fee: u32,
    tick_spacing: i32,
    sqrt_price_x96: U256,
    tick: i32,
    liquidity: u128,
}

fn signed_to_i32(value: I24) -> i32 {
    i128::try_from(value).unwrap_or(0) as i32
}

async fn fetch_v3_state(
    provider: Arc<crate::provider::RpcProvider>,
    pool: Address,
    block: Option<u64>,
    multicall3: Option<Address>,
) -> anyhow::Result<V3State> {
    if let Some(multicall) = multicall3 {
        if let Ok(state) = fetch_v3_state_multicall(provider.clone(), pool, block, multicall).await
        {
            return Ok(state);
        }
    }

    let pool_contract = IUniswapV3Pool::new(pool, provider.clone());
    let (slot0, liquidity, tick_spacing, fee, token0, token1) = match block {
        Some(block_number) => {
            let slot0 = raw_call(
                &provider,
                pool,
                pool_contract.slot0().calldata().clone(),
                Some(block_number),
            )
            .await
            .and_then(|data| decode_slot0(&data))?;
            let liquidity = pool_contract
                .liquidity()
                .block(block_id(block_number))
                .call()
                .await?;
            let tick_spacing = pool_contract
                .tickSpacing()
                .block(block_id(block_number))
                .call()
                .await?;
            let fee = pool_contract
                .fee()
                .block(block_id(block_number))
                .call()
                .await?;
            let token0 = pool_contract
                .token0()
                .block(block_id(block_number))
                .call()
                .await?;
            let token1 = pool_contract
                .token1()
                .block(block_id(block_number))
                .call()
                .await?;
            (slot0, liquidity, tick_spacing, fee, token0, token1)
        }
        None => {
            let slot0 = raw_call(&provider, pool, pool_contract.slot0().calldata().clone(), None)
                .await
                .and_then(|data| decode_slot0(&data))?;
            let liquidity = pool_contract.liquidity().call().await?;
            let tick_spacing = pool_contract.tickSpacing().call().await?;
            let fee = pool_contract.fee().call().await?;
            let token0 = pool_contract.token0().call().await?;
            let token1 = pool_contract.token1().call().await?;
            (slot0, liquidity, tick_spacing, fee, token0, token1)
        }
    };

    Ok(V3State {
        token0,
        token1,
        fee: fee.to::<u32>(),
        tick_spacing: signed_to_i32(tick_spacing),
        sqrt_price_x96: slot0.0,
        tick: slot0.1,
        liquidity,
    })
}

async fn fetch_v3_state_multicall(
    provider: Arc<crate::provider::RpcProvider>,
    pool: Address,
    block: Option<u64>,
    multicall3: Address,
) -> anyhow::Result<V3State> {
    let pool_contract = IUniswapV3Pool::new(pool, provider.clone());
    let multicall = IMulticall3::new(multicall3, provider.clone());

    let calls = vec![
        IMulticall3::Call {
            target: pool,
            callData: pool_contract.slot0().calldata().clone(),
        },
        IMulticall3::Call {
            target: pool,
            callData: pool_contract.liquidity().calldata().clone(),
        },
        IMulticall3::Call {
            target: pool,
            callData: pool_contract.tickSpacing().calldata().clone(),
        },
        IMulticall3::Call {
            target: pool,
            callData: pool_contract.fee().calldata().clone(),
        },
        IMulticall3::Call {
            target: pool,
            callData: pool_contract.token0().calldata().clone(),
        },
        IMulticall3::Call {
            target: pool,
            callData: pool_contract.token1().calldata().clone(),
        },
    ];

    let result = match block {
        Some(block_number) => {
            multicall
                .aggregate(calls)
                .block(block_id(block_number))
                .call()
                .await?
        }
        None => multicall.aggregate(calls).call().await?,
    };

    let IMulticall3::aggregateReturn { returnData, .. } = result;
    let slot0 = decode_slot0(&returnData[0])?;
    let liquidity = IUniswapV3Pool::liquidityCall::abi_decode_returns(&returnData[1])?;
    let tick_spacing = IUniswapV3Pool::tickSpacingCall::abi_decode_returns(&returnData[2])?;
    let fee = IUniswapV3Pool::feeCall::abi_decode_returns(&returnData[3])?;
    let token0 = IUniswapV3Pool::token0Call::abi_decode_returns(&returnData[4])?;
    let token1 = IUniswapV3Pool::token1Call::abi_decode_returns(&returnData[5])?;

    Ok(V3State {
        token0,
        token1,
        fee: fee.to::<u32>(),
        tick_spacing: signed_to_i32(tick_spacing),
        sqrt_price_x96: slot0.0,
        tick: slot0.1,
        liquidity,
    })
}

/// `eth_call` returning raw bytes, for calls whose return layout differs between
/// v3 forks (Slipstream's `slot0` has 6 fields, Uniswap's has 7).
async fn raw_call(
    provider: &Arc<RpcProvider>,
    to: Address,
    calldata: alloy_primitives::Bytes,
    block: Option<u64>,
) -> anyhow::Result<alloy_primitives::Bytes> {
    use alloy::network::TransactionBuilder;
    use alloy::providers::Provider;
    use alloy::rpc::types::TransactionRequest;

    let tx = TransactionRequest::default().with_to(to).with_input(calldata);
    let call = provider.call(tx);
    let out = match block {
        Some(block_number) => call.block(block_id(block_number)).await?,
        None => call.await?,
    };
    Ok(out)
}

fn words(data: &[u8], min_words: usize) -> anyhow::Result<Vec<&[u8]>> {
    if data.len() < min_words * 32 {
        return Err(anyhow::anyhow!(
            "short return data: {} bytes, expected at least {}",
            data.len(),
            min_words * 32
        ));
    }
    Ok(data.chunks_exact(32).collect())
}

/// Reads `(sqrtPriceX96, tick)` from the first two words of `slot0()`, which is
/// the same across Uniswap v3, PancakeSwap v3 and Slipstream.
fn decode_slot0(data: &[u8]) -> anyhow::Result<(U256, i32)> {
    let words = words(data, 2)?;
    let sqrt_price_x96 = U256::from_be_slice(words[0]);
    let tick = i32::try_from(I256::from_raw(U256::from_be_slice(words[1])))
        .map_err(|_| anyhow::anyhow!("tick out of range"))?;
    Ok((sqrt_price_x96, tick))
}
