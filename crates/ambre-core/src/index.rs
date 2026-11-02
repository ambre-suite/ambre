//! Index families: a position, a count, a difference and a span, one generic
//! type each over an axis marker.
//!
//! A row number, a number of rows and the gap between two rows are three
//! quantities. A bare integer lets one stand for another, and a row passed
//! where a column is expected compiles. Each type here takes the axis as its
//! parameter, so `Pos<Row>` never mixes with `Pos<Col>`. Each carries only the
//! arithmetic the domain allows (`ai/ambre-rust-style.md`, the newtype rule).
//!
//! ```
//! use ambre_core::index::{Axis, Count, Delta, Pos, Span};
//!
//! /// A line of a list.
//! #[derive(PartialEq, Eq)]
//! enum Line {}
//! impl Axis for Line {
//!     const NAME: &'static str = "Line";
//! }
//!
//! let first = Pos::<Line>::new(2);
//! let last = first + Count::new(3);
//! assert_eq!(last - first, Delta::new(3));
//! assert_eq!(Span::new(first, last).into_iter().count(), 3);
//! ```
//!
//! The operators are the ones a position allows:
//!
//! | Expression | Result | On overflow |
//! | :--- | :--- | :--- |
//! | `Pos + Count` | `Pos` | saturates |
//! | `Pos - Count` | `Pos` | saturates at zero |
//! | `Pos + Delta` | `Pos` | clamps to the axis |
//! | `Pos - Pos` | `Delta` | exact, in the wider signed type |
//! | `Delta::from_count(count)` | `Delta` | exact |
//! | `pos.checked_sub(count)` | `Option<Pos>` | `None` |
//! | `Count + Count` | `Count` | saturates |
//! | `pos.checked_offset(delta)` | `Option<Pos>` | `None` |
//! | `pos.saturating_offset(delta)` | `Pos` | clamps to the axis |
//!
//! `Pos + Pos` does not compile. No type derefs to its integer. None converts
//! from it through `From`: `new` wraps it, and `get` unwraps it for the call
//! into a foreign API. Each of `Pos`, `Count` and `Delta` serialises as its
//! bare integer, the shape `#[serde(transparent)]` writes.
//!
//! Every axis is 32 bits wide. A position or a count is a `u32`, and a
//! difference is an `i64`, which holds the gap between any two positions.
//! Postcard writes a `u32` and a `usize` of one value as the same varint, and
//! JSON writes the same digits. So an axis that held a `usize` before keeps
//! its bytes on the wire and in the store, for every value a 32-bit axis
//! holds. The client is wasm32, where a `usize` is 32 bits too.
//!
//! [`IdxVec`] takes its position in `[]`, as a slice takes a `usize`. The
//! operator panics past the end, as a slice index does. The workspace forbids
//! that panic outside a test, so production code calls `get`, which returns
//! an `Option`.
//!
//! A span is half-open, and no position lies past `MAX`. So
//! `pos.through(MAX)` stops before `MAX` and leaves it out, and
//! `MAX.through(MAX)` is empty. No Ambre axis reaches its top, so no caller
//! loses a position it holds.
//!
//! The span exists because `core::iter::Step` is unstable (tracking issue
//! rust-lang/rust#73121), so a `Range` of a newtype cannot iterate. When
//! `Step` reaches stable, implement it for [`Pos`], replace [`Span`] with
//! `Range`, and delete [`Span`].
//!
//! An axis opts in to more through a capability trait:
//!
//! | Trait | What the axis gets |
//! | :--- | :--- |
//! | [`Numbered`] | the number a reader sees for a position |
//! | [`Shown`] | the `Display` of a count |
//! | [`Circular`] | [`Pos::wrapped`], a move that wraps at either end |
//! | [`CountsAs`] | the crossing of a count to an axis that counts the same thing |
//! | [`MapsTo`] | the crossing of a position and a span as well |

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::iter::Sum;
use std::marker::PhantomData;
use std::ops::{Add, AddAssign, Index, IndexMut, Range, Sub, SubAssign};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::mode::StepDirection;

/// The marker of one axis: the rows of a sheet, the pages of a layout.
///
/// The marker is an uninhabited type: it names the axis and holds no value.
/// The trait is open, because the axes live in the crates that index them.
///
/// The marker derives `PartialEq` and `Eq`. A position, a count and a
/// difference derive their equality, since only a derive makes it structural.
/// A constant such as `Pos::<Row>::ZERO` then works as a match pattern. The
/// derive bounds the marker, and this supertrait meets that bound.
pub trait Axis: Eq + Sized + 'static {
    /// The name `Debug` prints before the kind of value: `RowCount(3)`,
    /// `RowPos(3)`. A count prints as the hand-written type of its axis
    /// printed; a position prints the kind `Pos` after the name.
    const NAME: &'static str;
}

