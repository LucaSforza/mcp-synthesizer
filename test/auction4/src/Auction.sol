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
    uint256 public highestBid; // UML
    address public highestBidder; // UML

    struct NFT {
        // UML
        uint256 tokenId; // UML
    } // UML

    function setNFT(uint256 tokenId) public {
        // UML
        require(auctionedToken.tokenId == 0, "NFT already set"); // UML
        require(coll.ownerOf(tokenId) == msg.sender, "Sender must own the NFT"); // UML
        auctionedToken.tokenId = tokenId;
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
        owner = _owner;
        token = _token;
        coll = _collection;
        startTime = _startTime;
        endTime = _endTime;
        auctionedToken.tokenId = tokenId;
    } // UML

    mapping(address => uint256) public bids; // UML
    mapping(address => bool) public hasBid; // UML

    function bid(uint256 amount) public payable {
        // UML
        require(block.timestamp >= startTime && block.timestamp <= endTime); // UML
        require(amount > highestBid, "Bid must be higher than current highest bid"); // UML
        require(msg.value == amount, "Incorrect ETH amount sent"); // UML

        if (!hasBid[msg.sender]) {
            bidders.push(msg.sender);
            hasBid[msg.sender] = true;
        }

        bids[msg.sender] = amount;
        highestBid = amount;
        highestBidder = msg.sender;
    } // UML

    function getWinner() public returns (address) {
        // UML
        require(block.timestamp > endTime, "Auction has not ended yet"); // UML
        require(highestBid > 0, "No bids received"); // UML

        // Transfer NFT from owner to this contract first
        coll.transferFrom(owner, address(this), auctionedToken.tokenId);
        
        // Transfer NFT to winner using safeTransferFrom
        address(this).call(
            abi.encodeWithSignature(
                "safeTransferFrom(address,address,uint256)",
                address(this),
                highestBidder,
                auctionedToken.tokenId
            )
        );
        return highestBidder;
    } // UML
} // UML
