use alloy_primitives::{Address, U256};
use base_depth_core::curve::build_unified_curve;
use base_depth_core::schedule::{Spacing, StepSchedule};
use base_depth_core::types::{Side, StatefulSimulator, TokenInfo, Venue};
use base_depth_core::utils::{amount_to_u256, u256_to_f64};

#[derive(Clone)]
struct DummySim {
    price: f64,
    base: TokenInfo,
    quote: TokenInfo,
    venue: Venue,
    label: String,
}

#[async_trait::async_trait]
impl StatefulSimulator for DummySim {
    fn clone_box(&self) -> Box<dyn StatefulSimulator> {
        Box::new(self.clone())
    }

    fn venue(&self) -> Venue {
        self.venue
    }

    fn pool_label(&self) -> &str {
        &self.label
    }

    fn base(&self) -> &TokenInfo {
        &self.base
    }

    fn quote(&self) -> &TokenInfo {
        &self.quote
    }

    fn mid_price(&self) -> anyhow::Result<f64> {
        Ok(self.price)
    }

    async fn swap_exact_in(&mut self, token_in: Address, amount_in: U256) -> anyhow::Result<U256> {
        let (dec_in, dec_out, out_is_base) = if token_in == self.quote.address {
            (self.quote.decimals, self.base.decimals, true)
        } else {
            (self.base.decimals, self.quote.decimals, false)
        };
        let in_f64 = u256_to_f64(amount_in, dec_in);
        let out_f64 = if out_is_base {
            in_f64 / self.price
        } else {
            in_f64 * self.price
        };
        Ok(amount_to_u256(out_f64, dec_out))
    }
}

fn make_token(address_byte: u8, symbol: &str) -> TokenInfo {
    TokenInfo {
        address: Address::from([address_byte; 20]),
        symbol: symbol.to_string(),
        decimals: 18,
    }
}

#[tokio::test]
async fn unified_selects_best_ask() {
    let base = make_token(1, "BASE");
    let quote = make_token(2, "QUOTE");

    let sim_low = DummySim {
        price: 2000.0,
        base: base.clone(),
        quote: quote.clone(),
        venue: Venue::UniswapV3,
        label: "low".to_string(),
    };
    let sim_high = DummySim {
        price: 2100.0,
        base: base.clone(),
        quote: quote.clone(),
        venue: Venue::PancakeV3,
        label: "high".to_string(),
    };

    let schedule = StepSchedule {
        min: 1000.0,
        max: 1000.0,
        steps: 1,
        spacing: Spacing::Linear,
    };

    let curve = build_unified_curve(
        vec![Box::new(sim_low), Box::new(sim_high)],
        &schedule,
        Side::Ask,
        "UNIFIED",
    )
    .await
    .unwrap();
    let price = curve.points[0].marginal_price;
    assert!((price - 2000.0).abs() < 1.0);
}

#[tokio::test]
async fn unified_selects_best_bid() {
    let base = make_token(1, "BASE");
    let quote = make_token(2, "QUOTE");

    let sim_low = DummySim {
        price: 2000.0,
        base: base.clone(),
        quote: quote.clone(),
        venue: Venue::UniswapV3,
        label: "low".to_string(),
    };
    let sim_high = DummySim {
        price: 2100.0,
        base: base.clone(),
        quote: quote.clone(),
        venue: Venue::PancakeV3,
        label: "high".to_string(),
    };

    let schedule = StepSchedule {
        min: 1.0,
        max: 1.0,
        steps: 1,
        spacing: Spacing::Linear,
    };

    let curve = build_unified_curve(
        vec![Box::new(sim_low), Box::new(sim_high)],
        &schedule,
        Side::Bid,
        "UNIFIED",
    )
    .await
    .unwrap();
    let price = curve.points[0].marginal_price;
    assert!((price - 2100.0).abs() < 1.0);
}
