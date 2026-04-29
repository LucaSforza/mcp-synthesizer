// SPDX-License-Identifier: GPL-3.0 or above
pragma solidity ^0.8.33; // UML

contract Auction {// UML
    address public owner; // UML
    NFT public auctionedToken; // UML
    IERC20 public token; // UML
    IERC721 public coll; // UML
    uint256 public startTime; // UML
    uint256 public endTime; // UML
    address[] public bidders; // UML

    struct NFT { // UML
        uint256 tokenId; // UML
    } // UML

    constructor( // UML
        IERC20 _token, // UML
        IERC721 _collection, // UML
        uint256 tokenId, // UML
        address _owner, // UML
        uint256 _startTime, // UML
        uint256 _endTime // UML
    ) public { // UML
        require(_collection.ownerOf(tokenId) == msg.sender && msg.sender == _owner && block.timestamp < _endTime); // UML
        owner = _owner; // UML
        token = _token; // UML
        coll = _collection; // UML
        startTime = _startTime; // UML
        endTime = _endTime; // UML
        auctionedToken = NFT({tokenId: tokenId}); // UML
    } // UML

    mapping(address => uint256) public bids; // UML
    mapping(address => bool) public hasBid; // UML

    function bid(uint256 amount) public payable {// UML
        require(block.timestamp >= startTime && block.timestamp <= endTime); // UML
        bids[msg.sender] += amount; // UML
        token.transferFrom(msg.sender, address(this), amount); // UML
        if (!hasBid[msg.sender]) { // UML
            hasBid[msg.sender] = true; // UML
            bidders.push(msg.sender); // UML
        } // UML
    } // UML

    function getWinner() public returns (address) {// UML
        require(block.timestamp > endTime); // UML
        uint256 maxBid = 0; // UML
        address winner = address(0); // UML
        for (uint256 i = 0; i < bidders.length; i++) { // UML
            address bidder = bidders[i]; // UML
            if (bids[bidder] > maxBid) { // UML
                maxBid = bids[bidder]; // UML
                winner = bidder; // UML
            } // UML
        } // UML
        if (winner != address(0)) { // UML
            coll.transferFrom(address(this), winner, auctionedToken.tokenId); // UML
        } // UML
        return winner; // UML
    } // UML
} // UML
