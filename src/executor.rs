use futures::executor::{LocalPool, LocalSpawner};
use futures::task::LocalSpawnExt;
use std::cell::RefCell;

thread_local! {
    static LOCAL_POOL: RefCell<LocalPool> = RefCell::new(LocalPool::new());
    static LOCAL_SPAWNER: LocalSpawner = LOCAL_POOL.with(|pool| pool.borrow().spawner());
}

pub struct SynchronousExecutor;

impl any_spawner::CustomExecutor for SynchronousExecutor {
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

pub fn tick() {
    any_spawner::Executor::poll_local();
}

pub fn init_synchronous_executor() -> Result<(), any_spawner::ExecutorError> {
    any_spawner::Executor::init_custom_executor(SynchronousExecutor)
}

pub fn init_tokio_executor() -> Result<(), any_spawner::ExecutorError> {
    any_spawner::Executor::init_tokio()
}
