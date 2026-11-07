//! TODO: describe the copper layout.

use super::Layout;
use super::blocks::{WireConn, redstone_torch, wire_state};
use crate::{BlockPos, BlockState, RedStoneTick};
use rsnbs::note::Tone;
use rsnbs::types::TimeAnchor;

// Layout: CopperLayout
//
// ++++++++++++============++++++++++++============++++++++++++============

/// TODO
pub struct CopperLayout {
    // TODO: fields
}

impl CopperLayout {
    /// TODO
    pub fn new<A, T>(
        _clk1: impl IntoIterator<Item = (A, T)>,
        _clk2: impl IntoIterator<Item = (A, T)>,
        _clk4: impl IntoIterator<Item = (A, T)>,
        _clk8_rise: impl IntoIterator<Item = (A, T)>,
        _clk8_fall: impl IntoIterator<Item = (A, T)>,
    ) -> Self
    where
        A: TimeAnchor,
        T: Into<Tone>,
    {
        todo!()
    }
}

impl Layout<()> for CopperLayout {
    fn block_at_with(&self, _pos: BlockPos, _ctx: &()) -> Option<BlockState> {
        todo!()
    }

    fn size_with(&self, _ctx: &()) -> BlockPos {
        todo!()
    }
}

// Template: Controller
//
// ++++++++++++============++++++++++++============++++++++++++============

/// The fixed controller circuit the copper layout runs on.
pub struct Controller {
    /// Clock ring length in redstone ticks.
    delay: RedStoneTick,
    /// Whether the status signal from the 2clk stage to the 4clk stage is
    /// inverted.
    clk2_inverted: bool,
    /// Whether the status signal from the 4clk stage to the 8clk stage is
    /// inverted.
    clk4_inverted: bool,
    /// Lit state of the bulbs from low to high clock, at `x = 5, 9, 13, 17`.
    lit: [bool; 4],
}

impl Controller {
    /// Builds a controller with the given clock ring length, choosing whether
    /// the first two status links invert their signal.
    pub fn new(delay: RedStoneTick, clk2_inverted: bool, clk4_inverted: bool) -> Self {
        let lit = match (clk2_inverted, clk4_inverted) {
            (false, false) => [false, false, false, false],
            (false, true) => [false, false, true, false],
            (true, false) => [false, true, true, true],
            (true, true) => [false, true, true, false],
        };
        Self {
            delay,
            clk2_inverted,
            clk4_inverted,
            lit,
        }
    }
}

impl<Ctx> Layout<Ctx> for Controller {
    fn block_at_with(&self, pos: BlockPos, _ctx: &Ctx) -> Option<BlockState> {
        use WireConn::{Idle, Side};

        let (x, y, z) = (pos.x, pos.y, pos.z);
        match (y, z) {
            (0, 0) if (5..=17).contains(&x) => Some(cut_copper()),
            (0, 1 | 2) if (1..=17).contains(&x) => Some(cut_copper()),
            (1, 0) if x == 5 => Some(cut_copper()),
            // The chain points west, so its stored facing is east.
            (1, 0) if (6..=16).contains(&x) => {
                todo!("split {} redstone ticks across the chain", self.delay)
            }
            (1, 0) if x == 17 => Some(redstone_torch(Some("north"), false)),
            (1, 1) if (1..=17).contains(&x) => Some(cut_copper()),
            (1, 2) if (5..=17).contains(&x) && x % 4 == 1 => {
                Some(bulb(self.lit[((x - 5) / 4) as usize]))
            }
            (1, 2) if (6..=16).contains(&x) => {
                let stage = ((x - 6) / 4) as usize;
                let inverted = [self.clk2_inverted, self.clk4_inverted, false][stage];
                let local = BlockPos::new((x - 6) % 4, 0, 0);
                Link.block_at_with(local, &(self.lit[stage], inverted))
            }
            (1, 2) if (1..=17).contains(&x) => Some(cut_copper()),
            (2, 0) if x == 5 => Some(wire_state(Idle, Idle, Side, Side, "14")),
            (2, 1) if (1..=17).contains(&x) => Some(ladder(x)),
            (2, 2) if x == 1 => Some(wire_state(Idle, Idle, Side, Side, "10")),
            (2, 2) if x == 5 => Some(lever()),
            _ => None,
        }
    }

    fn size_with(&self, _ctx: &Ctx) -> BlockPos {
        BlockPos::new(19, 3, 3)
    }
}

/// The status row between two consecutive clock bulbs, read within the source
/// bulb's lit state and whether the signal is inverted.
struct Link;

impl Layout<(bool, bool)> for Link {
    fn block_at_with(&self, pos: BlockPos, ctx: &(bool, bool)) -> Option<BlockState> {
        let (lit, inverted) = *ctx;
        match (pos.x, inverted) {
            (0, true) | (1, false) => Some(comparator(lit)),
            (2, true) => Some(redstone_torch(Some("east"), !lit)),
            _ => Some(cut_copper()),
        }
    }

    fn size_with(&self, _ctx: &(bool, bool)) -> BlockPos {
        BlockPos::new(3, 1, 1)
    }
}

/// The copper the floor and wall are built from.
fn cut_copper() -> BlockState {
    BlockState {
        name: "minecraft:oxidized_cut_copper".into(),
        properties: Default::default(),
    }
}

/// A copper bulb in the given lit state.
fn bulb(lit: bool) -> BlockState {
    BlockState {
        name: "minecraft:oxidized_copper_bulb".into(),
        properties: From::from([
            ("lit".into(), lit.to_string().into()),
            ("powered".into(), "true".into()),
        ]),
    }
}

/// A comparator pointing west, powered as given.
fn comparator(powered: bool) -> BlockState {
    BlockState {
        name: "minecraft:comparator".into(),
        properties: From::from([
            ("facing".into(), "west".into()),
            ("mode".into(), "compare".into()),
            ("powered".into(), powered.to_string().into()),
        ]),
    }
}

/// The controller's lever.
fn lever() -> BlockState {
    BlockState {
        name: "minecraft:lever".into(),
        properties: From::from([
            ("face".into(), "floor".into()),
            ("facing".into(), "south".into()),
            ("powered".into(), "true".into()),
        ]),
    }
}

/// The power ladder fed by the lever.
fn ladder(x: i32) -> BlockState {
    use WireConn::{Idle, Side};

    let west = if x > 1 { Side } else { Idle };
    let north = if x == 5 { Side } else { Idle };
    let south = if x == 1 || x == 5 { Side } else { Idle };
    let power = (15 - (x - 5).abs()).to_string();
    wire_state(west, Side, north, south, power)
}
