// SPDX-License-Identifier: GPL-3.0 or above
pragma solidity ^0.8.33; // UML

import "@openzeppelin/contracts/token/ERC20/IERC20.sol"; // UML
import "@openzeppelin/contracts/token/ERC721/IERC721.sol"; // UML

contract Auction {
    // UML
    address public owner; // UML
    NFT public auctionedToken; // UML
    IERC20 public token; // UML
    IERC721 public coll; // UML
    uint256 public startTime; // UML
    uint256 public endTime; // UML
    address[] public bidders; // UML

    struct NFT {
        // UML
        uint256 tokenId; // UML
    } // UML

    constructor( // UML
        IERC20 _token, // UML
        IERC721 _collection, // UML
        uint256 tokenId, // UML
        address _owner, // UML
        uint256 _startTime, // UML
        uint256 _endTime // UML
    ) public {
        // UML
        require(_collection.ownerOf(tokenId) == msg.sender && msg.sender == _owner && block.timestamp < _endTime); // UML
    } // UML

    mapping(address => uint256) public bids; // UML
    mapping(address => bool) public hasBid; // UML

    function bid(uint256 amount) public payable {
        // UML
        require(block.timestamp >= startTime && block.timestamp <= endTime); // UML
    } // UML

    function getWinner() public returns (address) {
        // UML
        require(block.timestamp > endTime); // UML
    } // UML
} // UML
