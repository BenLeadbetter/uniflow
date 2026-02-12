pub trait State: Clone + PartialEq + Send + 'static {}

impl<S: Clone + PartialEq + Send + 'static> State for S {}

pub trait Action: Send + 'static {}

impl<A: Send + 'static> Action for A {}

pub trait Reducer<S: State, A: Action>: Fn(S, A) -> S + 'static {}

impl<S: State, A: Action, R: Fn(S, A) -> S + 'static> Reducer<S, A> for R {}

pub struct Store<S: State, A: Action, R: Reducer<S, A>> {
    state: Option<S>,
    reducer: R,
    action: std::marker::PhantomData<A>,
}

impl<S: State, A: Action, R: Reducer<S, A>> Store<S, A, R> {
    pub fn new(state: S, reducer: R) -> Self {
        Self {
            state: Some(state),
            reducer,
            action: Default::default(),
        }
    }

    pub fn get(&self) -> S {
        self.state.clone().unwrap()
    }

    pub fn dispatch(&mut self, action: A) {
        self.state = Some((self.reducer)(self.state.take().unwrap(), action));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Default, Debug, PartialEq)]
    struct State {
        counter: i32,
    }

    enum Action {
        Increment,
    }

    fn reducer(mut state: State, action: Action) -> State {
        use Action::*;
        match action {
            Increment => {
                state.counter += 1;
            }
        }
        state
    }

    #[test]
    fn get_returns_state() {
        let store = Store::new(State::default(), reducer);
        assert_eq!(store.get(), State::default());
    }

    #[test]
    fn dispatching_an_action_mutates_the_state() {
        let mut store = Store::new(State::default(), reducer);
        store.dispatch(Action::Increment);
        assert_eq!(store.get(), State { counter: 1 });
    }
}
