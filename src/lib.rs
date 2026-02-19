use futures::StreamExt;
use futures::channel::mpsc::{Sender, channel};
use reactive_graph::{
    computed::Memo, effect::Effect, owner::Owner, prelude::*, signal::RwSignal,
    traits::Get as RgGet,
};

pub mod executor;

pub trait State: Clone + PartialEq + Send + Sync + 'static {}

impl<S: Clone + PartialEq + Send + Sync + 'static> State for S {}

pub trait Action: Send + 'static {}

impl<A: Send + 'static> Action for A {}

pub trait Reducer<S: State, A: Action>: Fn(S, A) -> S + Send + 'static {}

impl<S: State, A: Action, R: Fn(S, A) -> S + Send + 'static> Reducer<S, A> for R {}

pub struct Store<S: State, A: Action> {
    state: RwSignal<S>,
    owner: Owner,
    watch_owner: Owner,
    sender: Sender<A>,
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

impl<S: State, A: Action> Store<S, A> {
    pub fn new_with_capacity<R: Reducer<S, A>>(state: S, reducer: R, capacity: usize) -> Self {
        let owner = Owner::new();
        let (state, watch_owner) = owner.with(|| {
            let state = RwSignal::new(state);
            let watch_owner = Owner::new();
            (state, watch_owner)
        });
        let (sender, mut receiver) = channel(capacity);
        let reducer_state = state;
        any_spawner::Executor::spawn(async move {
            while let Some(action) = receiver.next().await {
                reducer_state.set((reducer)(reducer_state.get(), action));
            }
        });
        Self {
            state,
            owner,
            watch_owner,
            sender,
        }
    }

    pub fn new<R: Reducer<S, A>>(state: S, reducer: R) -> Self {
        Self::new_with_capacity(state, reducer, 128)
    }

    pub fn dispatch(&mut self, action: A) {
        // TODO: handle the error states here:
        // * the channel is full
        // * the channel is closed
        let _ = self.sender.try_send(action);
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

impl<S: State, A: Action> Get<S> for Store<S, A> {
    fn get(&self) -> S {
        self.state.get()
    }
}

impl<S: State, A: Action> Watch<S> for Store<S, A> {
    fn watch<F>(&self, callback: F)
    where
        F: Fn(&S) + Send + Sync + 'static,
    {
        let state = self.state;
        self.watch_owner.with(|| {
            Effect::watch(
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
            Effect::watch(
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
        self.memo.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static EXECUTOR: std::sync::OnceLock<()> = std::sync::OnceLock::new();

    fn init_executor() {
        EXECUTOR.get_or_init(|| {
            executor::init_synchronous_executor().expect("Initialize global sync executor")
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
}
