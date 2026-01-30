use crate::contracts::{IERC20Metadata, IERC20SymbolBytes};
use crate::provider::{block_id, RpcProvider};
use alloy_primitives::{Address, FixedBytes};
use base_depth_core::TokenInfo;
use std::sync::Arc;

pub async fn fetch_token_info(
    provider: Arc<RpcProvider>,
    address: Address,
    block: Option<u64>,
) -> anyhow::Result<TokenInfo> {
    let (symbol, decimals) = fetch_symbol_decimals(provider, address, block).await?;
    Ok(TokenInfo {
        address,
        symbol,
        decimals,
    })
}

async fn fetch_symbol_decimals(
    provider: Arc<RpcProvider>,
    address: Address,
    block: Option<u64>,
) -> anyhow::Result<(String, u8)> {
    let symbol = match block {
        Some(block_number) => {
            IERC20Metadata::new(address, provider.clone())
                .symbol()
                .block(block_id(block_number))
                .call()
                .await
        }
        None => {
            IERC20Metadata::new(address, provider.clone())
                .symbol()
                .call()
                .await
        }
    };

    let symbol = match symbol {
        Ok(sym) => sym,
        Err(_) => {
            let bytes = match block {
                Some(block_number) => {
                    IERC20SymbolBytes::new(address, provider.clone())
                        .symbol()
                        .block(block_id(block_number))
                        .call()
                        .await?
                }
                None => {
                    IERC20SymbolBytes::new(address, provider.clone())
                        .symbol()
                        .call()
                        .await?
                }
            };
            bytes32_to_string(bytes)
        }
    };

    let decimals = match block {
        Some(block_number) => {
            IERC20Metadata::new(address, provider.clone())
                .decimals()
                .block(block_id(block_number))
                .call()
                .await?
        }
        None => {
            IERC20Metadata::new(address, provider.clone())
                .decimals()
                .call()
                .await?
        }
    };

    Ok((symbol, decimals))
}

fn bytes32_to_string(value: FixedBytes<32>) -> String {
    let trimmed: Vec<u8> = value.0.iter().copied().take_while(|b| *b != 0).collect();
    String::from_utf8_lossy(&trimmed).to_string()
}
