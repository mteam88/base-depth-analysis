use alloy_primitives::U256;

pub fn scale_from_decimals(decimals: u8) -> f64 {
    10f64.powi(decimals as i32)
}

pub fn amount_to_u256(amount: f64, decimals: u8) -> U256 {
    if amount <= 0.0 {
        return U256::ZERO;
    }
    let scale = scale_from_decimals(decimals);
    let scaled = (amount * scale).round();
    if scaled <= 0.0 {
        U256::from(1u128)
    } else {
        U256::from(scaled as u128)
    }
}

pub fn u256_to_f64(amount: U256, decimals: u8) -> f64 {
    if amount.is_zero() {
        return 0.0;
    }
    let scale = scale_from_decimals(decimals);
    let value: u128 = amount.try_into().unwrap_or(u128::MAX);
    (value as f64) / scale
}
