# Uniflow Roadmap

## Immediate Iterations

Small, incremental steps toward a working library. Each step should compile and add clear value.

### 1. Project Setup
Add dependencies to `Cargo.toml`:
- `reactive_graph` for reactive primitives
- `any_spawner` for pluggable async execution

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

### 6. Effect Type ✅
Implement real `Effect<A, D>`:
- Wraps `Box<dyn FnOnce(Context<A, D>) -> BoxFuture<'static, ()> + Send>`
- `Effect::new(|ctx| async move { ... })` constructor
- `Effect::none()` for no-op effects
- Captures async work that can dispatch actions

### 7. Context Type ✅
Implement `Context<A, D>` for effect dispatch:
- Holds `Sender<A>` and injected `D: Deps` internally
- `ctx.dispatch(action)` sends action to store (sync, non-blocking)
- `ctx.deps()` accessor for injected dependencies
- Clone-able for use across await points

### 8. Channel-Based Dispatch
Convert dispatch to channel-based:
- Store holds `futures::channel::mpsc::UnboundedSender<A>` (runtime-agnostic)
- `dispatch(&self, action)` sends to channel synchronously (non-blocking, thread-safe)
- Safe to call from any thread, including real-time audio threads

### 9. Internal Reducer Task
Spawn the reducer task internally in `Store::new()`:
- Receiver task is spawned via `any_spawner::Executor::spawn()`
- Receives actions sequentially from channel
- Applies reducer and updates signal
- `shutdown()` method closes channel, allowing reducer task to drain and exit
- Works with any executor: tokio in production, synchronous executor for testing
- Future: `new_with_task()` variant for users needing manual task management

### 10. Effect Spawning ✅
Complete the async cycle:
- After reducer returns effect, spawn it via `any_spawner`
- Pass `Context` (with sender + deps) to effect so it can dispatch new actions
- Effects run concurrently while reducer processes sequentially
- `Store::new_with_deps()` and `Store::new_with_deps_and_capacity()` constructors for DI

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

### Custom Scheduler Integration
- Expose `any_spawner` API for injecting custom executors
- Document how apps can use `any_spawner::Executor::init_custom_executor()`
- Provide utilities for common patterns (test executor, single-threaded)
- Leverage Leptos ecosystem compatibility

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
