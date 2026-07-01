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

## Quick Start

```rust
use uniflow::{Dispatch, Read};

// Actions encode the semantics of different operations which
// might be done to the state, normally mutating it into a new value.
enum Action {
    Increment,
    Multiply(u32),
}

// The reducer function is a pure function which mutates state
// according to the provided action.
fn reducer(mut state: u32, action: Action) -> u32 {
    use Action::*;
    match action {
        Increment => {
            state += 1;
        }
        Multiply(m) => {
            state *= m;
        }
    }
    state
}

#[tokio::main]
async fn main() {
    // Uniflow integrates closely with the async runtime.
    // Configured via the any_spawner crate from the Leptos project.
    uniflow::any_spawner::Executor::init_tokio().expect("initialize tokio executor");

    // The store acts as the source of truth for the application data
    let store = uniflow::Store::new(0_u32, reducer);

    // Subscribe to state changes — the callback fires whenever the value is updated.
    store.watch(|value| println!("counter is now {}", value));

    // State is mutated by dispatching actions to the store.
    // Action dispatch is thread-safe and asynchronous.
    store.dispatch(Action::Increment);
    store.dispatch(Action::Increment);
    store.dispatch(Action::Multiply(3));
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    assert_eq!(store.get(), 6);
}
```

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

## Features

### Readers

The `Reader` type is both a push-based signal and a pull-based reference to
some application state. Readers can be zoomed and mapped to create a view
into the core data which updates with new values as they change over time.

```rust
use uniflow::{Read, Write};
use std::sync::{Arc, atomic::{AtomicU32, Ordering}};

#[derive(Clone, PartialEq)]
struct Record {
    age: u32,
    name: String,
}

fn main() {
    let state = uniflow::State::new(Record { age: 33, name: "Dredd".into() });

    // zoom into just the age field
    let age = state.reader().map(|s: Record| s.age);

    // pull the current value from the reader at any time
    assert_eq!(age.get(), 33);

    // register a callback that fires whenever age changes
    let call_count = Arc::new(AtomicU32::new(0));
    let watcher_count = call_count.clone();
    age.watch(move |_| { watcher_count.fetch_add(1, Ordering::Relaxed); });

    // when the watched value changes the callback fires
    state.set(Record { age: 34, ..state.get() });
    assert_eq!(call_count.load(Ordering::Relaxed), 1);

    // when only an unrelated field changes the callback does not fire
    state.set(Record { name: "Halo".into(), ..state.get() });
    assert_eq!(call_count.load(Ordering::Relaxed), 1);
}
```

Several readers can be combined with `uniflow::with` to create a new,
composed reader.

```rust
use uniflow::{Read, Write};
use std::sync::{Arc, atomic::{AtomicU32, Ordering}};

#[derive(Clone, PartialEq)]
struct Rect {
    width: u32,
    height: u32,
    x: u32,
    y: u32,
}

fn main() {
    let state = uniflow::State::new(Rect { width: 800, height: 600, x: 100, y: 200 });

    let width = state.reader().map(|s: Rect| s.width);
    let height = state.reader().map(|s: Rect| s.height);
    let recompute_count = Arc::new(AtomicU32::new(0));
    let area = {
        let count = recompute_count.clone();
        uniflow::with((width, height)).map(move |(width, height)| { 
            count.fetch_add(1, Ordering::Relaxed);
            width * height
        })
    };

    assert_eq!(area.get(), 480_000);
    assert_eq!(recompute_count.load(Ordering::Relaxed), 1);

    // updating the watched state triggers a recomputation
    state.set(Rect { width: 600, ..state.get() });
    assert_eq!(area.get(), 360_000);
    assert_eq!(recompute_count.load(Ordering::Relaxed), 2);

    // updating irrelevant state does not
    state.set(Rect { x: 0, ..state.get() });
    assert_eq!(area.get(), 360_000);
    assert_eq!(recompute_count.load(Ordering::Relaxed), 2);
}
```

`Reader` together with `Context` can be used to create a decoupled API
for a downstream unit, like a GUI widget for example. The unit can then
be tested without the main store — perhaps using `State` — and connected
to the store data in production.

