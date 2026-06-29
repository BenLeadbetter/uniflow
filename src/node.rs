use crate::State;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

type NodeId = u64;
type Callback = Box<dyn Fn()>;
type Batch = Vec<(NodeId, Callback)>;

static NEXT_NODE_ID: AtomicU64 = AtomicU64::new(0);

fn push_or_replace(batch: &mut Batch, id: NodeId, cb: Callback) {
    if let Some(entry) = batch.iter_mut().find(|(i, _)| *i == id) {
        entry.1 = cb; // same node visited twice (diamond): newer value wins
    } else {
        batch.push((id, cb));
    }
}

// ── Core traits ───────────────────────────────────────────────────────────────

pub(crate) trait Propagate: Send + Sync {
    fn propagate(&self, batch: &mut Batch);
}

pub(crate) trait ReadableNode<T: State>: Send + Sync {
    fn get(&self) -> T;
    fn add_watcher(&self, slot: WatchSlot<T>);
    fn add_child(&self, child: Weak<dyn Propagate>);
}

pub(crate) struct WatchSlot<T> {
    pub(crate) alive: Weak<()>,
    pub(crate) callback: Arc<dyn Fn(&T) + Send + Sync>,
}

// ── SourceNode ────────────────────────────────────────────────────────────────

struct SourceNodeInner<T> {
    value: T,
    children: Vec<Weak<dyn Propagate>>,
    watchers: Vec<WatchSlot<T>>,
}

pub(crate) struct SourceNode<T: State> {
    node_id: NodeId,
    inner: Mutex<SourceNodeInner<T>>,
}

impl<T: State> SourceNode<T> {
    pub(crate) fn new(value: T) -> Arc<Self> {
        Arc::new(SourceNode {
            node_id: NEXT_NODE_ID.fetch_add(1, Ordering::Relaxed),
            inner: Mutex::new(SourceNodeInner {
                value,
                children: Vec::new(),
                watchers: Vec::new(),
            }),
        })
    }

    pub(crate) fn set(&self, new_value: T) {
        let mut batch = Batch::new();
        let children = {
            let mut guard = self.inner.lock().unwrap();
            if guard.value == new_value {
                return;
            }
            guard.value = new_value.clone();
            guard.watchers.retain(|s| s.alive.upgrade().is_some());
            let cbs: Vec<_> = guard.watchers.iter().map(|s| s.callback.clone()).collect();
            if !cbs.is_empty() {
                let v = new_value.clone();
                push_or_replace(
                    &mut batch,
                    self.node_id,
                    Box::new(move || {
                        for cb in &cbs {
                            cb(&v);
                        }
                    }),
                );
            }
            guard.children.clone()
        };
        for weak in &children {
            if let Some(child) = weak.upgrade() {
                child.propagate(&mut batch);
            }
        }
        for (_, cb) in batch {
            cb();
        }
    }
}

impl<T: State> ReadableNode<T> for SourceNode<T> {
    fn get(&self) -> T {
        self.inner.lock().unwrap().value.clone()
    }

    fn add_watcher(&self, slot: WatchSlot<T>) {
        self.inner.lock().unwrap().watchers.push(slot);
    }

    fn add_child(&self, child: Weak<dyn Propagate>) {
        self.inner.lock().unwrap().children.push(child);
    }
}

// ── DerivedNode ───────────────────────────────────────────────────────────────

struct DerivedNodeInner<T> {
    cached: T,
    watchers: Vec<WatchSlot<T>>,
    children: Vec<Weak<dyn Propagate>>,
}

pub(crate) struct DerivedNode<S, T>
where
    S: State,
    T: State,
{
    node_id: NodeId,
    parent: Arc<dyn ReadableNode<S>>,
    selector: Arc<dyn Fn(S) -> T + Send + Sync>,
    inner: Mutex<DerivedNodeInner<T>>,
}

impl<S, T> DerivedNode<S, T>
where
    S: State,
    T: State,
{
    pub(crate) fn new(
        parent: Arc<dyn ReadableNode<S>>,
        selector: impl Fn(S) -> T + Send + Sync + 'static,
    ) -> Arc<Self> {
        let initial = selector(parent.get());
        let node = Arc::new(DerivedNode {
            node_id: NEXT_NODE_ID.fetch_add(1, Ordering::Relaxed),
            parent: parent.clone(),
            selector: Arc::new(selector),
            inner: Mutex::new(DerivedNodeInner {
                cached: initial,
                watchers: Vec::new(),
                children: Vec::new(),
            }),
        });
        let arc_prop: Arc<dyn Propagate> = node.clone();
        parent.add_child(Arc::downgrade(&arc_prop));
        node
    }
}

