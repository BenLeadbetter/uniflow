use futures::executor::{LocalPool, LocalSpawner};
use futures::task::LocalSpawnExt;
use std::cell::RefCell;

thread_local! {
    static LOCAL_POOL: RefCell<LocalPool> = RefCell::new(LocalPool::new());
    static LOCAL_SPAWNER: LocalSpawner = LOCAL_POOL.with(|pool| pool.borrow().spawner());
}

pub(crate) struct ManualExecutor;

impl any_spawner::CustomExecutor for ManualExecutor {
    fn spawn(&self, fut: any_spawner::PinnedFuture<()>) {
        self.spawn_local(fut);
    }

    fn spawn_local(&self, fut: any_spawner::PinnedLocalFuture<()>) {
        LOCAL_SPAWNER.with(|spawner| {
            spawner.spawn_local(fut).unwrap();
        });
    }

    fn poll_local(&self) {
        LOCAL_POOL.with(|pool| {
            pool.borrow_mut().run_until_stalled();
        });
    }
}

/// Initialise the manual spawner as the global async executor.
///
/// Call once before creating any [`Store`](crate::Store). Returns an error if
/// an executor has already been initialised.
///
/// The manual spawner runs all tasks synchronously on the calling thread.
/// After dispatching actions, call [`step`] to settle state.
pub fn init() -> Result<(), any_spawner::ExecutorError> {
    any_spawner::Executor::init_custom_executor(ManualExecutor)
}

/// Drive all pending tasks to completion.
///
/// Runs the store's processing loop and any spawned effects until no further
/// progress can be made. For effects that dispatch follow-up actions, a single
/// `step` processes the entire chain as long as no `.await` point depends on
/// an external event.
pub fn step() {
    any_spawner::Executor::poll_local();
}
