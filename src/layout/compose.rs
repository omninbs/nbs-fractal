//! Composite layouts: wrappers that position other layouts relative to one
//! another, and the floor they may stand on.

use super::base::{Axis, Layout, Mask};
use crate::{BlockPos, BlockState};
use std::collections::HashSet;

// Provided
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Binds a context to a subtree, erasing it into [`Layout<()>`](Layout).
///
/// The context becomes an ordinary field rather than a borrow, so a finished
/// subtree carries no lifetime and every leaf below reaches the shared
/// knowledge through its queries alone.
pub struct Provided<Ctx, L> {
    ctx: Ctx,
    inner: L,
}

impl<Ctx, L> Provided<Ctx, L> {
    /// Binds `ctx` to `inner`, which was built within it.
    pub fn new(ctx: Ctx, inner: L) -> Self {
        Self { ctx, inner }
    }
}

impl<Ctx, L: Layout<Ctx>> Layout<()> for Provided<Ctx, L> {
    fn block_at_with(&self, pos: BlockPos, _ctx: &()) -> Option<BlockState> {
        self.inner.block_at_with(pos, &self.ctx)
    }

    fn size_with(&self, _ctx: &()) -> BlockPos {
        self.inner.size_with(&self.ctx)
    }

    fn block_palette_with(&self, _ctx: &()) -> HashSet<BlockState> {
        self.inner.block_palette_with(&self.ctx)
    }
}

// Arranged & EdgeArranged
//
// ++++++++++++============++++++++++++============++++++++++++============

/// A layout wrapper that arranges sub-layouts along an [`Axis`].
///
/// Each item is anchored `gap` past the far edge of the previous one, so the
/// edge-to-edge spacing is fixed regardless of item size. A negative `gap`
/// pulls items together: they overlap, and a later item covers an earlier one
/// wherever both draw. The arrangement is shifted so its bounding box starts at
/// the origin even when negative gaps push anchors backwards.
///
/// Query cost is O(log n) with `n` the number of sub-layouts while the anchors
/// stay ordered along the axis, which every non-negative `gap` guarantees. A
/// `gap` negative enough to reverse the anchors falls back to a linear scan.
pub struct Arranged<L> {
    bands: Vec<(L, BlockPos)>,
    /// Anchor of each band along the arrangement axis, in band order.
    starts: Vec<i32>,
    /// Running maximum of the far edges along the axis, for interval lookup.
    ends: Vec<i32>,
    /// Whether `starts` is non-decreasing and can be binary-searched.
    ordered: bool,
    axis: Axis,
    size: BlockPos,
}

impl<L> Arranged<L> {
    /// Places `layouts` along `axis`, each anchored `gap` past the far edge of
    /// the previous one.
    pub fn new<Ctx, I>(layouts: I, axis: Axis, gap: i32, ctx: &Ctx) -> Self
    where
        I: IntoIterator<Item = L>,
        L: Layout<Ctx>,
    {
        let unit: Mask = axis.unit();
        let mut cursor: i32 = 0;
        let mut start_min: i32 = 0;
        let mut far_max: i32 = 0;
        let mut extent: BlockPos = BlockPos::ORIGIN;
        let mut placed = Vec::new();

        for layout in layouts {
            let size: BlockPos = layout.size_with(ctx);
            let start = cursor;
            let far = start + unit.dot(size);
            cursor = far + gap;
            start_min = start_min.min(start);
            far_max = far_max.max(far);
            extent = extent.max(size);
            placed.push((layout, start, far));
        }

        let shift = -start_min;
        let mut starts = Vec::with_capacity(placed.len());
        let mut ends = Vec::with_capacity(placed.len());
        let mut reach = i32::MIN;
        let mut bands = Vec::with_capacity(placed.len());
        for (layout, start, far) in placed {
            let start = start + shift;
            let far = far + shift;
            reach = reach.max(far);
            starts.push(start);
            ends.push(reach);
            bands.push((layout, unit * start));
        }
        let ordered = starts.windows(2).all(|pair| pair[0] <= pair[1]);

        let span = far_max - start_min;
        let size = match axis {
            Axis::Easting => BlockPos::new(span, extent.y, extent.z),
            Axis::Elevation => BlockPos::new(extent.x, span, extent.z),
            Axis::Southing => BlockPos::new(extent.x, extent.y, span),
        };
        Self {
            bands,
            starts,
            ends,
            ordered,
            axis,
            size,
        }
    }
}

