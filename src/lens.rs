//! Lenses: composable, bidirectional focus on one part of a larger value.
//!
//! A [`Lens<W>`] pairs a `view` from a whole `W` to one of its parts with a
//! `set` that puts a new part back into the whole. [`Reader::zoom`](crate::Reader::zoom)
//! uses `view` to derive a reader of the part; `set` is there for writable
//! zooms (cursors), which a later iteration will add.
//!
//! The library provides:
//!
//! - [`FieldLens`], built with [`field!`](crate::field), focusing on a struct field.
//! - [`IndexLens`], built with [`at`], focusing on an element of a container by
//!   index or key. Its part is `Option<Item>`, `None` when the element is missing.
//! - [`EnumLens`], built with [`variant!`](crate::variant), focusing on the payload
//!   of an enum variant. Its part is `Option<Payload>`, `None` when the whole
//!   holds another variant.
//! - [`GetSet`], built with [`getset`], a lens from a getter and a setter.
//!
//! Lenses compose with [`LensExt::then`].

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::hash::{BuildHasher, Hash};
use std::marker::PhantomData;

use crate::Value;

// ── Lens trait ────────────────────────────────────────────────────────────────

/// A focus on one part of a whole `W`.
///
/// Lawful lenses satisfy, for any `whole` and `part`:
///
/// - `view(&set(whole, part)) == part` — you view what you set.
/// - `set(whole, view(&whole)) == whole` — setting what you viewed changes nothing.
/// - `set(set(whole, a), b) == set(whole, b)` — the last set wins.
///
/// Partial lenses ([`IndexLens`], [`EnumLens`]) satisfy the first law only when
/// the focused element exists.
pub trait Lens<W>: Send + Sync + 'static {
    type Part: Value;

    /// Returns the focused part of `whole`.
    fn view(&self, whole: &W) -> Self::Part;

    /// Returns `whole` with its focused part replaced by `part`.
    fn set(&self, whole: W, part: Self::Part) -> W;
}

/// Combinators available on every [`Lens`].
pub trait LensExt<W>: Lens<W> + Sized {
    /// Returns `whole` with `f` applied to its focused part.
    fn over<F: FnOnce(Self::Part) -> Self::Part>(&self, whole: W, f: F) -> W {
        let part = f(self.view(&whole));
        self.set(whole, part)
    }

    /// Composes this lens with `inner`, which focuses further into this lens's part.
    fn then<L: Lens<Self::Part>>(self, inner: L) -> Then<Self, L> {
        Then { outer: self, inner }
    }
}

impl<W, L: Lens<W>> LensExt<W> for L {}

// ── Composition ───────────────────────────────────────────────────────────────

/// Two lenses composed with [`LensExt::then`]: `outer` then `inner`.
#[derive(Clone, Copy)]
pub struct Then<A, B> {
    outer: A,
    inner: B,
}

impl<W, A, B> Lens<W> for Then<A, B>
where
    A: Lens<W>,
    B: Lens<A::Part>,
{
    type Part = B::Part;

    fn view(&self, whole: &W) -> Self::Part {
        self.inner.view(&self.outer.view(whole))
    }

    fn set(&self, whole: W, part: Self::Part) -> W {
        let outer_part = self.outer.view(&whole);
        self.outer.set(whole, self.inner.set(outer_part, part))
    }
}

// ── FieldLens ─────────────────────────────────────────────────────────────────

/// A lens on a field `P` of a struct `W`, usually built with [`field!`](crate::field).
///
/// NOTE: Rust has no member pointers, so the field is named by a pair of
/// accessors. They are plain `fn` pointers, so the lens type is nameable and `Copy`.
pub struct FieldLens<W, P> {
    get: fn(&W) -> &P,
    get_mut: fn(&mut W) -> &mut P,
}

impl<W, P> FieldLens<W, P> {
    pub fn new(get: fn(&W) -> &P, get_mut: fn(&mut W) -> &mut P) -> Self {
        Self { get, get_mut }
    }
}

