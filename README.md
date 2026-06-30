# uniflow

A Redux-inspired, unidirectional state management library for Rust.
Modelled after the [lager C++ project](https://github.com/arximboldi/lager),
with async execution via
[any_spawner](https://crates.io/crates/any_spawner).

## Overview

Uniflow provides a predictable state container following the Redux/Elm architecture pattern.

- **Actions** as enums representing events
- **Pure reducers** that return new state plus optional effects
- **Effects** for async operations that can dispatch new actions
- **State** — a standalone reactive read/write primitive
- **Reader** — observe and derive subsets of application state
- **Context mapping** — pass a narrowed `Context<SubAction>` to subsystems; actions
  are mapped back to the store's action type before dispatch (contravariant, as in lager)
- **Watch** — callbacks whenever state, or a derived subset of it, changes
- **Asynchronous** — abstract async runtime integration; works with Tokio, wasm-bindgen, and more
- **Value semantics** — state and actions are plain values: no references, no shared mutation

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

`uniflow` uses [`any_spawner`](https://crates.io/crates/any_spawner)
for pluggable async execution, allowing you to integrate with the async runtime of your
choice: Tokio or wasm-bindgen, for example.

## Alternatives

There are a number of other projects with similar goals to `uniflow`.
Many of them are either deprecated or not under active development.
Others don't have the key feature set which `uniflow` aims to cover.

- [redux-rs](https://crates.io/crates/redux-rs)
- [rs-store](https://crates.io/crates/rs-store)
- [reducer](https://crates.io/crates/reducer)
- [r3bl_redux](https://crates.io/crates/r3bl_redux)

## Name

`uniflow` is an abbreviation of
[unidirectional data flow](https://en.wikipedia.org/wiki/Unidirectional_data_flow).

## License

MIT OR Apache-2.0
