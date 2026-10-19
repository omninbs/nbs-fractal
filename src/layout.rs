//! Projection layouts: the note-to-block arrangement a song projects into.

pub mod bamboo;
pub mod blocks;
pub mod cascade;
pub mod compact;
pub mod compose;
pub mod fishbone;
pub mod linear;
pub mod tapped;
pub mod torrent;
pub use base::{Axis, Layout, LayoutExt, Mask};
pub use litematic::Metadata;

pub(crate) use base::air;

mod base {
    //! The layout query traits and the primitives they are read with.

    use super::litematic::{Metadata, write};
    use crate::{BlockPos, BlockState};
    use itertools::iproduct;
    use std::collections::HashSet;
    use std::io::{self, Write};

    // Layout
    //
    // ++++++++++++============++++++++++++============++++++++++++============

    /// A queryable projection layout, generic over the context its parent declares.
    ///
    /// `Ctx` is the shared knowledge a subtree is read within: a parent picks the
    /// type and hands out one value, and every query receives a borrow of it, so a
    /// leaf can read what only its ancestors know without holding its own copy. A
    /// layout whose context is erased implements `Layout<()>`, the context of the
    /// writer and of every finished layout.
    ///
    /// Every query is named `*_with` and reads a borrow of `Ctx`. A layout whose
    /// context is `()` gets the plain queries and the projection materialization of
    /// [`LayoutExt`] for free, so a finished layout reads as `layout.size()`.
    pub trait Layout<Ctx = ()> {
        /// Block at the given world position, read within `ctx`, assumed in bounds.
        fn block_at_with(&self, pos: BlockPos, ctx: &Ctx) -> Option<BlockState>;

        /// Total size of the bounding box, read within `ctx`.
        fn size_with(&self, ctx: &Ctx) -> BlockPos;

        /// Block at the given world position; `None` when it misses the bounding box.
        fn get_block_with(&self, pos: BlockPos, ctx: &Ctx) -> Option<BlockState> {
            match self.contains_with(pos, ctx) {
                true => self.block_at_with(pos, ctx),
                false => None,
            }
        }

        /// Block at the given world position, or air where the layout is empty.
        fn get_block_or_air_with(&self, pos: BlockPos, ctx: &Ctx) -> BlockState {
            self.get_block_with(pos, ctx).unwrap_or_else(air)
        }

        /// Whether `pos` is inside the bounding box.
        fn contains_with(&self, pos: BlockPos, ctx: &Ctx) -> bool {
            let size = self.size_with(ctx);
            (0..size.x).contains(&pos.x)
                && (0..size.y).contains(&pos.y)
                && (0..size.z).contains(&pos.z)
        }

        /// The block states this instance owns: a set containing every state that
        /// [`block_at_with`](Self::block_at_with) can return, empty cells counted as
        /// air.
        ///
        /// A layout may over-approximate, so declaring it spares the writer a
        /// whole-box discovery walk; the default does exactly that walk via
        /// [`get_block_or_air_with`](Self::get_block_or_air_with). The writer alone
        /// fixes the id order, so the set need not be ordered.
        fn block_palette_with(&self, ctx: &Ctx) -> HashSet<BlockState> {
            let size = self.size_with(ctx);
            let at = |(y, z, x)| self.get_block_or_air_with(BlockPos::new(x, y, z), ctx);
            // Insert one by one: `collect` would reserve the box's whole volume
            // from the iterator's size hint, however few blocks it ends up holding.
            let mut palette = HashSet::new();
            for block in iproduct!(0..size.y, 0..size.z, 0..size.x).map(at) {
                palette.insert(block);
            }
            palette
        }
    }

    /// The plain surface of a finished layout: the queries of [`Layout`] with its
    /// context fixed to `()`, plus materialization into a projection.
    ///
    /// Every `Layout<()>` gets this for free, so a finished layout reads as
    /// `layout.size()` or `layout.get_block(pos)` rather than spelling out the
    /// empty context. A layout still carrying a context has only the `*_with`
    /// queries.
    pub trait LayoutExt: Layout<()> {
        /// Block at the given world position, assumed in bounds.
        fn block_at(&self, pos: BlockPos) -> Option<BlockState> {
            self.block_at_with(pos, &())
        }