/// An axis whose positions a reader sees as numbers: a page, a slide.
///
/// The axis counts from zero, and a reader counts from one. [`Pos::number`]
/// is the one way from the position to that number.
pub trait Numbered: Axis {
    /// The number a reader sees.
    type Number;

    /// The number a reader sees for `pos`.
    fn number_of(pos: Pos<Self>) -> Self::Number;
}

/// An axis whose count a reader sees: [`Count`] of it implements `Display`
/// as the bare number.
pub trait Shown: Axis {}

/// An axis whose positions wrap: past the last comes the first.
///
/// A mention list or a match cursor wraps by design. A page, a row or a line
/// that wraps is a bug, so [`Pos::wrapped`] exists only on an axis that opts
/// in here.
pub trait Circular: Axis {}

/// An axis whose counts are counts of axis `B`: the same number of things.
///
/// The impl states the condition under which the two axes count the same
/// thing. [`Count::as_axis`] then crosses from one axis to the other. A
/// position does not cross: the two axes may start at different places, as a
/// window of pages starts at some page of the document. A crossing back is
/// its own impl.
pub trait CountsAs<B: Axis>: Axis {}

/// An axis whose positions are the positions of axis `B`, one for one: both
/// start at the same place and count the same thing.
///
/// [`Pos::as_axis`] and [`Span::as_axis`] then cross from one axis to the
/// other, and a count crosses through [`CountsAs`]. A crossing back is its
/// own impl.
pub trait MapsTo<B: Axis>: CountsAs<B> {}

/// A position on the axis `A`: a row, a page, a code-point offset.
#[derive(PartialEq, Eq)]
pub struct Pos<A: Axis> {
    raw: u32,
    axis: PhantomData<fn() -> A>,
}

/// A number of positions on the axis `A`.
#[derive(PartialEq, Eq)]
pub struct Count<A: Axis> {
    raw: u32,
    axis: PhantomData<fn() -> A>,
}

/// The signed gap between two positions on the axis `A`.
#[derive(PartialEq, Eq)]
pub struct Delta<A: Axis> {
    raw: i64,
    axis: PhantomData<fn() -> A>,
}

/// A run of positions on the axis `A`.
///
/// Half-open: it holds `start` and stops before `end`. A span whose end is
/// not past its start is empty.
#[derive(PartialEq, Eq)]
pub struct Span<A: Axis> {
    /// The first position of the span.
    pub start: Pos<A>,
    /// The first position past the span.
    pub end: Pos<A>,
}

/// A list indexed by a position on the axis `A`.
pub struct IdxVec<A: Axis, T> {
    items: Vec<T>,
    axis: PhantomData<fn() -> A>,
}

impl<A: Axis> Pos<A> {
    /// The first position.
    pub const ZERO: Self = Self::new(0);
    /// The last position the axis holds.
    pub const MAX: Self = Self::new(u32::MAX);

    /// Wraps a position.
    pub const fn new(value: u32) -> Self {
        Self {
            raw: value,
            axis: PhantomData,
        }
    }

    /// The position, for the call into a foreign API.
    pub const fn get(self) -> u32 {
        self.raw
    }

    /// The position past this one, saturating at [`Self::MAX`].
    #[must_use]
    pub const fn next(self) -> Self {
        Self::new(self.raw.saturating_add(1))
    }

    /// The position before this one, `None` at [`Self::ZERO`].
    pub const fn prev(self) -> Option<Self> {
        match self.raw.checked_sub(1) {
            Some(value) => Some(Self::new(value)),
            None => None,
        }
    }

    /// The position `count` before this one, `None` past the start.
    pub const fn checked_sub(self, count: Count<A>) -> Option<Self> {
        match self.raw.checked_sub(count.raw) {
            Some(value) => Some(Self::new(value)),
            None => None,
        }
    }

    /// The position `count` before this one, clamped to [`Self::ZERO`].
    #[must_use]
    pub const fn saturating_sub(self, count: Count<A>) -> Self {
        Self::new(self.raw.saturating_sub(count.raw))
    }

    /// The position `delta` away, `None` off either end of the axis.
    pub fn checked_offset(self, delta: Delta<A>) -> Option<Self> {
        let moved = self.widen().checked_add(delta.raw)?;
        u32::try_from(moved).ok().map(Self::new)
    }

    /// The position `delta` away, clamped to the axis.
    #[must_use]
    pub fn saturating_offset(self, delta: Delta<A>) -> Self {
        let moved = self.widen().saturating_add(delta.raw);
        match u32::try_from(moved) {
            Ok(value) => Self::new(value),
            Err(_) if moved < 0 => Self::ZERO,
            Err(_) => Self::MAX,
        }
    }

