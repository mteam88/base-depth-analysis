use alloy_primitives::Address;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use base_depth_core::curve::{
    build_curve_from_quote_engine, build_curve_from_simulator, build_unified_curve, DepthCurve,
};
use base_depth_core::schedule::{Spacing, StepSchedule};
use base_depth_core::snapshot::{
    KyberFlags, SnapshotConfig, SnapshotMeta, SnapshotWriter, VenueInfo,
};
use base_depth_core::types::{
    PairInfo, QuoteContext, QuoteEngine, QuoteExactInRequest, Side, Venue,
};
use base_depth_core::utils::{amount_to_u256, u256_to_f64};
use base_depth_evm::discovery::{
    discover_simulators, discover_v3_pools, V3PoolEntry, DEFAULT_V3_FEES,
};
use base_depth_evm::{
    Addresses, KyberConfig, KyberQuoteEngine, RpcContext, TesseraConfig, TesseraQuoteEngine,
    V3QuoterEngine,
};
use clap::{Args, Parser, Subcommand, ValueEnum};
use dotenvy::dotenv;
use serde::Serialize;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tokio::time::{sleep, Duration};
use tower_http::services::ServeDir;
use tracing::{info, warn};

#[derive(Parser)]
#[command(
    name = "base-depth",
    version,
    about = "Depth curves for Base liquidity sources"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Snapshot(SnapshotArgs),
    Serve(ServeArgs),
}

#[derive(Args, Clone)]
struct CommonArgs {
    #[arg(long)]
    rpc_url: Option<String>,
    #[arg(long)]
    base: Option<String>,
    #[arg(long)]
    quote: Option<String>,
    #[arg(long, value_delimiter = ',')]
    venues: Vec<String>,
    #[arg(long, default_value_t = 100.0)]
    max_base: f64,
    #[arg(long, default_value_t = 0.01)]
    min_base: f64,
    #[arg(long, default_value_t = 64)]
    steps: usize,
    #[arg(long)]
    kyber_steps: Option<usize>,
    #[arg(long, value_enum, default_value_t = SpacingOpt::Geometric)]
    spacing: SpacingOpt,
    #[arg(long, value_enum, default_value_t = SideOpt::Both)]
    side: SideOpt,
    #[arg(long)]
    block: Option<u64>,
    #[arg(long, value_delimiter = ',')]
    v3_fees: Vec<u32>,
    #[arg(long, default_value_t = false)]
    v3_quoter: bool,
    #[arg(long, default_value_t = 8)]
    v3_quoter_concurrency: usize,
    #[arg(long)]
    mid_price: Option<f64>,
    #[arg(long)]
    kyber_client_id: Option<String>,
    #[arg(long)]
    kyber_base_url: Option<String>,
    #[arg(long, default_value_t = true)]
    kyber_exclude_rfq: bool,
    #[arg(long, default_value_t = false)]
    kyber_only_direct: bool,
    #[arg(long, default_value_t = false)]
    kyber_gas_include: bool,
    #[arg(long, value_delimiter = ',')]
    kyber_included_sources: Vec<String>,
    #[arg(long, value_delimiter = ',')]
    kyber_excluded_sources: Vec<String>,
    #[arg(long, default_value_t = 0)]
    kyber_adaptive_depth: usize,
    #[arg(long, default_value_t = 30.0)]
    kyber_adaptive_bps: f64,
    #[arg(long, default_value_t = 8)]
    kyber_concurrency: usize,
    #[arg(long)]
    tessera_swap: Option<String>,
    #[arg(long)]
    tessera_engine: Option<String>,
    #[arg(long)]
    tessera_treasury: Option<String>,
    #[arg(long)]
    tessera_sender: Option<String>,
}

#[derive(Args, Clone)]
struct SnapshotArgs {
    #[command(flatten)]
    common: CommonArgs,
    #[arg(long)]
    out_dir: Option<PathBuf>,
}

#[derive(Args, Clone)]
struct ServeArgs {
    #[command(flatten)]
    common: CommonArgs,
    #[arg(long, default_value = "0.0.0.0:8080")]
    listen: String,
    #[arg(long, default_value_t = 10)]
    interval: u64,
    #[arg(long, default_value = "web")]
    web_dir: PathBuf,
}

