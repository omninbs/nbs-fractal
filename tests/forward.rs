//! `forward_layout!` is exported, so a downstream crate can define a `Layout`
//! wrapper with it.

use nbs_fractal::layout::{Layout, LayoutExt};
use nbs_fractal::{BlockPos, BlockState, forward_layout};

/// A one-wide solid line, declared from outside the crate.
struct Solid;

impl Layout<()> for Solid {
    fn block_at_with(&self, pos: BlockPos, _ctx: &()) -> Option<BlockState> {
        (0..2).contains(&pos.x).then_some(BlockState::AIR)
    }

    fn size_with(&self, _ctx: &()) -> BlockPos {
        BlockPos::new(2, 1, 1)
    }
}

/// A wrapper forwarding its `Layout` to the inner `.0` through the macro.
struct Wrapper(Solid);

forward_layout!(Wrapper, ());

#[test]
fn forwards_through_the_exported_macro() {
    let wrapper = Wrapper(Solid);
    assert_eq!(wrapper.size(), BlockPos::new(2, 1, 1));
    assert!(wrapper.get_block(BlockPos::new(1, 0, 0)).is_some());
    assert!(wrapper.get_block(BlockPos::new(2, 0, 0)).is_none());
}