impl<Ctx, L: Layout<Ctx>> Layout<Ctx> for Arranged<L> {
    fn block_at_with(&self, pos: BlockPos, ctx: &Ctx) -> Option<BlockState> {
        let along = self.axis.unit().dot(pos);
        let (first, last) = match self.ordered {
            true => (
                self.ends.partition_point(|&far| far <= along),
                self.starts.partition_point(|&start| start <= along),
            ),
            false => (0, self.bands.len()),
        };
        (first..last).rev().find_map(|index| {
            let (layout, anchor) = &self.bands[index];
            layout.get_block_with(pos - *anchor, ctx)
        })
    }

    fn size_with(&self, _ctx: &Ctx) -> BlockPos {
        self.size
    }

    /// The union of every band's palette.
    fn block_palette_with(&self, ctx: &Ctx) -> HashSet<BlockState> {
        let mut palette = HashSet::new();
        for (layout, _) in &self.bands {
            palette.extend(layout.block_palette_with(ctx));
        }
        palette
    }
}

/// Like [`Arranged`], but aligns sub-layouts by their far-edge on cross-axes.
pub struct EdgeArranged<L>(Reverse<Arranged<Reverse<L>>>);

impl<L> EdgeArranged<L> {
    /// `align` is passed to both inner (per-sub-layout) and outer (whole) Reverse.
    pub fn new<Ctx, I>(layouts: I, axis: Axis, gap: i32, align: Mask, ctx: &Ctx) -> Self
    where
        I: IntoIterator<Item = L>,
        L: Layout<Ctx>,
    {
        let reversed = layouts.into_iter().map(|l| Reverse::new(l, align, ctx));
        let arranged = Arranged::new(reversed, axis, gap, ctx);
        Self(Reverse::new(arranged, align, ctx))
    }
}

impl<Ctx, L: Layout<Ctx>> Layout<Ctx> for EdgeArranged<L> {
    fn block_at_with(&self, pos: BlockPos, ctx: &Ctx) -> Option<BlockState> {
        self.0.block_at_with(pos, ctx)
    }

    fn size_with(&self, ctx: &Ctx) -> BlockPos {
        self.0.size_with(ctx)
    }

    fn block_palette_with(&self, ctx: &Ctx) -> HashSet<BlockState> {
        self.0.block_palette_with(ctx)
    }
}

// LatticeArranged
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Names the two maps that place items: `forward` gives item `i`'s anchor and
/// `inverse` gives the last item whose anchor has not passed a position, the
/// two inverting each other.
///
/// A named implementation is what lets a composite type spell the field it
/// stores, while a closure pair is what lets an ad-hoc arrangement stay
/// anonymous.
pub trait Lattice {
    /// Anchor of item `i`.
    fn forward(&self, i: usize) -> BlockPos;

    /// Last item whose anchor does not pass `pos`.
    fn inverse(&self, pos: BlockPos) -> usize;
}

/// Any pair of closures of the right shapes is a lattice.
impl<F, G> Lattice for (F, G)
where
    F: Fn(usize) -> BlockPos,
    G: Fn(BlockPos) -> usize,
{
    fn forward(&self, i: usize) -> BlockPos {
        (self.0)(i)
    }

    fn inverse(&self, pos: BlockPos) -> usize {
        (self.1)(pos)
    }
}

/// Arranges sub-layouts at the anchors a [`Lattice`] names.
///
/// Query cost is O(k) with `k` the candidate window width, independent of the
/// item count: the window top is one `inverse` call, the measured span covers
/// the rest, and no search over anchors happens at query time. Anchors need not
/// be evenly spaced, so the lattice may describe any non-decreasing progression,
/// including one whose steps grow. They may also lie at negative coordinates;
/// the box shifts to the origin.
pub struct LatticeArranged<L, K> {
    /// Sub-layouts in anchor order; item `i` sits at `lattice.forward(i)`.
    items: Vec<L>,
    /// The anchors, as maps rather than as a list.
    lattice: K,
    /// Least window width that still reaches the highest covering item.
    span: usize,
    /// Shift making the least anchor the box origin.
    origin: BlockPos,
    /// Bounding box size, zero when empty.
    size: BlockPos,
}