#[derive(ValueEnum, Copy, Clone)]
enum SpacingOpt {
    Linear,
    Geometric,
}

#[derive(ValueEnum, Copy, Clone)]
enum SideOpt {
    Bid,
    Ask,
    Both,
}

impl From<SpacingOpt> for Spacing {
    fn from(value: SpacingOpt) -> Self {
        match value {
            SpacingOpt::Linear => Spacing::Linear,
            SpacingOpt::Geometric => Spacing::Geometric,
        }
    }
}

#[derive(Clone, Serialize)]
struct SnapshotPayload {
    meta: SnapshotMeta,
    curves: Vec<DepthCurve>,
}

#[derive(Clone)]
struct AppState {
    latest: Arc<RwLock<Option<SnapshotPayload>>>,
    tx: broadcast::Sender<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    match cli.command {
        Commands::Snapshot(args) => snapshot_command(args).await?,
        Commands::Serve(args) => serve_command(args).await?,
    }

    Ok(())
}

async fn snapshot_command(args: SnapshotArgs) -> anyhow::Result<()> {
    let payload = build_snapshot(&args.common).await?;
    let out_dir = args.out_dir.unwrap_or_else(|| {
        let ts = payload
            .meta
            .snapshot_time
            .format("%Y%m%d_%H%M%S")
            .to_string();
        PathBuf::from("snapshots").join(ts)
    });
    let writer = SnapshotWriter::new(&out_dir);
    writer.write(&payload.meta, &payload.curves)?;
    info!("snapshot written to {}", out_dir.display());
    Ok(())
}

async fn serve_command(args: ServeArgs) -> anyhow::Result<()> {
    let listen: SocketAddr = args.listen.parse()?;
    let web_dir = args.web_dir.clone();

    let (tx, _) = broadcast::channel(16);
    let state = AppState {
        latest: Arc::new(RwLock::new(None)),
        tx,
    };

    let state_clone = state.clone();
    let common = args.common.clone();
    tokio::spawn(async move {
        let mut last_block: Option<u64> = None;
        loop {
            match build_snapshot(&common).await {
                Ok(payload) => {
                    if Some(payload.meta.block_number) != last_block {
                        let json = serde_json::to_string(&payload).unwrap_or_default();
                        *state_clone.latest.write().await = Some(payload.clone());
                        let _ = state_clone.tx.send(json);
                        last_block = Some(payload.meta.block_number);
                    }
                }
                Err(err) => {
                    warn!("snapshot build failed: {err:#}");
                }
            }
            sleep(Duration::from_secs(args.interval)).await;
        }
    });

    let app = Router::new()
        .route("/api/snapshots/latest", get(get_latest))
        .route("/ws", get(ws_handler))
        .fallback_service(ServeDir::new(web_dir).append_index_html_on_directories(true))
        .with_state(state);

    info!("serving on {}", listen);
    axum::serve(tokio::net::TcpListener::bind(listen).await?, app).await?;
    Ok(())
}

async fn get_latest(State(state): State<AppState>) -> impl IntoResponse {
    let guard = state.latest.read().await;
    if let Some(payload) = guard.as_ref() {
        (StatusCode::OK, Json(payload.clone())).into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| ws_session(socket, state))
}

