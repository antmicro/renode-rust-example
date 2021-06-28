# Renode Rust UART peripheral example

Copyright (c) 2021 [Antmicro](https://www.antmicro.com)

## Overview

Sample of a peripheral implemented in Rust, to be used with [the Renode Framework](https://renode.io).

## Building

To compile the peripheral, just run: `cargo build --target wasm32-unknown-unknown --release --lib`.

The resulting binary will be available under `target/wasm32-unknown-unknown/release/rust_uart.wasm`.

To use it in Renode, you must download and build it from [a branch](https://github.com/renode/renode/tree/26999-rust_uart).

For build instructions, please refer to [documentation](https://renode.readthedocs.io/en/latest/advanced/building_from_sources.html).


## Usage

The specified Renode branch contains an implementation of `RustUART` peripheral, that can be used with a compiled `rust_uart.rs`.

To start the simulation, run the following in your compiled Renode:

```
(monitor) s @fe310-rust.resc
```

This will run a [Zephyr RTOS](https://www.zephyrproject.org/) shell example on RISC-V based SiFive FE310 with a Rust implementation of UART.

To run sample test case, run the following in your console:

```
path-to/renode/test.sh fe310_rust.robot
```
