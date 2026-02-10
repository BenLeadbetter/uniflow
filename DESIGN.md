# Uniflow Design Document

A Redux-like, unidirectional dataflow library for Rust, inspired by the [lager](https://github.com/arximboldi/lager) C++ library.

## Goals

- Provide a predictable state container following the Redux/Elm architecture
- Enable unidirectional data flow with immutable state updates
- Support reactive observation of state changes via `Reader` objects
- Allow "zooming" into subsets of state with derived readers
- Integrate with the tokio async runtime for effect execution
- Maintain Rust idioms (ownership, borrowing, Send/Sync constraints)

## Core Concepts

### Unidirectional Data Flow

```
┌─────────────────────────────────────────┐
│         STORE (Model Storage)           │
│   - Holds immutable state (Model)       │
└────────────────┬────────────────────────┘
                 │
         dispatch(Action)
                 │
                 ▼
┌─────────────────────────────────────────┐
│        REDUCER (Pure Function)          │
│  fn(Model, Action) -> Model             │
│  fn(Model, Action) -> (Model, Effect)   │
└────────────────┬────────────────────────┘
                 │
                 ▼
         ┌───────────────────┐
         │  Update complete  │
         │  Notify observers │
         └─────────┬─────────┘
                   │
        ┌──────────┼──────────┐
        ▼          ▼          ▼
     Render   Execute    Dispatch
     Views    Effects    New Actions
```

### Actions

Actions are value types representing events or interactions. In Rust, these are typically enums:

```rust
enum Action {
    Increment,
    Decrement,
    SetValue(i32),
    FetchData { url: String },
}
```

### Model

The model represents the entire application state. It should be `Clone` for efficient snapshot support:

```rust
#[derive(Clone, Default, PartialEq)]
struct AppState {
    counter: i32,
    loading: bool,
    data: Option<String>,
}
```

### Reducer

A pure function that computes the next state given the current state and an action:

```rust
fn reducer(state: AppState, action: Action) -> AppState {
    match action {
        Action::Increment => AppState { counter: state.counter + 1, ..state },
        Action::Decrement => AppState { counter: state.counter - 1, ..state },
        Action::SetValue(v) => AppState { counter: v, ..state },
        Action::FetchData { .. } => AppState { loading: true, ..state },
    }
}
```

### Effects

Side effects are operations that can dispatch new actions. They run asynchronously on the tokio runtime:

```rust
type Effect<A> = Box<dyn FnOnce(Context<A>) -> BoxFuture<'static, ()> + Send>;

// Or with a more ergonomic approach:
async fn fetch_effect(ctx: Context<Action>, url: String) {
    let data = reqwest::get(&url).await.unwrap().text().await.unwrap();
    ctx.dispatch(Action::DataReceived(data));
}
```

### Reducer with Effects

Reducers can return effects alongside the new state:

```rust
fn reducer(state: AppState, action: Action) -> ReducerResult<AppState, Action> {
    match action {
        Action::FetchData { url } => {
            let new_state = AppState { loading: true, ..state };
            let effect = effect(move |ctx| async move {
                let data = fetch_data(&url).await;
                ctx.dispatch(Action::DataReceived(data));
            });
            (new_state, Some(effect))
        }
        _ => (compute_new_state(state, action), None)
    }
}
```

## Store

The `Store` is the central component holding state and coordinating updates.

### API

```rust
pub struct Store<Model, Action> {
    // Internal state, reducer, effect queue, subscribers
}

impl<Model, Action> Store<Model, Action>
where
    Model: Clone + Send + Sync + 'static,
    Action: Send + 'static,
{
    /// Create a new store with initial state and reducer
    pub fn new<R>(initial: Model, reducer: R) -> Self
    where
        R: Fn(Model, Action) -> ReducerResult<Model, Action> + Send + Sync + 'static;

    /// Dispatch an action to the store.
    /// This is safe to call from any thread.
    pub fn dispatch(&self, action: Action);

    /// Get a snapshot of the current state
    pub fn get(&self) -> Model;

    /// Create a Reader for observing state changes
    pub fn reader(&self) -> Reader<Model>;

    /// Create a Reader zoomed into a subset of state
    pub fn zoom<T, L>(&self, lens: L) -> Reader<T>
    where
        L: Lens<Model, T>;
}
```

### Thread Safety

The store must be `Send + Sync` to work across async tasks and threads:

- State is protected by interior mutability (e.g., `Arc<RwLock<Model>>`)
- **`dispatch()` is safe to call from any thread** - actions are sent via a thread-safe channel
- The store processes actions sequentially on the tokio runtime
- Observers are notified after each state update

## Reader

Readers provide reactive, read-only access to derived/zoomed state with automatic change detection.

### API

```rust
pub struct Reader<Model, T> {
    store: /* reference to store */,
    lens: /* lens from Model to T */,
}

impl<Model, T> Reader<Model, T>
where
    Model: Clone + Send + Sync + 'static,
    T: Clone + PartialEq + Send + 'static,
{
    /// Get the current value
    pub fn get(&self) -> T;

    /// Watch for changes, calling the callback only when the value changes.
    /// The callback runs synchronously on the reducer task.
    pub fn watch<F>(&self, callback: F) -> Subscription
    where
        F: Fn(&T) + Send + Sync + 'static;

    /// Create a derived reader by composing lenses
    pub fn zoom<U, L>(&self, lens: L) -> Reader<Model, U>
    where
        L: Lens<T, U>;

    /// Map the value through a transformation
    pub fn map<U, F>(&self, f: F) -> Reader<Model, U>
    where
        F: Fn(&T) -> U + Send + Sync + 'static;
}
```

### Zooming with Lenses

Lenses allow focusing on a part of the state:

```rust
pub trait Lens<S, A> {
    fn get(&self, source: &S) -> A;
}

// Simple field lens via closure
let counter_reader = store.zoom(|state: &AppState| state.counter);

// Further zoom on a reader
let nested_reader = counter_reader.zoom(|count: &i32| *count > 0);
```

### Change Detection

Readers track the previous value and only call the watcher when the zoomed value changes:

```rust
// Internally, Reader::watch does something like:
let mut last: Option<T> = None;
store.watch(move |state| {
    let current = lens.get(state);
    if last.as_ref() != Some(&current) {
        last = Some(current.clone());
        callback(&current);
    }
});
```

This means:
- Watchers on the root store are called on every state update
- Watchers on Readers are only called when the zoomed value changes (via `PartialEq`)

## Event Loop Abstraction

The store is parameterized over an `EventLoop` trait, allowing integration with different runtimes (tokio, Qt, etc.). A tokio implementation is provided behind a feature flag.

### EventLoop Trait

```rust
/// Abstraction over an event loop / async runtime.
/// Implementations handle action dispatch and effect execution.
pub trait EventLoop: Clone + Send + Sync + 'static {
    type ActionSender<A: Send + 'static>: ActionSender<A>;
    type ActionReceiver<A: Send + 'static>: ActionReceiver<A>;

    /// Create a channel for dispatching actions
    fn action_channel<A: Send + 'static>(&self) -> (Self::ActionSender<A>, Self::ActionReceiver<A>);

    /// Spawn an async task (for effects)
    fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static;
}

/// Sender half of the action channel
pub trait ActionSender<A>: Clone + Send + Sync {
    fn send(&self, action: A) -> Result<(), SendError<A>>;
}

/// Receiver half of the action channel
pub trait ActionReceiver<A>: Send {
    async fn recv(&mut self) -> Option<A>;
}
```

### Store Internals

The store holds:
- An action sender for dispatch
- Shared state protected by `Arc<RwLock<Model>>`
- A list of watcher callbacks

Watchers are called **synchronously on the reducer task** after each state update. If a watcher needs to do async work or communicate across threads, it can spawn its own task or set up its own channel internally.

```rust
pub struct Store<Model, Action, E: EventLoop> {
    action_tx: E::ActionSender<Action>,
    state: Arc<RwLock<Model>>,
    watchers: Arc<RwLock<Vec<Box<dyn Fn(&Model) + Send + Sync>>>>,
    event_loop: E,
}

impl<Model, Action, E> Store<Model, Action, E>
where
    Model: Clone + Send + Sync + 'static,
    Action: Send + 'static,
    E: EventLoop,
{
    /// Create a new store with the given event loop
    pub fn new<R>(event_loop: E, initial: Model, reducer: R) -> Self
    where
        R: Fn(Model, Action) -> ReducerResult<Model, Action> + Send + Sync + 'static;

    /// Dispatch an action to the store.
    /// Safe to call from any thread.
    pub fn dispatch(&self, action: Action) {
        self.action_tx.send(action).ok();
    }

    /// Get a snapshot of the current state
    pub fn get(&self) -> Model {
        self.state.read().unwrap().clone()
    }

    /// Register a watcher callback. Called on the reducer task after each update.
    pub fn watch<F>(&self, callback: F) -> Subscription
    where
        F: Fn(&Model) + Send + Sync + 'static;
}
```

## Tokio Event Loop Implementation

Enabled via the `tokio` feature flag (default).

```toml
[features]
default = ["tokio"]
tokio = ["dep:tokio"]
```

### Background: Tokio vs Traditional Event Loops

Unlike Qt or SDL, **tokio does not provide a built-in central event loop** with a "post callback to run on next tick" primitive. Tokio is a task-based async runtime where:

- Tasks are spawned and run concurrently
- There's no single "main loop thread" where callbacks are queued
- Work is distributed across a thread pool (multi-threaded runtime) or run cooperatively (single-threaded)

To implement the `EventLoop` trait for tokio, we need to **construct our own sequential event loop** using tokio primitives:

1. **`tokio::sync::mpsc`** - Provides the action queue (channel)
2. **A dedicated task** - Runs `while let Some(action) = rx.recv().await { ... }` to process actions sequentially
3. **`tokio::spawn`** - Used for spawning effect tasks that run concurrently

This pattern effectively creates a "main loop" within the tokio runtime: a single task that owns the reducer and state, processing actions one at a time in the order they arrive via the channel.

### Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         TokioEventLoop                          │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│   dispatch()  (from any thread)                                 │
│       │                                                         │
│       ▼                                                         │
│   ┌───────────────────┐                                         │
│   │ tokio::sync::mpsc │  Actions channel (unbounded)            │
│   │    (Sender)       │──────────┐                              │
│   └───────────────────┘          │                              │
│                                  ▼                              │
│                         ┌────────────────────┐                  │
│                         │   Reducer Task     │                  │
│                         │  (single async     │                  │
│                         │   task loop)       │                  │
│                         │                    │                  │
│                         │  1. Apply reducer  │                  │
│                         │  2. Update state   │                  │
│                         │  3. Call watchers  │◄── synchronous   │
│                         │  4. Spawn effects  │                  │
│                         └────────────────────┘                  │
│                                                                 │
│   Effects spawned via tokio::spawn()                            │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### Implementation

The `TokioEventLoop` is a lightweight handle. The actual "event loop" is the reducer task
spawned when the store is created—it's just an async task with a `while` loop over the
mpsc receiver.

```rust
#[cfg(feature = "tokio")]
pub mod tokio_loop {
    use tokio::sync::mpsc;

    /// Tokio-based event loop implementation.
    ///
    /// This adaptor constructs a sequential event loop on top of tokio's
    /// task-based runtime using an mpsc channel and a dedicated reducer task.
    #[derive(Clone)]
    pub struct TokioEventLoop;

    impl EventLoop for TokioEventLoop {
        type ActionSender<A: Send + 'static> = mpsc::UnboundedSender<A>;
        type ActionReceiver<A: Send + 'static> = mpsc::UnboundedReceiver<A>;

        fn action_channel<A: Send + 'static>(&self) -> (Self::ActionSender<A>, Self::ActionReceiver<A>) {
            mpsc::unbounded_channel()
        }

        fn spawn<F>(&self, future: F)
        where
            F: Future<Output = ()> + Send + 'static,
        {
            tokio::spawn(future);
        }
    }
}
```

**Note:** The `TokioEventLoop` itself doesn't "run" anything—it just provides the primitives.
The sequential processing happens in the reducer task (spawned by `Store::new`), which loops
over the action channel.

### Reducer Task

The reducer runs as a single async task, ensuring sequential action processing.
Watchers are called synchronously after each state update:

```rust
async fn reducer_task<Model, Action, R, E>(
    mut action_rx: E::ActionReceiver<Action>,
    state: Arc<RwLock<Model>>,
    watchers: Arc<RwLock<Vec<Box<dyn Fn(&Model) + Send + Sync>>>>,
    reducer: R,
    action_tx: E::ActionSender<Action>,
    event_loop: E,
)
where
    Model: Clone + Send + Sync + 'static,
    Action: Send + 'static,
    R: Fn(Model, Action) -> ReducerResult<Model, Action> + Send + Sync + 'static,
    E: EventLoop,
{
    while let Some(action) = action_rx.recv().await {
        // 1. Apply reducer
        let current = state.read().unwrap().clone();
        let (new_state, effect) = reducer(current, action);

        // 2. Update state
        *state.write().unwrap() = new_state;

        // 3. Notify watchers (synchronously on this task)
        {
            let state_ref = state.read().unwrap();
            for watcher in watchers.read().unwrap().iter() {
                watcher(&*state_ref);
            }
        }

        // 4. Spawn effect if any
        if let Some(eff) = effect {
            let ctx = Context::new(action_tx.clone());
            event_loop.spawn(async move {
                eff(ctx).await;
            });
        }
    }
}
```

### Watcher Design Rationale

Watchers are called synchronously on the reducer task because:

1. **Simplicity** - No need for broadcast channels or complex subscription machinery
2. **Predictable ordering** - Watchers see state updates in dispatch order
3. **Flexibility** - Watchers that need async/cross-thread behavior can set up their own channels

If a watcher needs to notify another thread, it can do so explicitly:

```rust
let (tx, rx) = std::sync::mpsc::channel();
store.watch(move |state| {
    tx.send(state.clone()).ok();
});
// rx can be polled on another thread
```

### Context for Effects

```rust
pub struct Context<Action> {
    action_tx: /* ActionSender<Action> */,
}

impl<Action: Send + 'static> Context<Action> {
    pub fn dispatch(&self, action: Action) {
        self.action_tx.send(action).ok();
    }
}
```

## Example Usage

```rust
use uniflow::{Store, effect};
use uniflow::tokio_loop::TokioEventLoop;
use std::time::Duration;

#[derive(Clone, Default, PartialEq)]
struct State {
    count: i32,
    message: String,
}

enum Action {
    Increment,
    Decrement,
    SetMessage(String),
    FetchGreeting,
}

fn reducer(state: State, action: Action) -> (State, Option<Effect<Action>>) {
    match action {
        Action::Increment => (State { count: state.count + 1, ..state }, None),
        Action::Decrement => (State { count: state.count - 1, ..state }, None),
        Action::SetMessage(msg) => (State { message: msg, ..state }, None),
        Action::FetchGreeting => {
            let eff = effect(|ctx| async move {
                let greeting = fetch_greeting().await;
                ctx.dispatch(Action::SetMessage(greeting));
            });
            (state, Some(eff))
        }
    }
}

#[tokio::main]
async fn main() {
    // Create the store with tokio event loop
    let event_loop = TokioEventLoop;
    let store = Store::new(event_loop, State::default(), reducer);

    // Watch for state changes (called synchronously on reducer task)
    store.watch(|state| {
        println!("Count: {}, Message: {}", state.count, state.message);
    });

    // Or watch a derived/zoomed value with change detection
    let mut last_count = None;
    store.watch(move |state| {
        if last_count != Some(state.count) {
            last_count = Some(state.count);
            println!("Count changed: {}", state.count);
        }
    });

    // Dispatch actions (safe from any thread)
    store.dispatch(Action::Increment);
    store.dispatch(Action::Increment);
    store.dispatch(Action::FetchGreeting);

    // Let effects complete
    tokio::time::sleep(Duration::from_secs(1)).await;
}
```

## Future Considerations

### Alternative Event Loop Implementations

The `EventLoop` trait allows for other runtime integrations:

```rust
// Qt integration (hypothetical)
pub struct QtEventLoop {
    // QCoreApplication reference
}

impl EventLoop for QtEventLoop {
    // Use Qt's signal/slot mechanism for channels
    // Use QTimer::singleShot for post()
    // Effects could run on QThreadPool
}

// Manual/synchronous event loop for testing
pub struct ManualEventLoop {
    pending: RefCell<VecDeque<Box<dyn FnOnce()>>>,
}

impl ManualEventLoop {
    pub fn step(&self) {
        // Process one pending item
    }

    pub fn run_all(&self) {
        // Process all pending items
    }
}
```

### Middleware

Support for middleware to intercept and transform actions:

```rust
pub trait Middleware<Action> {
    fn process(&self, action: Action, next: impl Fn(Action)) -> Option<Action>;
}
```

### Cursor (Read-Write Access)

For bidirectional data binding scenarios:

```rust
pub struct Cursor<T> {
    reader: Reader<T>,
    setter: Box<dyn Fn(T) + Send + Sync>,
}

impl<T> Cursor<T> {
    pub fn get(&self) -> T;
    pub fn set(&self, value: T);
    pub fn update(&self, f: impl FnOnce(T) -> T);
}
```

## References

- [Lager C++ Library](https://github.com/arximboldi/lager)
- [Redux](https://redux.js.org/)
- [The Elm Architecture](https://guide.elm-lang.org/architecture/)
