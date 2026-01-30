use crate::curve::DepthCurve;
use crate::types::{PairInfo, Side, Venue};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SnapshotMeta {
    pub chain_id: u64,
    pub block_number: u64,
    pub block_timestamp: i64,
    pub snapshot_time: DateTime<Utc>,
    pub pair: PairInfo,
    pub venues: Vec<VenueInfo>,
    pub config: SnapshotConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VenueInfo {
    pub venue: Venue,
    pub pool: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SnapshotConfig {
    pub min_base: f64,
    pub max_base: f64,
    pub steps: usize,
    pub spacing: String,
    pub kyber_flags: Option<KyberFlags>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KyberFlags {
    pub exclude_rfq: bool,
    pub only_direct: bool,
    pub gas_include: bool,
}

pub struct SnapshotWriter {
    out_dir: PathBuf,
}

impl SnapshotWriter {
    pub fn new(out_dir: impl AsRef<Path>) -> Self {
        Self {
            out_dir: out_dir.as_ref().to_path_buf(),
        }
    }

    pub fn write(&self, meta: &SnapshotMeta, curves: &[DepthCurve]) -> anyhow::Result<()> {
        fs::create_dir_all(&self.out_dir)?;
        let meta_path = self.out_dir.join("snapshot_meta.json");
        let mut meta_file = File::create(meta_path)?;
        let meta_json = serde_json::to_string_pretty(meta)?;
        meta_file.write_all(meta_json.as_bytes())?;

        let csv_path = self.out_dir.join("depth_points.csv");
        let mut writer = csv::Writer::from_path(csv_path)?;
        writer.write_record([
            "ts",
            "block_number",
            "pair",
            "venue",
            "pool",
            "side",
            "price",
            "cum_base",
            "seg_base",
            "seg_quote",
            "marginal_price",
            "avg_price",
            "liquidity_tag",
        ])?;

        let pair_name = format!("{}/{}", meta.pair.base.symbol, meta.pair.quote.symbol);
        for curve in curves {
            for point in &curve.points {
                writer.write_record([
                    meta.snapshot_time.to_rfc3339(),
                    meta.block_number.to_string(),
                    pair_name.clone(),
                    curve.venue.as_str().to_string(),
                    curve.pool.clone(),
                    match curve.side {
                        Side::Bid => "bid".to_string(),
                        Side::Ask => "ask".to_string(),
                    },
                    point.price.to_string(),
                    point.cum_base.to_string(),
                    point.seg_base.to_string(),
                    point.seg_quote.to_string(),
                    point.marginal_price.to_string(),
                    point.avg_price.to_string(),
                    point.liquidity_tag.clone().unwrap_or_default(),
                ])?;
            }
        }

        writer.flush()?;
        Ok(())
    }
}
