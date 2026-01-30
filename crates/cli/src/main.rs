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
use base_depth_core::types::{PairInfo, QuoteContext, Side, Venue};
use base_depth_evm::discovery::{discover_simulators, SimulatorEntry, DEFAULT_V3_FEES};
use base_depth_evm::{Addresses, KyberConfig, KyberQuoteEngine, RpcContext};
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
    #[arg(long)]
    min_base: Option<f64>,
    #[arg(long, default_value_t = 64)]
    steps: usize,
    #[arg(long, value_enum, default_value_t = SpacingOpt::Geometric)]
    spacing: SpacingOpt,
    #[arg(long, value_enum, default_value_t = SideOpt::Both)]
    side: SideOpt,
    #[arg(long)]
    block: Option<u64>,
    #[arg(long, value_delimiter = ',')]
    v3_fees: Vec<u32>,
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
    if venues.contains(&Venue::TesseraSwap) {
        warn!("tessera venue selected but not implemented yet");
    }
    let v3_fees = if common.v3_fees.is_empty() {
        DEFAULT_V3_FEES.to_vec()
    } else {
        common.v3_fees.clone()
    };

    let block = Some(block_number);
    let discovery_venues: Vec<Venue> = venues
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

    let addresses = Addresses::base();
    let sim_entries = discover_simulators(
        &ctx,
        &addresses,
        &base,
        &quote,
        &discovery_venues,
        &v3_fees,
        block,
    )
    .await?;

    let mid_price = resolve_mid_price(common.mid_price, &sim_entries)?;
    let spacing: Spacing = common.spacing.into();
    let min_base = common
        .min_base
        .unwrap_or_else(|| (common.max_base / 1000.0).max(1e-6));

    let schedule_bid = build_schedule(
        min_base,
        common.max_base,
        common.steps,
        spacing,
        Side::Bid,
        mid_price,
    );
    let schedule_ask = build_schedule(
        min_base,
        common.max_base,
        common.steps,
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

    let meta = SnapshotMeta {
        chain_id: ctx.chain_id,
        block_number,
        block_timestamp,
        snapshot_time: chrono::Utc::now(),
        pair,
        venues: dedupe_venues(venue_info),
        config: SnapshotConfig {
            min_base,
            max_base: common.max_base,
            steps: common.steps,
            spacing: spacing_string(spacing),
            kyber_flags: if venues.contains(&Venue::Kyber) || venues.contains(&Venue::UnifiedKyber)
            {
                Some(kyber_flags)
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
    Ok(alloy_primitives::Address::parse_checksummed(&raw, None)
        .unwrap_or_else(|_| raw.parse().expect("invalid address")))
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

fn resolve_mid_price(
    candidate: Option<f64>,
    sim_entries: &[SimulatorEntry],
) -> anyhow::Result<f64> {
    if let Some(value) = candidate {
        return Ok(value);
    }
    if let Some(entry) = sim_entries.first() {
        let price = entry.simulator.mid_price()?;
        if price > 0.0 {
            return Ok(price);
        }
    }
    Ok(1.0)
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
