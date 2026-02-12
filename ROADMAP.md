# Uniflow Roadmap

## Immediate Iterations

Small, incremental steps toward a working library. Each step should compile and add clear value.

### 1. Project Setup
Add dependencies to `Cargo.toml`:
- `reactive_graph` for reactive primitives
- `tokio` with sync and rt features

Verify the project compiles with dependencies.

### 2. Core Type Definitions
Define the foundational types and trait bounds:
- `Effect<A>` as a placeholder struct (empty for now)
- Document required bounds for Model (`Clone + PartialEq + Send + 'static`)
- Document required bounds for Action (`Send + 'static`)

### 3. Synchronous Store
Implement a basic `Store<M, A>` with:
- `new(initial, reducer)` - creates store with initial state
- `get()` - returns current state snapshot
- `dispatch(action)` - applies reducer directly (blocking, single-threaded)

This gives a working Redux loop without async complexity.

### 4. Signal-Backed State
Replace internal `M` storage with `reactive_graph::signal::RwSignal<M>`:
- State changes trigger reactive graph updates
- `get()` reads from signal
- Dispatch writes to signal after reducer

### 5. Reader with Memo
Implement derived state via `reader()`:
- `Reader<T>` type wrapping `Memo<T>` internally
- `store.reader(|state| state.field)` creates cached derived view
- Reader updates automatically when selected state changes

### 6. Effect Type
Implement real `Effect<A>`:
- Wraps `Box<dyn FnOnce(Context<A>) -> BoxFuture<'static, ()> + Send>`
- `Effect::new(|ctx| async move { ... })` constructor
- Captures async work that can dispatch actions

### 7. Context Type
Implement `Context<A>` for effect dispatch:
- Holds `mpsc::UnboundedSender<A>` internally
- `ctx.dispatch(action)` sends action to store
- Clone-able for use across await points

### 8. Async Channel Dispatch
Convert dispatch to channel-based:
- Store holds `mpsc::UnboundedSender<A>`
- `dispatch()` sends to channel (non-blocking, thread-safe)
- Prepare for separate processing task

### 9. Reducer Task
Implement the main loop pattern:
- `Store::run()` returns a `Future` that processes the action queue
- Receives actions sequentially from channel
- Applies reducer and updates signal
- User spawns this task on their runtime

### 10. Effect Spawning
Complete the async cycle:
- After reducer returns `Some(effect)`, spawn it via `tokio::spawn`
- Pass `Context` to effect so it can dispatch new actions
- Effects run concurrently while reducer processes sequentially

---

## Long-Term Roadmap

Features for future consideration, roughly ordered by utility:

### Watch/Subscribe
- `store.watch(callback)` for imperative subscriptions
- Returns `Subscription` handle for cleanup
- Useful for logging, persistence, debugging

### Middleware
- Action interception before reducer
- Transform or filter actions
- Logging, analytics, dev tools integration

### Lens-Based Zoom
- Composable readers focused on state subsets
- `reader.zoom(lens)` for nested access
- Type-safe path into deep state structures

### Alternative Runtimes
- `EventLoop` trait abstracting the runtime
- Implementations for: tokio (default), async-std, manual/test
- Qt integration for desktop apps

### Cursor (Bidirectional Access)
- Read-write access for form bindings
- `cursor.get()` and `cursor.set(value)`
- Automatically dispatches update actions

### Transducer-Style Transforms
- Chainable transformations on readers
- `reader.map(f).filter(p).dedupe()`
- Composable reactive pipelines

### Batched Updates
- Coalesce multiple dispatches into single state update
- Reduce reactive churn during initialization
- `store.batch(|| { dispatch(a); dispatch(b); })`

### Time-Travel Debugging
- Record action history
- Replay/rewind state
- Integration with external dev tools