        /// Total size of the bounding box.
        fn size(&self) -> BlockPos {
            self.size_with(&())
        }

        /// Block at the given world position; `None` when it misses the bounding box.
        fn get_block(&self, pos: BlockPos) -> Option<BlockState> {
            self.get_block_with(pos, &())
        }

        /// Block at the given world position, or air where the layout is empty.
        fn get_block_or_air(&self, pos: BlockPos) -> BlockState {
            self.get_block_or_air_with(pos, &())
        }

        /// Whether `pos` is inside the bounding box.
        fn contains(&self, pos: BlockPos) -> bool {
            self.contains_with(pos, &())
        }

        /// The block states this instance owns.
        fn block_palette(&self) -> HashSet<BlockState> {
            self.block_palette_with(&())
        }

        /// Writes this layout to `sink` as a gzip-compressed litematic projection,
        /// described by `metadata`.
        fn as_litematic<W: Write>(&self, metadata: Metadata, sink: W) -> io::Result<()>
        where
            Self: Sized,
        {
            write(self, metadata, sink)
        }
    }

    impl<T: Layout<()>> LayoutExt for T {}

    impl<Ctx> Layout<Ctx> for () {
        fn block_at_with(&self, _pos: BlockPos, _ctx: &Ctx) -> Option<BlockState> {
            None
        }

        fn size_with(&self, _ctx: &Ctx) -> BlockPos {
            BlockPos::ORIGIN
        }
    }

    impl<Ctx> Layout<Ctx> for Box<dyn Layout<Ctx> + '_> {
        fn block_at_with(&self, pos: BlockPos, ctx: &Ctx) -> Option<BlockState> {
            self.as_ref().block_at_with(pos, ctx)
        }

        fn size_with(&self, ctx: &Ctx) -> BlockPos {
            self.as_ref().size_with(ctx)
        }

        fn block_palette_with(&self, ctx: &Ctx) -> HashSet<BlockState> {
            self.as_ref().block_palette_with(ctx)
        }
    }

    /// Implements [`Layout`] on a wrapper by forwarding every query to one of its
    /// fields: the first, `.0`, unless a field name or index is given as the third
    /// argument.
    ///
    /// Rust rejects the blanket impl that would make this automatic: it cannot rule
    /// out a downstream impl overlapping a hand-written one, however private the
    /// wrapper, so each wrapper states its delegation here in one line.
    ///
    /// Exported for downstream crates, which use it the same way:
    /// `forward_layout!(MyWrapper, ());` forwards to `.0` and
    /// `forward_layout!(MyWrapper, (), inner);` to `inner`.
    #[macro_export]
    macro_rules! forward_layout {
        ($ty:ty, $ctx:ty) => {
            $crate::forward_layout!($ty, $ctx, 0);
        };
        ($ty:ty, $ctx:ty, $field:tt) => {
            impl $crate::layout::Layout<$ctx> for $ty {
                fn block_at_with(
                    &self,
                    pos: $crate::BlockPos,
                    ctx: &$ctx,
                ) -> Option<$crate::BlockState> {
                    self.$field.block_at_with(pos, ctx)
                }

                fn size_with(&self, ctx: &$ctx) -> $crate::BlockPos {
                    self.$field.size_with(ctx)
                }

                fn block_palette_with(
                    &self,
                    ctx: &$ctx,
                ) -> ::std::collections::HashSet<$crate::BlockState> {
                    self.$field.block_palette_with(ctx)
                }
            }
        };
    }

    // Axis
    //
    // ++++++++++++============++++++++++++============++++++++++++============

    /// Which spatial direction sub-layouts are placed along.
    #[derive(Clone, Copy)]
    pub enum Axis {
        /// East-west axis (X).
        Easting,
        /// Vertical axis (Y).
        Elevation,
        /// South-north axis (Z).
        Southing,
    }

    impl Axis {
        /// Unit mask vector for this axis.
        pub fn unit(self) -> Mask {
            match self {
                Axis::Easting => Mask::new(BlockPos::new(1, 0, 0)).unwrap(),
                Axis::Elevation => Mask::new(BlockPos::new(0, 1, 0)).unwrap(),
                Axis::Southing => Mask::new(BlockPos::new(0, 0, 1)).unwrap(),
            }
        }
    }

    // Mask
    //
    // ++++++++++++============++++++++++++============++++++++++++============

    /// Component-wise mask (0 or 1 on each axis) for BlockPos operations.
    ///
    /// Unlike `BlockPos * BlockPos` (cross product), `Mask * BlockPos` is component-wise.
    #[derive(Clone, Copy)]
    pub struct Mask(BlockPos);

    impl Mask {
        /// A mask from a component vector, or `None` unless every component is 0 or
        /// 1.
        pub fn new(sign: BlockPos) -> Option<Self> {
            match (sign.x == 0 || sign.x == 1)
                && (sign.y == 0 || sign.y == 1)
                && (sign.z == 0 || sign.z == 1)
            {
                true => Some(Self(sign)),
                false => None,
            }
        }

        /// Unwrap into the underlying [`BlockPos`].
        pub fn into_inner(self) -> BlockPos {
            self.0
        }

        /// The scalar projection onto this mask.
        pub fn dot(self, v: BlockPos) -> i32 {
            self.0.dot(v)
        }
    }

    impl std::ops::Mul<BlockPos> for Mask {
        type Output = BlockPos;

        fn mul(self, rhs: BlockPos) -> BlockPos {
            self.0.component_mul(rhs)
        }
    }

    impl std::ops::Mul<i32> for Mask {
        type Output = BlockPos;

        fn mul(self, rhs: i32) -> BlockPos {
            self.0 * rhs
        }
    }

    // Helpers
    //
    // ++++++++++++============++++++++++++============++++++++++++============

    pub(crate) fn air() -> BlockState {
        BlockState::AIR
    }
}

