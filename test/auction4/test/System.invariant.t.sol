// SPDX-License-Identifier: GPL-3.0 or above
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {Auction} from "../src/Auction.sol";
import {ZPunks} from "../src/impls/RealNFT.sol";
import {MyToken} from "../src/impls/realtoken.sol";
import "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import "@openzeppelin/contracts/token/ERC721/IERC721.sol";

contract SystemInvariantTest is Test {
    Auction public auction;
    ZPunks public nft;
    MyToken public token;

    uint256 public ghost_bidSum;
    address ghost_max_bidder;
    uint256 ghost_nbid;
    uint256 ghost_id;

    address[] public bidders;
    address internal currentBidder;

    address owner;

    mapping(address => uint256) ghost_bids;

    modifier useBidder(uint256 bidderSeed) {
        currentBidder = bidders[bound(bidderSeed, 0, bidders.length - 1)];
        vm.startPrank(currentBidder);
        _;
        vm.stopPrank();
    }

    constructor() {
        token = new MyToken(10 ** 24);
        ghost_nbid = 0;
        // Give each bidder tokens from the minted amount (no ether)
        for (uint256 i = 0; i < 10; i++) {
            address bidder = makeAddr(string(abi.encodePacked("bidder", i)));
            bidders.push(bidder);
            token.transfer(bidder, 100_000);
        }

        owner = makeAddr(string(abi.encodePacked("bidder", uint256(11))));
        nft = new ZPunks(owner);

        // Mint NFT directly to the auction contract
        vm.startPrank(owner);
        ghost_id = nft.safeMint(owner);
        // Create auction first
        //IERC721 _collection, uint256 tokenId, address _owner, uint256 _startTime, uint256 _endTime)
        auction = new Auction(
            IERC20(address(token)), IERC721(address(nft)), ghost_id, owner, block.timestamp, block.timestamp + 10
        );
        nft.setApprovalForAll(address(auction), true);
        vm.stopPrank();
    }

    function bid(uint256 amount, uint256 bidderSeed) external useBidder(bidderSeed) {
        auction.bid(amount);
        ghost_nbid += 1;
        ghost_bids[currentBidder] += amount;
        ghost_bidSum += amount;

        if (ghost_max_bidder == address(0) || ghost_bids[currentBidder] > ghost_bids[ghost_max_bidder]) {
            ghost_max_bidder = currentBidder;
        }
    }

    address ghost_winner;

    function finish_auction(uint256 amount, uint256 bidderSeed) external useBidder(bidderSeed) {
        vm.assume(ghost_nbid >= 100);
        vm.warp(block.timestamp + 100);
        ghost_winner = auction.getWinner();
    }

    // function invariant_bid() public view {
    //     for (uint256 i = 0; i < bidders.length; i++) {
    //         address bidder = bidders[i];
    //         assertEq(auction.getBid(bidder), ghost_bids[bidder]);
    //     }
    // }

    function invariant_getWinner() public view {
        require(ghost_winner != address(0));
        assertEq(ghost_max_bidder, ghost_winner);
        assertEq(ghost_winner, nft.ownerOf(ghost_id));
    }
}
