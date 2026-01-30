use alloy_primitives::{Address, U256};
use base_depth_core::Venue;
use base_depth_core::{StatefulSimulator, TokenInfo};
use base_depth_evm::v2::V2PoolSimulator;

fn token(address_byte: u8, symbol: &str) -> TokenInfo {
    TokenInfo {
        address: Address::from([address_byte; 20]),
        symbol: symbol.to_string(),
        decimals: 18,
    }
}

#[tokio::test]
async fn v2_constant_product_swap_matches_formula() {
    let base = token(1, "BASE");
    let quote = token(2, "QUOTE");

    let mut sim = V2PoolSimulator::new(
        Venue::PancakeV2,
        Address::from([9u8; 20]),
        base.address,
        quote.address,
        1_000_000,
        1_000_000,
        25,
        base.clone(),
        quote.clone(),
    );

    let amount_in = U256::from(10_000u64);
    let amount_out = sim.swap_exact_in(base.address, amount_in).await.unwrap();

    let fee_numerator = 10_000u128 - 25u128;
    let amount_in_with_fee = 10_000u128 * fee_numerator / 10_000u128;
    let numerator = amount_in_with_fee * 1_000_000u128;
    let denominator = 1_000_000u128 + amount_in_with_fee;
    let expected = numerator / denominator;

    assert_eq!(amount_out, U256::from(expected));
}
