use std::sync::{Arc, Weak};

#[allow(dead_code)]
pub(crate) struct Subscription(Arc<()>);

impl Subscription {
    pub(crate) fn new() -> (Self, Weak<()>) {
        let inner = Arc::new(());
        let weak = Arc::downgrade(&inner);
        (Subscription(inner), weak)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_returns_live_weak() {
        let (_sub, weak) = Subscription::new();
        assert!(weak.upgrade().is_some());
    }

    #[test]
    fn drop_makes_weak_dead() {
        let (sub, weak) = Subscription::new();
        drop(sub);
        assert!(weak.upgrade().is_none());
    }
}