impl<L, K: Lattice> LatticeArranged<L, K> {
    /// Arranges `items` at the anchors `lattice` names.
    pub fn new<Ctx, I>(items: I, lattice: K, ctx: &Ctx) -> Self
    where
        I: IntoIterator<Item = L>,
        L: Layout<Ctx>,
    {
        let items = Vec::from_iter(items);
        let inner: Vec<BlockPos> = items
            .iter()
            .map(|item| (item.size_with(ctx) - BlockPos::UNIT).max(BlockPos::ORIGIN))
            .collect();
        let anchors: Vec<BlockPos> = (0..items.len()).map(|i| lattice.forward(i)).collect();
        debug_assert!(
            (0..items.len()).all(|i| lattice.inverse(anchors[i]) >= i),
            "inverse must not run ahead of forward"
        );

        let origin = anchors
            .iter()
            .copied()
            .fold(BlockPos::ORIGIN, BlockPos::min);
        let size = (0..items.len())
            .map(|i| anchors[i] + inner[i] + BlockPos::UNIT)
            .fold(BlockPos::ORIGIN, BlockPos::max)
            - origin;
        let span = (0..items.len())
            .map(|i| lattice.inverse(anchors[i] + inner[i]).saturating_sub(i))
            .max()
            .unwrap_or_default();
        Self {
            items,
            lattice,
            span,
            origin,
            size,
        }
    }
}

impl<Ctx, L, K> Layout<Ctx> for LatticeArranged<L, K>
where
    L: Layout<Ctx>,
    K: Lattice,
{
    fn block_at_with(&self, pos: BlockPos, ctx: &Ctx) -> Option<BlockState> {
        let last = self.items.len().checked_sub(1)?;
        let world = pos + self.origin;
        let top = self.lattice.inverse(world).min(last);
        let bottom = top.saturating_sub(self.span);
        (bottom..=top)
            .rev()
            .find_map(|i| self.items[i].get_block_with(world - self.lattice.forward(i), ctx))
    }

    fn size_with(&self, _ctx: &Ctx) -> BlockPos {
        self.size
    }

    /// The union of every item's palette.
    fn block_palette_with(&self, ctx: &Ctx) -> HashSet<BlockState> {
        let mut palette = HashSet::new();
        for item in &self.items {
            palette.extend(item.block_palette_with(ctx));
        }
        palette
    }
}

// EvenlyArranged
//
// ++++++++++++============++++++++++++============++++++++++++============

/// The arithmetic lattice: item `i` sits at `i * pitch`.
///
/// The inverse is the closed form the lattice trait asks for, so an evenly
/// spaced arrangement stays O(1) per lookup however long it grows.
struct Pitch {
    /// Non-zero step between anchors.
    pitch: BlockPos,
    /// `pitch.dot(pitch)`, always positive.
    spacing: i32,
    /// Negated reach of one item's box backwards along `pitch`, added before the
    /// division so the inverse lands on the last item that may still cover a
    /// position rather than one below it.
    bias: i32,
}

impl Lattice for Pitch {
    fn forward(&self, i: usize) -> BlockPos {
        self.pitch * i as i32
    }

    fn inverse(&self, pos: BlockPos) -> usize {
        (self.pitch.dot(pos) - self.bias)
            .div_euclid(self.spacing)
            .max(0) as usize
    }
}

/// Evenly arranges sub-layouts along the direction of `pitch`: item `i` is
/// anchored at `i * pitch`, an arithmetic progression of equal spacing.
///
/// A special case of [`LatticeArranged`], so the query cost is O(k) with `k` the
/// candidate window width. `pitch` must be non-zero; an empty arrangement
/// covers nothing and reports zero size.
pub struct EvenlyArranged<L>(LatticeArranged<L, Pitch>);

impl<L> EvenlyArranged<L> {
    /// Places item `i` at `i * pitch`, which must be non-zero.
    pub fn new<Ctx, I>(items: I, pitch: BlockPos, ctx: &Ctx) -> Self
    where
        I: IntoIterator<Item = L>,
        L: Layout<Ctx>,
    {
        assert!(pitch != BlockPos::ORIGIN, "pitch must be non-zero");
        let items = Vec::from_iter(items);
        let extent = items
            .iter()
            .map(|item| item.size_with(ctx))
            .fold(BlockPos::ORIGIN, BlockPos::max);
        let inner = (extent - BlockPos::new(1, 1, 1)).max(BlockPos::ORIGIN);
        let lattice = Pitch {
            pitch,
            spacing: pitch.dot(pitch),
            bias: -(-pitch).max(BlockPos::ORIGIN).dot(inner),
        };
        Self(LatticeArranged::new(items, lattice, ctx))
    }
}