    /// This position, or the last of an axis of `len` positions when it lies
    /// past it. [`Self::ZERO`] when `len` is zero.
    #[must_use]
    pub const fn within(self, len: Count<A>) -> Self {
        let last = match len.last() {
            Some(last) => last,
            None => Self::ZERO,
        };
        if self.raw < last.raw { self } else { last }
    }

    /// The position halfway from this one to `end`, rounded towards this
    /// one. This position when `end` is not past it.
    #[must_use]
    pub const fn midpoint(self, end: Self) -> Self {
        Self::new(
            self.raw
                .saturating_add(end.raw.saturating_sub(self.raw) / 2),
        )
    }

    /// The number of positions from `start` up to this one, zero when
    /// `start` is not before it.
    pub const fn count_from(self, start: Self) -> Count<A> {
        Count::new(self.raw.saturating_sub(start.raw))
    }

    /// The span from this position up to and with `last`.
    ///
    /// A span stops before its end, and no position lies past
    /// [`Self::MAX`]. So `through(MAX)` holds every position up to
    /// `MAX` and leaves `MAX` out.
    pub const fn through(self, last: Self) -> Span<A> {
        Span::new(self, last.next())
    }

    /// The same position on the axis `B`, which counts what this axis
    /// counts under the condition the [`MapsTo`] impl states.
    pub const fn as_axis<B: Axis>(self) -> Pos<B>
    where
        A: MapsTo<B>,
    {
        Pos::new(self.raw)
    }

    /// The position at `index` of a slice, `None` past the axis.
    pub fn from_index(index: usize) -> Option<Self> {
        u32::try_from(index).ok().map(Self::new)
    }

    /// The index of a slice this position names, `None` past what the
    /// platform addresses.
    pub fn to_index(self) -> Option<usize> {
        usize::try_from(self.raw).ok()
    }

    /// The position at `index` of a slice, saturating at [`Self::MAX`]: for
    /// the value a foreign API hands back.
    pub fn from_usize(index: usize) -> Self {
        Self::new(u32::try_from(index).unwrap_or(u32::MAX))
    }

    /// The position as a slice index, saturating at `usize::MAX`: for the
    /// call into a foreign API that takes a `usize`.
    pub fn to_usize(self) -> usize {
        usize::try_from(self.raw).unwrap_or(usize::MAX)
    }

    /// The position in the signed type of the difference.
    fn widen(self) -> i64 {
        i64::from(self.raw)
    }
}

impl<A: Circular> Pos<A> {
    /// The position `by` away on an axis of `len` positions, wrapping at
    /// either end. Past the last comes the first, and before the first comes
    /// the last. A position past `len` wraps as if it stood at the last one.
    /// `None` when `len` is zero, since the axis then holds no position.
    pub fn wrapped(self, by: Delta<A>, len: Count<A>) -> Option<Self> {
        if len.is_zero() {
            return None;
        }
        let at = i64::from(self.within(len).raw);
        let moved = (at + by.raw.rem_euclid(i64::from(len.raw))).rem_euclid(i64::from(len.raw));
        u32::try_from(moved).ok().map(Self::new)
    }
}

impl<A: Numbered> Pos<A> {
    /// The number a reader sees for this position.
    pub fn number(self) -> A::Number {
        A::number_of(self)
    }
}

impl<A: Axis> Count<A> {
    /// No position.
    pub const ZERO: Self = Self::new(0);
    /// One position.
    pub const ONE: Self = Self::new(1);

    /// Wraps a count.
    pub const fn new(value: u32) -> Self {
        Self {
            raw: value,
            axis: PhantomData,
        }
    }

    /// The count, for the call into a foreign API.
    pub const fn get(self) -> u32 {
        self.raw
    }

    /// Whether the count is zero.
    pub const fn is_zero(self) -> bool {
        self.raw == 0
    }

    /// This count less `other`, clamped to zero.
    #[must_use]
    pub const fn saturating_sub(self, other: Self) -> Self {
        Self::new(self.raw.saturating_sub(other.raw))
    }

    /// The count of a slice of `len` items, saturating past the axis.
    pub fn from_len(len: usize) -> Self {
        Self::new(u32::try_from(len).unwrap_or(u32::MAX))
    }

    /// The count as a `usize`, saturating at `usize::MAX`: for the call into
    /// a foreign API that takes a length.
    pub fn to_usize(self) -> usize {
        usize::try_from(self.raw).unwrap_or(usize::MAX)
    }

    /// The position this count reaches from the start of the axis: the first
    /// position past `count` items.
    pub const fn as_end(self) -> Pos<A> {
        Pos::new(self.raw)
    }

    /// The number of positions from the first up to and with `last`.
    pub const fn through(last: Pos<A>) -> Self {
        last.next().count_from(Pos::ZERO)
    }

