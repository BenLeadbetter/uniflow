# uniflow

A Redux-like state management library for Rust, 
built on top of [reactive_graph](https://crates.io/crates/reactive_graph) 
from the Leptos project.

## Overview

Uniflow provides a predictable state container following the Redux/Elm architecture pattern.
The design of the API is loosely inspired by the excellent 
[lager C++ project](https://github.com/arximboldi/lager).

- **Actions** as enums representing events
- **Pure reducers** that return new state plus optional effects
- **Effects** for async operations that can dispatch new actions
- **Reader** allow selecting and observing a subset of the application state
- **Watch** get a callback whenever the state, or a subset of it changes
- **Asynchronous** via abstract async runtime integration
- **Value Semantics** state and actions are treated as simple values - no references

See [DESIGN.md](./DESIGN.md) for architecture details.

## Value Semantics

`uniflow` is built around a value semantics paradigm. 
This means that your model and actions should be value types
with deep copy and logical-equality semantics.
Your reducer function should be a 
[pure function](https://en.wikipedia.org/wiki/Pure_function).

To achieve a scalable application model with value semantics without needing to 
pay the overhead for copying the data all over the place a persistent
data structure crate is recommended: for example [rpds](https://crates.io/crates/rpds).

## Asynchronous by Design

Dispatching actions back to the store is non-blocking and safe from 
any thread or async process.

The `uniflow::Effect` allows the running of heavy IO, or other
non-pure application work in an asynchronously spawned task.

`uniflow` is built on top of a 
["spawner" abstraction from the Leptos project](https://crates.io/crates/any_spawner),
allowing you to integrate with the async runtime of your choice: 
Tokio or wasmbindgen, for example.


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
