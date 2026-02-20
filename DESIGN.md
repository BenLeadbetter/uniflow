# Uniflow Design Document

A Redux-like state management library for Rust, built on top of [reactive_graph](https://crates.io/crates/reactive_graph) from the Leptos project.

## Goals

- Provide a predictable state container following the Redux/Elm architecture
- Leverage `reactive_graph` for battle-tested reactive primitives
- Support actions, pure reducers, and async effects that can dispatch new actions
- Use `any_spawner` for pluggable async execution and cross-thread dispatch
- Keep the implementation minimal by building on existing foundations

## Architecture Overview

```
┌──────────────────────────────────────────────────────────┐
│                        uniflow                           │
│  ┌────────────────────────────────────────────────────┐  │
│  │  Store<Model, Action>                              │  │
│  │    - dispatch(action) → thread-safe, any thread    │  │
│  │    - reader() → derived reactive views             │  │
│  │    - Reducer: fn(Model, Action) → (Model, Effect)  │  │
│  └────────────────────────────────────────────────────┘  │
│                           │                              │
│                     builds on                            │
│                           ▼                              │
│  ┌────────────────────────────────────────────────────┐  │
│  │  reactive_graph (from Leptos)                      │  │
│  │    - Signal<T> → state storage                     │  │
│  │    - Memo<T> → derived/cached values               │  │
│  │    - Effect → subscriptions/watchers               │  │
│  └────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────┘
```

## Core Concepts

### Actions

Actions are enums representing events:

```rust
enum Action {
    Increment,
    SetValue(i32),
    FetchData { url: String },
    DataReceived(String),
}
```

### Model

Application state, must be `Clone + PartialEq`:

```rust
#[derive(Clone, Default, PartialEq)]
struct AppState {
    counter: i32,
    loading: bool,
    data: Option<String>,
}
```

### Reducer

A pure function returning the new state and an optional effect:

```rust
fn reducer(state: AppState, action: Action) -> (AppState, Option<Effect<Action>>) {
    match action {
        Action::Increment => (
            AppState { counter: state.counter + 1, ..state },
            None
        ),
        Action::FetchData { url } => (
            AppState { loading: true, ..state },
            Some(Effect::new(move |ctx| async move {
                let data = fetch(&url).await;
                ctx.dispatch(Action::DataReceived(data));
            }))
        ),
        Action::DataReceived(data) => (
            AppState { loading: false, data: Some(data), ..state },
            None
        ),
    }
}
```

### Dependency Injection

Dependencies are injected into the store at creation time and made available to effects via `Context`. Any `Clone + Send + Sync + 'static` type satisfies the `Deps` trait (blanket impl). `()` is the default "no deps" case.

```rust
#[derive(Clone)]
struct AppDeps {
    api_client: ApiClient,
    db: Database,
}

let store = Store::new_with_deps(initial_state, reducer, AppDeps { api_client, db });
```

### Effects

Async operations that can dispatch new actions via a `Context`. Effects are returned from the reducer alongside the new state:

```rust
pub struct Context<A, D = ()> { /* action sender + deps */ }

impl<A: Action, D: Deps> Context<A, D> {
    pub fn dispatch(&self, action: A);
    pub fn deps(&self) -> &D;
}

pub struct Effect<A, D = ()> { /* async closure */ }

impl<A: Action, D: Deps> Effect<A, D> {
    pub fn new(f: impl FnOnce(Context<A, D>) -> impl Future<Output = ()>) -> Self;
    pub fn none() -> Self;
}
```

## Store

The `Store` wraps `reactive_graph` primitives with Redux semantics:

```rust
impl<Model, Action> Store<Model, Action> {
    /// Create a store with initial state and reducer.
    /// Spawns internal reducer task via any_spawner.
    /// Requires executor to be initialized first.
    pub fn new<R>(initial: Model, reducer: R) -> Self;

    /// Dispatch an action. Synchronous, non-blocking, thread-safe.
    /// Sends to internal channel and returns immediately.
    /// No-op if store is shut down.
    pub fn dispatch(&self, action: Action);

    /// Get current state snapshot
    pub fn get(&self) -> Model;

    /// Create a reactive reader (wraps Memo internally)
    pub fn reader<T, F>(&self, selector: F) -> Reader<T>
    where
        F: Fn(&Model) -> T;

    /// Subscribe to state changes
    pub fn watch<F>(&self, callback: F) -> Subscription;

    /// Signal shutdown. Closes channel, reducer task drains and exits.
    pub fn shutdown(&self);
}
```

## Async Execution

Uniflow uses `any_spawner` from the Leptos ecosystem for pluggable async execution, with `futures::channel::mpsc` for runtime-agnostic channels:

- **`futures::channel::mpsc`** - Runtime-agnostic unbounded channel
- **`any_spawner::Executor`** - Abstract executor interface
- **Internal reducer task** - Spawned in `Store::new()`, processes actions sequentially
- **`Executor::spawn()`** - Runs effects concurrently

```
dispatch(&self, action)     ← sync, non-blocking, any thread
        │
        ▼
   unbounded channel        ← futures::channel::mpsc
        │
        ▼
┌─────────────────────┐
│   Reducer Task      │  ← spawned internally, sequential processing
│   1. Apply reducer  │
│   2. Update signal  │
│   3. Spawn effects  │
└─────────────────────┘
```

Key design decisions:
- **Dispatch is synchronous**: `dispatch()` sends to an unbounded channel and returns immediately. Safe for real-time contexts (e.g., audio threads) that cannot block.
- **Internal task spawning**: The reducer task is spawned internally in `Store::new()` via `any_spawner::Executor::spawn()`. Simple API, no manual task management required. Requires executor initialization before store creation.
- **Graceful shutdown**: `shutdown()` closes the channel. The reducer task drains remaining actions and exits cleanly.
- **Runtime-agnostic**: Using `futures::channel::mpsc` instead of `tokio::sync::mpsc` allows the library to work with any executor, including a synchronous executor for unit testing.
- **Future flexibility**: A `new_with_task()` variant may be added for users needing manual control over task spawning and lifecycle.

Apps initialize their preferred executor via `any_spawner::Executor::init_tokio()`, `init_custom_executor()`, or other provided initializers.

### Minimal Example

```rust
use std::sync::Arc;
use tokio::signal;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize executor before creating stores
    any_spawner::Executor::init_tokio()?;

    // Create store (reducer task spawned internally)
    let store = Arc::new(Store::new(AppState::default(), reducer));

    // Dispatch from anywhere
    store.dispatch(Action::Increment);

    // Share with other tasks
    let worker_store = store.clone();
    tokio::spawn(async move {
        worker_store.dispatch(Action::DoWork);
    });

    // Wait for shutdown signal
    signal::ctrl_c().await?;

    // Graceful shutdown
    store.shutdown();

    Ok(())
}
```

## Dependencies

```toml
[dependencies]
reactive_graph = "0.x"
any_spawner = "0.x"
futures = "0.3"          # runtime-agnostic channels and StreamExt
```

## Future Features

- **Lens-based zoom** - Composable readers focused on state subsets
- **Transducer-style transforms** - Chainable transformations on readers
- **Custom schedulers** - any_spawner integration for injecting custom executors
- **Middleware** - Action interception and transformation
- **Cursor** - Bidirectional read-write access for forms

## References

- [reactive_graph](https://docs.rs/reactive_graph) - Leptos reactive primitives
- [Leptos Book: Reactive Graph](https://book.leptos.dev/appendix_reactive_graph.html)
- [Lager C++ Library](https://github.com/arximboldi/lager) - Original inspiration
- [Redux](https://redux.js.org/)
