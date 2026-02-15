pub struct SynchronousExecutor;

impl any_spawner::CustomExecutor for SynchronousExecutor {
    fn spawn(&self, fut: any_spawner::PinnedFuture<()>) {
        futures::executor::block_on(fut);
    }

    fn spawn_local(&self, fut: any_spawner::PinnedLocalFuture<()>) {
        futures::executor::block_on(fut);
    }

    fn poll_local(&self) {
        // No-op: futures are executed synchronously when spawned
    }
}

pub fn init_synchronous_executor() -> Result<(), any_spawner::ExecutorError> {
    any_spawner::Executor::init_custom_executor(SynchronousExecutor)
}

pub fn init_tokio_executor() -> Result<(), any_spawner::ExecutorError> {
    any_spawner::Executor::init_tokio()
}
