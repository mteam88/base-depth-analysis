use alloy_primitives::Address;
use std::str::FromStr;

#[derive(Clone, Debug)]
pub struct Addresses {
    pub chain_id: u64,
    pub multicall3: Address,
    pub uniswap_v3_factory: Address,
    pub uniswap_v3_quoter_v2: Address,
    pub pancake_v2_factory: Address,
    pub pancake_v2_router: Address,
    pub pancake_v3_factory: Address,
    pub pancake_v3_quoter_v2: Address,
    pub pancake_v3_router: Address,
    pub aerodrome_factory: Address,
    pub aerodrome_router: Address,
    pub slipstream_factories: Vec<Address>,
}

impl Addresses {
    pub fn base() -> Self {
        Self {
            chain_id: 8453,
            multicall3: parse("0xcA11bde05977b3631167028862bE2a173976CA11"),
            uniswap_v3_factory: parse("0x33128a8fC17869897dcE68Ed026d694621f6FDfD"),
            uniswap_v3_quoter_v2: parse("0x3d4e44Eb1374240CE5F1B871ab261CD16335B76a"),
            pancake_v2_factory: parse("0x02a84c1b3BBD7401a5f7fa98a384EBC70bB5749E"),
            pancake_v2_router: parse("0x8cFe327CEc66d1C090Dd72bd0FF11d690C33a2Eb"),
            pancake_v3_factory: parse("0x0BFbCF9fa4f9C56B0F40a671Ad40E0805A091865"),
            pancake_v3_quoter_v2: parse("0xB048Bbc1Ee6b733FFfCFb9e9CeF7375518e25997"),
            pancake_v3_router: parse("0x1b81D678ffb9C0263b24A97847620C99d213eB14"),
            aerodrome_factory: parse("0x420dd381b31aef6683db6b902084cb0ffece40da"),
            aerodrome_router: parse("0xcF77a3Ba9A5CA399B7c97c74d54e5b1Beb874E43"),
            slipstream_factories: vec![parse("0x5e7BB104d84c7CB9B682AaC2F3d509f5F406809A")],
        }
    }
}

fn parse(addr: &str) -> Address {
    Address::parse_checksummed(addr, None).unwrap_or_else(|_| Address::from_str(addr).unwrap())
}
