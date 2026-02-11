# Uniflow Design Document

A Redux-like state management library for Rust, built on top of [reactive_graph](https://crates.io/crates/reactive_graph) from the Leptos project.

## Goals

- Provide a predictable state container following the Redux/Elm architecture
- Leverage `reactive_graph` for battle-tested reactive primitives
- Support actions, pure reducers, and async effects that can dispatch new actions
- Integrate with tokio for cross-thread dispatch and a "main loop" task pattern
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

### Effects

Async operations that can dispatch new actions via a `Context`:

```rust
pub struct Context<Action> { /* action sender */ }

impl<Action> Context<Action> {
    pub fn dispatch(&self, action: Action);
}
```

## Store

The `Store` wraps `reactive_graph` primitives with Redux semantics:

```rust
impl<Model, Action> Store<Model, Action> {
    /// Create a store with initial state and reducer
    pub fn new<R>(initial: Model, reducer: R) -> Self;

    /// Dispatch an action. Safe to call from any thread.
    pub fn dispatch(&self, action: Action);

    /// Get current state snapshot
    pub fn get(&self) -> Model;

    /// Create a reactive reader (wraps Memo internally)
    pub fn reader<T, F>(&self, selector: F) -> Reader<T>
    where
        F: Fn(&Model) -> T;

    /// Subscribe to state changes
    pub fn watch<F>(&self, callback: F) -> Subscription;
}
```

## Tokio Integration

Uniflow constructs a "main loop" pattern on tokio using:

- **`tokio::sync::mpsc`** - Action queue for cross-thread dispatch
- **Dedicated reducer task** - Processes actions sequentially
- **`tokio::spawn`** - Runs effects concurrently

```
dispatch() from any thread
        │
        ▼
   mpsc channel
        │
        ▼
┌─────────────────────┐
│   Reducer Task      │  ← single task, sequential processing
│   1. Apply reducer  │
│   2. Update signal  │
│   3. Spawn effects  │
└─────────────────────┘
```

This ensures predictable state updates while allowing dispatch from worker threads.

## Dependencies

```toml
[dependencies]
reactive_graph = "0.x"
tokio = { version = "1", features = ["sync", "rt"] }
```

## Future Features

- **Lens-based zoom** - Composable readers focused on state subsets
- **Transducer-style transforms** - Chainable transformations on readers
- **Alternative runtimes** - EventLoop trait for Qt, manual/test loops
- **Middleware** - Action interception and transformation
- **Cursor** - Bidirectional read-write access for forms

## References

- [reactive_graph](https://docs.rs/reactive_graph) - Leptos reactive primitives
- [Leptos Book: Reactive Graph](https://book.leptos.dev/appendix_reactive_graph.html)
- [Lager C++ Library](https://github.com/arximboldi/lager) - Original inspiration
- [Redux](https://redux.js.org/)
