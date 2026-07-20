// bot/src/abi.rs
// Re-exports for convenience — the abigen! macros in pool.rs and executor.rs
// generate the necessary ABI bindings. This module can hold additional
// manually-defined ABIs if needed.

use ethers::prelude::*;

// ERC20 minimal ABI
abigen!(
    IERC20,
    r#"[
        function balanceOf(address account) external view returns (uint256)
        function transfer(address to, uint256 amount) external returns (bool)
        function approve(address spender, uint256 amount) external returns (bool)
        function allowance(address owner, address spender) external view returns (uint256)
        function decimals() external view returns (uint8)
        function symbol() external view returns (string)
    ]"#
);
