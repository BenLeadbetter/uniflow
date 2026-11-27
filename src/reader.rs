use std::sync::{Arc, Mutex};

use crate::node::{Callback, DerivedNode, MergeNode, ReadableNode, WatchSlot};
use crate::subscription::Subscription;
use crate::{Lens, Read, Value};

pub struct Reader<T>
where
    T: Value,
{
    pub(crate) node: Arc<dyn ReadableNode<T>>,
    connections: Mutex<Vec<Subscription>>,
}

impl<T: Value> Reader<T> {
    pub(crate) fn new(node: Arc<dyn ReadableNode<T>>) -> Self {
        Reader {
            node,
            connections: Mutex::new(Vec::new()),
        }
    }

    pub fn map<U, F>(&self, f: F) -> Reader<U>
    where
        U: Clone + PartialEq + Send + Sync + 'static,
        F: FnMut(T) -> U + Send + Sync + 'static,
    {
        Reader::new(DerivedNode::new(self.node.clone(), f))
    }

    /// Returns a reader of the part of this reader's value that `lens` focuses on.
    ///
    /// The zoomed reader only notifies its watchers when the focused part changes.
    pub fn zoom<L: Lens<T>>(&self, lens: L) -> Reader<L::Part> {
        // NOTE: a read-only zoom is a map of the lens's view, so it reuses `DerivedNode`.
        // A writable zoom (cursor) will need its own node, which writes back through `Lens::set`.
        self.map(move |whole| lens.view(&whole))
    }
}

impl<T: Value> Read<T> for Reader<T> {
    fn get(&self) -> T {
        self.node.get()
    }

    fn watch<F: FnMut(&T) + Send + Sync + 'static>(&self, f: F) -> &Self {
        let (sub, weak) = Subscription::new();
        self.node.add_watcher(WatchSlot {
            alive: weak,
            callback: Arc::new(Mutex::new(Box::new(f) as Callback<T>)),
        });
        self.connections.lock().unwrap().push(sub);
        self
    }

    fn bind<F: FnMut(&T) + Send + Sync + 'static>(&self, f: F) -> &Self {
        let mut f = f;
        f(&self.get());
        self.watch(f)
    }

    fn unbind(&self) {
        self.connections.lock().unwrap().clear();
    }
}

impl<T: Value> Clone for Reader<T> {
    fn clone(&self) -> Self {
        Reader {
            node: self.node.clone(),
            connections: Mutex::new(Vec::new()),
        }
    }
}

// ── Merge trait ───────────────────────────────────────────────────────────────

/// Implemented on tuples and `Vec`s of [`Reader`]s. Merge into a single reader
/// that yields the combined state and updates whenever any source reader changes.
///
/// Tuples of 2–5 readers yield the corresponding tuple type; `Vec<Reader<T>>`
/// yields `Vec<T>` (all sources must share the same type). Use [`with`] as a
/// free-function shorthand.
pub trait Merge {
    type Combined: Value;
    fn merge(self) -> Reader<Self::Combined>;
}

