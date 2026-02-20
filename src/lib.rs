use futures::StreamExt;
use futures::channel::mpsc::{Sender, TrySendError, channel};
use futures::future::BoxFuture;
use reactive_graph::{
    computed::Memo, owner::Owner, prelude::*, signal::RwSignal, traits::GetUntracked,
};
use std::marker::PhantomData;

#[cfg(test)]
mod executor;

pub use any_spawner;

pub trait State: Clone + PartialEq + Send + Sync + 'static {}

impl<S: Clone + PartialEq + Send + Sync + 'static> State for S {}

pub trait Action: Send + 'static {}

impl<A: Send + 'static> Action for A {}

pub trait Deps: Clone + Send + Sync + 'static {}

impl<D: Clone + Send + Sync + 'static> Deps for D {}

pub trait Reducer<S: State, A: Action>: Fn(S, A) -> S + Send + 'static {}

impl<S: State, A: Action, R: Fn(S, A) -> S + Send + 'static> Reducer<S, A> for R {}

pub trait EffectReducer<S: State, A: Action, D: Deps>:
    Fn(S, A) -> (S, Effect<A, D>) + Send + 'static
{
}

impl<S: State, A: Action, D: Deps, R: Fn(S, A) -> (S, Effect<A, D>) + Send + 'static>
    EffectReducer<S, A, D> for R
{
}

pub struct Context<A: Action, D: Deps = ()> {
    sender: Sender<A>,
    deps: D,
}

impl<A: Action, D: Deps> Clone for Context<A, D> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            deps: self.deps.clone(),
        }
    }
}

impl<A: Action, D: Deps> Context<A, D> {
    pub fn dispatch(&self, action: A) {
        let mut sender = self.sender.clone();
        let result = sender.try_send(action);
        handle_dispatch_result(result);
    }

    pub fn deps(&self) -> &D {
        &self.deps
    }
}

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

    fn run(self, ctx: Context<A, D>) {
        if let Some(f) = self.inner {
            any_spawner::Executor::spawn(f(ctx));
        }
    }
}

pub struct Store<S: State, A: Action, D: Deps = ()> {
    state: RwSignal<S>,
    owner: Owner,
    watch_owner: Owner,
    sender: Sender<A>,
    _deps: PhantomData<D>,
}

pub trait Get<S: State> {
    fn get(&self) -> S;
}

pub trait Watch<S: State> {
    fn watch<F>(&self, callback: F)
    where
        F: Fn(&S) + Send + Sync + 'static;
    fn disconnect(&self);
}

impl<S: State, A: Action> Store<S, A, ()> {
    pub fn new_with_capacity<R: Reducer<S, A>>(state: S, reducer: R, capacity: usize) -> Self {
        let effect_reducer =
            move |s: S, a: A| -> (S, Effect<A, ()>) { (reducer(s, a), Effect::none()) };
        Store::<S, A, ()>::new_with_deps_and_capacity(state, effect_reducer, (), capacity)
    }

    pub fn new<R: Reducer<S, A>>(state: S, reducer: R) -> Self {
        Self::new_with_capacity(state, reducer, 128)
    }
}

impl<S: State, A: Action, D: Deps> Store<S, A, D> {
    pub fn new_with_deps_and_capacity<R: EffectReducer<S, A, D>>(
        state: S,
        reducer: R,
        deps: D,
        capacity: usize,
    ) -> Self {
        let owner = Owner::new();
        let (state, watch_owner) = owner.with(|| {
            let state = RwSignal::new(state);
            let watch_owner = Owner::new();
            (state, watch_owner)
        });
        let (sender, mut receiver) = channel(capacity);
        let reducer_state = state;
        let effect_sender = sender.clone();
        any_spawner::Executor::spawn(async move {
            while let Some(action) = receiver.next().await {
                let (new_state, effect) = (reducer)(reducer_state.get_untracked(), action);
                reducer_state.set(new_state);
                let ctx = Context {
                    sender: effect_sender.clone(),
                    deps: deps.clone(),
                };
                effect.run(ctx);
            }
        });
        Self {
            state,
            owner,
            watch_owner,
            sender,
            _deps: PhantomData,
        }
    }

    pub fn new_with_deps<R: EffectReducer<S, A, D>>(state: S, reducer: R, deps: D) -> Self {
        Self::new_with_deps_and_capacity(state, reducer, deps, 128)
    }

    pub fn dispatch(&mut self, action: A) {
        let result = self.sender.try_send(action);
        handle_dispatch_result(result);
    }

    pub fn shutdown(&mut self) {
        self.sender.close_channel();
    }