    /// The last of this many positions, `None` for none.
    pub const fn last(self) -> Option<Pos<A>> {
        self.as_end().prev()
    }

    /// Every position of this many, from the first.
    pub const fn span(self) -> Span<A> {
        Span::new(Pos::ZERO, self.as_end())
    }

    /// The same count on the axis `B`, which counts what this axis counts
    /// under the condition the [`CountsAs`] impl states.
    pub const fn as_axis<B: Axis>(self) -> Count<B>
    where
        A: CountsAs<B>,
    {
        Count::new(self.raw)
    }
}

impl<A: Axis> Delta<A> {
    /// No move.
    pub const ZERO: Self = Self::new(0);

    /// Wraps a difference.
    pub const fn new(value: i64) -> Self {
        Self {
            raw: value,
            axis: PhantomData,
        }
    }

    /// The difference, for the call into a foreign API.
    pub const fn get(self) -> i64 {
        self.raw
    }

    /// The same move, in the other direction.
    #[must_use]
    pub const fn reversed(self) -> Self {
        Self::new(self.raw.saturating_neg())
    }

    /// The move of one position in `direction`.
    pub const fn step(direction: StepDirection) -> Self {
        match direction {
            StepDirection::Forward => Self::new(1),
            StepDirection::Backward => Self::new(-1),
        }
    }

    /// The forward move of `count` positions.
    pub fn from_count(count: Count<A>) -> Self {
        Self::new(i64::from(count.raw))
    }

    /// Whether the move goes back.
    pub const fn is_negative(self) -> bool {
        self.raw < 0
    }

    /// How many positions the move covers, either way, saturating past the
    /// axis.
    pub fn magnitude(self) -> Count<A> {
        Count::new(u32::try_from(self.raw.unsigned_abs()).unwrap_or(u32::MAX))
    }
}

impl<A: Axis> Span<A> {
    /// The span from `start` up to, and without, `end`.
    pub const fn new(start: Pos<A>, end: Pos<A>) -> Self {
        Self { start, end }
    }

    /// The span of `count` positions from `start`.
    pub const fn from_count(start: Pos<A>, count: Count<A>) -> Self {
        Self::new(start, Pos::new(start.raw.saturating_add(count.raw)))
    }

    /// The number of positions the span holds.
    pub const fn len(self) -> Count<A> {
        self.end.count_from(self.start)
    }

    /// Whether the span holds no position.
    pub const fn is_empty(self) -> bool {
        self.end.raw <= self.start.raw
    }

    /// Whether the span holds `pos`.
    pub const fn contains(self, pos: Pos<A>) -> bool {
        self.start.raw <= pos.raw && pos.raw < self.end.raw
    }

    /// The span of the slice indices `range`, saturating at the end of the
    /// axis: for the range a foreign API hands back.
    pub fn from_usize_range(range: Range<usize>) -> Self {
        Self::new(Pos::from_usize(range.start), Pos::from_usize(range.end))
    }

    /// The span as slice indices, saturating at `usize::MAX`: for the call
    /// into a foreign API that takes a range.
    pub fn to_usize_range(self) -> Range<usize> {
        self.start.to_usize()..self.end.to_usize()
    }

    /// The last position the span holds, `None` when it is empty.
    pub const fn last(self) -> Option<Pos<A>> {
        if self.is_empty() {
            None
        } else {
            self.end.prev()
        }
    }

    /// The same span on the axis `B`, which counts what this axis counts
    /// under the condition the [`MapsTo`] impl states.
    pub const fn as_axis<B: Axis>(self) -> Span<B>
    where
        A: MapsTo<B>,
    {
        Span::new(self.start.as_axis(), self.end.as_axis())
    }

    /// The first position of the span where `pred` holds, or `end` when it
    /// holds at none. `pred` is monotone over the span: once it holds, it
    /// holds at every later position. The search halves the span at each
    /// step, so it reads `pred` a logarithmic number of times.
    pub fn first_where(self, pred: impl Fn(Pos<A>) -> bool) -> Pos<A> {
        let (mut low, mut high) = (self.start.raw, self.end.raw.max(self.start.raw));
        while low < high {
            let mid = low + (high - low) / 2;
            if pred(Pos::new(mid)) {
                high = mid;
            } else {
                low = mid + 1;
            }
        }
        Pos::new(low)
    }
}

impl<A: Axis> IntoIterator for Span<A> {
    type Item = Pos<A>;
    type IntoIter = std::iter::Map<Range<u32>, fn(u32) -> Pos<A>>;

    fn into_iter(self) -> Self::IntoIter {
        (self.start.raw..self.end.raw).map(Pos::new as fn(u32) -> Pos<A>)
    }
}

