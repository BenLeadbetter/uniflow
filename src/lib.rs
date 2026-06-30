use std::sync::Arc;

use futures::channel::mpsc::TrySendError;
use futures::future::BoxFuture;

mod node;
mod reader;
mod state;
mod store;
mod subscription;

#[cfg(test)]
mod executor;

pub use any_spawner;
pub use reader::{Merge, Reader, with};
pub use state::State;
pub use store::Store;

pub mod prelude {
    pub use crate::{Dispatch, Read, ReadWrite, Write};
}

// ── Core trait aliases ────────────────────────────────────────────────────────

pub trait Value: Clone + PartialEq + Send + Sync + 'static {}
impl<T: Clone + PartialEq + Send + Sync + 'static> Value for T {}

pub trait Action: Send + 'static {}
impl<A: Send + 'static> Action for A {}

pub trait Deps: Clone + Send + Sync + 'static {}
impl<D: Clone + Send + Sync + 'static> Deps for D {}

pub trait Reducer<S: Value, A: Action>: Fn(S, A) -> S + Send + 'static {}
impl<S: Value, A: Action, R: Fn(S, A) -> S + Send + 'static> Reducer<S, A> for R {}

pub trait EffectReducer<S: Value, A: Action, D: Deps>:
    Fn(S, A) -> (S, Effect<A, D>) + Send + 'static
{
}
impl<S: Value, A: Action, D: Deps, R: Fn(S, A) -> (S, Effect<A, D>) + Send + 'static>
    EffectReducer<S, A, D> for R
{
}

// ── Read trait ────────────────────────────────────────────────────────────────

pub trait Read<T: Value>: Send + Sync {
    fn get(&self) -> T;
    fn watch<F: Fn(&T) + Send + Sync + 'static>(&self, f: F) -> &Self;
    fn bind<F: Fn(&T) + Send + Sync + 'static>(&self, f: F) -> &Self;
    fn unbind(&self);
}

// ── Write trait ───────────────────────────────────────────────────────────────

pub trait Write<T: Value>: Send + Sync {
    fn set(&self, value: T);
}

// ── ReadWrite trait ───────────────────────────────────────────────────────────

pub trait ReadWrite<T: Value>: Read<T> + Write<T> {
    fn update<F: Fn(T) -> T>(&self, f: F) {
        self.set(f(self.get()));
    }
}

impl<T: Value, X: Read<T> + Write<T>> ReadWrite<T> for X {}

// ── Dispatch trait ────────────────────────────────────────────────────────────

pub trait Dispatch<A: Action> {
    fn dispatch(&self, action: A);
}

// ── Context ───────────────────────────────────────────────────────────────────

pub struct Context<A: Action, D: Deps = ()> {
    pub(crate) dispatcher: Arc<dyn Fn(A) + Send + Sync>,
    pub(crate) deps: D,
}

impl<A: Action, D: Deps> Clone for Context<A, D> {
    fn clone(&self) -> Self {
        Self {
            dispatcher: self.dispatcher.clone(),
            deps: self.deps.clone(),
        }
    }
}

impl<A: Action, D: Deps> Context<A, D> {
    pub fn dispatch(&self, action: A) {
        (self.dispatcher)(action);
    }

    pub fn deps(&self) -> &D {
        &self.deps
    }

    /// Returns a new `Context<B, D>` that maps actions `B -> A` before dispatching
    /// to this context. Useful for passing a narrowed context to subsystems that
    /// only know about a subset of the store's action type.
    pub fn map<B, F>(&self, f: F) -> Context<B, D>
    where
        B: Action,
        F: Fn(B) -> A + Send + Sync + 'static,
    {
        let parent = self.dispatcher.clone();
        Context {
            dispatcher: Arc::new(move |b| parent(f(b))),
            deps: self.deps.clone(),
        }
    }
}

impl<A: Action, D: Deps> Dispatch<A> for Context<A, D> {
    fn dispatch(&self, action: A) {
        Context::dispatch(self, action);
    }
}

// ── Effect ────────────────────────────────────────────────────────────────────

pub struct Effect<A: Action, D: Deps = ()> {
    #[allow(clippy::type_complexity)]
    inner: Option<Box<dyn FnOnce(Context<A, D>) -> BoxFuture<'static, ()> + Send>>,
}

impl<A: Action, D: Deps> Effect<A, D> {
    pub fn new<F, Fut>(f: F) -> Self
    where
        F: FnOnce(Context<A, D>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        Self {
            inner: Some(Box::new(move |ctx| Box::pin(f(ctx)))),
        }
    }

