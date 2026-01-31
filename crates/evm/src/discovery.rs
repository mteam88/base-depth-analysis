use crate::addresses::Addresses;
use crate::contracts::IUniswapV3Factory;
use crate::provider::{block_id, RpcContext};
use crate::{aerodrome::AerodromePoolSimulator, v2::V2PoolSimulator, v3::V3PoolSimulator};
use alloy_primitives::aliases::U24;
use alloy_primitives::Address;
use base_depth_core::{StatefulSimulator, TokenInfo, Venue};
use std::sync::Arc;
use tracing::warn;

pub const DEFAULT_V3_FEES: &[u32] = &[100, 500, 3000, 10_000];

#[derive(Clone)]
pub struct SimulatorEntry {
    pub venue: Venue,
    pub pool: Address,
    pub label: String,
    pub simulator: Box<dyn StatefulSimulator>,
}

#[derive(Clone, Debug)]
pub struct V3PoolEntry {
    pub venue: Venue,
    pub pool: Address,
    pub fee: u32,
    pub label: String,
}

pub async fn discover_simulators(
    ctx: &RpcContext,
    addresses: &Addresses,
    base: &TokenInfo,
    quote: &TokenInfo,
    venues: &[Venue],
    v3_fees: &[u32],
    block: Option<u64>,
) -> anyhow::Result<Vec<SimulatorEntry>> {
    let provider = ctx.provider.clone();
    let mut entries = Vec::new();

    for venue in venues {
        match venue {
            Venue::UniswapV3 => {
                let sims = discover_v3_simulators(
                    provider.clone(),
                    addresses.uniswap_v3_factory,
                    base,
                    quote,
                    *venue,
                    v3_fees,
                    block,
                    Some(addresses.multicall3),
                )
                .await?;
                entries.extend(sims);
            }
            Venue::PancakeV3 => {
                let sims = discover_v3_simulators(
                    provider.clone(),
                    addresses.pancake_v3_factory,
                    base,
                    quote,
                    *venue,
                    v3_fees,
                    block,
                    Some(addresses.multicall3),
                )
                .await?;
                entries.extend(sims);
            }
            Venue::Slipstream => {
                for factory in &addresses.slipstream_factories {
                    let sims = discover_v3_simulators(
                        provider.clone(),
                        *factory,
                        base,
                        quote,
                        *venue,
                        v3_fees,
                        block,
                        Some(addresses.multicall3),
                    )
                    .await?;
                    entries.extend(sims);
                }
            }
            Venue::PancakeV2 => {
                let sim = V2PoolSimulator::from_factory(
                    provider.clone(),
                    addresses.pancake_v2_factory,
                    base.address,
                    quote.address,
                    block,
                    *venue,
                    25,
                    base.clone(),
                    quote.clone(),
                )
                .await?;
                if let Some(mut sim) = sim {
                    let label = format!("{}:{:?}", venue.as_str(), sim.pool);
                    sim.pool_label = label.clone();
                    entries.push(SimulatorEntry {
                        venue: *venue,
                        pool: sim.pool,
                        label,
                        simulator: Box::new(sim),
                    });
                }
            }
            Venue::AerodromeV2 => {
                let stable_sim = AerodromePoolSimulator::from_factory(
                    provider.clone(),
                    addresses.aerodrome_factory,
                    base.address,
                    quote.address,
                    true,
                    block,
                    base.clone(),
                    quote.clone(),
                )
                .await?;
                if let Some(mut sim) = stable_sim {
                    let label = format!("{}:{:?}:stable", venue.as_str(), sim.pool);
                    sim.pool_label = label.clone();
                    entries.push(SimulatorEntry {
                        venue: *venue,
                        pool: sim.pool,
                        label,
                        simulator: Box::new(sim),
                    });
                }

                let volatile_sim = AerodromePoolSimulator::from_factory(
                    provider.clone(),
                    addresses.aerodrome_factory,
                    base.address,
                    quote.address,
                    false,
                    block,
                    base.clone(),
                    quote.clone(),
                )
                .await?;
                if let Some(mut sim) = volatile_sim {
                    let label = format!("{}:{:?}:volatile", venue.as_str(), sim.pool);
                    sim.pool_label = label.clone();
                    entries.push(SimulatorEntry {
                        venue: *venue,
                        pool: sim.pool,
                        label,
                        simulator: Box::new(sim),
                    });
                }
            }
            _ => {}
        }
    }

    Ok(entries)
}

pub async fn discover_v3_pools(
    provider: Arc<crate::provider::RpcProvider>,
    factory: Address,
    base: &TokenInfo,
    quote: &TokenInfo,
    venue: Venue,
    v3_fees: &[u32],
    block: Option<u64>,
) -> anyhow::Result<Vec<V3PoolEntry>> {
    let factory_contract = IUniswapV3Factory::new(factory, provider.clone());
    let mut entries = Vec::new();

    for fee in v3_fees {
        let pool_result = match block {
            Some(block_number) => {
                factory_contract
                    .getPool(base.address, quote.address, U24::from(*fee))
                    .block(block_id(block_number))
                    .call()
                    .await
            }
            None => {
                factory_contract
                    .getPool(base.address, quote.address, U24::from(*fee))
                    .call()
                    .await
            }
        };
        let pool = match pool_result {
            Ok(pool) => pool,
            Err(err) => {
                warn!(
                    "failed to fetch pool for {} fee {}: {err}",
                    venue.as_str(),
                    fee
                );
                continue;
            }
        };
        if pool.is_zero() {
            continue;
        }
        let label = format!("{}:{:?}:{}bps", venue.as_str(), pool, fee);
        entries.push(V3PoolEntry {
            venue,
            pool,
            fee: *fee,
            label,
        });
    }

    Ok(entries)
}

async fn discover_v3_simulators(
    provider: Arc<crate::provider::RpcProvider>,
    factory: Address,
    base: &TokenInfo,
    quote: &TokenInfo,
    venue: Venue,
    v3_fees: &[u32],
    block: Option<u64>,
    multicall3: Option<Address>,
) -> anyhow::Result<Vec<SimulatorEntry>> {
    let factory_contract = IUniswapV3Factory::new(factory, provider.clone());
    let mut entries = Vec::new();

    for fee in v3_fees {
        let pool_result = match block {
            Some(block_number) => {
                factory_contract
                    .getPool(base.address, quote.address, U24::from(*fee))
                    .block(block_id(block_number))
                    .call()
                    .await
            }
            None => {
                factory_contract
                    .getPool(base.address, quote.address, U24::from(*fee))
                    .call()
                    .await
            }
        };
        let pool = match pool_result {
            Ok(pool) => pool,
            Err(err) => {
                warn!(
                    "failed to fetch pool for {} fee {}: {err}",
                    venue.as_str(),
                    fee
                );
                continue;
            }
        };
        if pool.is_zero() {
            continue;
        }
        let mut sim = V3PoolSimulator::from_pool(
            provider.clone(),
            pool,
            base.clone(),
            quote.clone(),
            venue,
            block,
            multicall3,
        )
        .await?;
        let label = format!("{}:{:?}:{}bps", venue.as_str(), pool, fee);
        sim.pool_label = label.clone();
        entries.push(SimulatorEntry {
            venue,
            pool,
            label,
            simulator: Box::new(sim),
        });
    }

    Ok(entries)
}