impl<A: Axis> Add<Count<A>> for Pos<A> {
    type Output = Pos<A>;

    fn add(self, count: Count<A>) -> Pos<A> {
        Pos::new(self.raw.saturating_add(count.raw))
    }
}

impl<A: Axis> AddAssign<Count<A>> for Pos<A> {
    fn add_assign(&mut self, count: Count<A>) {
        *self = *self + count;
    }
}

impl<A: Axis> Sub<Count<A>> for Pos<A> {
    type Output = Pos<A>;

    fn sub(self, count: Count<A>) -> Pos<A> {
        self.saturating_sub(count)
    }
}

impl<A: Axis> SubAssign<Count<A>> for Pos<A> {
    fn sub_assign(&mut self, count: Count<A>) {
        *self = *self - count;
    }
}

impl<A: Axis> Add<Delta<A>> for Pos<A> {
    type Output = Pos<A>;

    fn add(self, delta: Delta<A>) -> Pos<A> {
        self.saturating_offset(delta)
    }
}

impl<A: Axis> AddAssign<Delta<A>> for Pos<A> {
    fn add_assign(&mut self, delta: Delta<A>) {
        *self = *self + delta;
    }
}

impl<A: Axis> Sub for Pos<A> {
    type Output = Delta<A>;

    fn sub(self, other: Pos<A>) -> Delta<A> {
        Delta::new(self.widen() - other.widen())
    }
}

impl<A: Axis> Add for Count<A> {
    type Output = Count<A>;

    fn add(self, other: Count<A>) -> Count<A> {
        Count::new(self.raw.saturating_add(other.raw))
    }
}

impl<A: Axis> AddAssign for Count<A> {
    fn add_assign(&mut self, other: Count<A>) {
        *self = *self + other;
    }
}

impl<A: Axis> Add for Delta<A> {
    type Output = Delta<A>;

    fn add(self, other: Delta<A>) -> Delta<A> {
        Delta::new(self.raw.saturating_add(other.raw))
    }
}

impl<A: Axis> Sum for Delta<A> {
    fn sum<I: Iterator<Item = Delta<A>>>(iter: I) -> Delta<A> {
        iter.fold(Self::ZERO, |sum, delta| sum + delta)
    }
}

impl<A: Shown> fmt::Display for Count<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.raw.fmt(f)
    }
}

impl<A: Axis, T> IdxVec<A, T> {
    /// An empty collection.
    pub const fn new() -> Self {
        Self::from_vec(Vec::new())
    }

    /// The collection of `items`, the first at [`Pos::ZERO`].
    const fn from_vec(items: Vec<T>) -> Self {
        Self {
            items,
            axis: PhantomData,
        }
    }

    /// The number of items.
    pub fn len(&self) -> Count<A> {
        Count::from_len(self.items.len())
    }

    /// Whether the collection holds no item.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The item at `pos`, `None` past the end.
    pub fn get(&self, pos: Pos<A>) -> Option<&T> {
        self.items.get(pos.to_index()?)
    }

    /// The item at `pos`, mutable, `None` past the end.
    pub fn get_mut(&mut self, pos: Pos<A>) -> Option<&mut T> {
        self.items.get_mut(pos.to_index()?)
    }

    /// Appends `item`, and returns the position it takes.
    pub fn push(&mut self, item: T) -> Pos<A> {
        let pos = self.len().as_end();
        self.items.push(item);
        pos
    }

    /// Inserts `item` at `pos`, shifting the items after it.
    ///
    /// # Errors
    ///
    /// Returns the item back when `pos` lies past the end.
    pub fn insert(&mut self, pos: Pos<A>, item: T) -> Result<(), T> {
        match pos.to_index() {
            Some(index) if index <= self.items.len() => {
                self.items.insert(index, item);
                Ok(())
            }
            _ => Err(item),
        }
    }

    /// Removes the item at `pos`, `None` past the end.
    pub fn remove(&mut self, pos: Pos<A>) -> Option<T> {
        let index = pos.to_index().filter(|index| *index < self.items.len())?;
        Some(self.items.remove(index))
    }

    /// The items in order.
    pub fn as_slice(&self) -> &[T] {
        &self.items
    }

    /// The items in order.
    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.items.iter()
    }

    /// The item at the first position, `None` when the list is empty.
    pub fn first(&self) -> Option<&T> {
        self.items.first()
    }

    /// The items in order, each one writable in place. The positions stay as
    /// they are: nothing is added or removed.
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, T> {
        self.items.iter_mut()
    }

    /// Each item beside its position, in order.
    pub fn iter_enumerated(&self) -> impl Iterator<Item = (Pos<A>, &T)> + '_ {
        self.items
            .iter()
            .enumerate()
            .map(|(index, item)| (Pos::from_usize(index), item))
    }

    /// The span of every position the collection holds.
    pub fn positions(&self) -> Span<A> {
        self.len().span()
    }
}