async fn ws_session(mut socket: WebSocket, state: AppState) {
    if let Some(payload) = state.latest.read().await.clone() {
        if let Ok(json) = serde_json::to_string(&payload) {
            let _ = socket.send(Message::Text(json)).await;
        }
    }
    let mut rx = state.tx.subscribe();
    loop {
        match rx.recv().await {
            Ok(msg) => {
                if socket.send(Message::Text(msg)).await.is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

async fn build_snapshot(common: &CommonArgs) -> anyhow::Result<SnapshotPayload> {
    let rpc_url = resolve_rpc_url(common)?;
    let ctx = RpcContext::new(&rpc_url, common.block).await?;
    let block_number = ctx.block_number().await?;
    let block_timestamp = ctx.block_timestamp(block_number).await?;

    let base_address = resolve_address(common.base.clone(), "WETH_ADDRESS")?;
    let quote_address = resolve_address(common.quote.clone(), "USDC_ADDRESS")?;

    let base = base_depth_evm::token::fetch_token_info(
        ctx.provider.clone(),
        base_address,
        Some(block_number),
    )
    .await?;
    let quote = base_depth_evm::token::fetch_token_info(
        ctx.provider.clone(),
        quote_address,
        Some(block_number),
    )
    .await?;

    let pair = PairInfo {
        base: base.clone(),
        quote: quote.clone(),
    };

    let venues = resolve_venues(&common.venues)?;
    let v3_fees = if common.v3_fees.is_empty() {
        DEFAULT_V3_FEES.to_vec()
    } else {
        common.v3_fees.clone()
    };

    let block = Some(block_number);
    let use_v3_quoter = common.v3_quoter;
    let mut discovery_venues: Vec<Venue> = venues
        .iter()
        .copied()
        .filter(|v| {
            matches!(
                v,
                Venue::UniswapV3
                    | Venue::PancakeV3
                    | Venue::Slipstream
                    | Venue::PancakeV2
                    | Venue::AerodromeV2
            )
        })
        .collect();
    if use_v3_quoter {
        discovery_venues.retain(|v| !matches!(v, Venue::UniswapV3 | Venue::PancakeV3));
    }

    let addresses = Addresses::base();
    let allowed_pools = allowed_pool_set();
    let mut sim_entries = discover_simulators(
        &ctx,
        &addresses,
        &base,
        &quote,
        &discovery_venues,
        &v3_fees,
        block,
    )
    .await?;
    sim_entries.retain(|entry| allowed_pools.contains(&entry.pool));
    if sim_entries.is_empty() {
        warn!("no allowed DEX pools discovered; only Kyber/Tessera curves will render");
    }

    let mut v3_quoter_entries: Vec<V3PoolEntry> = Vec::new();
    if use_v3_quoter {
        if venues.contains(&Venue::UniswapV3) {
            v3_quoter_entries.extend(
                discover_v3_pools(
                    ctx.provider.clone(),
                    addresses.uniswap_v3_factory,
                    &base,
                    &quote,
                    Venue::UniswapV3,
                    &v3_fees,
                    block,
                )
                .await?,
            );
        }
        if venues.contains(&Venue::PancakeV3) {
            v3_quoter_entries.extend(
                discover_v3_pools(
                    ctx.provider.clone(),
                    addresses.pancake_v3_factory,
                    &base,
                    &quote,
                    Venue::PancakeV3,
                    &v3_fees,
                    block,
                )
                .await?,
            );
        }
    }
    v3_quoter_entries.retain(|entry| allowed_pools.contains(&entry.pool));

    let mid_price = if let Some(value) = common.mid_price {
        value
    } else if let Some(entry) = sim_entries.first() {
        let price = entry.simulator.mid_price()?;
        if price > 0.0 {
            price
        } else {
            1.0
        }
    } else if let Some(entry) = v3_quoter_entries.first() {
        let quoter = match entry.venue {
            Venue::UniswapV3 => addresses.uniswap_v3_quoter_v2,
            Venue::PancakeV3 => addresses.pancake_v3_quoter_v2,
            _ => Address::ZERO,
        };
        if quoter.is_zero() {
            1.0
        } else {
            let engine = V3QuoterEngine::new(ctx.provider.clone(), quoter, entry.fee, entry.venue);
            let ctx_quote = QuoteContext {
                chain_id: ctx.chain_id,
                block_number: Some(block_number),
            };
            let probe = 0.01_f64;
            let amount_in = amount_to_u256(probe, base.decimals);
            match engine
                .quote_exact_in(
                    QuoteExactInRequest {
                        token_in: base.address,
                        token_out: quote.address,
                        amount_in,
                    },
                    &ctx_quote,
                )
                .await
            {
                Ok(quote_result) => {
                    let out_f64 = u256_to_f64(quote_result.amount_out, quote.decimals);
                    if out_f64 > 0.0 {
                        out_f64 / probe
                    } else {
                        1.0
                    }
                }
                Err(err) => {
                    warn!("failed to derive mid price from v3 quoter: {err:#}");
                    1.0
                }
            }
        }
    } else {
        1.0
    };
    let spacing: Spacing = common.spacing.into();
    let mut min_base = common.min_base;
    let mut max_base = common.max_base;
    if min_base < 0.01 {
        warn!("min_base {} below 0.01, clamping to 0.01", min_base);
        min_base = 0.01;
    }
    if max_base > 100.0 {
        warn!("max_base {} above 100, clamping to 100", max_base);
        max_base = 100.0;
    }
    if max_base < min_base {
        return Err(anyhow::anyhow!(
            "max_base ({max_base}) must be >= min_base ({min_base})"
        ));
    }

    let schedule_bid = build_schedule(
        min_base,
        max_base,
        common.steps,
        spacing,
        Side::Bid,
        mid_price,
    );
    let schedule_ask = build_schedule(
        min_base,
        max_base,
        common.steps,
        spacing,
        Side::Ask,
        mid_price,
    );
    let kyber_steps = resolve_kyber_steps(common.steps, common.kyber_steps);
    let schedule_kyber_bid = build_schedule(
        min_base,
        max_base,
        kyber_steps,
        spacing,
        Side::Bid,
        mid_price,
    );
    let schedule_kyber_ask = build_schedule(
        min_base,
        max_base,
        kyber_steps,
        spacing,
        Side::Ask,
        mid_price,
    );

    let sides = match common.side {
        SideOpt::Bid => vec![Side::Bid],
        SideOpt::Ask => vec![Side::Ask],
        SideOpt::Both => vec![Side::Bid, Side::Ask],
    };

    let mut curves = Vec::new();
    let mut venue_info = Vec::new();

    for entry in &sim_entries {
        venue_info.push(VenueInfo {
            venue: entry.venue,
            pool: entry.label.clone(),
        });
        for side in &sides {
            let schedule = if *side == Side::Bid {
                &schedule_bid
            } else {
                &schedule_ask
            };
            let curve =
                build_curve_from_simulator(entry.simulator.as_ref(), schedule, *side).await?;
            curves.push(curve);
        }
    }

    if use_v3_quoter && !v3_quoter_entries.is_empty() {
        let ctx_quote = QuoteContext {
            chain_id: ctx.chain_id,
            block_number: Some(block_number),
        };
        for entry in &v3_quoter_entries {
            let quoter = match entry.venue {
                Venue::UniswapV3 => addresses.uniswap_v3_quoter_v2,
                Venue::PancakeV3 => addresses.pancake_v3_quoter_v2,
                _ => continue,
            };
            let engine = V3QuoterEngine::new(ctx.provider.clone(), quoter, entry.fee, entry.venue);
            let mut ok = true;
            for side in &sides {
                let schedule = if *side == Side::Bid {
                    &schedule_bid
                } else {
                    &schedule_ask
                };
                let (token_in, token_out, dec_in, dec_out) = if *side == Side::Ask {
                    (quote.address, base.address, quote.decimals, base.decimals)
                } else {
                    (base.address, quote.address, base.decimals, quote.decimals)
                };
                match build_curve_from_quote_engine(
                    &engine,
                    &ctx_quote,
                    entry.venue,
                    entry.label.as_str(),
                    schedule,
                    *side,
                    (token_in, dec_in),
                    (token_out, dec_out),
                    common.v3_quoter_concurrency,
                    None,
                )
                .await
                {
                    Ok(curve) => curves.push(curve),
                    Err(err) => {
                        warn!(
                            "v3 quoter curve build failed ({:?} {:?}): {err:#}",
                            entry.venue, side
                        );
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                venue_info.push(VenueInfo {
                    venue: entry.venue,
                    pool: entry.label.clone(),
                });
            }
        }
    }

    if venues.contains(&Venue::Unified) && !sim_entries.is_empty() {
        let sims: Vec<_> = sim_entries
            .iter()
            .map(|entry| entry.simulator.clone())
            .collect();
        for side in &sides {
            let schedule = if *side == Side::Bid {
                &schedule_bid
            } else {
                &schedule_ask
            };
            let curve = build_unified_curve(sims.clone(), schedule, *side, "UNIFIED").await?;
            curves.push(curve);
        }
        venue_info.push(VenueInfo {
            venue: Venue::Unified,
            pool: "UNIFIED".to_string(),
        });
    }

    let kyber_cfg = build_kyber_config(common);
    let kyber_flags = KyberFlags {
        exclude_rfq: kyber_cfg.exclude_rfq,
        only_direct: kyber_cfg.only_direct,
        gas_include: kyber_cfg.gas_include,
    };

    if venues.contains(&Venue::Kyber) || venues.contains(&Venue::UnifiedKyber) {
        let engine = KyberQuoteEngine::new(kyber_cfg)?;
        let ctx_quote = QuoteContext {
            chain_id: ctx.chain_id,
            block_number: Some(block_number),
        };
        let adaptive = if common.kyber_adaptive_depth > 0 {
            Some((common.kyber_adaptive_depth, common.kyber_adaptive_bps))
        } else {
            None
        };

        if venues.contains(&Venue::Kyber) {
            let mut kyber_ok = true;
            for side in &sides {
                let schedule = if *side == Side::Bid {
                    &schedule_kyber_bid
                } else {
                    &schedule_kyber_ask
                };
                let (token_in, token_out, dec_in, dec_out) = if *side == Side::Ask {
                    (quote.address, base.address, quote.decimals, base.decimals)
                } else {
                    (base.address, quote.address, base.decimals, quote.decimals)
                };
                match build_curve_from_quote_engine(
                    &engine,
                    &ctx_quote,
                    Venue::Kyber,
                    "KYBER",
                    schedule,
                    *side,
                    (token_in, dec_in),
                    (token_out, dec_out),
                    common.kyber_concurrency,
                    adaptive,
                )
                .await
                {
                    Ok(curve) => curves.push(curve),
                    Err(err) => {
                        warn!("kyber curve build failed ({side:?}): {err:#}");
                        kyber_ok = false;
                        break;
                    }
                }
            }
            if kyber_ok {
                venue_info.push(VenueInfo {
                    venue: Venue::Kyber,
                    pool: "KYBER".to_string(),
                });
            }
        }

        if venues.contains(&Venue::UnifiedKyber) {
            let mut kyber_unified_ok = true;
            for side in &sides {
                let schedule = if *side == Side::Bid {
                    &schedule_kyber_bid
                } else {
                    &schedule_kyber_ask
                };
                let (token_in, token_out, dec_in, dec_out) = if *side == Side::Ask {
                    (quote.address, base.address, quote.decimals, base.decimals)
                } else {
                    (base.address, quote.address, base.decimals, quote.decimals)
                };
                match build_curve_from_quote_engine(
                    &engine,
                    &ctx_quote,
                    Venue::UnifiedKyber,
                    "KYBER_UNIFIED",
                    schedule,
                    *side,
                    (token_in, dec_in),
                    (token_out, dec_out),
                    common.kyber_concurrency,
                    adaptive,
                )
                .await
                {
                    Ok(curve) => curves.push(curve),
                    Err(err) => {
                        warn!("kyber unified curve build failed ({side:?}): {err:#}");
                        kyber_unified_ok = false;
                        break;
                    }
                }
            }
            if kyber_unified_ok {
                venue_info.push(VenueInfo {
                    venue: Venue::UnifiedKyber,
                    pool: "KYBER_UNIFIED".to_string(),
                });
            }
        }
    }

    if venues.contains(&Venue::TesseraSwap) {
        let tessera_cfg = resolve_tessera_config(common)?;
        let tessera_engine =
            TesseraQuoteEngine::new(ctx.provider.clone(), tessera_cfg.swap, tessera_cfg.sender);
        let ctx_quote = QuoteContext {
            chain_id: ctx.chain_id,
            block_number: Some(block_number),
        };
        let adaptive = None;
        for side in &sides {
            let schedule = if *side == Side::Bid {
                &schedule_bid
            } else {
                &schedule_ask
            };
            let (token_in, token_out, dec_in, dec_out) = if *side == Side::Ask {
                (quote.address, base.address, quote.decimals, base.decimals)
            } else {
                (base.address, quote.address, base.decimals, quote.decimals)
            };
            let curve = build_curve_from_quote_engine(
                &tessera_engine,
                &ctx_quote,
                Venue::TesseraSwap,
                "TESSERA",
                schedule,
                *side,
                (token_in, dec_in),
                (token_out, dec_out),
                4,
                adaptive,
            )
            .await?;
            curves.push(curve);
        }
        venue_info.push(VenueInfo {
            venue: Venue::TesseraSwap,
            pool: "TESSERA".to_string(),
        });
    }

    let meta = SnapshotMeta {
        chain_id: ctx.chain_id,
        block_number,
        block_timestamp,
        snapshot_time: chrono::Utc::now(),
        pair,
        venues: dedupe_venues(venue_info),
        config: SnapshotConfig {
            min_base,
            max_base,
            steps: common.steps,
            spacing: spacing_string(spacing),
            kyber_flags: if venues.contains(&Venue::Kyber) || venues.contains(&Venue::UnifiedKyber)
            {
                Some(kyber_flags)
            } else {
                None
            },
            kyber_steps: if venues.contains(&Venue::Kyber) || venues.contains(&Venue::UnifiedKyber)
            {
                Some(kyber_steps)
            } else {
                None
            },
        },
    };

    Ok(SnapshotPayload { meta, curves })
}

fn resolve_rpc_url(common: &CommonArgs) -> anyhow::Result<String> {
    if let Some(url) = &common.rpc_url {
        return Ok(url.clone());
    }
    if let Ok(url) = std::env::var("BASE_RPC_URL") {
        return Ok(url);
    }
    Err(anyhow::anyhow!("BASE_RPC_URL missing"))
}

fn resolve_address(
    value: Option<String>,
    env_key: &str,
) -> anyhow::Result<alloy_primitives::Address> {
    let raw = if let Some(value) = value {
        value
    } else {
        std::env::var(env_key)?
    };
    parse_address(&raw)
}

fn resolve_venues(values: &[String]) -> anyhow::Result<Vec<Venue>> {
    if values.is_empty() {
        return Ok(default_venues());
    }
    let mut venues = Vec::new();
    for value in values {
        for item in value.split(',') {
            let venue: Venue = item.parse().map_err(|err: String| anyhow::anyhow!(err))?;
            if !venues.contains(&venue) {
                venues.push(venue);
            }
        }
    }
    Ok(venues)
}

fn default_venues() -> Vec<Venue> {
    vec![
        Venue::UniswapV3,
        Venue::PancakeV3,
        Venue::Slipstream,
        Venue::PancakeV2,
        Venue::AerodromeV2,
        Venue::Kyber,
        Venue::Unified,
        Venue::UnifiedKyber,
    ]
}

fn build_schedule(
    min_base: f64,
    max_base: f64,
    steps: usize,
    spacing: Spacing,
    side: Side,
    mid_price: f64,
) -> StepSchedule {
    let mid = if mid_price <= 0.0 { 1.0 } else { mid_price };
    let (min_in, max_in) = match side {
        Side::Bid => (min_base, max_base),
        Side::Ask => (min_base * mid, max_base * mid),
    };
    StepSchedule {
        min: min_in,
        max: max_in,
        steps,
        spacing,
    }
}

fn spacing_string(spacing: Spacing) -> String {
    match spacing {
        Spacing::Linear => "linear".to_string(),
        Spacing::Geometric => "geometric".to_string(),
    }
}

fn resolve_kyber_steps(common_steps: usize, override_steps: Option<usize>) -> usize {
    if let Some(override_steps) = override_steps {
        return override_steps.max(1).min(common_steps.max(1));
    }
    let reduced = ((common_steps as f64) * 0.5).ceil() as usize;
    let capped = reduced.min(24).max(6);
    capped.min(common_steps.max(1))
}

fn build_kyber_config(common: &CommonArgs) -> KyberConfig {
    let mut cfg = KyberConfig::default();
    if let Some(client_id) = common
        .kyber_client_id
        .clone()
        .or_else(|| std::env::var("KYBER_CLIENT_ID").ok())
    {
        cfg.client_id = client_id;
    }
    if let Some(base_url) = common
        .kyber_base_url
        .clone()
        .or_else(|| std::env::var("KYBER_BASE_URL").ok())
    {
        cfg.base_url = base_url;
    }
    if let Ok(value) = std::env::var("KYBER_EXCLUDE_RFQ") {
        cfg.exclude_rfq = value == "1" || value.eq_ignore_ascii_case("true");
    } else {
        cfg.exclude_rfq = common.kyber_exclude_rfq;
    }
    if let Ok(value) = std::env::var("KYBER_ONLY_DIRECT") {
        cfg.only_direct = value == "1" || value.eq_ignore_ascii_case("true");
    } else {
        cfg.only_direct = common.kyber_only_direct;
    }
    if let Ok(value) = std::env::var("KYBER_GAS_INCLUDE") {
        cfg.gas_include = value == "1" || value.eq_ignore_ascii_case("true");
    } else {
        cfg.gas_include = common.kyber_gas_include;
    }

    let env_included = std::env::var("KYBER_INCLUDED_SOURCES").unwrap_or_default();
    let env_excluded = std::env::var("KYBER_EXCLUDED_SOURCES").unwrap_or_default();

    let mut included = parse_csv_list(&env_included);
    included.extend(common.kyber_included_sources.clone());
    included.sort();
    included.dedup();

    let mut excluded = parse_csv_list(&env_excluded);
    excluded.extend(common.kyber_excluded_sources.clone());
    excluded.sort();
    excluded.dedup();

    cfg.included_sources = included;
    cfg.excluded_sources = excluded;
    cfg
}

fn resolve_tessera_config(common: &CommonArgs) -> anyhow::Result<TesseraConfig> {
    if let Some(swap) = &common.tessera_swap {
        let swap = parse_address(swap)?;
        let treasury = match &common.tessera_treasury {
            Some(value) => Some(parse_address(value)?),
            None => std::env::var("TESSERA_TREASURY")
                .ok()
                .and_then(|v| parse_address(&v).ok()),
        };
        let engine = match &common.tessera_engine {
            Some(value) => Some(parse_address(value)?),
            None => std::env::var("TESSERA_ENGINE")
                .ok()
                .and_then(|v| parse_address(&v).ok()),
        };
        let sender = match &common.tessera_sender {
            Some(value) => Some(parse_address(value)?),
            None => std::env::var("TESSERA_SENDER")
                .ok()
                .and_then(|v| parse_address(&v).ok()),
        };
        return Ok(TesseraConfig {
            swap,
            treasury,
            engine,
            sender,
        });
    }

    if let Some(cfg) = TesseraConfig::from_env() {
        return Ok(cfg);
    }

    let swap = parse_address("0x55555522005BcAE1c2424D474BfD5ed477749E3e")?;
    let engine = parse_address("0x31e99e05fee3dce580af777c3fd63ee1b3b40c17").ok();
    let treasury = parse_address("0x3dbe077e7986657e95e1cc50089f17a5a4af0aae").ok();
    Ok(TesseraConfig {
        swap,
        treasury,
        engine,
        sender: None,
    })
}

fn parse_address(value: &str) -> anyhow::Result<alloy_primitives::Address> {
    Ok(alloy_primitives::Address::parse_checksummed(value, None)
        .unwrap_or_else(|_| value.parse().expect("invalid address")))
}

fn allowed_pool_set() -> HashSet<Address> {
    [
        "0xb2cc224c1c9fee385f8ad6a55b4d94e92359dc59",
        "0xd0b53d9277642d899df5c87a3966a349a798f224",
        "0x72ab388e2e2f6facef59e3c3fa2c4e29011c2d38",
    ]
    .into_iter()
    .map(|addr| parse_address(addr).expect("invalid allowed pool address"))
    .collect()
}

fn parse_csv_list(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

fn dedupe_venues(mut venues: Vec<VenueInfo>) -> Vec<VenueInfo> {
    let mut seen = HashSet::new();
    venues.retain(|info| seen.insert((info.venue, info.pool.clone())));
    venues
}
