// contracts/src/MobiusExecutor.sol
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

interface IUniswapV2Pair {
    function getReserves() external view returns (uint112 reserve0, uint112 reserve1, uint32 blockTimestampLast);
    function token0() external view returns (address);
    function token1() external view returns (address);
    function swap(uint amount0Out, uint amount1Out, address to, bytes calldata data) external;
}

interface IERC20 {
    function balanceOf(address) external view returns (uint256);
    function transfer(address to, uint256 amount) external returns (bool);
    function approve(address spender, uint256 amount) external returns (bool);
}

interface IWETH {
    function deposit() external payable;
    function withdraw(uint256) external;
    function transfer(address to, uint256 amount) external returns (bool);
    function balanceOf(address) external view returns (uint256);
}

/// @title MobiusExecutor
/// @notice Atomic multi-hop arbitrage executor using direct pair swaps
/// @dev Uses the low-level swap() on Uniswap V2 pairs for gas efficiency.
///      Supports up to 4-hop cycles. Profits are kept in the contract
///      and withdrawn by the owner.
contract MobiusExecutor {
    address public immutable owner;
    address public immutable WETH;

    error NotOwner();
    error NotProfitable();
    error SwapFailed();

    modifier onlyOwner() {
        if (msg.sender != owner) revert NotOwner();
        _;
    }

    constructor(address _weth) {
        owner = msg.sender;
        WETH = _weth;
    }

    receive() external payable {}

    /// @notice Execute a multi-hop arbitrage cycle
    /// @param pairs Array of Uniswap V2 pair addresses in swap order
    /// @param zeroForOnes Whether each swap is token0 → token1
    /// @param amountIn Amount of input token to start with
    /// @param minProfit Minimum profit required (in input token)
    function executeArbitrage(
        address[] calldata pairs,
        bool[] calldata zeroForOnes,
        uint256 amountIn,
        uint256 minProfit
    ) external onlyOwner {
        uint256 hops = pairs.length;
        
        // Determine the starting token
        address startToken;
        if (zeroForOnes[0]) {
            startToken = IUniswapV2Pair(pairs[0]).token0();
        } else {
            startToken = IUniswapV2Pair(pairs[0]).token1();
        }

        uint256 balanceBefore = IERC20(startToken).balanceOf(address(this));

        // Send input tokens to first pair
        IERC20(startToken).transfer(pairs[0], amountIn);

        // Execute each hop
        for (uint256 i = 0; i < hops; i++) {
            IUniswapV2Pair pair = IUniswapV2Pair(pairs[i]);
            (uint112 r0, uint112 r1,) = pair.getReserves();

            // Determine input/output amounts
            uint256 amountInForHop;
            if (zeroForOnes[i]) {
                // token0 → token1
                amountInForHop = IERC20(pair.token0()).balanceOf(address(pair)) - r0;
                uint256 amountOut = _getAmountOut(amountInForHop, r0, r1);
                address to = (i < hops - 1) ? pairs[i + 1] : address(this);
                pair.swap(0, amountOut, to, "");
            } else {
                // token1 → token0
                amountInForHop = IERC20(pair.token1()).balanceOf(address(pair)) - r1;
                uint256 amountOut = _getAmountOut(amountInForHop, r1, r0);
                address to = (i < hops - 1) ? pairs[i + 1] : address(this);
                pair.swap(amountOut, 0, to, "");
            }
        }

        uint256 balanceAfter = IERC20(startToken).balanceOf(address(this));
        if (balanceAfter < balanceBefore + minProfit) revert NotProfitable();
    }

    /// @notice Calculate output amount for a Uniswap V2 swap (0.3% fee)
    function _getAmountOut(
        uint256 amountIn,
        uint256 reserveIn,
        uint256 reserveOut
    ) internal pure returns (uint256) {
        uint256 amountInWithFee = amountIn * 997;
        uint256 numerator = amountInWithFee * reserveOut;
        uint256 denominator = reserveIn * 1000 + amountInWithFee;
        return numerator / denominator;
    }

    /// @notice Withdraw tokens from the contract
    function withdraw(address token) external onlyOwner {
        uint256 balance = IERC20(token).balanceOf(address(this));
        if (balance > 0) {
            IERC20(token).transfer(owner, balance);
        }
    }

    /// @notice Withdraw ETH from the contract
    function withdrawETH() external onlyOwner {
        uint256 balance = address(this).balance;
        if (balance > 0) {
            (bool ok,) = owner.call{value: balance}("");
            require(ok);
        }
    }
}
