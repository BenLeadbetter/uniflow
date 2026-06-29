use std::sync::{Arc, Mutex};

use crate::node::{DerivedNode, MergeNode, ReadableNode, WatchSlot};
use crate::subscription::Subscription;
use crate::{Read, State};

pub struct Reader<T>
where
    T: Clone + PartialEq + Send + Sync + 'static,
{
    pub(crate) node: Arc<dyn ReadableNode<T>>,
    connections: Mutex<Vec<Subscription>>,
}

impl<T: Clone + PartialEq + Send + Sync + 'static> Reader<T> {
    pub(crate) fn new(node: Arc<dyn ReadableNode<T>>) -> Self {
        Reader {
            node,
            connections: Mutex::new(Vec::new()),
        }
    }

    pub fn map<U, F>(&self, f: F) -> Reader<U>
    where
        U: Clone + PartialEq + Send + Sync + 'static,
        F: Fn(T) -> U + Send + Sync + 'static,
    {
        Reader::new(DerivedNode::new(self.node.clone(), f))
    }
}

impl<T: Clone + PartialEq + Send + Sync + 'static> Read<T> for Reader<T> {
    fn get(&self) -> T {
        self.node.get()
    }

    fn watch<F: Fn(&T) + Send + Sync + 'static>(&self, f: F) -> &Self {
        let (sub, weak) = Subscription::new();
        self.node.add_watcher(WatchSlot {
            alive: weak,
            callback: Arc::new(f),
        });
        self.connections.lock().unwrap().push(sub);
        self
    }

    fn bind<F: Fn(&T) + Send + Sync + 'static>(&self, f: F) -> &Self {
        f(&self.get());
        self.watch(f)
    }

    fn unbind(&self) {
        self.connections.lock().unwrap().clear();
    }
}

impl<T: Clone + PartialEq + Send + Sync + 'static> Clone for Reader<T> {
    fn clone(&self) -> Self {
        Reader {
            node: self.node.clone(),
            connections: Mutex::new(Vec::new()),
        }
    }
}

// ── Merge trait ───────────────────────────────────────────────────────────────

/// Implemented on tuples of [`Reader`]s. Merge the tuple into a single reader
/// that yields the corresponding state tuple and updates whenever any source
/// reader changes.
///
/// Implemented for tuples of 2–5 readers. Use [`with`] as a free-function
/// shorthand.
pub trait Merge {
    type Combined: State;
    fn merge(self) -> Reader<Self::Combined>;
}

macro_rules! impl_merge {
    ($(($T:ident, $r:ident)),+ $(,)?) => {
        impl<$($T: State),+> Merge for ($(Reader<$T>,)+) {
            type Combined = ($($T,)+);
            fn merge(self) -> Reader<($($T,)+)> {
                let ($($r,)+) = self;
                Reader::new(MergeNode::<($($T,)+)>::new(($($r.node,)+)))
            }
        }
    };
}

impl_merge!((A, a), (B, b));
impl_merge!((A, a), (B, b), (C, c));
impl_merge!((A, a), (B, b), (C, c), (D, d));
impl_merge!((A, a), (B, b), (C, c), (D, d), (E, e));

