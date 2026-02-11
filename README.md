# uniflow

A Redux-like state management library for Rust, 
built on top of [reactive_graph](https://crates.io/crates/reactive_graph) 
from the Leptos project.

## Overview

Uniflow provides a predictable state container following the Redux/Elm architecture pattern:

- **Actions** as enums representing events
- **Pure reducers** that return new state plus optional effects
- **Effects** for async operations that can dispatch new actions
- **Thread-safe dispatch** via tokio integration

See [DESIGN.md](./DESIGN.md) for architecture details.

## Alternatives

There are a number of other projects with similar goals to `uniflow`. 
Many of them are either deprecated or not under active development. 
Others don't have the key feature set which `uniflow` aims to cover.

* [redux-rs](https://crates.io/crates/redux-rs)
* [rs-store](https://crates.io/crates/rs-store)
* [reducer](https://crates.io/crates/reducer)
* [r3bl_redux](https://crates.io/crates/r3bl_redux)

## Name

`uniflow` is an abbreviation of
[unidirectional data flow](https://en.wikipedia.org/wiki/Unidirectional_data_flow).

## License

MIT OR Apache-2.0
