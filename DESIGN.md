# Uniflow Design Document

A Redux-inspired, unidirectional state management library for Rust, modelled after the
[lager C++ project](https://github.com/arximboldi/lager).
Built on hand-rolled reactive primitives and pluggable async execution via
[any_spawner](https://crates.io/crates/any_spawner).

## Goals

- Provide a predictable state container following the Redux/Elm architecture
- Implement a minimal reactive graph (source nodes, derived nodes, watchers) without
  depending on an external reactive framework
- Support actions, pure reducers, and async effects that can dispatch new actions
- Use `any_spawner` for pluggable async execution and cross-thread dispatch
- Mirror the lager design: contravariant `Context` mapping, value semantics, injected deps

## Architecture Overview

```
┌──────────────────────────────────────────────────────────────┐
│                           uniflow                            │
│                                                              │
│  ┌──────────────────────────────────────────────────────┐   │
│  │  Store<State, Action, Deps>                          │   │
│  │    - dispatch(action)  → sync, non-blocking          │   │
│  │    - reader() / derived(f) → reactive views          │   │
│  │    - context() → Context<A, D> for effect dispatch   │   │
│  │    - Reducer: fn(State, Action) → (State, Effect)    │   │
│  └───────────────────────┬──────────────────────────────┘   │
│                          │ owns                              │
│                          ▼                                   │
│  ┌──────────────────────────────────────────────────────┐   │
│  │  Reactive Node Graph (hand-rolled)                   │   │
│  │    - SourceNode<T>   → root value cell               │   │
│  │    - DerivedNode<T>  → cached derived value          │   │
│  │    - MergeNode<T>    → combines multiple upstreams   │   │
│  │    - WatchSlot / Subscription → watcher lifecycle    │   │
│  └──────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────┘
```

## Core Concepts

### Actions

Actions are values (typically enums) representing events:

```rust
enum Action {
    Increment,
    SetValue(i32),
    FetchData { url: String },
    DataReceived(String),
}
```

### State / Model

Application state must satisfy `Value` (`Clone + PartialEq + Send + Sync + 'static`):

```rust
#[derive(Clone, Default, PartialEq)]
struct AppState {
    counter: i32,
    loading: bool,
    data: Option<String>,
}
```

### Reducer

A pure function returning new state and an optional effect:

```rust
fn reducer(state: AppState, action: Action) -> (AppState, Effect<Action>) {
    match action {
        Action::Increment => (
            AppState { counter: state.counter + 1, ..state },
            Effect::none(),
        ),
        Action::FetchData { url } => (
            AppState { loading: true, ..state },
            Effect::new(move |ctx| async move {
                let data = fetch(&url).await;
                ctx.dispatch(Action::DataReceived(data));
            }),
        ),
        Action::DataReceived(data) => (
            AppState { loading: false, data: Some(data), ..state },
            Effect::none(),
        ),
    }
}
```

For stores without effects, the simpler `Reducer` signature (`fn(S, A) -> S`) is also
accepted via `Store::new`.

### Dependency Injection

Dependencies are injected at store creation and forwarded to effects via `Context`.
Any `Clone + Send + Sync + 'static` type satisfies the `Deps` bound (blanket impl).
`()` is the default "no deps" case.

```rust
#[derive(Clone)]
struct AppDeps {
    api_client: ApiClient,
    db: Database,
}

let store = Store::new_with_deps(initial_state, reducer, AppDeps { api_client, db });
```

### Effects

Async work returned alongside the new state from the reducer. Spawned by the store
after the reducer runs; effects can dispatch further actions via their `Context`.

```rust
pub struct Effect<A: Action, D: Deps = ()> { /* async closure */ }

impl<A: Action, D: Deps> Effect<A, D> {
    pub fn new<F, Fut>(f: F) -> Self
    where
        F: FnOnce(Context<A, D>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static;

    pub fn none() -> Self;
}
```

### Context

Passed to each effect; carries dispatch capability and injected dependencies.
Designed after lager's `context`: contravariant in the action type, meaning a
`Context<CoreAction>` can produce a `Context<SubAction>` via `map`.

```rust
pub struct Context<A: Action, D: Deps = ()> { /* boxed dispatcher + deps */ }

impl<A: Action, D: Deps> Context<A, D> {
    /// Non-blocking dispatch; safe from any thread or async context.
    pub fn dispatch(&self, action: A);

    /// Access injected dependencies.
    pub fn deps(&self) -> &D;

    /// Returns a Context<B, D> that applies `f: B -> A` before dispatching.
    /// Useful for passing a narrowed context to subsystems that only know
    /// about a subset of the store's full action type.
    pub fn map<B: Action, F: Fn(B) -> A + Send + Sync + 'static>(
        &self, f: F,
    ) -> Context<B, D>;
}
```

The dispatcher is stored as `Arc<dyn Fn(A) + Send + Sync>`, which allows `map` to
compose a new closure around the parent without requiring a channel of type `Sender<B>`.

## Reactive Node Graph

Uniflow's reactivity does not depend on any external reactive library.
Instead it uses a small set of node types:

| Node                   | Role                                                                                                                       |
| ---------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `SourceNode<T>`        | Root value cell. `set(value)` updates and notifies watchers if the value changed (`PartialEq`).                            |
| `DerivedNode<T>`       | Lazy cached projection. Recomputes when its upstream fires; notifies its own watchers only when the derived value changes. |
| `MergeNode<(A,B,...)>` | Combines two–five upstream nodes into a tuple. Fires whenever any upstream changes.                                        |

All nodes implement `ReadableNode<T>` (the internal trait), allowing `Reader<T>` to
work uniformly across all of them.

Subscriptions use weak references (`Weak<()>` inside `WatchSlot`) so that dropping a
`Reader` or `State` automatically removes its watchers without any explicit deregistration.

## Store

```rust
impl<S: Value, A: Action, D: Deps> Store<S, A, D> {
    // Constructors
    pub fn new<R: Reducer<S, A>>(state: S, reducer: R) -> Store<S, A, ()>;
    pub fn new_with_capacity<R: Reducer<S, A>>(state: S, reducer: R, capacity: usize) -> Store<S, A, ()>;
    pub fn new_with_deps<R: EffectReducer<S, A, D>>(state: S, reducer: R, deps: D) -> Self;
    pub fn new_with_deps_and_capacity<R: EffectReducer<S, A, D>>(
        state: S, reducer: R, deps: D, capacity: usize,
    ) -> Self;

    // State access (also impl Read<S>)
    pub fn get(&self) -> S;
    pub fn watch<F: Fn(&S) + Send + Sync + 'static>(&self, f: F) -> &Self;
    pub fn bind<F: Fn(&S) + Send + Sync + 'static>(&self, f: F) -> &Self;
    pub fn unbind(&self);

    // Derived readers
    pub fn reader(&self) -> Reader<S>;
    pub fn derived<T, F: Fn(&S) -> T + Send + Sync + 'static>(&self, f: F) -> Reader<T>;

    // Context factory
    pub fn context(&self) -> Context<A, D>;

    // Lifecycle
    pub fn dispatch(&self, action: A);   // also impl Dispatch<A>
    pub fn shutdown(&self);
}
```

## Async Execution

```
dispatch(&self, action)        ← sync, non-blocking, any thread
        │
        ▼
  bounded mpsc channel         ← futures::channel::mpsc (capacity: 128 default)
        │
        ▼
┌──────────────────────┐
│   Reducer Task       │  ← spawned in Store::new via any_spawner::Executor::spawn
│   1. Apply reducer   │
│   2. Update SourceNode│
│   3. Spawn effects   │  ← each effect runs concurrently via Executor::spawn
└──────────────────────┘
```

Key design decisions:

- **Dispatch is synchronous**: `dispatch()` sends to a bounded channel and returns
  immediately. Safe for real-time contexts (e.g., audio threads) that cannot block.
- **Bounded channel**: Defaults to capacity 128; configurable. If the channel is full,
  the dispatch is dropped with a `debug_assert` (never panics in release).
- **Sequential reducer**: Actions are processed one at a time by a single internal task,
  preserving ordering guarantees.
- **Concurrent effects**: Effects are spawned independently and may complete out of order.
- **Graceful shutdown**: `shutdown()` closes the sender. The reducer task drains
  remaining buffered actions and exits.
- **Runtime-agnostic**: `futures::channel::mpsc` works with any executor. Apps
  initialise their preferred runtime via `any_spawner::Executor::init_tokio()` or
  a custom initialiser.

### Minimal Example

```rust
use uniflow::{Store, Effect, Context, Read, Dispatch};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    any_spawner::Executor::init_tokio()?;

    let store = Store::new(0i32, |state: i32, delta: i32| state + delta);

    store.dispatch(1);
    store.dispatch(2);

    // give the reducer task a chance to run
    tokio::task::yield_now().await;

    println!("state = {}", store.get()); // 3

    store.shutdown();
    Ok(())
}
```

### Context Mapping Example

```rust
enum CoreAction { Log(String), Increment }
enum SubAction  { Increment }

let ctx: Context<CoreAction> = store.context();

// Subsystem only needs to know about SubAction
let sub_ctx: Context<SubAction> = ctx.map(|a| match a {
    SubAction::Increment => CoreAction::Increment,
});

sub_ctx.dispatch(SubAction::Increment); // → CoreAction::Increment → store
```

## State Primitive

`State<T>` is a standalone reactive read/write cell, independent of a store.
Useful for local UI state or any reactive value that doesn't need a reducer.

```rust
pub struct State<T: Value> { /* SourceNode + Reader */ }

impl<T: Value> State<T> {
    pub fn new(value: T) -> Self;
    pub fn reader(&self) -> Reader<T>;   // read-only view
    // + Read<T> (get / watch / bind / unbind)
    // + Write<T> (set)
    // + ReadWrite<T> (update(f))
}
```

Cloning a `State` shares the underlying `SourceNode`; subscriptions are independent
per clone.

## Dependencies

```toml
[dependencies]
any_spawner = "0.3"
futures     = "0.3"
```

## Future Features

- **Middleware** — action interception before the reducer (logging, analytics, dev tools)
- **Lens-based zoom** — composable readers focused on state subsets
- **Cursor** — bidirectional read/write access for form bindings
- **Transducer-style transforms** — chainable `map`, `filter`, `dedupe` on readers
- **Batched updates** — coalesce multiple dispatches into a single state update
- **Time-travel debugging** — record and replay action history

## References

- [lager C++ library](https://github.com/arximboldi/lager) — primary design inspiration
- [any_spawner](https://crates.io/crates/any_spawner) — pluggable async executor abstraction
- [Redux](https://redux.js.org/) — unidirectional data flow pattern
- [The Elm Architecture](https://guide.elm-lang.org/architecture/)
