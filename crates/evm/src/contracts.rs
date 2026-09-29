use alloy::sol;

sol! {
    #[sol(rpc)]
    interface IERC20Metadata {
        function symbol() external view returns (string);
        function decimals() external view returns (uint8);
    }

    #[sol(rpc)]
    interface IERC20SymbolBytes {
        function symbol() external view returns (bytes32);
        function decimals() external view returns (uint8);
    }

    #[sol(rpc)]
    interface IUniswapV3Factory {
        function getPool(address tokenA, address tokenB, uint24 fee) external view returns (address);
    }

    /// Aerodrome Slipstream keys pools by tick spacing (int24), not by fee tier.
    #[sol(rpc)]
    interface ISlipstreamFactory {
        function getPool(address tokenA, address tokenB, int24 tickSpacing) external view returns (address);
    }

    #[sol(rpc)]
    interface IUniswapV3Pool {
        function slot0()
            external
            view
            returns (
                uint160 sqrtPriceX96,
                int24 tick,
                uint16 observationIndex,
                uint16 observationCardinality,
                uint16 observationCardinalityNext,
                uint8 feeProtocol,
                bool unlocked
            );
        function liquidity() external view returns (uint128);
        function tickSpacing() external view returns (int24);
        function fee() external view returns (uint24);
        function token0() external view returns (address);
        function token1() external view returns (address);
        function tickBitmap(int16) external view returns (uint256);
        function ticks(int24)
            external
            view
            returns (
                uint128 liquidityGross,
                int128 liquidityNet,
                uint256 feeGrowthOutside0X128,
                uint256 feeGrowthOutside1X128,
                int56 tickCumulativeOutside,
                uint160 secondsPerLiquidityOutsideX128,
                uint32 secondsOutside,
                bool initialized
            );
    }

    #[sol(rpc)]
    interface IUniswapV3QuoterV2 {
        struct QuoteExactInputSingleParams {
            address tokenIn;
            address tokenOut;
            uint256 amountIn;
            uint24 fee;
            uint160 sqrtPriceLimitX96;
        }

        function quoteExactInputSingle(QuoteExactInputSingleParams calldata params)
            external
            returns (uint256 amountOut, uint160 sqrtPriceX96After, uint32 initializedTicksCrossed, uint256 gasEstimate);
    }

    #[sol(rpc)]
    interface IUniswapV2Factory {
        function getPair(address tokenA, address tokenB) external view returns (address);
    }

    #[sol(rpc)]
    interface IUniswapV2Pair {
        function token0() external view returns (address);
        function token1() external view returns (address);
        function getReserves() external view returns (uint112 reserve0, uint112 reserve1, uint32 blockTimestampLast);
    }

    #[sol(rpc)]
    interface IAerodromePool {
        function metadata()
            external
            view
            returns (
                uint256 dec0,
                uint256 dec1,
                uint256 r0,
                uint256 r1,
                bool st,
                address t0,
                address t1
            );
        function getAmountOut(uint256 amountIn, address tokenIn) external view returns (uint256);
    }

    #[sol(rpc)]
    interface IAerodromePoolFactory {
        function getPool(address tokenA, address tokenB, uint24 fee) external view returns (address);
        function getFee(address pool, bool stable) external view returns (uint256);
    }

    #[sol(rpc)]
    interface IMulticall3 {
        struct Call {
            address target;
            bytes callData;
        }

        struct Result {
            bool success;
            bytes returnData;
        }

        function aggregate(Call[] memory calls) external payable returns (uint256 blockNumber, bytes[] memory returnData);
    }

    #[sol(rpc)]
    interface ITesseraSwap {
        function tesseraSwapViewAmounts(address tokenIn, address tokenOut, int256 amountSpecified)
            external
            view
            returns (uint256 amountIn, uint256 amountOut);
    }
}
