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
}

impl<S: Value, A: Action, D: Deps> Store<S, A, D> {
    pub fn new_with_deps<R: EffectReducer<S, A, D>>(state: S, reducer: R, deps: D) -> Self {
        Self::new_with_deps_and_capacity(state, reducer, deps, 128)
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

#[cfg(test)]
fn _assert_send_sync<S: Value, A: Action, D: Deps>() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Store<S, A, D>>();
}
