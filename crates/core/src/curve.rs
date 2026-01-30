use crate::schedule::StepSchedule;
use crate::types::{
    QuoteContext, QuoteEngine, QuoteExactInRequest, Side, StatefulSimulator, Venue,
};
use crate::utils::{amount_to_u256, u256_to_f64};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DepthPoint {
    pub price: f64,
    pub cum_base: f64,
    pub seg_base: f64,
    pub seg_quote: f64,
    pub marginal_price: f64,
    pub avg_price: f64,
    pub liquidity_tag: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DepthCurve {
    pub venue: Venue,
    pub pool: String,
    pub side: Side,
    pub points: Vec<DepthPoint>,
}

pub async fn build_curve_from_simulator(
    simulator: &dyn StatefulSimulator,
    schedule: &StepSchedule,
    side: Side,
) -> anyhow::Result<DepthCurve> {
    let mut sim = simulator.clone_box();
    let (token_in, decimals_in, decimals_out) = match side {
        Side::Ask => (
            sim.quote().address,
            sim.quote().decimals,
            sim.base().decimals,
        ),
        Side::Bid => (
            sim.base().address,
            sim.base().decimals,
            sim.quote().decimals,
        ),
    };

    let mut points = Vec::new();
    let mut cum_base = 0.0;
    let mut cum_quote = 0.0;

    for seg_in in schedule.segments() {
        let amount_in = amount_to_u256(seg_in, decimals_in);
        if amount_in.is_zero() {
            continue;
        }
        let amount_out = sim.swap_exact_in(token_in, amount_in).await?;
        let out_f64 = u256_to_f64(amount_out, decimals_out);
        if out_f64 <= 0.0 {
            continue;
        }

        let (seg_base, seg_quote) = match side {
            Side::Ask => (out_f64, seg_in),
            Side::Bid => (seg_in, out_f64),
        };

        if seg_base <= 0.0 {
            continue;
        }

        cum_base += seg_base;
        cum_quote += seg_quote;

        let marginal = seg_quote / seg_base;
        let avg = if cum_base > 0.0 {
            cum_quote / cum_base
        } else {
            marginal
        };

        points.push(DepthPoint {
            price: marginal,
            cum_base,
            seg_base,
            seg_quote,
            marginal_price: marginal,
            avg_price: avg,
            liquidity_tag: sim.liquidity_tag(),
        });
    }

    Ok(DepthCurve {
        venue: simulator.venue(),
        pool: simulator.pool_label().to_string(),
        side,
        points,
    })
}

pub async fn build_curve_from_quote_engine(
    engine: &dyn QuoteEngine,
    ctx: &QuoteContext,
    venue: Venue,
    pool: &str,
    schedule: &StepSchedule,
    side: Side,
    token_in: (alloy_primitives::Address, u8),
    token_out: (alloy_primitives::Address, u8),
    concurrency: usize,
    adaptive: Option<(usize, f64)>,
) -> anyhow::Result<DepthCurve> {
    use futures::stream::{self, StreamExt};

    let (token_in_addr, decimals_in) = token_in;
    let (token_out_addr, decimals_out) = token_out;
    let concurrency = concurrency.max(1);
    let totals: Vec<f64> = schedule
        .cumulative()
        .into_iter()
        .filter(|total_in| {
            let amount_in = amount_to_u256(*total_in, decimals_in);
            !amount_in.is_zero()
        })
        .collect();

    let mut samples = Vec::with_capacity(totals.len());
    let mut stream = stream::iter(totals.into_iter().map(|total_in| async move {
        let amount_in = amount_to_u256(total_in, decimals_in);
        let quote = engine
            .quote_exact_in(
                QuoteExactInRequest {
                    token_in: token_in_addr,
                    token_out: token_out_addr,
                    amount_in,
                },
                ctx,
            )
            .await?;
        let out = u256_to_f64(quote.amount_out, decimals_out);
        Ok::<(f64, f64), anyhow::Error>((total_in, out))
    }))
    .buffer_unordered(concurrency);

    while let Some(result) = stream.next().await {
        samples.push(result?);
    }

    if let Some((max_depth, tol_bps)) = adaptive {
        samples = refine_samples(
            engine,
            ctx,
            token_in_addr,
            token_out_addr,
            decimals_in,
            decimals_out,
            samples,
            max_depth,
            tol_bps,
        )
        .await?;
    }

    samples.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let mut points = Vec::new();
    let mut last_in = 0.0;
    let mut last_out = 0.0;
    let mut cum_base = 0.0;
    let mut cum_quote = 0.0;

    for (total_in, total_out) in samples {
        let seg_in = total_in - last_in;
        let seg_out = total_out - last_out;
        if seg_in <= 0.0 || seg_out <= 0.0 {
            last_in = total_in;
            last_out = total_out;
            continue;
        }

        let (seg_base, seg_quote) = match side {
            Side::Ask => (seg_out, seg_in),
            Side::Bid => (seg_in, seg_out),
        };

        if seg_base <= 0.0 {
            last_in = total_in;
            last_out = total_out;
            continue;
        }

        cum_base += seg_base;
        cum_quote += seg_quote;
        let marginal = seg_quote / seg_base;
        let avg = if cum_base > 0.0 {
            cum_quote / cum_base
        } else {
            marginal
        };

        points.push(DepthPoint {
            price: marginal,
            cum_base,
            seg_base,
            seg_quote,
            marginal_price: marginal,
            avg_price: avg,
            liquidity_tag: None,
        });

        last_in = total_in;
        last_out = total_out;
    }

    Ok(DepthCurve {
        venue,
        pool: pool.to_string(),
        side,
        points,
    })
}

pub async fn build_unified_curve(
    mut simulators: Vec<Box<dyn StatefulSimulator>>,
    schedule: &StepSchedule,
    side: Side,
    label: &str,
) -> anyhow::Result<DepthCurve> {
    if simulators.is_empty() {
        return Ok(DepthCurve {
            venue: Venue::Unified,
            pool: label.to_string(),
            side,
            points: Vec::new(),
        });
    }

    let (token_in, decimals_in, decimals_out) = match side {
        Side::Ask => {
            let quote = simulators[0].quote();
            let base = simulators[0].base();
            (quote.address, quote.decimals, base.decimals)
        }
        Side::Bid => {
            let base = simulators[0].base();
            let quote = simulators[0].quote();
            (base.address, base.decimals, quote.decimals)
        }
    };

    let mut points = Vec::new();
    let mut cum_base = 0.0;
    let mut cum_quote = 0.0;

    for seg_in in schedule.segments() {
        let amount_in = amount_to_u256(seg_in, decimals_in);
        if amount_in.is_zero() {
            continue;
        }

        let mut best_idx = None;
        let mut best_out = 0.0;
        for (idx, sim) in simulators.iter().enumerate() {
            let out = sim.quote_exact_in(token_in, amount_in).await?;
            let out_f64 = u256_to_f64(out, decimals_out);
            if out_f64 > best_out {
                best_out = out_f64;
                best_idx = Some(idx);
            }
        }

        let Some(best_idx) = best_idx else { continue };
        if best_out <= 0.0 {
            continue;
        }

        let out = simulators[best_idx]
            .swap_exact_in(token_in, amount_in)
            .await?;
        let out_f64 = u256_to_f64(out, decimals_out);
        if out_f64 <= 0.0 {
            continue;
        }

        let (seg_base, seg_quote) = match side {
            Side::Ask => (out_f64, seg_in),
            Side::Bid => (seg_in, out_f64),
        };

        if seg_base <= 0.0 {
            continue;
        }

        cum_base += seg_base;
        cum_quote += seg_quote;
        let marginal = seg_quote / seg_base;
        let avg = if cum_base > 0.0 {
            cum_quote / cum_base
        } else {
            marginal
        };

        points.push(DepthPoint {
            price: marginal,
            cum_base,
            seg_base,
            seg_quote,
            marginal_price: marginal,
            avg_price: avg,
            liquidity_tag: simulators[best_idx].liquidity_tag(),
        });
    }

    Ok(DepthCurve {
        venue: Venue::Unified,
        pool: label.to_string(),
        side,
        points,
    })
}

async fn refine_samples(
    engine: &dyn QuoteEngine,
    ctx: &QuoteContext,
    token_in: alloy_primitives::Address,
    token_out: alloy_primitives::Address,
    decimals_in: u8,
    decimals_out: u8,
    mut samples: Vec<(f64, f64)>,
    max_depth: usize,
    tol_bps: f64,
) -> anyhow::Result<Vec<(f64, f64)>> {
    use ordered_float::OrderedFloat;
    use std::collections::BTreeMap;

    let mut map: BTreeMap<OrderedFloat<f64>, f64> = BTreeMap::new();
    for (a, b) in samples.drain(..) {
        map.insert(OrderedFloat(a), b);
    }

    let mut changed = true;
    let mut depth = 0usize;
    while changed && depth < max_depth {
        changed = false;
        depth += 1;
        let keys: Vec<f64> = map.keys().map(|k| k.0).collect();
        let mut inserts = Vec::new();
        for window in keys.windows(2) {
            let a = window[0];
            let b = window[1];
            let out_a = *map.get(&OrderedFloat(a)).unwrap_or(&0.0);
            let out_b = *map.get(&OrderedFloat(b)).unwrap_or(&0.0);
            let mid_in = (a + b) * 0.5;
            if map.contains_key(&OrderedFloat(mid_in)) {
                continue;
            }
            let amount_in = amount_to_u256(mid_in, decimals_in);
            if amount_in.is_zero() {
                continue;
            }
            let quote = engine
                .quote_exact_in(
                    QuoteExactInRequest {
                        token_in,
                        token_out,
                        amount_in,
                    },
                    ctx,
                )
                .await?;
            let out_mid = u256_to_f64(quote.amount_out, decimals_out);
            let linear = out_a + (out_b - out_a) * (mid_in - a) / (b - a);
            let err_bps = if linear.abs() > 0.0 {
                ((out_mid - linear) / linear) * 10_000.0
            } else {
                0.0
            };
            if err_bps.abs() > tol_bps {
                inserts.push((OrderedFloat(mid_in), out_mid));
                changed = true;
            }
        }

        for (k, v) in inserts {
            map.insert(k, v);
        }
    }

    Ok(map.into_iter().map(|(k, v)| (k.0, v)).collect())
}
