pub fn tick() {
    crate::manual_spawner::step();
}

pub fn init_test_executer() -> Result<(), any_spawner::ExecutorError> {
    crate::manual_spawner::init()
}