    pub fn none() -> Self {
        Self { inner: None }
    }

    pub(crate) fn run(self, ctx: Context<A, D>) {
        if let Some(f) = self.inner {
            any_spawner::Executor::spawn(f(ctx));
        }
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

pub(crate) fn handle_dispatch_result<A>(result: Result<(), TrySendError<A>>) {
    match result {
        Ok(()) => {}
        Err(e) => {
            debug_assert!(!e.is_full())
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    static EXECUTOR: std::sync::OnceLock<()> = std::sync::OnceLock::new();

    fn init_executor() {
        EXECUTOR.get_or_init(|| {
            executor::init_test_executer().expect("Initialize global sync executor")
        });
    }

    #[derive(Clone, Default, Debug, PartialEq)]
    struct Item {
        what: String,
        done: bool,
    }

    #[derive(Clone, Default, Debug, PartialEq)]
    struct ToDo {
        items: Vec<Item>,
    }

    enum Action {
        Add(String),
        Done(usize),
    }

    fn reducer(mut state: ToDo, action: Action) -> ToDo {
        use Action::*;
        match action {
            Add(what) => {
                state.items.push(Item { what, done: false });
            }
            Done(index) => {
                state.items[index].done = true;
            }
        }
        state
    }

    #[test]
    fn get_returns_state() {
        init_executor();
        let store = Store::new(ToDo::default(), reducer);
        assert_eq!(store.get(), ToDo::default());
    }

    #[test]
    fn dispatching_an_action_mutates_the_state() {
        init_executor();
        let store = Store::new(
            ToDo {
                items: vec![Item {
                    what: "Washing up".into(),
                    done: false,
                }],
            },
            reducer,
        );
        store.dispatch(Action::Done(0));
        executor::tick();
        assert!(store.get().items[0].done);
    }

    #[test]
    fn watch_store_calls_callback_on_state_change() {
        use std::sync::{Arc, RwLock};

        init_executor();

        let store = Store::new(
            ToDo {
                items: vec![Item {
                    what: "Washing up".into(),
                    done: false,
                }],
            },
            reducer,
        );

        let received: Arc<RwLock<Option<ToDo>>> = Arc::new(RwLock::new(None));
        let received_clone = received.clone();

        store.watch(move |todo| {
            *received_clone.write().unwrap() = Some(todo.clone());
        });

        executor::tick();
        assert!(received.read().unwrap().is_none());

        store.dispatch(Action::Done(0));
        executor::tick();

        assert_eq!(
            *received.read().unwrap(),
            Some(ToDo {
                items: vec![Item {
                    what: "Washing up".into(),
                    done: true,
                }],
            })
        );
    }

    #[test]
    fn derived_reads_current_value_from_state() {
        init_executor();
        let store = Store::new(
            ToDo {
                items: vec![Item {
                    what: "Washing up".into(),
                    done: false,
                }],
            },
            reducer,
        );
        let reader = store.derived(|state| state.items[0].clone());
        assert_eq!(
            reader.get(),
            Item {
                what: "Washing up".into(),
                done: false,
            }
        );
        store.dispatch(Action::Done(0));
        executor::tick();
        assert_eq!(
            reader.get(),
            Item {
                what: "Washing up".into(),
                done: true,
            }
        );
    }

    #[test]
    fn watch_derived_calls_callback_on_state_change() {
        use std::sync::{Arc, RwLock};

        init_executor();

        let store = Store::new(
            ToDo {
                items: vec![Item {
                    what: "Washing up".into(),
                    done: false,
                }],
            },
            reducer,
        );

        let received: Arc<RwLock<Option<Item>>> = Arc::new(RwLock::new(None));
        let received_clone = received.clone();

        let reader = store.derived(|state| state.items[0].clone());
        reader.watch(move |item| {
            *received_clone.write().unwrap() = Some(item.clone());
        });

        executor::tick();
        assert!(received.read().unwrap().is_none());

        store.dispatch(Action::Done(0));
        executor::tick();

        assert_eq!(
            *received.read().unwrap(),
            Some(Item {
                what: "Washing up".into(),
                done: true,
            })
        );
    }

    #[test]
    fn unbind_store_stops_watch_callbacks() {
        use std::sync::{Arc, RwLock};

        init_executor();

        let store = Store::new(
            ToDo {
                items: vec![Item {
                    what: "Washing up".into(),
                    done: false,
                }],
            },
            reducer,
        );

        let call_count: Arc<RwLock<usize>> = Arc::new(RwLock::new(0));
        let call_count_clone = call_count.clone();

        store.watch(move |_| {
            *call_count_clone.write().unwrap() += 1;
        });

        executor::tick();
        assert_eq!(*call_count.read().unwrap(), 0);

        store.dispatch(Action::Done(0));
        executor::tick();
        assert_eq!(*call_count.read().unwrap(), 1);

        store.unbind();

        store.dispatch(Action::Add("New item".into()));
        executor::tick();
        assert_eq!(*call_count.read().unwrap(), 1); // still 1
    }

    #[test]
    fn unbind_reader_stops_watch_callbacks() {
        use std::sync::{Arc, RwLock};

        init_executor();

        let store = Store::new(
            ToDo {
                items: vec![Item {
                    what: "Washing up".into(),
                    done: false,
                }],
            },
            reducer,
        );

        let call_count: Arc<RwLock<usize>> = Arc::new(RwLock::new(0));
        let call_count_clone = call_count.clone();

        let reader = store.derived(|state| state.items[0].clone());
        reader.watch(move |_| {
            *call_count_clone.write().unwrap() += 1;
        });

        executor::tick();
        assert_eq!(*call_count.read().unwrap(), 0);

        store.dispatch(Action::Done(0));
        executor::tick();
        assert_eq!(*call_count.read().unwrap(), 1);

        reader.unbind();

        store.dispatch(Action::Add("New item".into()));
        executor::tick();
        assert_eq!(*call_count.read().unwrap(), 1); // still 1

        assert_eq!(
            reader.get(),
            Item {
                what: "Washing up".into(),
                done: true,
            }
        );
    }

    #[test]
    fn can_rewatch_after_unbind() {
        use std::sync::{Arc, RwLock};

        init_executor();

        let store = Store::new(
            ToDo {
                items: vec![Item {
                    what: "Washing up".into(),
                    done: false,
                }],
            },
            reducer,
        );

        let call_count: Arc<RwLock<usize>> = Arc::new(RwLock::new(0));
        let call_count_clone = call_count.clone();

        store.watch({
            let call_count = call_count.clone();
            move |_| {
                *call_count.write().unwrap() += 1;
            }
        });

        executor::tick();
        assert_eq!(*call_count.read().unwrap(), 0);

        store.dispatch(Action::Done(0));
        executor::tick();
        assert_eq!(*call_count.read().unwrap(), 1);

        store.unbind();

        store.watch(move |_| {
            *call_count_clone.write().unwrap() += 1;
        });

        executor::tick();
        assert_eq!(*call_count.read().unwrap(), 1);

        store.dispatch(Action::Add("New item".into()));
        executor::tick();
        assert_eq!(*call_count.read().unwrap(), 2);
    }

    #[test]
    fn effect_dispatches_follow_up_action() {
        init_executor();

        let store = Store::new_with_deps(
            0i32,
            |state: i32, action: i32| -> (i32, Effect<i32>) {
                if action > 0 {
                    let follow_up = action - 1;
                    (
                        state + action,
                        Effect::new(move |ctx: Context<i32, ()>| async move {
                            ctx.dispatch(follow_up);
                        }),
                    )
                } else {
                    (state + action, Effect::none())
                }
            },
            (),
        );

        store.dispatch(3);
        executor::tick();
        assert_eq!(store.get(), 6); // 3 + 2 + 1 + 0
    }

    #[test]
    fn effect_accesses_injected_deps() {
        #[derive(Clone)]
        struct TestDeps {
            multiplier: i32,
        }

        #[derive(Clone, Debug, PartialEq)]
        enum CountAction {
            Multiply(i32),
            Set(i32),
        }

        init_executor();

        let store = Store::new_with_deps(
            0i32,
            |state: i32, action: CountAction| -> (i32, Effect<CountAction, TestDeps>) {
                match action {
                    CountAction::Multiply(val) => (
                        state,
                        Effect::new(move |ctx: Context<CountAction, TestDeps>| async move {
                            let result = val * ctx.deps().multiplier;
                            ctx.dispatch(CountAction::Set(result));
                        }),
                    ),
                    CountAction::Set(val) => (val, Effect::none()),
                }
            },
            TestDeps { multiplier: 10 },
        );

        store.dispatch(CountAction::Multiply(5));
        executor::tick();
        executor::tick();
        assert_eq!(store.get(), 50);
    }

    #[test]
    fn effect_none_is_inert() {
        init_executor();

        let store = Store::new_with_deps(
            0i32,
            |state: i32, action: i32| -> (i32, Effect<i32>) { (state + action, Effect::none()) },
            (),
        );

        store.dispatch(5);
        executor::tick();
        assert_eq!(store.get(), 5);

        store.dispatch(3);
        executor::tick();
        assert_eq!(store.get(), 8);
    }

    #[test]
    fn store_reader_returns_full_state_reader() {
        init_executor();
        let store = Store::new(ToDo::default(), reducer);
        let reader = store.reader();
        assert_eq!(reader.get(), ToDo::default());
        store.dispatch(Action::Add("Task".into()));
        executor::tick();
        assert_eq!(reader.get().items.len(), 1);
    }

    #[test]
    fn multiple_independent_subscriptions() {
        use std::sync::{Arc, RwLock};

        init_executor();

        let store = Store::new(
            ToDo {
                items: vec![Item {
                    what: "Task".into(),
                    done: false,
                }],
            },
            reducer,
        );

        let count_a: Arc<RwLock<usize>> = Arc::new(RwLock::new(0));
        let count_b: Arc<RwLock<usize>> = Arc::new(RwLock::new(0));

        let reader_a = store.reader();
        let reader_b = reader_a.clone();

        let ca = count_a.clone();
        let cb = count_b.clone();
        reader_a.watch(move |_| *ca.write().unwrap() += 1);
        reader_b.watch(move |_| *cb.write().unwrap() += 1);

        store.dispatch(Action::Done(0));
        executor::tick();
        assert_eq!(*count_a.read().unwrap(), 1);
        assert_eq!(*count_b.read().unwrap(), 1);

        drop(reader_a); // disconnect a

        store.dispatch(Action::Add("New".into()));
        executor::tick();
        assert_eq!(*count_a.read().unwrap(), 1); // still 1
        assert_eq!(*count_b.read().unwrap(), 2); // still active
    }

    #[test]
    fn with_on_two_derived_readers() {
        use std::sync::{Arc, RwLock};

        init_executor();

        let store = Store::new(
            ToDo {
                items: vec![
                    Item {
                        what: "A".into(),
                        done: false,
                    },
                    Item {
                        what: "B".into(),
                        done: false,
                    },
                ],
            },
            reducer,
        );

        let r0 = store.derived(|s| s.items[0].done);
        let r1 = store.derived(|s| s.items[1].done);
        let combined = with((r0, r1));

        let received: Arc<RwLock<Option<(bool, bool)>>> = Arc::new(RwLock::new(None));
        let rc = received.clone();
        combined.watch(move |v| *rc.write().unwrap() = Some(*v));

        store.dispatch(Action::Done(0));
        executor::tick();
        assert_eq!(*received.read().unwrap(), Some((true, false)));

        store.dispatch(Action::Done(1));
        executor::tick();
        assert_eq!(*received.read().unwrap(), Some((true, true)));
    }

    #[test]
    fn store_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Store<ToDo, Action>>();
    }

    fn channel_context<A: crate::Action>(sender: futures::channel::mpsc::Sender<A>) -> Context<A> {
        Context {
            dispatcher: Arc::new(move |action: A| {
                let mut s = sender.clone();
                let result = s.try_send(action);
                handle_dispatch_result(result);
            }),
            deps: (),
        }
    }

    #[test]
    fn mapped_context_dispatches_via_parent() {
        use futures::channel::mpsc::channel;

        let (sender, mut receiver) = channel::<i32>(10);
        let ctx = channel_context(sender);
        let mapped: Context<&'static str> = ctx.map(|s: &'static str| s.len() as i32);
        mapped.dispatch("hello"); // len == 5
        assert_eq!(receiver.try_recv().unwrap(), 5);
    }

    #[test]
    fn mapped_context_shares_deps() {
        use futures::channel::mpsc::channel;

        #[derive(Clone)]
        struct MyDeps {
            value: i32,
        }

        let (sender, _) = channel::<i32>(10);
        let base = channel_context(sender);
        let ctx: Context<i32, MyDeps> = Context {
            dispatcher: base.dispatcher,
            deps: MyDeps { value: 42 },
        };
        let mapped: Context<bool, MyDeps> = ctx.map(|b: bool| if b { 1 } else { 0 });
        assert_eq!(mapped.deps().value, 42);
    }

    #[test]
    fn mapped_context_can_map_again() {
        use futures::channel::mpsc::channel;

        let (sender, mut receiver) = channel::<i32>(10);
        let ctx = channel_context(sender);
        let mid: Context<bool> = ctx.map(|b: bool| if b { 1 } else { 0 });
        let leaf: Context<&'static str> = mid.map(|s: &'static str| s == "yes");
        leaf.dispatch("yes"); // "yes" == "yes" → true → 1
        assert_eq!(receiver.try_recv().unwrap(), 1);
    }
}
