//! TODO: describe the copper layout.

use super::Layout;
use super::bamboo::Joint;
use super::blocks::{Facing, WireConn, chain_block, redstone_torch, repeater, wire_state};
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
            (0, _) => Some(bulb(lit)),                                     // bulb
            (1, true) | (2, false) => Some(comparator(Facing::West, lit)), // signal
            (3, true) => Some(redstone_torch(Some("east"), !lit)),         // inverter
            _ => Some(cut_copper()),                                       // blocks
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

// Template: TrackHead
//
// ++++++++++++============++++++++++++============++++++++++++============

/// The head of a 1clk track, tapping the raw clock through a repeater.
pub struct ClkHead {
    /// The first cell: its notes, a repeater holding them back, or nothing.
    start: Option<Joint>,
}

impl ClkHead {
    /// Builds a head carrying `start`.
    pub fn new(start: Option<Joint>) -> Self {
        Self { start }
    }
}

impl Layout<()> for ClkHead {
    fn block_at_with(&self, pos: BlockPos, _ctx: &()) -> Option<BlockState> {
        match (pos.z, pos.y, pos.x) {
            (0, 0, 1) => Some(chain_block()),
            // A held note lengthens the tap instead of playing.
            (0, 1, 1) => {
                let tick = self.start.as_ref().map_or(0, Joint::stem).max(1);
                Some(repeater(tick.to_string(), Facing::South, false, false))
            }
            _ => self.start.as_ref()?.block_at_with(pos, &true),
        }
    }

    fn size_with(&self, _ctx: &()) -> BlockPos {
        BlockPos::new(3, 2, 2)
    }
}

/// The head of a 2clk or 4clk track, tapping a bulb's output through a
/// comparator.
pub struct BulbHead {
    /// The first cell: its notes, a repeater holding them back, or nothing.
    start: Option<Joint>,
}

impl BulbHead {
    /// Builds a head carrying `start`.
    pub fn new(start: Option<Joint>) -> Self {
        Self { start }
    }
}

impl Layout<()> for BulbHead {
    fn block_at_with(&self, pos: BlockPos, _ctx: &()) -> Option<BlockState> {
        match (pos.z, pos.y, pos.x) {
            (0, 0, 1) => Some(chain_block()),
            (0, 1, 1) => Some(comparator(Facing::North, false)),
            _ => self.start.as_ref()?.block_at_with(pos, &true),
        }
    }

    fn size_with(&self, _ctx: &()) -> BlockPos {
        BlockPos::new(3, 2, 2)
    }
}

/// The joined head of the two 8clk tracks, tapping two bulbs and inverting one
/// into the other.
pub struct SynthHead {
    /// The two halves' first cells, west to east.
    halves: [Joint; 2],
}

impl SynthHead {
    /// Builds a head from the two halves' first cells.
    pub fn new(halves: [Joint; 2]) -> Self {
        Self { halves }
    }
}

impl Layout<()> for SynthHead {
    fn block_at_with(&self, pos: BlockPos, _ctx: &()) -> Option<BlockState> {
        use WireConn::{Idle, Side};

        let inverter = |y: i32| match (y, pos.x) {
            (0, _) => Some(chain_block()),
            // Points east.
            (1, 2) => Some(repeater("1", Facing::West, false, false)),
            (1, 3) => Some(wire_state(Side, Side, Idle, Idle, "0")),
            (1, 4) => Some(redstone_torch(Some("west"), true)),
            _ => None,
        };
        let local = BlockPos::new(pos.x % 4, pos.y, pos.z);
        match (pos.z, pos.y, pos.x) {
            (0, 0, 1 | 5) => Some(chain_block()),
            (0, 1, 1 | 5) => Some(comparator(Facing::North, false)),
            (1, _, 2..=4) => inverter(pos.y),
            _ => self.halves[(pos.x > 3) as usize].block_at_with(local, &true),
        }
    }

    fn size_with(&self, _ctx: &()) -> BlockPos {
        BlockPos::new(7, 2, 2)
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

/// A comparator toward `facing`, powered as given.
fn comparator(facing: Facing, powered: bool) -> BlockState {
    BlockState {
        name: "minecraft:comparator".into(),
        properties: From::from([
            ("facing".into(), facing.name().into()),
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
