use uniflow::Watch;

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
    match action {
        Action::Add(what) => {
            state.items.push(Item { what, done: false });
        }
        Action::Done(index) => {
            state.items[index].done = true;
        }
    }
    state
}

fn main() {
    type Store = uniflow::Store<ToDo, Action>;

    uniflow::any_spawner::Executor::init_tokio().expect("initialize tokio executor");

    let rt = tokio::runtime::Runtime::new().unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let mut store = Store::new(ToDo::default(), reducer);

        store.watch(|todo| {
            println!("\n--- Todo List ---");
            for (i, item) in todo.items.iter().enumerate() {
                let check = if item.done { "x" } else { " " };
                println!("  {i}. [{check}] {}", item.what);
            }
            println!("-----------------");
        });

        store.dispatch(Action::Add("Washing up".into()));
        store.dispatch(Action::Add("Haircut".into()));
        store.dispatch(Action::Add("Call mum".into()));
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        store.dispatch(Action::Done(2));
        store.dispatch(Action::Done(0));
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        store.disconnect();
        store.shutdown()
    });
}
