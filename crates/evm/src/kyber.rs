use alloy_primitives::U256;
use base_depth_core::{Quote, QuoteContext, QuoteEngine, QuoteExactInRequest, Venue};
use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct KyberConfig {
    pub base_url: String,
    pub client_id: String,
    pub exclude_rfq: bool,
    pub only_direct: bool,
    pub gas_include: bool,
    pub included_sources: Vec<String>,
    pub excluded_sources: Vec<String>,
}

impl Default for KyberConfig {
    fn default() -> Self {
        Self {
            base_url: "https://aggregator-api.kyberswap.com/base/api/v1".to_string(),
            client_id: "base-depth-analysis".to_string(),
            exclude_rfq: true,
            only_direct: false,
            gas_include: false,
            included_sources: Vec::new(),
            excluded_sources: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub struct KyberQuoteEngine {
    cfg: KyberConfig,
    client: reqwest::Client,
}

impl KyberQuoteEngine {
    pub fn new(cfg: KyberConfig) -> anyhow::Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert("accept", HeaderValue::from_static("application/json"));
        headers.insert(
            "x-client-id",
            HeaderValue::from_str(cfg.client_id.as_str())?,
        );

        let client = reqwest::Client::builder()
            .user_agent(concat!("base-depth-analysis/", env!("CARGO_PKG_VERSION")))
            .default_headers(headers)
            .timeout(Duration::from_secs(12))
            .build()?;

        Ok(Self { cfg, client })
    }
}

#[derive(Deserialize)]
struct KyberRouteResponse {
    code: i64,
    message: Option<String>,
    data: Option<KyberRouteData>,
}

#[derive(Deserialize)]
struct KyberRouteData {
    #[serde(rename = "routeSummary")]
    route_summary: KyberRouteSummary,
}

#[derive(Deserialize)]
struct KyberRouteSummary {
    #[serde(rename = "amountIn")]
    amount_in: String,
    #[serde(rename = "amountOut")]
    amount_out: String,
}

#[async_trait::async_trait]
impl QuoteEngine for KyberQuoteEngine {
    async fn quote_exact_in(
        &self,
        request: QuoteExactInRequest,
        _ctx: &QuoteContext,
    ) -> anyhow::Result<Quote> {
        let base = self.cfg.base_url.trim_end_matches('/');
        let mut url = format!(
            "{base}/routes?tokenIn={}&tokenOut={}&amountIn={}",
            request.token_in, request.token_out, request.amount_in
        );
        url.push_str(&format!("&excludeRFQSources={}", self.cfg.exclude_rfq));
        url.push_str(&format!("&onlyDirectPools={}", self.cfg.only_direct));
        url.push_str(&format!("&gasInclude={}", self.cfg.gas_include));
        if !self.cfg.included_sources.is_empty() {
            url.push_str("&includedSources=");
            url.push_str(&self.cfg.included_sources.join(","));
        }
        if !self.cfg.excluded_sources.is_empty() {
            url.push_str("&excludedSources=");
            url.push_str(&self.cfg.excluded_sources.join(","));
        }

        let resp = self.client.get(url).send().await?.error_for_status()?;
        let body: KyberRouteResponse = resp.json().await?;
        if body.code != 0 {
            return Err(anyhow::anyhow!(
                "kyber route error: {}",
                body.message.unwrap_or_else(|| "unknown".to_string())
            ));
        }
        let data = body
            .data
            .ok_or_else(|| anyhow::anyhow!("kyber route missing data"))?;
        let summary = data.route_summary;
        let amount_in = U256::from_str_radix(summary.amount_in.as_str(), 10)?;
        let amount_out = U256::from_str_radix(summary.amount_out.as_str(), 10)?;

        Ok(Quote {
            amount_in,
            amount_out,
            venue: Venue::Kyber,
        })
    }
}