macro_rules! impl_merge {
    ($(($T:ident, $r:ident)),+ $(,)?) => {
        impl<$($T: Value),+> Merge for ($(Reader<$T>,)+) {
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

impl<T: Value> Merge for Vec<Reader<T>> {
    type Combined = Vec<T>;
    fn merge(self) -> Reader<Vec<T>> {
        let nodes = self.into_iter().map(|r| r.node).collect();
        Reader::new(MergeNode::<Vec<T>>::new(nodes))
    }
}

/// Combine a tuple or `Vec` of readers into a single reader. See [`Merge`].
pub fn with<R: Merge>(readers: R) -> Reader<R::Combined> {
    readers.merge()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::SourceNode;
    use crate::{Read, Write};
    use std::sync::{Arc, Mutex};

    fn source_reader<T: Value>(value: T) -> (Arc<SourceNode<T>>, Reader<T>) {
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

    #[derive(Clone, Debug, PartialEq)]
    struct Record {
        age: u32,
        tags: Vec<String>,
    }

    fn record_reader() -> (Arc<SourceNode<Record>>, Reader<Record>) {
        source_reader(Record {
            age: 33,
            tags: vec!["judge".into()],
        })
    }

    #[test]
    fn zoom_reads_focused_part() {
        let (_, reader) = record_reader();
        assert_eq!(reader.zoom(crate::field!(Record, age)).get(), 33);
    }

    #[test]
    fn zoom_follows_source_changes() {
        let (source, reader) = record_reader();
        let age = reader.zoom(crate::field!(Record, age));
        source.set(Record {
            age: 34,
            ..source.get()
        });
        assert_eq!(age.get(), 34);
    }

    #[test]
    fn zoom_skips_changes_outside_focus() {
        let (source, reader) = record_reader();
        let age = reader.zoom(crate::field!(Record, age));
        let calls = Arc::new(Mutex::new(vec![]));
        let c = calls.clone();
        age.watch(move |v| c.lock().unwrap().push(*v));
        source.set(Record {
            tags: vec![],
            ..source.get()
        });
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn zoom_composed_partial_lens() {
        use crate::lens::{LensExt, at};

        let (source, reader) = record_reader();
        let second_tag = reader.zoom(crate::field!(Record, tags).then(at(1)));
        assert_eq!(second_tag.get(), None);
        source.set(Record {
            tags: vec!["judge".into(), "mega".into()],
            ..source.get()
        });
        assert_eq!(second_tag.get(), Some("mega".into()));
    }

    #[test]
    fn zoom_chains() {
        let (_, reader) = record_reader();
        let first_tag = reader
            .zoom(crate::field!(Record, tags))
            .zoom(crate::lens::at(0));
        assert_eq!(first_tag.get(), Some("judge".into()));
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

    #[test]
    fn vec_merge_initial_value() {
        let (_, r1) = source_reader(1i32);
        let (_, r2) = source_reader(2i32);
        let (_, r3) = source_reader(3i32);
        let combined = vec![r1, r2, r3].merge();
        assert_eq!(combined.get(), vec![1, 2, 3]);
    }

    #[test]
    fn vec_merge_change_propagates() {
        let (s1, r1) = source_reader(1i32);
        let (_, r2) = source_reader(2i32);
        let combined = vec![r1, r2].merge();
        let calls = Arc::new(Mutex::new(vec![]));
        let c = calls.clone();
        combined.watch(move |v| c.lock().unwrap().push(v.clone()));
        s1.set(10);
        assert_eq!(*calls.lock().unwrap(), vec![vec![10, 2]]);
    }

    #[test]
    fn vec_merge_empty() {
        let combined: Reader<Vec<i32>> = Vec::<Reader<i32>>::new().merge();
        assert_eq!(combined.get(), Vec::<i32>::new());
    }

    #[test]
    fn vec_merge_many_sources() {
        let sources: Vec<_> = (0..10).map(source_reader).collect();
        let readers: Vec<_> = sources.iter().map(|(_, r)| r.clone()).collect();
        let combined = readers.merge();
        assert_eq!(combined.get(), (0..10).collect::<Vec<i32>>());
        sources[5].0.set(99);
        assert_eq!(combined.get()[5], 99);
    }

    #[test]
    fn reader_with_mutable_callback() {
        let init = 0u32;
        let state = crate::State::new(init);
        let duplicates = std::sync::Arc::<std::sync::atomic::AtomicU32>::default();
        {
            let mut cache = init;
            let duplicates = duplicates.clone();
            state.reader().map(move |value| {
                if cache == value {
                    duplicates.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                cache = value;
                value
            });
        }
        state.set(0);
        assert_eq!(duplicates.load(std::sync::atomic::Ordering::Relaxed), 1);
    }
}
