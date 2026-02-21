use uniflow::Get;

enum Action {
    Increment,
    Multiply(u32),
}

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
    uniflow::any_spawner::Executor::init_tokio().expect("initialize tokio executor");

    let mut store = uniflow::Store::new(0_u32, reducer);

    store.dispatch(Action::Increment);
    store.dispatch(Action::Increment);
    store.dispatch(Action::Multiply(3));
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    println!("state: {}", store.get());

    store.shutdown();
}