impl<Ctx, L: Layout<Ctx>> Layout<Ctx> for EvenlyArranged<L> {
    fn block_at_with(&self, pos: BlockPos, ctx: &Ctx) -> Option<BlockState> {
        self.0.block_at_with(pos, ctx)
    }

    fn size_with(&self, ctx: &Ctx) -> BlockPos {
        self.0.size_with(ctx)
    }

    fn block_palette_with(&self, ctx: &Ctx) -> HashSet<BlockState> {
        self.0.block_palette_with(ctx)
    }
}

// Overlaid
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Overlaps two positioned sub-layouts, the first covering the second, where a
/// non-negative anchor is an offset from the origin and a negative one an
/// offset from `size + 1`.
pub struct Overlaid<A, B> {
    first: (BlockPos, A),
    second: (BlockPos, B),
    size: BlockPos,
}

impl<A, B> Overlaid<A, B> {
    /// Lays `first` over `second` at the given anchors, within a box of `size`.
    pub fn new<Ctx>(
        first_anchor: BlockPos,
        first: A,
        second_anchor: BlockPos,
        second: B,
        size: BlockPos,
        ctx: &Ctx,
    ) -> Self
    where
        A: Layout<Ctx>,
        B: Layout<Ctx>,
    {
        fn anchor(anchor: BlockPos, inner: BlockPos, size: BlockPos) -> BlockPos {
            let axis = |a: i32, i: i32, s: i32| if a < 0 { s - i + a + 1 } else { a };
            let x = axis(anchor.x, inner.x, size.x);
            let y = axis(anchor.y, inner.y, size.y);
            let z = axis(anchor.z, inner.z, size.z);
            BlockPos::new(x, y, z)
        }
        Self {
            first: (anchor(first_anchor, first.size_with(ctx), size), first),
            second: (anchor(second_anchor, second.size_with(ctx), size), second),
            size,
        }
    }
}

impl<Ctx, A: Layout<Ctx>, B: Layout<Ctx>> Layout<Ctx> for Overlaid<A, B> {
    fn block_at_with(&self, pos: BlockPos, ctx: &Ctx) -> Option<BlockState> {
        let (anchor, layout) = &self.first;
        layout.get_block_with(pos - *anchor, ctx).or_else(|| {
            let (anchor, layout) = &self.second;
            layout.get_block_with(pos - *anchor, ctx)
        })
    }

    fn size_with(&self, _ctx: &Ctx) -> BlockPos {
        self.size
    }

    /// The union of both sub-layouts' palettes.
    fn block_palette_with(&self, ctx: &Ctx) -> HashSet<BlockState> {
        let mut palette = self.first.1.block_palette_with(ctx);
        palette.extend(self.second.1.block_palette_with(ctx));
        palette
    }
}

// Reverse
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Mirror-reverse a layout along given axes. Block facing unchanged:
/// the layout has no block-internal facing transform, so the block's own
/// orientation state is never dictated by it.
pub struct Reverse<L> {
    /// The wrapped layout; queries are mirrored into its local space.
    layout: L,
    /// Per-axis mirror offset `sign * (size - 1)`.
    bias: BlockPos,
    /// Per-axis mirror factor: `-1` on reversed axes, `+1` elsewhere.
    factor: BlockPos,
}

impl<L> Reverse<L> {
    /// Mirrors `layout` on the axes that `sign` marks.
    pub fn new<Ctx>(layout: L, sign: Mask, ctx: &Ctx) -> Self
    where
        L: Layout<Ctx>,
    {
        let bias = sign * (layout.size_with(ctx) - BlockPos::new(1, 1, 1));
        let factor = BlockPos::new(1, 1, 1) - sign * 2;
        Self {
            layout,
            bias,
            factor,
        }
    }
}

impl<Ctx, L: Layout<Ctx>> Layout<Ctx> for Reverse<L> {
    fn block_at_with(&self, pos: BlockPos, ctx: &Ctx) -> Option<BlockState> {
        let orig = BlockPos::new(
            self.bias.x + self.factor.x * pos.x,
            self.bias.y + self.factor.y * pos.y,
            self.bias.z + self.factor.z * pos.z,
        );
        self.layout.block_at_with(orig, ctx)
    }

