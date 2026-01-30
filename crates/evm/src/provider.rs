use alloy::eips::{BlockId, BlockNumberOrTag};
use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use std::sync::Arc;

pub type RpcProvider = DynProvider;

#[derive(Clone)]
pub struct RpcContext {
    pub provider: Arc<RpcProvider>,
    pub chain_id: u64,
    pub block_number: Option<u64>,
}

impl RpcContext {
    pub async fn new(rpc_url: &str, block_number: Option<u64>) -> anyhow::Result<Self> {
        let provider = ProviderBuilder::new().connect_http(rpc_url.parse()?);
        let provider: RpcProvider = provider.erased();
        let provider = Arc::new(provider);
        let chain_id = provider.get_chain_id().await?;
        Ok(Self {
            provider,
            chain_id,
            block_number,
        })
    }

    pub async fn block_number(&self) -> anyhow::Result<u64> {
        match self.block_number {
            Some(value) => Ok(value),
            None => Ok(self.provider.get_block_number().await?),
        }
    }

    pub async fn block_timestamp(&self, block_number: u64) -> anyhow::Result<i64> {
        let block = self
            .provider
            .get_block(BlockId::Number(BlockNumberOrTag::Number(
                block_number.into(),
            )))
            .await?;
        let Some(block) = block else {
            return Ok(0);
        };
        Ok(block.header.timestamp as i64)
    }
}

pub fn block_id(block_number: u64) -> BlockId {
    BlockId::Number(BlockNumberOrTag::Number(block_number.into()))
}