mod litematic {
    //! Projection output: metadata and the adapter to the litematica writer.

    use super::base::{Layout, LayoutExt};
    use crate::{BlockPos, BlockState};
    use litematica::knowledge::mc::BlockSize;
    use litematica::task::writer::{self, WriteOptions};
    use std::collections::HashSet;
    use std::io::{self, Write};

    // Litematic
    //
    // ++++++++++++============++++++++++++============++++++++++++============

    /// Target Minecraft data version written into the file.
    const MINECRAFT_DATA_VERSION: i32 = 2975;

    /// The metadata of a written projection: its name, description, and author.
    pub struct Metadata<'a> {
        /// Name of the schematic and of its single region.
        pub name: &'a str,
        /// Free-form description.
        pub description: &'a str,
        /// Author credited in the header.
        pub author: &'a str,
    }

    /// Writes `layout` to `sink` as a gzip-compressed litematic projection,
    /// described by `metadata`.
    pub(super) fn write<L: Layout, W: Write>(
        layout: &L,
        metadata: Metadata,
        sink: W,
    ) -> io::Result<()> {
        let mut options = WriteOptions::new(metadata.name, MINECRAFT_DATA_VERSION);
        options.description = metadata.description.to_owned();
        options.author = metadata.author.to_owned();
        let layout = Adapted { layout };
        writer::write(&layout, &options, sink).map_err(io::Error::other)
    }

    /// Adapts this crate's [`Layout`] to the [`litematica`] writer's trait.
    ///
    /// The adapter is a newtype so the conversion stays explicit: a span gains a
    /// non-negative [`BlockSize`], while the block and palette queries pass
    /// through unchanged.
    struct Adapted<'a, L> {
        layout: &'a L,
    }

    impl<L: Layout> writer::Layout for Adapted<'_, L> {
        fn size(&self) -> BlockSize {
            BlockSize::of(self.layout.size())
        }

        fn position(&self) -> BlockPos {
            BlockPos::ORIGIN
        }

        fn block_at(&self, pos: BlockPos) -> Option<BlockState> {
            self.layout.get_block(pos)
        }

        fn block_palette(&self) -> HashSet<BlockState> {
            self.layout.block_palette()
        }
    }
}
