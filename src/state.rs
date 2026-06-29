use std::sync::Arc;

use crate::node::{ReadableNode, SourceNode};
use crate::reader::Reader;
use crate::{Read, Value, Write};

pub struct State<T: Value> {
    source: Arc<SourceNode<T>>,
    reader: Reader<T>,
}

impl<T: Value> State<T> {
    pub fn new(value: T) -> Self {
        let source = SourceNode::new(value);
        let reader = Reader::new(source.clone() as Arc<dyn ReadableNode<T>>);
        State { source, reader }
    }

    pub fn reader(&self) -> Reader<T> {
        Reader::new(self.source.clone() as Arc<dyn ReadableNode<T>>)
    }
}

impl<T: Value> Write<T> for State<T> {
    fn set(&self, value: T) {
        self.source.set(value);
    }
}

impl<T: Value> Read<T> for State<T> {
    fn get(&self) -> T {
        self.reader.get()
    }

    fn watch<F: Fn(&T) + Send + Sync + 'static>(&self, f: F) -> &Self {
        self.reader.watch(f);
        self
    }

    fn bind<F: Fn(&T) + Send + Sync + 'static>(&self, f: F) -> &Self {
        self.reader.bind(f);
        self
    }

    fn unbind(&self) {
        self.reader.unbind();
    }
}

impl<T: Value> Clone for State<T> {
    fn clone(&self) -> Self {
        State {
            source: self.source.clone(),
            reader: self.reader.clone(),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Read, ReadWrite, Write};
    use std::sync::{Arc, Mutex};

    #[test]
    fn new_has_initial_value() {
        let s = State::new(42i32);
        assert_eq!(s.get(), 42);
    }

    #[test]
    fn set_updates_value() {
        let s = State::new(0i32);
        s.set(99);
        assert_eq!(s.get(), 99);
    }

    #[test]
    fn set_no_op_on_equal_value() {
        let s = State::new(42i32);
        let calls = Arc::new(Mutex::new(vec![]));
        let c = calls.clone();
        s.watch(move |v| c.lock().unwrap().push(*v));
        s.set(42);
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn watch_fires_on_set() {
        let s = State::new(0i32);
        let calls = Arc::new(Mutex::new(vec![]));
        let c = calls.clone();
        s.watch(move |v| c.lock().unwrap().push(*v));
        s.set(1);
        assert_eq!(*calls.lock().unwrap(), vec![1]);
    }

    #[test]
    fn bind_fires_immediately_then_watches() {
        let s = State::new(5i32);
        let calls = Arc::new(Mutex::new(vec![]));
        let c = calls.clone();
        s.bind(move |v| c.lock().unwrap().push(*v));
        assert_eq!(*calls.lock().unwrap(), vec![5]);
        s.set(10);
        assert_eq!(*calls.lock().unwrap(), vec![5, 10]);
    }

    #[test]
    fn unbind_stops_callbacks() {
        let s = State::new(0i32);
        let calls = Arc::new(Mutex::new(vec![]));
        let c = calls.clone();
        s.watch(move |v| c.lock().unwrap().push(*v));
        s.set(1);
        s.unbind();
        s.set(2);
        assert_eq!(*calls.lock().unwrap(), vec![1]);
    }

    #[test]
    fn clone_shares_node() {
        let s = State::new(42i32);
        let cloned = s.clone();
        assert_eq!(cloned.get(), 42);
        s.set(99);
        assert_eq!(cloned.get(), 99);
    }

    #[test]
    fn clone_write_affects_original() {
        let s = State::new(0i32);
        let cloned = s.clone();
        cloned.set(99);
        assert_eq!(s.get(), 99);
    }

    #[test]
    fn clone_connections_are_independent() {
        let s = State::new(0i32);
        let cloned = s.clone();
        let calls = Arc::new(Mutex::new(vec![]));
        let c = calls.clone();
        s.watch(move |v| c.lock().unwrap().push(*v));
        drop(cloned); // dropping the clone must not unbind s's watcher
        s.set(1);
        assert_eq!(*calls.lock().unwrap(), vec![1]);
    }

    #[test]
    fn reader_is_read_only_view() {
        let s = State::new(10i32);
        let r = s.reader();
        assert_eq!(r.get(), 10);
        s.set(20);
        assert_eq!(r.get(), 20);
    }

    #[test]
    fn reader_watches_state_changes() {
        let s = State::new(0i32);
        let r = s.reader();
        let calls = Arc::new(Mutex::new(vec![]));
        let c = calls.clone();
        r.watch(move |v| c.lock().unwrap().push(*v));
        s.set(7);
        assert_eq!(*calls.lock().unwrap(), vec![7]);
    }

    #[test]
    fn update_applies_function() {
        let s = State::new(10i32);
        s.update(|x| x + 1);
        assert_eq!(s.get(), 11);
    }

    #[test]
    fn state_implements_readwrite() {
        fn assert_readwrite<T: Value, S: ReadWrite<T>>(_: &S) {}
        let s = State::new(0i32);
        assert_readwrite(&s);
    }
}
