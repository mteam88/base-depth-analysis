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

## TesseraSwap integration

The TesseraSwap view function is wired as a black‑box quote source. Configure via CLI or env:

```
# Env (defaults to known Base addresses if not set)
TESSERA_SWAP=0x55555522005BcAE1c2424D474BfD5ed477749E3e
TESSERA_ENGINE=0x31e99e05fee3dce580af777c3fd63ee1b3b40c17
TESSERA_TREASURY=0x3dbe077e7986657e95e1cc50089f17a5a4af0aae
# Optional: set msg.sender for the view call
TESSERA_SENDER=0x0000000000000000000000000000000000000001
```

```
cargo run -p base-depth-cli -- snapshot --venues tessera
```

If Tessera engine requires a specific sender, pass `--tessera-sender`.

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

- To speed up v3 sampling, you can switch Uniswap/Pancake v3 to the Quoter v2 path:

```
--v3-quoter
--v3-quoter-concurrency 16
```

## Notes

- Slipstream factories can revert on `getPool` if the address is not a CL factory on the connected chain. The system logs a warning and continues.
- Kyber flags are configurable via CLI or environment variables:
  - `KYBER_CLIENT_ID`, `KYBER_BASE_URL`
  - `KYBER_EXCLUDE_RFQ`, `KYBER_ONLY_DIRECT`, `KYBER_GAS_INCLUDE`
  - `KYBER_INCLUDED_SOURCES`, `KYBER_EXCLUDED_SOURCES`