impl<A: Axis, T> Default for IdxVec<A, T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A: Axis, T> From<Vec<T>> for IdxVec<A, T> {
    fn from(items: Vec<T>) -> Self {
        Self::from_vec(items)
    }
}

impl<A: Axis, T> FromIterator<T> for IdxVec<A, T> {
    fn from_iter<I: IntoIterator<Item = T>>(items: I) -> Self {
        Self::from_vec(items.into_iter().collect())
    }
}

impl<A: Axis, T> Index<Pos<A>> for IdxVec<A, T> {
    type Output = T;

    /// The item at `pos`. It panics past the end, as a slice index does;
    /// `get` is the call that cannot.
    #[allow(
        clippy::indexing_slicing,
        reason = "the operator keeps the slice contract; `get` is the checked call"
    )]
    fn index(&self, pos: Pos<A>) -> &T {
        &self.items[pos.to_usize()]
    }
}

impl<A: Axis, T> IndexMut<Pos<A>> for IdxVec<A, T> {
    /// The item at `pos`, mutable. It panics past the end, as a slice index
    /// does; `get_mut` is the call that cannot.
    #[allow(
        clippy::indexing_slicing,
        reason = "the operator keeps the slice contract; `get_mut` is the checked call"
    )]
    fn index_mut(&mut self, pos: Pos<A>) -> &mut T {
        &mut self.items[pos.to_usize()]
    }
}

impl<'a, A: Axis, T> IntoIterator for &'a IdxVec<A, T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

impl<'a, A: Axis, T> IntoIterator for &'a mut IdxVec<A, T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter_mut()
    }
}

// By hand, every impl below: a derive would bound the marker, which has no
// value to clone, order, hash or print. Equality alone is derived, for the
// reason the doc of `Axis` gives.

impl<A: Axis> Clone for Pos<A> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<A: Axis> Copy for Pos<A> {}

impl<A: Axis> Default for Pos<A> {
    fn default() -> Self {
        Self::new(0)
    }
}

impl<A: Axis> PartialOrd for Pos<A> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<A: Axis> Ord for Pos<A> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.raw.cmp(&other.raw)
    }
}

impl<A: Axis> Hash for Pos<A> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}

/// `NAME` and the kind, then the bare integer: `RowCount(3)`.
impl<A: Axis> fmt::Debug for Pos<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}Pos({})", A::NAME, self.raw)
    }
}

/// The bare integer, so the wire keeps its bytes.
impl<A: Axis> Serialize for Pos<A> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.raw.serialize(serializer)
    }
}

impl<'de, A: Axis> Deserialize<'de> for Pos<A> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        u32::deserialize(deserializer).map(Self::new)
    }
}

impl<A: Axis> Clone for Count<A> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<A: Axis> Copy for Count<A> {}

impl<A: Axis> Default for Count<A> {
    fn default() -> Self {
        Self::new(0)
    }
}

impl<A: Axis> PartialOrd for Count<A> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<A: Axis> Ord for Count<A> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.raw.cmp(&other.raw)
    }
}

impl<A: Axis> Hash for Count<A> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}

/// `NAME` and the kind, then the bare integer: `RowCount(3)`.
impl<A: Axis> fmt::Debug for Count<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}Count({})", A::NAME, self.raw)
    }
}

/// The bare integer, so the wire keeps its bytes.
impl<A: Axis> Serialize for Count<A> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.raw.serialize(serializer)
    }
}

impl<'de, A: Axis> Deserialize<'de> for Count<A> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        u32::deserialize(deserializer).map(Self::new)
    }
}

impl<A: Axis> Clone for Delta<A> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<A: Axis> Copy for Delta<A> {}

impl<A: Axis> Default for Delta<A> {
    fn default() -> Self {
        Self::new(0)
    }
}

impl<A: Axis> PartialOrd for Delta<A> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<A: Axis> Ord for Delta<A> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.raw.cmp(&other.raw)
    }
}

impl<A: Axis> Hash for Delta<A> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}

/// `NAME` and the kind, then the bare integer: `RowCount(3)`.
impl<A: Axis> fmt::Debug for Delta<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}Delta({})", A::NAME, self.raw)
    }
}

/// The bare integer, so the wire keeps its bytes.
impl<A: Axis> Serialize for Delta<A> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.raw.serialize(serializer)
    }
}

impl<'de, A: Axis> Deserialize<'de> for Delta<A> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        i64::deserialize(deserializer).map(Self::new)
    }
}

impl<A: Axis> Clone for Span<A> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<A: Axis> Copy for Span<A> {}

impl<A: Axis> Default for Span<A> {
    fn default() -> Self {
        Self::new(Pos::ZERO, Pos::ZERO)
    }
}

