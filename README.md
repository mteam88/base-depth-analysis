# Base Depth Analysis

Rust tooling to snapshot and visualize depth curves across Base L2 liquidity venues.

## Quick start

1. Ensure `.env` has:

```
BASE_RPC_URL=...
WETH_ADDRESS=0x4200000000000000000000000000000000000006
USDC_ADDRESS=0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913
```

2. Run a snapshot:

```
cargo run -p base-depth-cli -- snapshot \
  --max-base 0.5 \
  --steps 6 \
  --spacing geometric \
  --out-dir ./snapshots/latest
```

3. Run the live UI:

```
cargo run -p base-depth-cli -- serve --interval 15
```

Then open `http://localhost:8080`.

## Performance tips

- Kyber API sampling is the slowest part. Defaults now use `--kyber-concurrency 8` and no adaptive refinement. You can tune:

```
--kyber-concurrency 8
--kyber-adaptive-depth 2
--kyber-adaptive-bps 30
```

- To skip Kyber for faster snapshots:

```
--venues uniswap_v3,pancake_v3,slipstream,pancake_v2,aerodrome_v2,unified
```

## Notes

- Slipstream factories can revert on `getPool` if the address is not a CL factory on the connected chain. The system logs a warning and continues.
- Kyber flags are configurable via CLI or environment variables:
  - `KYBER_CLIENT_ID`, `KYBER_BASE_URL`
  - `KYBER_EXCLUDE_RFQ`, `KYBER_ONLY_DIRECT`, `KYBER_GAS_INCLUDE`
  - `KYBER_INCLUDED_SOURCES`, `KYBER_EXCLUDED_SOURCES`