```rust
use uniflow::{Context, Dispatch, Read, Reader};

enum Action { Increment, Decrement, Reset }

fn reducer(state: u32, action: Action) -> u32 {
    match action {
        Action::Increment => state + 1,
        Action::Decrement => state.saturating_sub(1),
        Action::Reset => 0,
    }
}

struct Reset;

// the external contract is only Reader + Context.
// no Store required — swap in a plain State and Context for isolated unit tests.
struct Widget {
    reader: Reader<bool>,
    ctx: Context<Reset>,
}

impl Widget {
    fn is_even(&self) -> bool { self.reader.get() }
    fn reset(&self) { self.ctx.dispatch(Reset); }
}

fn main() {
    uniflow::manual_spawner::init().expect("init");

    let store = uniflow::Store::new(0u32, reducer);
    let widget = Widget {
        reader: store.reader().map(|v| v % 2 == 0),
        ctx: store.context().map(|reset| Action::Reset),
    };

    assert_eq!(widget.is_even(), true);

    store.dispatch(Action::Increment);
    uniflow::manual_spawner::step();
    assert_eq!(widget.is_even(), false);

    widget.reset();
    uniflow::manual_spawner::step();
    assert_eq!(widget.is_even(), true);
}
```

### Effects

Some operations that must follow an action dispatch depend on external,
environmental, or global resources — for example, heavy I/O, network queries,
or ML inference. For such operations we use `uniflow::Effect`.

Effects are spawned into an async task and have access to a
user-defined `Deps` type in addition to the stored application state.
This allows us to define our reducer as a true "pure function"
and keep our application state as plain-old-data while allowing
the production application to do the messy things it needs to do.

```rust
use uniflow::{Context, Dispatch, Effect, Read};

#[derive(Clone, PartialEq)]
struct State { result: Option<u32> }

// Deps are injected at store construction and available inside every effect.
// Keep them out of State so the reducer stays a pure function.
#[derive(Clone, Default)]
struct HeavyDependency {}

impl HeavyDependency {
    fn expensive_io_task(&self) -> u32 {
        42
    }
}

enum Action { Compute, StoreResult(u32) }

fn reducer(state: State, action: Action) -> (State, Effect<Action, HeavyDependency>) {
    match action {
        Action::Compute => (
            state,
            // effect is spawned as an async task
            Effect::new(move |ctx: Context<Action, HeavyDependency>| async move {
                let result = ctx.deps().expensive_io_task();
                ctx.dispatch(Action::StoreResult(result));
            }),
        ),
        Action::StoreResult(r) => (State { result: Some(r) }, Effect::none()),
    }
}

fn main() {
    uniflow::manual_spawner::init().expect("init");

    let store = uniflow::Store::new_with_deps(
        State { result: None },
        reducer,
        HeavyDependency::default(),
    );

    store.dispatch(Action::Compute);
    uniflow::manual_spawner::step();

    assert_eq!(store.get().result, Some(42));
}
```

### Middleware

The main reduce function, action type, and state type can be wrapped —
adding middleware — by using the `StoreBuilder`.

This is handy for adding listeners and hooks that you don't want to
bake into your core application logic. For example, logging, telemetry,
or an undo/redo system.

```rust
use uniflow::{Dispatch, Effect, Read};
use std::sync::{Arc, Mutex};

#[derive(Clone, PartialEq)]
struct State { count: u32 }

#[derive(Debug)]
enum Action { Increment, Add(u32) }

fn reducer(mut state: State, action: Action) -> State {
    match action {
        Action::Increment => state.count += 1,
        Action::Add(n) => state.count += n,
    }
    state
}

fn main() {
    uniflow::manual_spawner::init().expect("init");

    let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let log_mw = log.clone();

    let store = uniflow::Store::builder(State { count: 0 }, reducer)
        .wrap(move |inner, state| {
            let log = log_mw.clone();
            (
                move |s: State, a: Action| -> (State, Effect<Action>) {
                    log.lock().unwrap().push(format!("{a:?}"));
                    inner(s, a)
                },
                state,
            )
        })
        .build();

    store.dispatch(Action::Increment);
    store.dispatch(Action::Add(4));
    uniflow::manual_spawner::step();

    assert_eq!(store.get().count, 5);
    assert_eq!(*log.lock().unwrap(), vec!["Increment", "Add(4)"]);
}
```

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