impl<A: Axis> Hash for Span<A> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.start.hash(state);
        self.end.hash(state);
    }
}

/// `NAME` and the kind, then both ends: `RowSpan { start: .., end: .. }`.
impl<A: Axis> fmt::Debug for Span<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}Span {{ start: {:?}, end: {:?} }}",
            A::NAME,
            self.start,
            self.end
        )
    }
}

impl<A: Axis, T: Clone> Clone for IdxVec<A, T> {
    fn clone(&self) -> Self {
        Self::from_vec(self.items.clone())
    }
}

impl<A: Axis, T: PartialEq> PartialEq for IdxVec<A, T> {
    fn eq(&self, other: &Self) -> bool {
        self.items == other.items
    }
}

impl<A: Axis, T: Eq> Eq for IdxVec<A, T> {}

impl<A: Axis, T: Hash> Hash for IdxVec<A, T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.items.hash(state);
    }
}

/// `NAME` and the kind, then the items: `RowVec([..])`.
impl<A: Axis, T: fmt::Debug> fmt::Debug for IdxVec<A, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}Vec(", A::NAME)?;
        f.debug_list().entries(&self.items).finish()?;
        f.write_str(")")
    }
}

#[cfg(test)]
mod tests {
    use super::{Axis, Circular, Count, CountsAs, Delta, IdxVec, MapsTo, Pos, Span};

    /// A test line.
    #[derive(PartialEq, Eq)]
    enum Line {}

    impl Axis for Line {
        const NAME: &'static str = "Line";
    }

    /// A second test axis, one for one with [`Line`].
    #[derive(PartialEq, Eq)]
    enum Mirror {}

    impl Axis for Mirror {
        const NAME: &'static str = "Mirror";
    }

    impl CountsAs<Mirror> for Line {}

    impl MapsTo<Mirror> for Line {}

    impl Circular for Line {}

    fn line(raw: u32) -> Pos<Line> {
        Pos::new(raw)
    }

    fn count(raw: u32) -> Count<Line> {
        Count::new(raw)
    }

    fn delta(raw: i64) -> Delta<Line> {
        Delta::new(raw)
    }

    #[test]
    fn a_position_plus_a_count_is_a_position() {
        assert_eq!(line(2) + count(3), line(5));
        assert_eq!(Pos::<Line>::MAX + Count::ONE, Pos::<Line>::MAX);
    }

    #[test]
    fn two_positions_differ_by_a_signed_delta() {
        assert_eq!(line(2) - line(5), delta(-3));
        assert_eq!(
            Pos::<Line>::MAX - Pos::<Line>::ZERO,
            delta(i64::from(u32::MAX))
        );
    }

    #[test]
    fn an_offset_off_the_axis_is_checked_or_clamped() {
        let at = line(1);
        assert_eq!(at.checked_offset(delta(-1)), Some(Pos::<Line>::ZERO));
        assert_eq!(at.checked_offset(delta(-2)), None);
        assert_eq!(at.saturating_offset(delta(-2)), Pos::<Line>::ZERO);
        assert_eq!(at.saturating_offset(delta(i64::MAX)), Pos::<Line>::MAX);
    }

    #[test]
    fn a_count_moves_a_position_back_and_a_delta_moves_it_either_way() {
        assert_eq!(line(5) - count(3), line(2));
        assert_eq!(line(1) - count(3), Pos::<Line>::ZERO);
        assert_eq!(line(1).checked_sub(count(3)), None);
        let back = Delta::from_count(count(2)).reversed();
        assert_eq!(line(5) + back, line(3));
        assert_eq!(line(1) + back, Pos::<Line>::ZERO);
    }

    #[test]
    fn a_span_iterates_its_positions_and_stops_before_its_end() {
        let span = line(1).through(line(3));
        let lines: Vec<Pos<Line>> = span.into_iter().collect();
        assert_eq!(lines, vec![line(1), line(2), line(3)]);
        assert_eq!(span.len(), count(3));
        assert!(span.contains(line(3)) && !span.contains(line(4)));
        assert!(Span::new(line(4), line(2)).is_empty());
    }

    #[test]
    fn a_span_finds_the_first_position_a_monotone_test_holds_at() {
        let span = Span::new(line(2), line(9));
        assert_eq!(span.first_where(|at| at >= line(5)), line(5));
        assert_eq!(span.first_where(|_| true), line(2));
        assert_eq!(span.first_where(|_| false), line(9));
        let empty = Span::new(line(4), line(2));
        assert_eq!(empty.first_where(|_| true), line(4));
    }

