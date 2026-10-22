//! Index families: a position, a count, a difference and a span, one type each.
//!
//! A row number, a number of rows and the gap between two rows are three
//! quantities. A bare integer lets one stand for another, and a row passed
//! where a column is expected compiles. [`define_index!`] writes one family
//! per axis, with only the arithmetic the domain allows
//! (`ai/ambre-rust-style.md`, the newtype rule).

/// The serde paths the macro names, so a crate that expands it needs no serde
/// dependency of its own.
#[doc(hidden)]
pub mod __private {
    pub use serde::{Deserialize, Deserializer, Serialize, Serializer};
}

/// Writes one index family: a position, a count, a difference, a span and a
/// collection indexed by the position.
///
/// ```
/// ambre_core::define_index! {
///     /// A line of a list.
///     pub struct Line(u32);
///     /// A number of lines.
///     pub struct LineCount;
///     /// The signed gap between two lines.
///     pub struct LineDelta(i64);
///     /// A run of lines.
///     pub struct LineSpan;
///     /// A list indexed by line.
///     pub struct LineVec;
/// }
/// let first = Line::new(2);
/// let last = first + LineCount::new(3);
/// assert_eq!(last - first, LineDelta::new(3));
/// assert_eq!(LineSpan::new(first, last).into_iter().count(), 3);
/// ```
///
/// The operators are the ones a position allows:
///
/// | Expression | Result | On overflow |
/// | :--- | :--- | :--- |
/// | `Pos + Count` | `Pos` | saturates |
/// | `Pos - Count` | `Pos` | saturates at zero |
/// | `Pos + Delta` | `Pos` | clamps to the axis |
/// | `Pos - Pos` | `Delta` | exact, in the wider signed type |
/// | `Delta::from_count(count)` | `Delta` | exact |
/// | `pos.checked_sub(count)` | `Option<Pos>` | `None` |
/// | `Count + Count` | `Count` | saturates |
/// | `pos.checked_offset(delta)` | `Option<Pos>` | `None` |
/// | `pos.saturating_offset(delta)` | `Pos` | clamps to the axis |
///
/// `Pos + Pos` does not compile. No type derefs to its integer. None converts
/// from it through `From`: `new` wraps it, and `get` unwraps it for the call
/// into a foreign API. Each serialises as its bare integer, the
/// shape `#[serde(transparent)]` writes, so the wire keeps its bytes.
///
/// The collection takes its position in `[]`, as a slice takes a `usize`.
/// The operator panics past the end, as a slice index does. The workspace
/// forbids that panic outside a test, so production code calls `get`, which
/// returns an `Option`.
///
/// The span exists because `core::iter::Step` is unstable (tracking issue
/// rust-lang/rust#73121), so a `Range` of a newtype cannot iterate. When
/// `Step` reaches stable, implement it for every position type, replace each
/// span type with `Range`, and delete the span types.
///
/// The difference type must hold every position: `$delta_inner` implements
/// `From<$inner>`.
#[macro_export]
macro_rules! define_index {
    (
        $(#[$pos_meta:meta])* pub struct $pos:ident($inner:ty);
        $(#[$count_meta:meta])* pub struct $count:ident;
        $(#[$delta_meta:meta])* pub struct $delta:ident($delta_inner:ty);
        $(#[$span_meta:meta])* pub struct $span:ident;
        $(#[$vec_meta:meta])* pub struct $vec:ident;
    ) => {
        $(#[$pos_meta])*
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $pos($inner);

        $(#[$count_meta])*
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $count($inner);

        $(#[$delta_meta])*
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $delta($delta_inner);

        $(#[$span_meta])*
        ///
        /// Half-open: it holds `start` and stops before `end`. A span whose
        /// end is not past its start is empty.
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
        pub struct $span {
            /// The first position of the span.
            pub start: $pos,
            /// The first position past the span.
            pub end: $pos,
        }

        $(#[$vec_meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $vec<T>(Vec<T>);

        impl $pos {
            /// The first position.
            pub const ZERO: Self = Self(0);
            /// The last position the axis holds.
            pub const MAX: Self = Self(<$inner>::MAX);

            /// Wraps a position.
            pub const fn new(value: $inner) -> Self {
                Self(value)
            }

            /// The position, for the call into a foreign API.
            pub const fn get(self) -> $inner {
                self.0
            }

            /// The position past this one, saturating at [`Self::MAX`].
            pub const fn next(self) -> Self {
                Self(self.0.saturating_add(1))
            }

            /// The position before this one, `None` at [`Self::ZERO`].
            pub const fn prev(self) -> Option<Self> {
                match self.0.checked_sub(1) {
                    Some(value) => Some(Self(value)),
                    None => None,
                }
            }

            /// The position `count` before this one, `None` past the start.
            pub const fn checked_sub(self, count: $count) -> Option<Self> {
                match self.0.checked_sub(count.0) {
                    Some(value) => Some(Self(value)),
                    None => None,
                }
            }

            /// The position `count` before this one, clamped to [`Self::ZERO`].
            pub const fn saturating_sub(self, count: $count) -> Self {
                Self(self.0.saturating_sub(count.0))
            }

            /// The position `delta` away, `None` off either end of the axis.
            pub fn checked_offset(self, delta: $delta) -> Option<Self> {
                let moved = <$delta_inner>::from(self.0).checked_add(delta.0)?;
                <$inner>::try_from(moved).ok().map(Self)
            }

            /// The position `delta` away, clamped to the axis.
            pub fn saturating_offset(self, delta: $delta) -> Self {
                let moved = <$delta_inner>::from(self.0).saturating_add(delta.0);
                match <$inner>::try_from(moved) {
                    Ok(value) => Self(value),
                    Err(_) if moved < 0 => Self::ZERO,
                    Err(_) => Self::MAX,
                }
            }

            /// The number of positions from `start` up to this one, zero when
            /// `start` is not before it.
            pub const fn count_from(self, start: Self) -> $count {
                $count(self.0.saturating_sub(start.0))
            }

            /// The span from this position up to and with `last`.
            ///
            /// A span stops before its end, and no position lies past
            /// [`Self::MAX`]. So `through(MAX)` holds every position up to
            /// `MAX` and leaves `MAX` out.
            pub const fn through(self, last: Self) -> $span {
                $span::new(self, last.next())
            }

            /// The position at `index` of a slice, `None` past the axis.
            pub fn from_index(index: usize) -> Option<Self> {
                <$inner>::try_from(index).ok().map(Self)
            }

            /// The index of a slice this position names, `None` past what
            /// the platform addresses.
            pub fn to_index(self) -> Option<usize> {
                usize::try_from(self.0).ok()
            }
        }

        impl $count {
            /// No position.
            pub const ZERO: Self = Self(0);
            /// One position.
            pub const ONE: Self = Self(1);

            /// Wraps a count.
            pub const fn new(value: $inner) -> Self {
                Self(value)
            }

            /// The count, for the call into a foreign API.
            pub const fn get(self) -> $inner {
                self.0
            }

            /// Whether the count is zero.
            pub const fn is_zero(self) -> bool {
                self.0 == 0
            }

            /// This count less `other`, clamped to zero.
            pub const fn saturating_sub(self, other: Self) -> Self {
                Self(self.0.saturating_sub(other.0))
            }

            /// The count of a slice of `len` items, saturating past the axis.
            pub fn from_len(len: usize) -> Self {
                Self(<$inner>::try_from(len).unwrap_or(<$inner>::MAX))
            }

            /// The position this count reaches from the start of the axis:
            /// the first position past `count` items.
            pub const fn as_end(self) -> $pos {
                $pos(self.0)
            }
        }

        impl $delta {
            /// No move.
            pub const ZERO: Self = Self(0);

            /// Wraps a difference.
            pub const fn new(value: $delta_inner) -> Self {
                Self(value)
            }

            /// The difference, for the call into a foreign API.
            pub const fn get(self) -> $delta_inner {
                self.0
            }

            /// The same move, in the other direction.
            pub const fn reversed(self) -> Self {
                Self(self.0.saturating_neg())
            }

            /// The forward move of `count` positions.
            pub fn from_count(count: $count) -> Self {
                Self(<$delta_inner>::from(count.0))
            }
        }

        impl $span {
            /// The span from `start` up to, and without, `end`.
            pub const fn new(start: $pos, end: $pos) -> Self {
                Self { start, end }
            }

            /// The span of `count` positions from `start`.
            pub const fn from_count(start: $pos, count: $count) -> Self {
                Self::new(start, $pos(start.0.saturating_add(count.0)))
            }

            /// The number of positions the span holds.
            pub const fn len(self) -> $count {
                self.end.count_from(self.start)
            }

            /// Whether the span holds no position.
            pub const fn is_empty(self) -> bool {
                self.end.0 <= self.start.0
            }

            /// Whether the span holds `pos`.
            pub const fn contains(self, pos: $pos) -> bool {
                self.start.0 <= pos.0 && pos.0 < self.end.0
            }

            /// The last position the span holds, `None` when it is empty.
            pub const fn last(self) -> Option<$pos> {
                if self.is_empty() { None } else { self.end.prev() }
            }
        }

        impl IntoIterator for $span {
            type Item = $pos;
            type IntoIter = std::iter::Map<std::ops::Range<$inner>, fn($inner) -> $pos>;

            fn into_iter(self) -> Self::IntoIter {
                (self.start.0..self.end.0).map($pos as fn($inner) -> $pos)
            }
        }

        impl std::ops::Add<$count> for $pos {
            type Output = $pos;

            fn add(self, count: $count) -> $pos {
                $pos(self.0.saturating_add(count.0))
            }
        }

        impl std::ops::AddAssign<$count> for $pos {
            fn add_assign(&mut self, count: $count) {
                *self = *self + count;
            }
        }

        impl std::ops::Sub<$count> for $pos {
            type Output = $pos;

            fn sub(self, count: $count) -> $pos {
                self.saturating_sub(count)
            }
        }

        impl std::ops::SubAssign<$count> for $pos {
            fn sub_assign(&mut self, count: $count) {
                *self = *self - count;
            }
        }

        impl std::ops::Add<$delta> for $pos {
            type Output = $pos;

            fn add(self, delta: $delta) -> $pos {
                self.saturating_offset(delta)
            }
        }

        impl std::ops::AddAssign<$delta> for $pos {
            fn add_assign(&mut self, delta: $delta) {
                *self = *self + delta;
            }
        }

        impl std::ops::Sub for $pos {
            type Output = $delta;

            fn sub(self, other: $pos) -> $delta {
                $delta(<$delta_inner>::from(self.0) - <$delta_inner>::from(other.0))
            }
        }

        impl std::ops::Add for $count {
            type Output = $count;

            fn add(self, other: $count) -> $count {
                $count(self.0.saturating_add(other.0))
            }
        }

        impl std::ops::AddAssign for $count {
            fn add_assign(&mut self, other: $count) {
                *self = *self + other;
            }
        }

        impl<T> $vec<T> {
            /// An empty collection.
            pub const fn new() -> Self {
                Self(Vec::new())
            }

            /// The number of items.
            pub fn len(&self) -> $count {
                $count::from_len(self.0.len())
            }

            /// Whether the collection holds no item.
            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }

            /// The item at `pos`, `None` past the end.
            pub fn get(&self, pos: $pos) -> Option<&T> {
                self.0.get(pos.to_index()?)
            }

            /// The item at `pos`, mutable, `None` past the end.
            pub fn get_mut(&mut self, pos: $pos) -> Option<&mut T> {
                self.0.get_mut(pos.to_index()?)
            }

            /// Appends `item`, and returns the position it takes.
            pub fn push(&mut self, item: T) -> $pos {
                let pos = self.len().as_end();
                self.0.push(item);
                pos
            }

            /// Inserts `item` at `pos`, shifting the items after it. Returns
            /// the item back when `pos` lies past the end.
            pub fn insert(&mut self, pos: $pos, item: T) -> Result<(), T> {
                match pos.to_index() {
                    Some(index) if index <= self.0.len() => {
                        self.0.insert(index, item);
                        Ok(())
                    }
                    _ => Err(item),
                }
            }

            /// Removes the item at `pos`, `None` past the end.
            pub fn remove(&mut self, pos: $pos) -> Option<T> {
                let index = pos.to_index().filter(|index| *index < self.0.len())?;
                Some(self.0.remove(index))
            }

            /// The items in order.
            pub fn as_slice(&self) -> &[T] {
                &self.0
            }

            /// The items in order.
            pub fn iter(&self) -> std::slice::Iter<'_, T> {
                self.0.iter()
            }

            /// Each item beside its position, in order.
            pub fn iter_enumerated(&self) -> impl Iterator<Item = ($pos, &T)> + '_ {
                self.0.iter().enumerate().map(|(index, item)| {
                    ($pos(<$inner>::try_from(index).unwrap_or(<$inner>::MAX)), item)
                })
            }

            /// The span of every position the collection holds.
            pub fn positions(&self) -> $span {
                $span::new($pos::ZERO, self.len().as_end())
            }
        }

        impl<T> Default for $vec<T> {
            fn default() -> Self {
                Self::new()
            }
        }

        impl<T> From<Vec<T>> for $vec<T> {
            fn from(items: Vec<T>) -> Self {
                Self(items)
            }
        }

        impl<T> FromIterator<T> for $vec<T> {
            fn from_iter<I: IntoIterator<Item = T>>(items: I) -> Self {
                Self(items.into_iter().collect())
            }
        }

        impl<T> std::ops::Index<$pos> for $vec<T> {
            type Output = T;

            /// The item at `pos`. It panics past the end, as a slice index
            /// does; `get` is the call that cannot.
            #[allow(
                clippy::indexing_slicing,
                reason = "the operator keeps the slice contract; `get` is the checked call"
            )]
            fn index(&self, pos: $pos) -> &T {
                &self.0[usize::try_from(pos.0).unwrap_or(usize::MAX)]
            }
        }

        impl<T> std::ops::IndexMut<$pos> for $vec<T> {
            /// The item at `pos`, mutable. It panics past the end, as a slice
            /// index does; `get_mut` is the call that cannot.
            #[allow(
                clippy::indexing_slicing,
                reason = "the operator keeps the slice contract; `get_mut` is the checked call"
            )]
            fn index_mut(&mut self, pos: $pos) -> &mut T {
                &mut self.0[usize::try_from(pos.0).unwrap_or(usize::MAX)]
            }
        }

        impl<'a, T> IntoIterator for &'a $vec<T> {
            type Item = &'a T;
            type IntoIter = std::slice::Iter<'a, T>;

            fn into_iter(self) -> Self::IntoIter {
                self.0.iter()
            }
        }

        $crate::define_index!(@serde $pos, $inner);
        $crate::define_index!(@serde $count, $inner);
        $crate::define_index!(@serde $delta, $delta_inner);
    };
    (@serde $name:ident, $inner:ty) => {
        impl $crate::index::__private::Serialize for $name {
            fn serialize<S: $crate::index::__private::Serializer>(
                &self,
                serializer: S,
            ) -> Result<S::Ok, S::Error> {
                $crate::index::__private::Serialize::serialize(&self.0, serializer)
            }
        }

        impl<'de> $crate::index::__private::Deserialize<'de> for $name {
            fn deserialize<D: $crate::index::__private::Deserializer<'de>>(
                deserializer: D,
            ) -> Result<Self, D::Error> {
                <$inner as $crate::index::__private::Deserialize<'de>>::deserialize(deserializer)
                    .map(Self)
            }
        }
    };
}

#[cfg(test)]
mod tests {
    crate::define_index! {
        /// A test line.
        pub struct Line(u32);
        /// A count of test lines.
        pub struct LineCount;
        /// A gap between test lines.
        pub struct LineDelta(i64);
        /// A run of test lines.
        pub struct LineSpan;
        /// Test items by line.
        pub struct LineVec;
    }

    #[test]
    fn a_position_plus_a_count_is_a_position() {
        assert_eq!(Line::new(2) + LineCount::new(3), Line::new(5));
        assert_eq!(Line::MAX + LineCount::ONE, Line::MAX);
    }

    #[test]
    fn two_positions_differ_by_a_signed_delta() {
        assert_eq!(Line::new(2) - Line::new(5), LineDelta::new(-3));
        assert_eq!(Line::MAX - Line::ZERO, LineDelta::new(i64::from(u32::MAX)));
    }

    #[test]
    fn an_offset_off_the_axis_is_checked_or_clamped() {
        let at = Line::new(1);
        assert_eq!(at.checked_offset(LineDelta::new(-1)), Some(Line::ZERO));
        assert_eq!(at.checked_offset(LineDelta::new(-2)), None);
        assert_eq!(at.saturating_offset(LineDelta::new(-2)), Line::ZERO);
        assert_eq!(at.saturating_offset(LineDelta::new(i64::MAX)), Line::MAX);
    }

    #[test]
    fn a_count_moves_a_position_back_and_a_delta_moves_it_either_way() {
        assert_eq!(Line::new(5) - LineCount::new(3), Line::new(2));
        assert_eq!(Line::new(1) - LineCount::new(3), Line::ZERO);
        assert_eq!(Line::new(1).checked_sub(LineCount::new(3)), None);
        let back = LineDelta::from_count(LineCount::new(2)).reversed();
        assert_eq!(Line::new(5) + back, Line::new(3));
        assert_eq!(Line::new(1) + back, Line::ZERO);
    }

    #[test]
    fn a_span_iterates_its_positions_and_stops_before_its_end() {
        let span = Line::new(1).through(Line::new(3));
        let lines: Vec<Line> = span.into_iter().collect();
        assert_eq!(lines, vec![Line::new(1), Line::new(2), Line::new(3)]);
        assert_eq!(span.len(), LineCount::new(3));
        assert!(span.contains(Line::new(3)) && !span.contains(Line::new(4)));
        assert!(LineSpan::new(Line::new(4), Line::new(2)).is_empty());
    }

    #[test]
    fn the_collection_refuses_a_position_past_its_end() {
        let mut items: LineVec<char> = LineVec::new();
        assert_eq!(items.push('a'), Line::ZERO);
        assert_eq!(items.insert(Line::new(5), 'z'), Err('z'));
        assert_eq!(items.insert(Line::new(1), 'b'), Ok(()));
        assert_eq!(items.get(Line::new(1)), Some(&'b'));
        assert_eq!(items.remove(Line::new(9)), None);
        assert_eq!(items.len(), LineCount::new(2));
    }

    #[test]
    fn the_collection_takes_its_position_in_brackets() {
        let mut items: LineVec<char> = vec!['a', 'b'].into();
        items[Line::new(1)] = 'c';
        assert_eq!(items[Line::ZERO], 'a');
        assert_eq!(items[Line::new(1)], 'c');
    }

    #[test]
    #[should_panic(expected = "index out of bounds")]
    fn the_bracket_panics_past_the_end() {
        let items: LineVec<char> = vec!['a'].into();
        let _ = items[Line::new(1)];
    }

    #[test]
    fn a_span_through_the_last_position_leaves_it_out() {
        let span = Line::new(u32::MAX - 1).through(Line::MAX);
        assert_eq!(
            span.into_iter().collect::<Vec<_>>(),
            vec![Line::new(u32::MAX - 1)]
        );
        assert!(!span.contains(Line::MAX));
    }

    #[test]
    fn each_type_serialises_as_its_bare_integer() {
        let line = postcard::to_allocvec(&Line::new(300)).unwrap();
        assert_eq!(line, postcard::to_allocvec(&300u32).unwrap());
        let back: LineDelta =
            postcard::from_bytes(&postcard::to_allocvec(&-7i64).unwrap()).unwrap();
        assert_eq!(back, LineDelta::new(-7));
    }
}
