use reactive_graph::{computed::Memo, prelude::*, signal::RwSignal};

pub mod executor;

pub trait State: Clone + PartialEq + Send + Sync + 'static {}

impl<S: Clone + PartialEq + Send + Sync + 'static> State for S {}

pub trait Action: Send + 'static {}

impl<A: Send + 'static> Action for A {}

pub trait Reducer<S: State, A: Action>: Fn(S, A) -> S + 'static {}

impl<S: State, A: Action, R: Fn(S, A) -> S + 'static> Reducer<S, A> for R {}

pub struct Store<S: State, A: Action, R: Reducer<S, A>> {
    state: RwSignal<S>,
    reducer: R,
    action: std::marker::PhantomData<A>,
}

impl<S: State, A: Action, R: Reducer<S, A>> Store<S, A, R> {
    pub fn new(state: S, reducer: R) -> Self {
        Self {
            state: RwSignal::new(state),
            reducer,
            action: Default::default(),
        }
    }

    pub fn get(&self) -> S {
        self.state.get()
    }

    pub fn dispatch(&mut self, action: A) {
        self.state.set((self.reducer)(self.state.get(), action));
    }

    pub fn reader<T, F>(&self, selector: F) -> Reader<T>
    where
        F: Fn(&S) -> T + Send + Sync + 'static,
        T: State,
    {
        let state = self.state;
        Reader {
            memo: Memo::new(move |_| selector(&state.get())),
        }
    }
}

pub struct Reader<S: State> {
    memo: Memo<S>,
}

impl<S: State> Reader<S> {
    pub fn get(&self) -> S {
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
        assert!(store.get().items[0].done);
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
        assert_eq!(
            reader.get(),
            Item {
                what: "Washing up".into(),
                done: true,
            }
        );
    }
}