impl<W, P> Clone for FieldLens<W, P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<W, P> Copy for FieldLens<W, P> {}

impl<W: 'static, P: Value> Lens<W> for FieldLens<W, P> {
    type Part = P;

    fn view(&self, whole: &W) -> P {
        (self.get)(whole).clone()
    }

    fn set(&self, mut whole: W, part: P) -> W {
        *(self.get_mut)(&mut whole) = part;
        whole
    }
}

/// Builds a [`FieldLens`] on a field of a struct.
///
/// The field may be a nested path or a tuple index:
///
/// ```
/// use uniflow::lens::Lens;
///
/// #[derive(Clone, PartialEq)]
/// struct Address { street: String }
///
/// #[derive(Clone, PartialEq)]
/// struct Person { age: u32, address: Address, scores: (u8, u8) }
///
/// let person = Person { age: 33, address: Address { street: "Mega".into() }, scores: (1, 2) };
/// assert_eq!(uniflow::field!(Person, age).view(&person), 33);
/// assert_eq!(uniflow::field!(Person, address.street).view(&person), "Mega");
/// assert_eq!(uniflow::field!(Person, scores.1).view(&person), 2);
/// ```
#[macro_export]
macro_rules! field {
    ($W:ty, $($field:tt).+) => {
        $crate::lens::FieldLens::<$W, _>::new(
            |whole| &whole.$($field).+,
            |whole| &mut whole.$($field).+,
        )
    };
}

// ── IndexLens ─────────────────────────────────────────────────────────────────

/// Containers whose elements can be looked up by a key `K` without panicking.
pub trait Indexed<K> {
    type Item;

    /// Returns the element at `key`, or `None` when there is none.
    fn at(&self, key: &K) -> Option<&Self::Item>;

    /// Returns the element at `key` mutably, or `None` when there is none.
    fn at_mut(&mut self, key: &K) -> Option<&mut Self::Item>;
}

impl<T> Indexed<usize> for Vec<T> {
    type Item = T;

    fn at(&self, key: &usize) -> Option<&T> {
        self.get(*key)
    }

    fn at_mut(&mut self, key: &usize) -> Option<&mut T> {
        self.get_mut(*key)
    }
}

impl<T> Indexed<usize> for Box<[T]> {
    type Item = T;

    fn at(&self, key: &usize) -> Option<&T> {
        self.get(*key)
    }

    fn at_mut(&mut self, key: &usize) -> Option<&mut T> {
        self.get_mut(*key)
    }
}

impl<T, const N: usize> Indexed<usize> for [T; N] {
    type Item = T;

    fn at(&self, key: &usize) -> Option<&T> {
        self.get(*key)
    }

    fn at_mut(&mut self, key: &usize) -> Option<&mut T> {
        self.get_mut(*key)
    }
}

impl<T> Indexed<usize> for VecDeque<T> {
    type Item = T;

    fn at(&self, key: &usize) -> Option<&T> {
        self.get(*key)
    }

    fn at_mut(&mut self, key: &usize) -> Option<&mut T> {
        self.get_mut(*key)
    }
}

impl<K: Eq + Hash, V, S: BuildHasher> Indexed<K> for HashMap<K, V, S> {
    type Item = V;

    fn at(&self, key: &K) -> Option<&V> {
        self.get(key)
    }

    fn at_mut(&mut self, key: &K) -> Option<&mut V> {
        self.get_mut(key)
    }
}

impl<K: Ord, V> Indexed<K> for BTreeMap<K, V> {
    type Item = V;

    fn at(&self, key: &K) -> Option<&V> {
        self.get(key)
    }

    fn at_mut(&mut self, key: &K) -> Option<&mut V> {
        self.get_mut(key)
    }
}

