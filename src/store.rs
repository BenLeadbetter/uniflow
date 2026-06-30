use std::marker::PhantomData;
use std::sync::Arc;

use futures::StreamExt;
use futures::channel::mpsc::{Sender, channel};

use crate::node::{ReadableNode, SourceNode};
use crate::reader::Reader;
use crate::{
    Action, Context, Deps, Dispatch, Effect, EffectReducer, Read, Reducer, Value,
    handle_dispatch_result,
};

pub struct Store<S: Value, A: Action, D: Deps = ()> {
    source: Arc<SourceNode<S>>,
    self_reader: Reader<S>,
    sender: Sender<A>,
    deps: D,
}

impl<S: Value, A: Action> Store<S, A, ()> {
    pub fn new<R: Reducer<S, A>>(state: S, reducer: R) -> Self {
        let effect_reducer =
            move |s: S, a: A| -> (S, Effect<A, ()>) { (reducer(s, a), Effect::none()) };
        Store::<S, A, ()>::new_with_deps_and_capacity(state, effect_reducer, (), 128)
    }

    pub fn new_with_capacity<R: Reducer<S, A>>(state: S, reducer: R, capacity: usize) -> Self {
        let effect_reducer =
            move |s: S, a: A| -> (S, Effect<A, ()>) { (reducer(s, a), Effect::none()) };
        Store::<S, A, ()>::new_with_deps_and_capacity(state, effect_reducer, (), capacity)
    }

    pub fn builder<R: Reducer<S, A>>(
        state: S,
        reducer: R,
    ) -> StoreBuilder<S, A, impl EffectReducer<S, A, ()>, ()> {
        StoreBuilder {
            state,
            reducer: move |s: S, a: A| -> (S, Effect<A, ()>) { (reducer(s, a), Effect::none()) },
            deps: (),
            capacity: 128,
            _action: PhantomData,
        }
    }
}

impl<S: Value, A: Action, D: Deps> Store<S, A, D> {
    pub fn new_with_deps<R: EffectReducer<S, A, D>>(state: S, reducer: R, deps: D) -> Self {
        Self::new_with_deps_and_capacity(state, reducer, deps, 128)
    }

    pub fn builder_with_deps<R: EffectReducer<S, A, D>>(
        state: S,
        reducer: R,
        deps: D,
    ) -> StoreBuilder<S, A, R, D> {
        StoreBuilder {
            state,
            reducer,
            deps,
            capacity: 128,
            _action: PhantomData,
        }
    }

    pub fn new_with_deps_and_capacity<R: EffectReducer<S, A, D>>(
        state: S,
        reducer: R,
        deps: D,
        capacity: usize,
    ) -> Self {
        let source = SourceNode::new(state);
        let self_reader: Reader<S> = Reader::new(source.clone() as Arc<dyn ReadableNode<S>>);
        let (sender, mut receiver) = channel(capacity);
        let reducer_source = source.clone();
        let effect_sender = sender.clone();
        let deps_for_task = deps.clone();
        any_spawner::Executor::spawn(async move {
            while let Some(action) = receiver.next().await {
                let current = reducer_source.get();
                let (new_state, effect) = reducer(current, action);
                reducer_source.set(new_state);

                let ctx = {
                    let sender = effect_sender.clone();
                    let deps = deps_for_task.clone();
                    Context {
                        dispatcher: Arc::new(move |action: A| {
                            let mut s = sender.clone();
                            let result = s.try_send(action);
                            handle_dispatch_result(result);
                        }),
                        deps,
                    }
                };
                effect.run(ctx);
            }
        });
        Self {
            source,
            self_reader,
            sender,
            deps,
        }
    }

    /// Returns a `Context<A, D>` that dispatches into this store.
    pub fn context(&self) -> Context<A, D> {
        let deps = self.deps.clone();
        let sender = self.sender.clone();
        Context {
            dispatcher: Arc::new(move |action: A| {
                let mut s = sender.clone();
                let result = s.try_send(action);
                handle_dispatch_result(result);
            }),
            deps,
        }
    }

    /// Returns a new `Reader<S>` over the full store state with no connections.
    pub fn reader(&self) -> Reader<S> {
        Reader::new(self.source.clone() as Arc<dyn ReadableNode<S>>)
    }

    /// Returns a `Reader<T>` that projects the store state through `f`.
    pub fn derived<T, F>(&self, f: F) -> Reader<T>
    where
        T: Clone + PartialEq + Send + Sync + 'static,
        F: Fn(&S) -> T + Send + Sync + 'static,
    {
        self.reader().map(move |v| f(&v))
    }

    pub fn commit(&self) {
        self.source.send_down();
        self.source.notify();
    }

    pub fn shutdown(&self) {
        let mut sender = self.sender.clone();
        sender.close_channel();
    }
}

impl<S: Value, A: Action, D: Deps> Read<S> for Store<S, A, D> {
    fn get(&self) -> S {
        self.self_reader.get()
    }

    fn watch<F: Fn(&S) + Send + Sync + 'static>(&self, f: F) -> &Self {
        self.self_reader.watch(f);
        self
    }

    fn bind<F: Fn(&S) + Send + Sync + 'static>(&self, f: F) -> &Self {
        self.self_reader.bind(f);
        self
    }

    fn unbind(&self) {
        self.self_reader.unbind();
    }
}

impl<S: Value, A: Action, D: Deps> Dispatch<A> for Store<S, A, D> {
    fn dispatch(&self, action: A) {
        let mut sender = self.sender.clone();
        let result = sender.try_send(action);
        handle_dispatch_result(result);
    }
}

// ── StoreBuilder ──────────────────────────────────────────────────────────────

pub struct StoreBuilder<S, A, R, D = ()> {
    state: S,
    reducer: R,
    deps: D,
    capacity: usize,
    _action: PhantomData<fn(A)>,
}

impl<S, A, R, D> StoreBuilder<S, A, R, D>
where
    S: Value,
    A: Action,
    D: Deps,
    R: EffectReducer<S, A, D>,
{
    /// Applies a middleware by wrapping the current reducer.
    ///
    /// `f` receives the inner reducer and the current initial state, and returns
    /// the wrapped reducer together with the new initial state. Both the action
    /// type (`B`) and the state type (`T`) may differ from the inner types,
    /// enabling middlewares that transform actions (e.g. batch expansion) or
    /// enrich state (e.g. time-travel history).
    ///
    /// For middlewares that leave the state type unchanged, pass it through:
    /// `.wrap(|inner, state| (my_middleware(inner), state))`.
    ///
    /// Wraps are applied left-to-right: the first `.wrap` call produces the
    /// outermost layer that dispatched actions encounter first.
    pub fn wrap<T, B, R2, F>(self, f: F) -> StoreBuilder<T, B, R2, D>
    where
        T: Value,
        B: Action,
        R2: EffectReducer<T, B, D>,
        F: FnOnce(R, S) -> (R2, T),
    {
        let (new_reducer, new_state) = f(self.reducer, self.state);
        StoreBuilder {
            state: new_state,
            reducer: new_reducer,
            deps: self.deps,
            capacity: self.capacity,
            _action: PhantomData,
        }
    }

    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.capacity = capacity;
        self
    }

    pub fn build(self) -> Store<S, A, D> {
        Store::new_with_deps_and_capacity(self.state, self.reducer, self.deps, self.capacity)
    }
}

#[cfg(test)]
fn _assert_send_sync<S: Value, A: Action, D: Deps>() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Store<S, A, D>>();
}