    #[test]
    fn the_collection_refuses_a_position_past_its_end() {
        let mut items: IdxVec<Line, char> = IdxVec::new();
        assert_eq!(items.push('a'), Pos::<Line>::ZERO);
        assert_eq!(items.insert(line(5), 'z'), Err('z'));
        assert_eq!(items.insert(line(1), 'b'), Ok(()));
        assert_eq!(items.get(line(1)), Some(&'b'));
        assert_eq!(items.remove(line(9)), None);
        assert_eq!(items.len(), count(2));
    }

    #[test]
    fn the_collection_takes_its_position_in_brackets() {
        let mut items: IdxVec<Line, char> = vec!['a', 'b'].into();
        items[line(1)] = 'c';
        assert_eq!(items[Pos::<Line>::ZERO], 'a');
        assert_eq!(items[line(1)], 'c');
    }

    #[test]
    #[should_panic(expected = "index out of bounds")]
    fn the_bracket_panics_past_the_end() {
        let items: IdxVec<Line, char> = vec!['a'].into();
        let _ = items[line(1)];
    }

    #[test]
    fn a_delta_covers_its_magnitude_either_way() {
        assert_eq!(delta(-3).magnitude(), count(3));
        assert_eq!(delta(i64::MIN).magnitude(), count(u32::MAX));
        assert!(delta(-1).is_negative() && !Delta::<Line>::ZERO.is_negative());
        let sum: Delta<Line> = [delta(4), delta(-6)].into_iter().sum();
        assert_eq!(sum, delta(-2));
    }

    #[test]
    fn a_span_through_the_last_position_leaves_it_out() {
        let span = line(u32::MAX - 1).through(Pos::<Line>::MAX);
        assert_eq!(
            span.into_iter().collect::<Vec<_>>(),
            vec![line(u32::MAX - 1)]
        );
        assert!(!span.contains(Pos::<Line>::MAX));
        assert_eq!(span.len(), Count::ONE);
        assert_eq!(
            Pos::<Line>::MAX.through(Pos::<Line>::MAX).len(),
            Count::ZERO
        );
    }

    #[test]
    fn a_count_names_its_last_position_and_its_span() {
        assert_eq!(count(3).last(), Some(line(2)));
        assert_eq!(Count::<Line>::ZERO.last(), None);
        assert_eq!(count(3).span(), Span::new(Pos::<Line>::ZERO, line(3)));
        assert_eq!(Count::through(line(4)), count(5));
    }

    #[test]
    fn a_wrapped_position_comes_round_at_either_end() {
        let len = count(3);
        assert_eq!(line(2).wrapped(delta(1), len), Some(Pos::<Line>::ZERO));
        assert_eq!(Pos::<Line>::ZERO.wrapped(delta(-1), len), Some(line(2)));
        assert_eq!(line(1).wrapped(delta(8), len), Some(Pos::<Line>::ZERO));
        assert_eq!(line(9).wrapped(delta(1), len), Some(Pos::<Line>::ZERO));
        assert_eq!(Pos::<Line>::ZERO.wrapped(delta(1), Count::ZERO), None);
    }

    #[test]
    fn a_position_within_a_length_stops_at_its_last() {
        assert_eq!(line(7).within(count(3)), line(2));
        assert_eq!(line(1).within(count(3)), line(1));
        assert_eq!(line(1).within(Count::ZERO), Pos::<Line>::ZERO);
        assert_eq!(line(2).midpoint(line(7)), line(4));
        assert_eq!(line(7).midpoint(line(2)), line(7));
    }

    #[test]
    fn a_mapped_axis_keeps_the_position() {
        let mirrored: Pos<Mirror> = line(4).as_axis();
        assert_eq!(mirrored, Pos::new(4));
        assert_eq!(count(2).as_axis::<Mirror>(), Count::new(2));
    }

    #[test]
    fn debug_prints_the_axis_name_and_the_kind() {
        assert_eq!(format!("{:?}", count(12)), "LineCount(12)");
        assert_eq!(format!("{:?}", line(3)), "LinePos(3)");
        assert_eq!(
            format!("{:?}", Span::new(line(1), line(2))),
            "LineSpan { start: LinePos(1), end: LinePos(2) }"
        );
    }

    #[test]
    fn each_type_serialises_as_its_bare_integer() {
        let at = postcard::to_allocvec(&line(300)).unwrap();
        assert_eq!(at, postcard::to_allocvec(&300u32).unwrap());
        assert_eq!(at, postcard::to_allocvec(&300usize).unwrap());
        let back: Delta<Line> =
            postcard::from_bytes(&postcard::to_allocvec(&-7i64).unwrap()).unwrap();
        assert_eq!(back, delta(-7));
        let wide: Pos<Line> =
            postcard::from_bytes(&postcard::to_allocvec(&300usize).unwrap()).unwrap();
        assert_eq!(wide, line(300));
    }
}
