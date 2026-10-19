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

// EvenlyArranged
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Evenly arranges sub-layouts along the direction of `pitch`: item `i` is
/// anchored at `i * pitch`, a monotone linear lattice of equal spacing.
///
/// Query cost is O(k) with `k` the candidate window width, independent of
/// the item count. The window is the least that still reaches every item's
/// covering cell, so `k` is minimal. `pitch` must be non-zero; an empty
/// arrangement covers nothing and reports zero size.
pub struct EvenlyArranged<L> {
    /// Sub-layouts in lattice order; item `i` sits at `i * pitch`.
    items: Vec<L>,
    /// Non-zero step between item anchors.
    pitch: BlockPos,
    /// Shift so indices can go negative: `min(pitch, 0) * (len - 1)`.
    anchor: BlockPos,
    /// `pitch.dot(pitch)`, always positive.
    spacing: i32,
    /// Least `pitch.dot(local_pos - i * pitch)` over the integer points of one
    /// item box, so `top` never overshoots the highest covering item.
    bias: i32,
    /// Least candidate window width minus one that still reaches the highest
    /// covering item, `floor((L - M) / spacing)` with `L = |pitch|.extent` and
    /// `M = |pitch|.ones`.
    span: i32,
    /// Bounding box size: `pitch.abs() * (len - 1) + extent`, zero when empty.
    size: BlockPos,
}

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
        let far = (items.len() as i32 - 1).max(0);

        let inner = (extent - BlockPos::new(1, 1, 1)).max(BlockPos::ORIGIN);
        let fore = pitch.max(BlockPos::ORIGIN);
        let back = (-pitch).max(BlockPos::ORIGIN);
        let spacing = pitch.dot(pitch);

        let ahead = fore.dot(inner);
        let behind = back.dot(inner);
        Self {
            items,
            pitch,
            anchor: -(back * far),
            spacing,
            bias: -behind,
            span: (ahead + behind) / spacing,
            size: pitch.abs() * far + extent,
        }
    }
}

