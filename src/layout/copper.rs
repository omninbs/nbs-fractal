//! TODO: describe the copper layout.

use super::Layout;
use super::blocks::{Facing, WireConn, redstone_torch, repeater, wire_state};
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
    /// Half the clock period in redstone ticks.
    delay: RedStoneTick,
    /// Whether the status link from 2clk to 4clk inverts.
    clk2_inverted: bool,
    /// Whether the status link from 4clk to 8clk inverts.
    clk4_inverted: bool,
    /// Cached bulb lit state, low to high clock.
    lit: [bool; 4],
}

impl Controller {
    /// Builds a controller with the given half clock period, choosing whether
    /// the status links into the 4clk and 8clk stages invert.
    pub fn new(delay: RedStoneTick, clk2_inverted: bool, clk4_inverted: bool) -> Self {
        assert!(
            delay >= 3,
            "the T flip-flop chain needs 3 redstone ticks of clock high"
        );
        assert!(
            delay <= 45,
            "the clock period spans at most 11 repeaters and a torch"
        );
        // Bit 3 is the synthesis bit; may yield a different state-space family.
        let lit = match (clk2_inverted, clk4_inverted) {
            (false, false) => [false, false, false, false], // 0 0 0 0
            (false, true) => [false, false, true, false],   // 0 0 1 0
            (true, false) => [false, true, true, true],     // 0 1 1 1
            (true, true) => [false, true, true, false],     // 0 1 1 0
        };
        Self {
            delay,
            clk2_inverted,
            clk4_inverted,
            lit,
        }
    }

    /// The bulb unit at local offset `local_x`, four columns wide: a bulb
    /// trailed by its three-cell chain, the last chain cut off.
    fn bulb_unit(&self, local_x: i32) -> Option<BlockState> {
        let (stage, local) = ((local_x / 4) as usize, local_x % 4);
        let lit = self.lit[stage];
        let inverted = match stage {
            0 => self.clk2_inverted,
            1 => self.clk4_inverted,
            _ => false,
        };
        match (local, inverted) {
            (0, _) => Some(bulb(lit)),                             // bulb
            (1, true) | (2, false) => Some(comparator(lit)),       // signal
            (3, true) => Some(redstone_torch(Some("east"), !lit)), // inverter
            _ => Some(cut_copper()),                               // blocks
        }
    }

    /// The row above the clock and status rows.
    fn top(&self, pos: BlockPos) -> Option<BlockState> {
        use WireConn::{Idle, Side};

        let ladder = |x: i32| {
            let west = if x > 1 { Side } else { Idle };
            let north = if x == 5 { Side } else { Idle };
            let south = if x == 1 || x == 5 { Side } else { Idle };
            wire_state(west, Side, north, south, (15 - (x - 5).abs()).to_string())
        };
        match (pos.z, pos.x) {
            (1, x @ 1..=17) => Some(ladder(x)),
            (0, 5) => Some(wire_state(Idle, Idle, Side, Side, "14")),
            (2, 1) => Some(wire_state(Idle, Idle, Side, Side, "10")),
            (2, 5) => Some(lever()),
            _ => None,
        }
    }

    /// The clock chain from the lever to the torch.
    fn clock(&self, pos: BlockPos) -> Option<BlockState> {
        // Up to 4 rt per repeater, one more for the torch.
        let end = 6 + ((self.delay + 2) / 4) as i32;
        // Points west, so stores east.
        let chain = |x: i32| {
            let delay = 4.min(self.delay as i32 - 1 - 4 * (x - 6));
            repeater(delay.to_string(), Facing::West, false, false)
        };
        match (pos.y, pos.x) {
            // blocks
            (_, 5) => Some(cut_copper()),
            (0, x) if (5..=end).contains(&x) => Some(cut_copper()),
            // repeaters
            (1, x) if (6..end).contains(&x) => Some(chain(x)),
            // rs-torch
            (1, x) if x == end => Some(redstone_torch(Some("north"), false)),
            _ => None,
        }
    }
}

impl<Ctx> Layout<Ctx> for Controller {
    fn block_at_with(&self, pos: BlockPos, _ctx: &Ctx) -> Option<BlockState> {
        let shell = || match pos.x {
            1..=17 => Some(cut_copper()), // block
            _ => None,                    // none(air)
        };
        match (pos.y, pos.z, pos.x) {
            (2, ..) => self.top(pos),                    // top
            (_, 0, _) => self.clock(pos),                // clock
            (1, 2, 5..=17) => self.bulb_unit(pos.x - 5), // bulb unit
            _ => shell(),                                // shell
        }
    }

    fn size_with(&self, _ctx: &Ctx) -> BlockPos {
        BlockPos::new(19, 3, 3)
    }
}

// Blocks
//
// ++++++++++++============++++++++++++============++++++++++++============

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
