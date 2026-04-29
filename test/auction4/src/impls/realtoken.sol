// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "@openzeppelin/contracts/token/ERC20/ERC20.sol";

contract MyToken is ERC20 {
    /**
     * @dev Constructor that gives msg.sender all of existing tokens.
     * @param initialSupply The total amount of tokens to mint (in wei).
     */
    constructor(uint256 initialSupply) ERC20("MyToken", "MTK") {
        // Minting the initial supply to the deployer's account
        _mint(msg.sender, initialSupply);
    }
}