impl<S, T> Propagate for DerivedNode<S, T>
where
    S: State,
    T: State,
{
    fn propagate(&self, batch: &mut Batch) {
        let new_value = (self.selector)(self.parent.get());
        let children = {
            let mut guard = self.inner.lock().unwrap();
            if guard.cached == new_value {
                return;
            }
            guard.cached = new_value.clone();
            guard.watchers.retain(|s| s.alive.upgrade().is_some());
            let cbs: Vec<_> = guard.watchers.iter().map(|s| s.callback.clone()).collect();
            if !cbs.is_empty() {
                let v = new_value.clone();
                push_or_replace(
                    batch,
                    self.node_id,
                    Box::new(move || {
                        for cb in &cbs {
                            cb(&v);
                        }
                    }),
                );
            }
            guard.children.clone()
        };
        for weak in &children {
            if let Some(child) = weak.upgrade() {
                child.propagate(batch);
            }
        }
    }
}

impl<S, T> ReadableNode<T> for DerivedNode<S, T>
where
    S: State,
    T: State,
{
    fn get(&self) -> T {
        self.inner.lock().unwrap().cached.clone()
    }

    fn add_watcher(&self, slot: WatchSlot<T>) {
        self.inner.lock().unwrap().watchers.push(slot);
    }

    fn add_child(&self, child: Weak<dyn Propagate>) {
        self.inner.lock().unwrap().children.push(child);
    }
}

// ── MergeNode ─────────────────────────────────────────────────────────────────

struct MergeNodeInner<T, U> {
    cached: (T, U),
    watchers: Vec<WatchSlot<(T, U)>>,
    children: Vec<Weak<dyn Propagate>>,
}

pub(crate) struct MergeNode<T, U>
where
    T: State,
    U: State,
{
    node_id: NodeId,
    left: Arc<dyn ReadableNode<T>>,
    right: Arc<dyn ReadableNode<U>>,
    inner: Mutex<MergeNodeInner<T, U>>,
}

impl<T, U> MergeNode<T, U>
where
    T: State,
    U: State,
{
    pub(crate) fn new(
        left: Arc<dyn ReadableNode<T>>,
        right: Arc<dyn ReadableNode<U>>,
    ) -> Arc<Self> {
        let initial = (left.get(), right.get());
        let node = Arc::new(MergeNode {
            node_id: NEXT_NODE_ID.fetch_add(1, Ordering::Relaxed),
            left: left.clone(),
            right: right.clone(),
            inner: Mutex::new(MergeNodeInner {
                cached: initial,
                watchers: Vec::new(),
                children: Vec::new(),
            }),
        });
        let arc_prop: Arc<dyn Propagate> = node.clone();
        let weak = Arc::downgrade(&arc_prop);
        left.add_child(weak.clone());
        right.add_child(weak);
        node
    }
}

impl<T, U> Propagate for MergeNode<T, U>
where
    T: State,
    U: State,
{
    fn propagate(&self, batch: &mut Batch) {
        let new_value = (self.left.get(), self.right.get());
        let children = {
            let mut guard = self.inner.lock().unwrap();
            if guard.cached == new_value {
                return;
            }
            guard.cached = new_value.clone();
            guard.watchers.retain(|s| s.alive.upgrade().is_some());
            let cbs: Vec<_> = guard.watchers.iter().map(|s| s.callback.clone()).collect();
            if !cbs.is_empty() {
                let v = new_value.clone();
                push_or_replace(
                    batch,
                    self.node_id,
                    Box::new(move || {
                        for cb in &cbs {
                            cb(&v);
                        }
                    }),
                );
            }
            guard.children.clone()
        };
        for weak in &children {
            if let Some(child) = weak.upgrade() {
                child.propagate(batch);
            }
        }
    }
}