    fn size_with(&self, ctx: &Ctx) -> BlockPos {
        self.layout.size_with(ctx)
    }

    /// The inner palette: mirroring moves cells, it draws no new block.
    fn block_palette_with(&self, ctx: &Ctx) -> HashSet<BlockState> {
        self.layout.block_palette_with(ctx)
    }
}

// WithFloor
//
// ++++++++++++============++++++++++++============++++++++++++============

/// A layout wrapper that adds a floor layer beneath another layout.
pub struct WithFloor<L> {
    layout: L,
    full: bool,
}

impl<L> WithFloor<L> {
    /// Whether the floor fully covers the entire bounding box.
    /// When `false`, only positions with a gravity block above get a floor.
    pub fn new(layout: L, full: bool) -> Self {
        Self { layout, full }
    }
}

impl<Ctx, L: Layout<Ctx>> Layout<Ctx> for WithFloor<L> {
    fn block_at_with(&self, pos: BlockPos, ctx: &Ctx) -> Option<BlockState> {
        let inner = |pos: BlockPos| self.layout.block_at_with(pos, ctx);
        match pos.y {
            0 if self.full => Some(floor_block()),
            0 => inner(pos)
                .filter(|b| b.needs_floor())
                .map(|_| floor_block()),
            _ => inner(BlockPos::new(pos.x, pos.y - 1, pos.z)),
        }
    }

    fn size_with(&self, ctx: &Ctx) -> BlockPos {
        let size = self.layout.size_with(ctx);
        BlockPos::new(size.x, size.y + 1, size.z)
    }

    /// The inner palette plus the floor block, the only block this wrapper adds
    /// on top of what the inner layout draws.
    fn block_palette_with(&self, ctx: &Ctx) -> HashSet<BlockState> {
        let mut palette = self.layout.block_palette_with(ctx);
        palette.insert(floor_block());
        palette
    }
}

/// Whether a block state is a gravity block that needs floor support.
pub trait NeedsFloor {
    /// Whether this block state needs a floor to hold it up.
    fn needs_floor(&self) -> bool;
}

impl NeedsFloor for BlockState {
    fn needs_floor(&self) -> bool {
        self.name == "minecraft:sand"
    }
}

// Clipped
//
// ++++++++++++============++++++++++++============++++++++++++============

/// A layout wrapper that clips the inner layout along each axis by `pos`.
///
/// A non-negative component `n` shifts the inner layout by `n` and shrinks
/// the size by `n`; a negative component keeps the inner layout in place and
/// shrinks the size by `-n`.
pub struct Clipped<L> {
    layout: L,
    anchor: BlockPos,
    size: BlockPos,
}

impl<L> Clipped<L> {
    /// Clips `layout` by `pos` on each axis.
    pub fn new<Ctx>(layout: L, pos: BlockPos, ctx: &Ctx) -> Self
    where
        L: Layout<Ctx>,
    {
        let anchor = pos.max(BlockPos::ORIGIN);
        let size = layout.size_with(ctx) - pos.abs();

        Self {
            layout,
            anchor,
            size,
        }
    }
}

impl<Ctx, L: Layout<Ctx>> Layout<Ctx> for Clipped<L> {
    fn block_at_with(&self, pos: BlockPos, ctx: &Ctx) -> Option<BlockState> {
        self.layout.block_at_with(pos + self.anchor, ctx)
    }

    fn size_with(&self, _ctx: &Ctx) -> BlockPos {
        self.size
    }

    /// The inner palette: clipping only drops cells, it draws no new block.
    fn block_palette_with(&self, ctx: &Ctx) -> HashSet<BlockState> {
        self.layout.block_palette_with(ctx)
    }
}

// Helpers
//
// ++++++++++++============++++++++++++============++++++++++++============

fn floor_block() -> BlockState {
    BlockState {
        name: "minecraft:gray_stained_glass".into(),
        properties: Default::default(),
    }
}

