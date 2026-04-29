# my-mcp-server

An MCP (Model Context Protocol) server for automated Solidity smart contract verification using Foundry and Halmos.

## Overview

This Rust-based MCP server provides tools for compiling, testing, and formally verifying Ethereum smart contracts. It enables agentic workflows where LLMs can orchestrate verification pipelines for Solidity code.

## Architecture

```
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   MCP Client    │ →  │  SolTools       │ →  │  Foundry/Halmos │
│  (LLM/Agent)    │    │  Rust Server    │    │   Smart Contracts│
└─────────────────┘    └─────────────────┘    └─────────────────┘
```

### Workflow

```
Solidity Source
      ↓
   Compile (Foundry)
      ↓
Fuzzy Testing (Foundry)
      ↓
Symbolic Execution (Halmos)
      ↓
  Verification Report
```

## Features

- **Compilation**: Compile Foundry projects with verbose output
- **Fuzzy Testing**: Run property-based fuzzing tests on smart contracts
- **Formal Verification**: Use Halmos for symbolic execution and proof verification
- **MCP Integration**: Expose tools via Model Context Protocol for LLM consumption

## Prerequisites

- Rust 1.70+ (edition 2024)
- [Foundry](https://github.com/foundry-rs/foundry)
- [Halmos](https://github.com/a16z/halmos)

## Installation

```bash
cargo build --release
```

## Usage

### As a standalone MCP server

```bash
# Run with a specific project directory
./my-mcp-server --cwd /path/to/your/foundry/project

# Or pipe stdin/stdout for MCP protocol
./my-mcp-server --cwd /path/to/project
```

### MCP Tools

#### `compile`

Compile a Foundry project.

```json
{
  "tool": "compile",
  "description": "compile foundry project"
}
```

#### `fuzzy_testing`

Run Foundry's property-based fuzzing tests.

```json
{
  "tool": "fuzzy_testing",
  "description": "run foundry project fuzzy test"
}
```

#### `verify`

Run verification pipeline: fuzzy testing + Halmos symbolic execution.

```json
{
  "tool": "verify",
  "description": "verify contracts"
}
```

## Configuration

The server expects a working Foundry project with:
- `foundry.toml` configuration
- Test files in `test/` directory
- Contracts in `src/` or `contracts/`

## Example MCP Call

```bash
# Start the server
mcp-server --cwd ./my-foundry-project

# From a client, invoke the verify tool
curl -X POST http://localhost:3000/mcp \
  -H "Content-Type: application/json" \
  -d '{"tool": "verify"}'
```

## Project Structure

```
src/
├── main.rs          # MCP server entry point
└── tools/           # (future) Tool implementations
tests/
├── integration/     # Integration tests
└── unit/            # Unit tests
```

## Development

### Adding new tools

1. Add methods to the `SolTools` struct with `#[tool]` attribute
2. The `#[tool_router]` macro automatically registers them
3. Each tool should return a `String` or structured JSON

### Extending the workflow

The verification pipeline can be extended by:
- Adding more Halmos verification tools
- Integrating Slither for security analysis
- Adding custom proof strategies

## License

This project is licensed under the GNU General Public License v3.0 - see the [LICENSE](LICENSE) file for details.
