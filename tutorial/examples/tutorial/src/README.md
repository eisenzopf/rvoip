# SIP Core Interactive Tutorial

Welcome to the interactive tutorial for the `rvoip-sip-core` library! This tutorial will guide you through building and parsing SIP/SDP messages, from basic concepts to advanced real-world applications.

## What You'll Learn

This tutorial covers:

- SIP message structure and components
- Parsing SIP messages using the `json` module
- Creating SIP messages with the `builder` pattern
- SDP media negotiation
- SIP dialogs and transactions
- Authentication and security
- Real-world applications

## How to Use This Tutorial

Each chapter contains:

- **Explanations** of key concepts
- **Code examples** that you can run and modify directly in your browser
- **Exercises** to test your understanding
- **References** to relevant RFCs and documentation

## Prerequisites

To get the most out of this tutorial, you should have:

- Basic knowledge of Rust programming
- Understanding of networking concepts
- Familiarity with client-server architecture

No prior knowledge of SIP or VoIP is required.

## Running Examples Locally

All examples in this tutorial can be run locally. To do this:

```bash
# Clone the repository
git clone https://github.com/rudeless/rvoip.git
cd rvoip/crates/sip-core

# Run a specific tutorial example
cargo run --example tutorial_01_intro

# Run with logging enabled
RUST_LOG=debug cargo run --example tutorial_02_parsing
```

## Getting Help

If you have questions or encounter issues:

- Check the [Glossary](appendix/glossary.md) for terminology
- Refer to the [SIP RFCs](appendix/rfc_references.md) for protocol details
- Open an issue on the [GitHub repository](https://github.com/rudeless/rvoip)

Let's get started with [Introduction to SIP](part1/tutorial_01.md)! 