impl<Ctx, L: Layout<Ctx>> Layout<Ctx> for EvenlyArranged<L> {
    fn block_at_with(&self, pos: BlockPos, ctx: &Ctx) -> Option<BlockState> {
        let local_pos = pos + self.anchor;
        let offset = self.pitch.dot(local_pos);
        let top = (offset - self.bias)
            .div_euclid(self.spacing)
            .min(self.items.len() as i32 - 1);
        let bottom = (top - self.span).max(0);
        (bottom..=top)
            .rev()
            .find_map(|i| self.items[i as usize].get_block_with(local_pos - self.pitch * i, ctx))
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

    /// Whether any item covers `pos`, by scanning the whole lattice.
    fn covered(arranged: &EvenlyArranged<Solid>, pos: BlockPos) -> bool {
        let local = pos + arranged.anchor;
        arranged
            .items
            .iter()
            .enumerate()
            .any(|(i, item)| item.get_block(local - arranged.pitch * i as i32).is_some())
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

    /// Every cell any item covers is found, so the window never misses.
    #[test]
    fn window_covers_every_item() {
        for &(sizes, pitch) in &CASES {
            let arranged = build(sizes, pitch);
            let size = arranged.size();
            for z in 0..size.z {
                for y in 0..size.y {
                    for x in 0..size.x {
                        let pos = BlockPos::new(x, y, z);
                        let hit = arranged.get_block(pos).is_some();
                        assert_eq!(hit, covered(&arranged, pos), "{pos:?} at {pitch:?}");
                    }
                }
            }
        }
    }

    /// A solid cuboid drawing a block under a chosen name.
    struct Named(BlockPos, &'static str);

    impl Layout<()> for Named {
        fn block_at_with(&self, pos: BlockPos, ctx: &()) -> Option<BlockState> {
            self.contains_with(pos, ctx).then(|| BlockState {
                name: self.1.into(),
                properties: Default::default(),
            })
        }

        fn size_with(&self, _ctx: &()) -> BlockPos {
            self.0
        }
    }

    fn arrange(sizes: &[(i32, i32, i32)], gap: i32) -> Arranged<Solid> {
        let items = sizes.iter().map(|&(x, y, z)| Solid(BlockPos::new(x, y, z)));
        Arranged::new(items, Axis::Easting, gap, &())
    }

    /// A negative gap overlaps adjacent items, and the later one wins.
    #[test]
    fn negative_gap_layers_later_items_on_top() {
        let items = [
            Named(BlockPos::new(5, 1, 1), "minecraft:stone"),
            Named(BlockPos::new(5, 1, 1), "minecraft:dirt"),
        ];
        let arranged = Arranged::new(items, Axis::Easting, -2, &());
        let name = |x| {
            arranged
                .get_block(BlockPos::new(x, 0, 0))
                .map(|block| block.name)
        };

        assert_eq!(arranged.size(), BlockPos::new(8, 1, 1));
        assert_eq!(name(2), Some("minecraft:stone".into()));
        assert_eq!(name(3), Some("minecraft:dirt".into()));
        assert_eq!(name(4), Some("minecraft:dirt".into()));
        assert_eq!(name(7), Some("minecraft:dirt".into()));
        assert_eq!(name(8), None);
    }

    /// A signed gap works on any arrangement axis, not just easting.
    #[test]
    fn signed_gap_arranges_vertically() {
        let items = [Solid(BlockPos::new(1, 3, 1)), Solid(BlockPos::new(1, 2, 1))];
        let arranged = Arranged::new(items, Axis::Elevation, -1, &());

        assert_eq!(arranged.size(), BlockPos::new(1, 4, 1));
        assert!(arranged.get_block(BlockPos::new(0, 0, 0)).is_some());
        assert!(arranged.get_block(BlockPos::new(0, 2, 0)).is_some());
        assert!(arranged.get_block(BlockPos::new(0, 3, 0)).is_some());
        assert!(arranged.get_block(BlockPos::new(0, 4, 0)).is_none());
    }

    /// The southing axis mirrors easting on the z component.
    #[test]
    fn signed_gap_arranges_southwards() {
        let items = [Solid(BlockPos::new(1, 1, 3)), Solid(BlockPos::new(1, 1, 2))];
        let arranged = Arranged::new(items, Axis::Southing, -1, &());

        assert_eq!(arranged.size(), BlockPos::new(1, 1, 4));
        assert!(arranged.get_block(BlockPos::new(0, 0, 0)).is_some());
        assert!(arranged.get_block(BlockPos::new(0, 0, 2)).is_some());
        assert!(arranged.get_block(BlockPos::new(0, 0, 3)).is_some());
        assert!(arranged.get_block(BlockPos::new(0, 0, 4)).is_none());
    }

    /// A gap more negative than the item extent reverses the anchors, and the
    /// bounding box still starts at the origin.
    #[test]
    fn reversed_gap_shifts_back_to_the_origin() {
        let arranged = arrange(&[(1, 1, 1); 2], -5);

        assert_eq!(arranged.size(), BlockPos::new(5, 1, 1));
        assert!(arranged.get_block(BlockPos::new(0, 0, 0)).is_some());
        assert!(arranged.get_block(BlockPos::new(1, 0, 0)).is_none());
        assert!(arranged.get_block(BlockPos::new(4, 0, 0)).is_some());
    }

    /// Every covered cell of a signed gap matches a naive placement over the
    /// whole bounding box, including overlaps and reversed anchors.
    #[test]
    fn signed_gap_matches_naive_placement() {
        const CASES: [(&[(i32, i32, i32)], i32); 6] = [
            (&[(5, 1, 1); 4], 2),
            (&[(5, 1, 1); 4], -2),
            (&[(1, 1, 1); 3], -5),
            (&[(6, 2, 4), (1, 5, 2), (3, 1, 1)], -4),
            (&[(2, 2, 2); 3], 0),
            (&[(4, 1, 1), (1, 4, 1), (2, 3, 2)], -1),
        ];
        for &(sizes, gap) in &CASES {
            let arranged = arrange(sizes, gap);

            let mut anchors = Vec::new();
            let mut cursor = 0;
            for &(x, _, _) in sizes {
                anchors.push(cursor);
                cursor += x + gap;
            }
            let first = anchors.iter().copied().min().unwrap_or(0);
            let last = sizes
                .iter()
                .zip(&anchors)
                .map(|(&(x, _, _), &anchor)| anchor - first + x)
                .max()
                .unwrap_or(0);
            assert_eq!(arranged.size().x, last, "{sizes:?} at {gap}");

            let size = arranged.size();
            for z in 0..size.z {
                for y in 0..size.y {
                    for x in 0..size.x {
                        let pos = BlockPos::new(x, y, z);
                        let hit = sizes.iter().enumerate().any(|(i, &(sx, sy, sz))| {
                            let start = anchors[i] - first;
                            (start..start + sx).contains(&x)
                                && (0..sy).contains(&y)
                                && (0..sz).contains(&z)
                        });
                        assert_eq!(
                            arranged.get_block(pos).is_some(),
                            hit,
                            "{pos:?} in {sizes:?} at {gap}"
                        );
                    }
                }
            }
        }
    }

    /// An empty arrangement covers nothing and has zero size.
    #[test]
    fn empty_arrangement_has_no_size() {
        let arranged = Arranged::<Solid>::new([], Axis::Easting, -3, &());

        assert_eq!(arranged.size(), BlockPos::ORIGIN);
        assert!(arranged.get_block(BlockPos::ORIGIN).is_none());
    }

    /// The window always reaches the highest covering item, so no scan can start
    /// below a cell it must find.
    #[test]
    fn window_reaches_highest_covering_item() {
        for &(sizes, pitch) in &CASES {
            let arranged = build(sizes, pitch);
            let size = arranged.size();
            let mut needed = 0;
            for z in 0..size.z {
                for y in 0..size.y {
                    for x in 0..size.x {
                        let pos = BlockPos::new(x, y, z);
                        let local = pos + arranged.anchor;
                        let top = arranged.pitch.dot(local) - arranged.bias;
                        let top = top
                            .div_euclid(arranged.spacing)
                            .min(arranged.items.len() as i32 - 1);
                        let highest = (0..arranged.items.len()).rev().find(|&i| {
                            arranged.items[i]
                                .get_block(local - arranged.pitch * i as i32)
                                .is_some()
                        });
                        if let Some(highest) = highest {
                            needed = needed.max(top - highest as i32);
                        }
                    }
                }
            }
            assert!(
                arranged.span >= needed,
                "{pitch:?}: {needed} > {}",
                arranged.span
            );
        }
    }
}