/// A lens on the element at `key` of an [`Indexed`] container, built with [`at`].
///
/// Its part is `Some(element)`, or `None` when the container has no element at
/// `key`. Setting `None`, or setting a missing element, leaves the container
/// unchanged: the lens never inserts or removes elements.
#[derive(Clone, Copy)]
pub struct IndexLens<K> {
    key: K,
}

impl<K> IndexLens<K> {
    pub fn new(key: K) -> Self {
        Self { key }
    }
}

/// Returns an [`IndexLens`] on the element at `key`.
pub fn at<K>(key: K) -> IndexLens<K> {
    IndexLens::new(key)
}

impl<W, K> Lens<W> for IndexLens<K>
where
    W: Indexed<K>,
    W::Item: Value,
    K: Send + Sync + 'static,
{
    type Part = Option<W::Item>;

    fn view(&self, whole: &W) -> Self::Part {
        whole.at(&self.key).cloned()
    }

    fn set(&self, mut whole: W, part: Self::Part) -> W {
        if let Some(part) = part
            && let Some(element) = whole.at_mut(&self.key)
        {
            *element = part;
        }
        whole
    }
}

// ── EnumLens ──────────────────────────────────────────────────────────────────

/// A lens on the payload `P` of one variant of an enum `W`, usually built with
/// [`variant!`](crate::variant).
///
/// Its part is `Some(payload)`, or `None` when the whole holds another variant.
/// Setting `None`, or setting while the whole holds another variant, leaves the
/// whole unchanged: the lens never switches variants.
pub struct EnumLens<W, P> {
    get: fn(&W) -> Option<&P>,
    get_mut: fn(&mut W) -> Option<&mut P>,
}

impl<W, P> EnumLens<W, P> {
    pub fn new(get: fn(&W) -> Option<&P>, get_mut: fn(&mut W) -> Option<&mut P>) -> Self {
        Self { get, get_mut }
    }
}

impl<W, P> Clone for EnumLens<W, P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<W, P> Copy for EnumLens<W, P> {}

impl<W: 'static, P: Value> Lens<W> for EnumLens<W, P> {
    type Part = Option<P>;

    fn view(&self, whole: &W) -> Option<P> {
        (self.get)(whole).cloned()
    }

    fn set(&self, mut whole: W, part: Option<P>) -> W {
        if let Some(part) = part
            && let Some(payload) = (self.get_mut)(&mut whole)
        {
            *payload = part;
        }
        whole
    }
}

/// Builds an [`EnumLens`] on the payload of a single-field tuple variant.
///
/// The enum and payload types are inferred from the variant path:
///
/// ```
/// use uniflow::lens::Lens;
///
/// #[derive(Clone, PartialEq)]
/// enum Shape { Circle(f32), Square(f32) }
///
/// let circle = uniflow::variant!(Shape::Circle);
/// assert_eq!(circle.view(&Shape::Circle(1.0)), Some(1.0));
/// assert_eq!(circle.view(&Shape::Square(1.0)), None);
/// ```
// TODO: support struct variants and tuple variants with several fields.
#[macro_export]
macro_rules! variant {
    ($V:path) => {
        $crate::lens::EnumLens::new(
            |whole| match whole {
                $V(payload) => Some(payload),
                #[allow(unreachable_patterns)]
                _ => None,
            },
            |whole| match whole {
                $V(payload) => Some(payload),
                #[allow(unreachable_patterns)]
                _ => None,
            },
        )
    };
}

// ── GetSet ────────────────────────────────────────────────────────────────────

/// A lens from a getter and a setter, built with [`getset`].
pub struct GetSet<W, P, G, S> {
    get: G,
    set: S,
    types: PhantomData<fn(W) -> P>,
}

