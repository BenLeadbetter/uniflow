# Uniflow Roadmap

## Completed

### Project Setup ✅
- `any_spawner` for pluggable async execution
- `futures` for runtime-agnostic channels and async utilities
- No dependency on `reactive_graph` or other reactive frameworks

### Core Type Definitions ✅
Foundational types and blanket trait implementations:
- `Value` — `Clone + PartialEq + Send + Sync + 'static`
- `Action` — `Send + 'static`
- `Deps` — `Clone + Send + Sync + 'static`
- `Reducer` and `EffectReducer` function traits

### Node-Based Reactive Primitives ✅
Hand-rolled reactive graph (no `reactive_graph` dependency):
- `SourceNode<T>` — root value cell; notifies watchers on change
- `DerivedNode<T>` — cached derived value, updated lazily on upstream change
- `MergeNode<T>` — combines multiple upstream nodes into a tuple
- `WatchSlot` / `Subscription` — weak-reference watcher lifecycle

### State ✅
Standalone reactive read/write primitive:
- `State::new(value)` — wraps a `SourceNode`
- Implements `Read<T>` (`get`, `watch`, `bind`, `unbind`) and `Write<T>` (`set`)
- `State::reader()` — returns a read-only `Reader<T>` over the same node
- Cloning shares the underlying node; subscriptions are independent per clone

### Reader ✅
Read-only reactive view over any `ReadableNode`:
- `reader.get()` — current value snapshot
- `reader.watch(f)` — fire callback on each change
- `reader.bind(f)` — fire immediately, then watch
- `reader.unbind()` — drop all subscriptions held by this reader
- `reader.map(f)` — derive a new `Reader<U>` via `DerivedNode`
- `with((r1, r2, ...))` / `Merge` trait — combine up to five readers into a tuple reader

### Effect ✅
Async side effects returned alongside new state from the reducer:
- `Effect::new(|ctx| async { ... })` — wraps an async closure
- `Effect::none()` — no-op placeholder
- Spawned via `any_spawner::Executor::spawn` after the reducer runs

### Context ✅
Passed to effects; carries dispatch capability and injected dependencies:
- `ctx.dispatch(action)` — non-blocking send to the store channel
- `ctx.deps()` — access injected `D: Deps`
- `Clone`-able across `await` points
- `ctx.map(f: Fn(B) -> A)` — returns a `Context<B, D>` that maps actions through `f`
  before forwarding. Enables passing a narrowed context to subsystems that only know
  a subset of the store's action type (contravariant, as in lager).

### Channel-Based Dispatch ✅
- Store holds a bounded `futures::channel::mpsc::Sender<A>` (runtime-agnostic)
- `dispatch(&self, action)` sends synchronously; safe from any thread or real-time context
- Capacity defaults to 128; configurable via `Store::new_with_capacity`

### Internal Reducer Task ✅
Spawned in `Store::new()` via `any_spawner::Executor::spawn`:
- Receives actions sequentially; applies reducer; updates `SourceNode`; spawns effects
- `Store::shutdown()` closes the channel; task drains remaining actions and exits

### Store Constructors ✅
- `Store::new(state, reducer)` — simple reducer, no deps
- `Store::new_with_capacity(state, reducer, capacity)` — configurable channel buffer
- `Store::new_with_deps(state, reducer, deps)` — effect reducer with DI
- `Store::new_with_deps_and_capacity(state, reducer, deps, capacity)`

### Watch / Subscribe ✅
- `store.watch(f)` / `store.bind(f)` / `store.unbind()` — via `Read<S>` impl
- `store.reader()` — fresh `Reader<S>` over full state
- `store.derived(f)` — `Reader<T>` projecting state through `f`
- `store.context()` — `Context<A, D>` that dispatches into this store

---

## Long-Term Roadmap

Features for future consideration, roughly ordered by utility:

### Middleware
- Action interception before reducer
- Transform or filter actions
- Logging, analytics, dev tools integration

### Lens-Based Zoom
- Composable readers focused on state subsets
- `reader.zoom(lens)` for nested access
- Type-safe path into deep state structures

### Custom Scheduler Integration
- Document how apps can use `any_spawner::Executor::init_custom_executor()`
- Provide utilities for common patterns (test executor, single-threaded)

### Time-Travel Debugging
- Record action history
- Replay/rewind state
- Integration with external dev tools
