//! Demonstrates Effect::task, Effect::batch, and Effect::map
//! by fetching posts and users concurrently from the public JSONPlaceholder API.
//!
//! Run with:  cargo run --example effect

use std::sync::Arc;
use uniflow::{Dispatch, Effect, Read, Store, any_spawner};

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
struct Post {
    id: u32,
    title: String,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
struct User {
    id: u32,
    name: String,
    email: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct State {
    posts: Vec<Post>,
    users: Vec<User>,
}

#[derive(Clone, Debug)]
enum Action {
    Fetch,
    PostsFetched(Vec<Post>),
    UsersFetched(Vec<User>),
}

// Helper functions return Effect<T> rather than Effect<Action>, keeping them
// decoupled from the root action type. Effect::map at the call site does the
// lifting.

fn fetch_posts() -> Effect<Vec<Post>> {
    Effect::task(async {
        reqwest::get("https://jsonplaceholder.typicode.com/posts")
            .await
            .expect("GET /posts")
            .json()
            .await
            .expect("deserialize posts")
    })
}

fn fetch_users() -> Effect<Vec<User>> {
    Effect::task(async {
        reqwest::get("https://jsonplaceholder.typicode.com/users")
            .await
            .expect("GET /users")
            .json()
            .await
            .expect("deserialize users")
    })
}

fn reducer(state: State, action: Action) -> (State, Effect<Action>) {
    match action {
        Action::Fetch => (
            state,
            // concurrently dispatch two different web requests
            Effect::batch([
                fetch_posts().map(Action::PostsFetched),
                fetch_users().map(Action::UsersFetched),
            ]),
        ),
        Action::PostsFetched(posts) => (State { posts, ..state }, Effect::none()),
        Action::UsersFetched(users) => (State { users, ..state }, Effect::none()),
    }
}

#[tokio::main]
async fn main() {
    any_spawner::Executor::init_tokio().expect("init tokio executor");

    let store = Store::builder_with_effects(State::default(), reducer)
        .wrap(|inner_reducer, state| {
            (
                move |state, action| {
                    // logging middleware
                    match action {
                        Action::PostsFetched(_) => println!("Posts fetched!"),
                        Action::UsersFetched(_) => println!("Users fetched!"),
                        _ => (),
                    }
                    inner_reducer(state, action)
                },
                state,
            )
        })
        .build();

    let done = Arc::new(tokio::sync::Notify::new());
    {
        let done = done.clone();
        store.watch(move |state: &State| {
            if !state.posts.is_empty() && !state.users.is_empty() {
                done.notify_one();
            }
        });
    }

    store.dispatch(Action::Fetch);
    done.notified().await;
}