    pub fn reader<T, F>(&self, selector: F) -> Reader<T>
    where
        F: Fn(&S) -> T + Send + Sync + 'static,
        T: State,
    {
        let state = self.state;
        self.owner.with(|| {
            let owner = Owner::new();
            let (memo, watch_owner) = owner.with(|| {
                use reactive_graph::traits::Get;
                let memo = Memo::new(move |_| selector(&state.get()));
                let watch_owner = Owner::new();
                (memo, watch_owner)
            });
            Reader {
                memo,
                owner,
                watch_owner,
            }
        })
    }
}

impl<S: State, A: Action, D: Deps> Get<S> for Store<S, A, D> {
    fn get(&self) -> S {
        self.state.get_untracked()
    }
}

impl<S: State, A: Action, D: Deps> Watch<S> for Store<S, A, D> {
    fn watch<F>(&self, callback: F)
    where
        F: Fn(&S) + Send + Sync + 'static,
    {
        let state = self.state;
        self.watch_owner.with(|| {
            use reactive_graph::traits::Get;
            reactive_graph::effect::Effect::watch(
                move || state.get(),
                move |value, _, _| callback(value),
                false,
            );
        });
    }

    fn disconnect(&self) {
        self.watch_owner.cleanup();
    }
}

pub struct Reader<S: State> {
    memo: Memo<S>,
    #[allow(dead_code)]
    owner: Owner,
    watch_owner: Owner,
}

impl<S: State> Watch<S> for Reader<S> {
    fn watch<F>(&self, callback: F)
    where
        F: Fn(&S) + Send + Sync + 'static,
    {
        let memo = self.memo;
        self.watch_owner.with(|| {
            use reactive_graph::traits::Get;
            reactive_graph::effect::Effect::watch(
                move || memo.get(),
                move |value, _, _| callback(value),
                false,
            );
        });
    }

    fn disconnect(&self) {
        self.watch_owner.cleanup();
    }
}

impl<S: State> Get<S> for Reader<S> {
    fn get(&self) -> S {
        self.memo.get_untracked()
    }
}

fn handle_dispatch_result<A>(result: Result<(), TrySendError<A>>) {
    match result {
        Ok(()) => {}
        Err(e) => {
            // channel closed is not an error
            // but if the queue is backed up, that's
            // likely a bug
            debug_assert!(!e.is_full())
        }
    }
}

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
        items: std::vec::Vec<Item>,
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
        let mut store = Store::new(
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

        let mut store = Store::new(
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
    fn reader_reads_current_value_from_state() {
        init_executor();
        let mut store = Store::new(
            ToDo {
                items: vec![Item {
                    what: "Washing up".into(),
                    done: false,
                }],
            },
            reducer,
        );
        let reader = store.reader(|state| state.items[0].clone());
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
    fn watch_reader_calls_callback_on_state_change() {
        use std::sync::{Arc, RwLock};

        init_executor();

        let mut store = Store::new(
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

        let reader = store.reader(|state| state.items[0].clone());
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
    fn disconnect_store_stops_watch_callbacks() {
        use std::sync::{Arc, RwLock};

        init_executor();

        let mut store = Store::new(
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

        store.disconnect();

        store.dispatch(Action::Add("New item".into()));
        executor::tick();
        assert_eq!(*call_count.read().unwrap(), 1); // Still 1, not 2
    }

    #[test]
    fn disconnect_reader_stops_watch_callbacks() {
        use std::sync::{Arc, RwLock};

        init_executor();

        let mut store = Store::new(
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

        let reader = store.reader(|state| state.items[0].clone());
        reader.watch(move |_| {
            *call_count_clone.write().unwrap() += 1;
        });

        executor::tick();
        assert_eq!(*call_count.read().unwrap(), 0);

        store.dispatch(Action::Done(0));
        executor::tick();
        assert_eq!(*call_count.read().unwrap(), 1);

        reader.disconnect();

        store.dispatch(Action::Add("New item".into()));
        executor::tick();
        assert_eq!(*call_count.read().unwrap(), 1); // Still 1, not 2

        assert_eq!(
            reader.get(),
            Item {
                what: "Washing up".into(),
                done: true,
            }
        );
    }

    #[test]
    fn can_rewatch_after_disconnect() {
        use std::sync::{Arc, RwLock};

        init_executor();

        let mut store = Store::new(
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

        store.disconnect();

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

        let mut store = Store::new_with_deps(
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
        executor::tick(); // processes chain: 3 -> effect(2) -> effect(1) -> effect(0)
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

        let mut store = Store::new_with_deps(
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
        executor::tick(); // reducer processes Multiply(5), effect dispatches Set(50)
        executor::tick(); // reducer processes Set(50)
        assert_eq!(store.get(), 50);
    }

    #[test]
    fn effect_none_is_inert() {
        init_executor();

        let mut store = Store::new_with_deps(
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
}