/// Combine a tuple of readers into a single reader. See [`Merge`].
pub fn with<R: Merge>(readers: R) -> Reader<R::Combined> {
    readers.merge()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Read;
    use crate::node::SourceNode;
    use std::sync::{Arc, Mutex};

    fn source_reader<T: Clone + PartialEq + Send + Sync + 'static>(
        value: T,
    ) -> (Arc<SourceNode<T>>, Reader<T>) {
        let source = SourceNode::new(value);
        let reader = Reader::new(source.clone() as Arc<dyn ReadableNode<T>>);
        (source, reader)
    }

    #[test]
    fn get_returns_value() {
        let (_src, reader) = source_reader(42i32);
        assert_eq!(reader.get(), 42);
    }

    #[test]
    fn watch_fires_on_change() {
        let (source, reader) = source_reader(0i32);
        let calls = Arc::new(Mutex::new(vec![]));
        let calls2 = calls.clone();
        reader.watch(move |v| calls2.lock().unwrap().push(*v));
        source.set(1);
        assert_eq!(*calls.lock().unwrap(), vec![1]);
    }

    #[test]
    fn watch_connection_clears_on_reader_drop() {
        let (source, reader) = source_reader(0i32);
        let calls = Arc::new(Mutex::new(vec![]));
        let calls2 = calls.clone();
        reader.watch(move |v| calls2.lock().unwrap().push(*v));
        drop(reader);
        source.set(1);
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn unbind_stops_callbacks() {
        let (source, reader) = source_reader(0i32);
        let calls = Arc::new(Mutex::new(vec![]));
        let calls2 = calls.clone();
        reader.watch(move |v| calls2.lock().unwrap().push(*v));
        source.set(1);
        reader.unbind();
        source.set(2);
        assert_eq!(*calls.lock().unwrap(), vec![1]);
    }

    #[test]
    fn bind_fires_immediately_then_watches() {
        let (source, reader) = source_reader(5i32);
        let calls = Arc::new(Mutex::new(vec![]));
        let calls2 = calls.clone();
        reader.bind(move |v| calls2.lock().unwrap().push(*v));
        assert_eq!(*calls.lock().unwrap(), vec![5]); // immediate
        source.set(10);
        assert_eq!(*calls.lock().unwrap(), vec![5, 10]); // then watches
    }

    #[test]
    fn clone_shares_node_not_connections() {
        let (source, reader) = source_reader(0i32);
        let cloned = reader.clone();
        // Both see the same value
        assert_eq!(reader.get(), cloned.get());
        // Connections are independent
        let calls_a = Arc::new(Mutex::new(vec![]));
        let calls_b = Arc::new(Mutex::new(vec![]));
        let ca = calls_a.clone();
        let cb = calls_b.clone();
        reader.watch(move |v| ca.lock().unwrap().push(*v));
        cloned.watch(move |v| cb.lock().unwrap().push(*v));
        drop(reader); // disconnects reader's callback
        source.set(1);
        // reader's callback is gone, cloned's is alive
        assert!(calls_a.lock().unwrap().is_empty());
        assert_eq!(*calls_b.lock().unwrap(), vec![1]);
    }

    #[test]
    fn clone_watch_is_independent_scope() {
        let (source, reader) = source_reader(0i32);
        let scoped = reader.clone();
        let calls = Arc::new(Mutex::new(vec![]));
        let c = calls.clone();
        scoped.watch(move |v| c.lock().unwrap().push(*v));
        drop(scoped); // this scope's subscription dies
        source.set(1);
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn map_creates_derived_reader() {
        let (source, reader) = source_reader(3i32);
        let mapped = reader.map(|v| v * 10);
        assert_eq!(mapped.get(), 30);
        let calls = Arc::new(Mutex::new(vec![]));
        let c = calls.clone();
        mapped.watch(move |v| c.lock().unwrap().push(*v));
        source.set(5);
        assert_eq!(*calls.lock().unwrap(), vec![50]);
    }

    #[test]
    fn with_combines_two_readers() {
        let (_, r1) = source_reader(1i32);
        let (_, r2) = source_reader(2i32);
        let combined = with((r1, r2));
        assert_eq!(combined.get(), (1, 2));
    }

    #[test]
    fn with_left_changes_tuple() {
        let (s1, r1) = source_reader(1i32);
        let (_, r2) = source_reader(2i32);
        let combined = with((r1, r2));
        let calls = Arc::new(Mutex::new(vec![]));
        let c = calls.clone();
        combined.watch(move |v| c.lock().unwrap().push(*v));
        s1.set(10);
        assert_eq!(*calls.lock().unwrap(), vec![(10, 2)]);
    }

    #[test]
    fn with_right_changes_tuple() {
        let (_, r1) = source_reader(1i32);
        let (s2, r2) = source_reader(2i32);
        let combined = with((r1, r2));
        let calls = Arc::new(Mutex::new(vec![]));
        let c = calls.clone();
        combined.watch(move |v| c.lock().unwrap().push(*v));
        s2.set(20);
        assert_eq!(*calls.lock().unwrap(), vec![(1, 20)]);
    }
}