// Tests
//
// ++++++++++++============++++++++++++============++++++++++++============

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{LayoutExt, air};

    /// A solid cuboid filling its whole box.
    struct Solid(BlockPos);

    impl Layout<()> for Solid {
        fn block_at_with(&self, pos: BlockPos, ctx: &()) -> Option<BlockState> {
            self.contains_with(pos, ctx).then(air)
        }

        fn size_with(&self, _ctx: &()) -> BlockPos {
            self.0
        }
    }

    fn build(sizes: &[(i32, i32, i32)], pitch: (i32, i32, i32)) -> EvenlyArranged<Solid> {
        let items = sizes.iter().map(|&(x, y, z)| Solid(BlockPos::new(x, y, z)));
        EvenlyArranged::new(items, BlockPos::new(pitch.0, pitch.1, pitch.2), &())
    }

    /// The origin `LatticeArranged` shifts a lattice by: the least anchor.
    fn origin(sizes: &[(i32, i32, i32)], pitch: BlockPos) -> BlockPos {
        let far = (sizes.len() as i32 - 1).max(0);
        pitch.min(BlockPos::ORIGIN) * far
    }

    /// Whether item `i` covers `local`, a point already shifted into the lattice.
    fn hits(sizes: &[(i32, i32, i32)], i: usize, local: BlockPos) -> bool {
        let (x, y, z) = sizes[i];
        (0..x).contains(&local.x) && (0..y).contains(&local.y) && (0..z).contains(&local.z)
    }

    /// The loose box the old size formula reported: `|pitch| * far + extent`.
    fn loose(sizes: &[(i32, i32, i32)], pitch: BlockPos) -> BlockPos {
        let far = (sizes.len() as i32 - 1).max(0);
        let extent = sizes.iter().fold(BlockPos::ORIGIN, |acc, &(x, y, z)| {
            acc.max(BlockPos::new(x, y, z))
        });
        pitch.abs() * far + extent
    }

    /// Pitch and item sizes sweeping positive, negative, mixed and overlapping
    /// lattices.
    const CASES: [(&[(i32, i32, i32)], (i32, i32, i32)); 8] = [
        (&[(5, 1, 1); 4], (2, 0, 0)),
        (&[(5, 1, 1); 4], (-2, 0, 0)),
        (&[(5, 5, 1); 4], (1, 1, 0)),
        (&[(3, 3, 2); 4], (0, 0, 2)),
        (&[(3, 3, 2); 4], (0, 0, -2)),
        (&[(2, 6, 2); 4], (0, 4, 0)),
        (&[(5, 1, 1), (3, 1, 1), (7, 1, 1), (2, 1, 1)], (3, 0, 0)),
        (&[(4, 4, 3); 4], (2, 3, -1)),
    ];

    /// Every cell any item covers is found, so the window never misses. The box
    /// swept is the loose one, so a cell left outside the reported size would
    /// still be caught here.
    #[test]
    fn window_covers_every_item() {
        for &(sizes, pitch) in &CASES {
            let arranged = build(sizes, pitch);
            let pitch = BlockPos::new(pitch.0, pitch.1, pitch.2);
            let size = loose(sizes, pitch);
            for z in 0..size.z {
                for y in 0..size.y {
                    for x in 0..size.x {
                        let pos = BlockPos::new(x, y, z);
                        let local = pos + origin(sizes, pitch);
                        let covered =
                            (0..sizes.len()).any(|i| hits(sizes, i, local - pitch * i as i32));
                        let hit = arranged.get_block(pos).is_some();
                        assert_eq!(hit, covered, "{pos:?} at {pitch:?}");
                    }
                }
            }
        }
    }

    /// The window always reaches the highest covering item, so no scan can start
    /// below a cell it must find.
    #[test]
    fn window_reaches_highest_covering_item() {
        for &(sizes, pitch) in &CASES {
            let arranged = build(sizes, pitch);
            let pitch = BlockPos::new(pitch.0, pitch.1, pitch.2);
            let size = loose(sizes, pitch);
            let lattice = &arranged.0.lattice;
            let last = sizes.len() as i32 - 1;
            let mut needed = 0;
            for z in 0..size.z {
                for y in 0..size.y {
                    for x in 0..size.x {
                        let local = BlockPos::new(x, y, z) + origin(sizes, pitch);
                        let top = (lattice.pitch.dot(local) - lattice.bias)
                            .div_euclid(lattice.spacing)
                            .min(last);
                        let highest = (0..sizes.len())
                            .rev()
                            .find(|&i| hits(sizes, i, local - pitch * i as i32));
                        if let Some(highest) = highest {
                            needed = needed.max(top - highest as i32);
                        }
                    }
                }
            }
            assert!(
                arranged.0.span as i32 >= needed,
                "{pitch:?}: {needed} > {}",
                arranged.0.span
            );
        }
    }
}