impl<T, U> ReadableNode<(T, U)> for MergeNode<T, U>
where
    T: State,
    U: State,
{
    fn get(&self) -> (T, U) {
        self.inner.lock().unwrap().cached.clone()
    }

    fn add_watcher(&self, slot: WatchSlot<(T, U)>) {
        self.inner.lock().unwrap().watchers.push(slot);
    }

    fn add_child(&self, child: Weak<dyn Propagate>) {
        self.inner.lock().unwrap().children.push(child);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subscription::Subscription;
    use std::sync::{Arc, Mutex};

    fn make_slot<T: State>(received: Arc<Mutex<Vec<T>>>) -> (WatchSlot<T>, Subscription) {
        let (sub, weak) = Subscription::new();
        let slot = WatchSlot {
            alive: weak,
            callback: Arc::new(move |v: &T| received.lock().unwrap().push(v.clone())),
        };
        (slot, sub)
    }

    // ── SourceNode ────────────────────────────────────────────────────────────

    #[test]
    fn source_node_get_returns_initial_value() {
        let node = SourceNode::new(42i32);
        assert_eq!(node.get(), 42);
    }

    #[test]
    fn source_node_set_updates_value() {
        let node = SourceNode::new(42i32);
        node.set(100);
        assert_eq!(node.get(), 100);
    }

    #[test]
    fn source_node_set_equal_value_is_noop() {
        let node = SourceNode::new(42i32);
        let calls = Arc::new(Mutex::new(vec![]));
        let (slot, _sub) = make_slot(calls.clone());
        node.add_watcher(slot);
        node.set(42);
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn source_node_watcher_called_on_change() {
        let node = SourceNode::new(42i32);
        let calls = Arc::new(Mutex::new(vec![]));
        let (slot, _sub) = make_slot(calls.clone());
        node.add_watcher(slot);
        node.set(100);
        assert_eq!(*calls.lock().unwrap(), vec![100]);
    }

    #[test]
    fn source_node_dead_watcher_slot_pruned_on_set() {
        let node = SourceNode::new(42i32);
        let calls = Arc::new(Mutex::new(vec![]));
        let (slot, sub) = make_slot(calls.clone());
        node.add_watcher(slot);
        drop(sub);
        node.set(100);
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn source_node_multiple_watchers_all_called() {
        let node = SourceNode::new(0i32);
        let calls_a = Arc::new(Mutex::new(vec![]));
        let calls_b = Arc::new(Mutex::new(vec![]));
        let (slot_a, _sub_a) = make_slot(calls_a.clone());
        let (slot_b, _sub_b) = make_slot(calls_b.clone());
        node.add_watcher(slot_a);
        node.add_watcher(slot_b);
        node.set(1);
        assert_eq!(*calls_a.lock().unwrap(), vec![1]);
        assert_eq!(*calls_b.lock().unwrap(), vec![1]);
    }

    #[test]
    fn source_node_children_propagated_on_set() {
        let source = SourceNode::new(0i32);
        let derived = DerivedNode::new(source.clone() as Arc<dyn ReadableNode<i32>>, |v| v * 2);
        let calls = Arc::new(Mutex::new(vec![]));
        let (slot, _sub) = make_slot(calls.clone());
        derived.add_watcher(slot);
        source.set(5);
        assert_eq!(*calls.lock().unwrap(), vec![10]);
    }

    // ── DerivedNode ───────────────────────────────────────────────────────────

    #[test]
    fn derived_node_initial_value_computed_from_parent() {
        let source = SourceNode::new(10i32);
        let derived = DerivedNode::new(source.clone() as Arc<dyn ReadableNode<i32>>, |v| v + 1);
        assert_eq!(derived.get(), 11);
    }

    #[test]
    fn derived_node_propagates_on_parent_change() {
        let source = SourceNode::new(0i32);
        let derived = DerivedNode::new(source.clone() as Arc<dyn ReadableNode<i32>>, |v| v * 10);
        source.set(3);
        assert_eq!(derived.get(), 30);
    }

    #[test]
    fn derived_node_memoized_equal_output_stops_propagation() {
        let source = SourceNode::new(0i32);
        let derived = DerivedNode::new(source.clone() as Arc<dyn ReadableNode<i32>>, |_| 99i32);
        let calls = Arc::new(Mutex::new(vec![]));
        let (slot, _sub) = make_slot(calls.clone());
        derived.add_watcher(slot);
        source.set(1);
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn derived_node_watcher_called_on_change() {
        let source = SourceNode::new(0i32);
        let derived = DerivedNode::new(source.clone() as Arc<dyn ReadableNode<i32>>, |v| v + 1);
        let calls = Arc::new(Mutex::new(vec![]));
        let (slot, _sub) = make_slot(calls.clone());
        derived.add_watcher(slot);
        source.set(4);
        assert_eq!(*calls.lock().unwrap(), vec![5]);
    }

    #[test]
    fn derived_node_chain_propagates() {
        let source = SourceNode::new(1i32);
        let derived1: Arc<dyn ReadableNode<i32>> =
            DerivedNode::new(source.clone() as Arc<dyn ReadableNode<i32>>, |v| v * 2);
        let derived2 = DerivedNode::new(derived1, |v| v + 10);
        let calls = Arc::new(Mutex::new(vec![]));
        let (slot, _sub) = make_slot(calls.clone());
        derived2.add_watcher(slot);
        source.set(5); // * 2 = 10, + 10 = 20
        assert_eq!(*calls.lock().unwrap(), vec![20]);
    }

    // ── MergeNode ─────────────────────────────────────────────────────────────

    #[test]
    fn merge_node_initial_value_is_tuple() {
        let left = SourceNode::new(1i32);
        let right = SourceNode::new(2i32);
        let merge = MergeNode::new(
            left as Arc<dyn ReadableNode<i32>>,
            right as Arc<dyn ReadableNode<i32>>,
        );
        assert_eq!(merge.get(), (1, 2));
    }

    #[test]
    fn merge_node_left_change_propagates() {
        let left = SourceNode::new(1i32);
        let right = SourceNode::new(2i32);
        let merge = MergeNode::new(
            left.clone() as Arc<dyn ReadableNode<i32>>,
            right as Arc<dyn ReadableNode<i32>>,
        );
        let calls = Arc::new(Mutex::new(vec![]));
        let (slot, _sub) = make_slot(calls.clone());
        merge.add_watcher(slot);
        left.set(10);
        assert_eq!(*calls.lock().unwrap(), vec![(10, 2)]);
    }

    #[test]
    fn merge_node_right_change_propagates() {
        let left = SourceNode::new(1i32);
        let right = SourceNode::new(2i32);
        let merge = MergeNode::new(
            left as Arc<dyn ReadableNode<i32>>,
            right.clone() as Arc<dyn ReadableNode<i32>>,
        );
        let calls = Arc::new(Mutex::new(vec![]));
        let (slot, _sub) = make_slot(calls.clone());
        merge.add_watcher(slot);
        right.set(20);
        assert_eq!(*calls.lock().unwrap(), vec![(1, 20)]);
    }

    #[test]
    fn merge_node_diamond_pattern_fires_once() {
        // source → left_derived (+1) ─┐
        //       ↘ right_derived (+2) ─┴→ merge
        // merge watcher should fire exactly once per source change
        let source = SourceNode::new(0i32);
        let left: Arc<dyn ReadableNode<i32>> =
            DerivedNode::new(source.clone() as Arc<dyn ReadableNode<i32>>, |v| v + 1);
        let right: Arc<dyn ReadableNode<i32>> =
            DerivedNode::new(source.clone() as Arc<dyn ReadableNode<i32>>, |v| v + 2);
        let merge = MergeNode::new(left, right);
        let calls = Arc::new(Mutex::new(vec![]));
        let (slot, _sub) = make_slot(calls.clone());
        merge.add_watcher(slot);
        source.set(10); // left → 11, right → 12
        assert_eq!(*calls.lock().unwrap(), vec![(11, 12)]);
    }

    #[test]
    fn merge_node_equal_output_stops_propagation() {
        let left = SourceNode::new(1i32);
        let right = SourceNode::new(2i32);
        let merge = MergeNode::new(
            left as Arc<dyn ReadableNode<i32>>,
            right.clone() as Arc<dyn ReadableNode<i32>>,
        );
        let calls = Arc::new(Mutex::new(vec![]));
        let (slot, _sub) = make_slot(calls.clone());
        merge.add_watcher(slot);
        right.set(2);
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn merge_node_further_derived_from_merge() {
        let left = SourceNode::new(3i32);
        let right = SourceNode::new(4i32);
        let merge: Arc<dyn ReadableNode<(i32, i32)>> = MergeNode::new(
            left.clone() as Arc<dyn ReadableNode<i32>>,
            right as Arc<dyn ReadableNode<i32>>,
        );
        let derived = DerivedNode::new(merge, |(a, b)| a + b);
        assert_eq!(derived.get(), 7);
        let calls = Arc::new(Mutex::new(vec![]));
        let (slot, _sub) = make_slot(calls.clone());
        derived.add_watcher(slot);
        left.set(10);
        assert_eq!(*calls.lock().unwrap(), vec![14]);
    }
}