/// Returns a lens that views with `get` and sets with `set`.
///
/// ```
/// use uniflow::lens::{Lens, getset};
///
/// #[derive(Clone, PartialEq)]
/// struct Temperature { kelvin: f64 }
///
/// let celsius = getset(
///     |t: &Temperature| t.kelvin - 273.15,
///     |_: Temperature, c: f64| Temperature { kelvin: c + 273.15 },
/// );
/// assert_eq!(celsius.view(&Temperature { kelvin: 273.15 }), 0.0);
/// ```
pub fn getset<W, P, G, S>(get: G, set: S) -> GetSet<W, P, G, S>
where
    G: Fn(&W) -> P,
    S: Fn(W, P) -> W,
{
    GetSet {
        get,
        set,
        types: PhantomData,
    }
}

impl<W, P, G: Clone, S: Clone> Clone for GetSet<W, P, G, S> {
    fn clone(&self) -> Self {
        Self {
            get: self.get.clone(),
            set: self.set.clone(),
            types: PhantomData,
        }
    }
}

impl<W, P, G, S> Lens<W> for GetSet<W, P, G, S>
where
    W: 'static,
    P: Value,
    G: Fn(&W) -> P + Send + Sync + 'static,
    S: Fn(W, P) -> W + Send + Sync + 'static,
{
    type Part = P;

    fn view(&self, whole: &W) -> P {
        (self.get)(whole)
    }

    fn set(&self, whole: W, part: P) -> W {
        (self.set)(whole, part)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Debug;

    #[derive(Clone, Debug, PartialEq)]
    struct Address {
        street: String,
    }

    #[derive(Clone, Debug, PartialEq)]
    struct Person {
        age: u32,
        address: Address,
        scores: (u8, u8),
    }

    #[derive(Clone, Debug, PartialEq)]
    enum Shape {
        Circle(f32),
        Square(f32),
    }

    fn person() -> Person {
        Person {
            age: 33,
            address: Address {
                street: "Mega".into(),
            },
            scores: (1, 2),
        }
    }

    /// Asserts the three lens laws for `lens` at `whole`, using parts `a` and `b`.
    fn assert_lens_laws<W, L>(lens: &L, whole: W, a: L::Part, b: L::Part)
    where
        W: Clone + Debug + PartialEq,
        L: Lens<W>,
        L::Part: Debug,
    {
        assert_eq!(lens.view(&lens.set(whole.clone(), a.clone())), a, "set-get");
        assert_eq!(lens.set(whole.clone(), lens.view(&whole)), whole, "get-set");
        assert_eq!(
            lens.set(lens.set(whole.clone(), a), b.clone()),
            lens.set(whole, b),
            "set-set"
        );
    }

    // ── FieldLens ─────────────────────────────────────────────────────────────

    #[test]
    fn field_lens_views_field() {
        assert_eq!(field!(Person, age).view(&person()), 33);
    }

    #[test]
    fn field_lens_sets_field() {
        let older = field!(Person, age).set(person(), 34);
        assert_eq!(
            older,
            Person {
                age: 34,
                ..person()
            }
        );
    }

    #[test]
    fn field_lens_nested_path() {
        let lens = field!(Person, address.street);
        assert_eq!(lens.view(&person()), "Mega");
        assert_eq!(lens.set(person(), "City".into()).address.street, "City");
    }

    #[test]
    fn field_lens_tuple_index() {
        let lens = field!(Person, scores.1);
        assert_eq!(lens.view(&person()), 2);
        assert_eq!(lens.set(person(), 9).scores, (1, 9));
    }

    #[test]
    fn field_lens_obeys_laws() {
        assert_lens_laws(&field!(Person, age), person(), 1, 2);
    }

    // ── IndexLens ─────────────────────────────────────────────────────────────

    #[test]
    fn index_lens_views_present_element() {
        assert_eq!(at(1).view(&vec![10, 20, 30]), Some(20));
    }

    #[test]
    fn index_lens_views_missing_element_as_none() {
        assert_eq!(at(3).view(&vec![10, 20, 30]), None);
    }

    #[test]
    fn index_lens_sets_present_element() {
        assert_eq!(at(1).set(vec![10, 20, 30], Some(99)), vec![10, 99, 30]);
    }

    #[test]
    fn index_lens_set_missing_element_is_noop() {
        assert_eq!(at(3).set(vec![10, 20, 30], Some(99)), vec![10, 20, 30]);
    }

    #[test]
    fn index_lens_set_none_is_noop() {
        assert_eq!(at(1).set(vec![10, 20, 30], None), vec![10, 20, 30]);
    }

    #[test]
    fn index_lens_on_hash_map() {
        let map = HashMap::from([("a", 1), ("b", 2)]);
        assert_eq!(at("b").view(&map), Some(2));
        assert_eq!(at("c").view(&map), None);
        assert_eq!(at("b").set(map, Some(5))["b"], 5);
    }

    #[test]
    fn index_lens_set_missing_key_does_not_insert() {
        let map = BTreeMap::from([(1, "one")]);
        assert_eq!(at(2).set(map.clone(), Some("two")), map);
    }

    #[test]
    fn index_lens_on_array_and_boxed_slice() {
        assert_eq!(at(0).view(&[7u8, 8]), Some(7));
        let boxed: Box<[u8]> = vec![7, 8].into_boxed_slice();
        assert_eq!(at(1).view(&boxed), Some(8));
    }

    #[test]
    fn index_lens_obeys_laws_for_present_element() {
        assert_lens_laws(&at(1), vec![10, 20, 30], Some(1), Some(2));
    }

    // ── EnumLens ──────────────────────────────────────────────────────────────

    #[test]
    fn enum_lens_views_matching_variant() {
        assert_eq!(variant!(Shape::Circle).view(&Shape::Circle(1.0)), Some(1.0));
    }

    #[test]
    fn enum_lens_views_other_variant_as_none() {
        assert_eq!(variant!(Shape::Circle).view(&Shape::Square(1.0)), None);
    }

    #[test]
    fn enum_lens_sets_matching_variant() {
        let lens = variant!(Shape::Circle);
        assert_eq!(lens.set(Shape::Circle(1.0), Some(2.0)), Shape::Circle(2.0));
    }

    #[test]
    fn enum_lens_set_other_variant_is_noop() {
        let lens = variant!(Shape::Circle);
        assert_eq!(lens.set(Shape::Square(1.0), Some(2.0)), Shape::Square(1.0));
    }

    #[test]
    fn enum_lens_on_generic_enum() {
        let some = variant!(Option::Some);
        assert_eq!(some.view(&Some(5u8)), Some(5));
        assert_eq!(some.view(&None::<u8>), None);
    }

    #[test]
    fn enum_lens_obeys_laws_for_matching_variant() {
        assert_lens_laws(
            &variant!(Shape::Circle),
            Shape::Circle(0.0),
            Some(1.0),
            Some(2.0),
        );
    }

    // ── GetSet ────────────────────────────────────────────────────────────────

    #[test]
    fn getset_lens_uses_getter_and_setter() {
        let doubled = getset(|x: &i32| x * 2, |_: i32, d: i32| d / 2);
        assert_eq!(doubled.view(&4), 8);
        assert_eq!(doubled.set(4, 10), 5);
    }

    #[test]
    fn getset_lens_obeys_laws() {
        let doubled = getset(|x: &i32| x * 2, |_: i32, d: i32| d / 2);
        assert_lens_laws(&doubled, 4, 2, 6);
    }

    // ── Combinators ───────────────────────────────────────────────────────────

    #[test]
    fn over_applies_function_to_part() {
        assert_eq!(field!(Person, age).over(person(), |age| age + 1).age, 34);
    }

    #[test]
    fn then_composes_lenses() {
        let street = field!(Person, address).then(field!(Address, street));
        assert_eq!(street.view(&person()), "Mega");
        assert_eq!(street.set(person(), "City".into()).address.street, "City");
    }

    #[test]
    fn then_obeys_laws() {
        let street = field!(Person, address).then(field!(Address, street));
        assert_lens_laws(&street, person(), "A".into(), "B".into());
    }
}
