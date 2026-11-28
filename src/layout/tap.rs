//! The tick budget of one tap: how its delay splits between chain and bus.

use super::blocks::{Facing, WireConn, chain_block, repeater};
use super::blocks::{redstone_torch, wire_state};
use super::compose::Overlaid;
use super::{Layout, LayoutExt, counts, signed, size_of};
use crate::{BlockPos, BlockSize, BlockState, RedStoneTick};
use std::collections::HashSet;
use std::num::NonZero;

// Tap
//
// ++++++++++++============++++++++++++============++++++++++++============

/// One segment: a chain feeding the bus column east of it.
///
/// A tap therefore carries a group of delay elements, not a single one.
pub struct Tap {
    /// The chain and the column, overlaid without overlap.
    inner: Overlaid<Chain, Bus>,
}

impl Tap {
    /// The taps consuming `delay`, none over `gain`.
    pub fn row(
        delay: NonZero<RedStoneTick>,
        repeater_coarse: Option<NonZero<RedStoneTick>>,
        gain: Option<u32>,
    ) -> impl Iterator<Item = Self> {
        let (terminal_cap, relay_cap) = match gain {
            Some(gain) => (Self::capacity(gain, true), Self::capacity(gain, false)),
            None => (delay.get(), delay.get()),
        };
        let mut remaining = delay.get();
        std::iter::from_fn(move || match remaining {
            0 => None,
            rem if rem > terminal_cap => {
                let relay = NonZero::new(relay_cap.min(rem - 4)).expect("a relay is positive");
                remaining = rem - relay.get();
                Some(Self::new(relay, repeater_coarse, false))
            }
            rem => {
                let terminal = NonZero::new(rem).expect("the terminal tap is positive");
                remaining = 0;
                Some(Self::new(terminal, repeater_coarse, true))
            }
        })
    }

    fn new(
        delay: NonZero<RedStoneTick>,
        repeater_coarse: Option<NonZero<RedStoneTick>>,
        out: bool,
    ) -> Self {
        assert!(
            repeater_coarse.is_none_or(|c| c.get() >= 4),
            "coarse < 4 is not supported, adjust the step"
        );
        let floor = if out { 4 } else { 5 };
        assert!(
            delay.get() >= floor,
            "a segment carries at least {floor} rt, got {}",
            delay.get()
        );

        // bus
        let taken = (delay.get() - 3).min(if out { 4 } else { 8 });
        let back = taken.saturating_sub(4).max(u32::from(!out));
        let bus = Bus::new(taken - back, back);

        // chain
        let chain = Chain::new(delay.get() - taken);
        let (chain_size, bus_size) = (signed(chain.size()), signed(bus.size()));
        let size = BlockPos::new(
            chain_size.x + bus_size.x,
            chain_size.y.max(bus_size.y),
            chain_size.z.max(bus_size.z),
        );
        let anchor = BlockPos::new(chain_size.x, 0, 0);
        let inner = Overlaid::new(BlockPos::ORIGIN, chain, anchor, bus, size_of(size), &());
        Self { inner }
    }

    /// The most delay a segment carries at `gain` columns.
    fn capacity(gain: u32, out: bool) -> RedStoneTick {
        gain * 8 + if out { 13 } else { 17 }
    }
}

impl Layout<()> for Tap {
    fn block_at_with(&self, pos: BlockPos, ctx: &()) -> Option<BlockState> {
        self.inner.block_at_with(pos, ctx)
    }

    fn size_with(&self, ctx: &()) -> BlockSize {
        self.inner.size_with(ctx)
    }

    fn block_palette_with(&self, ctx: &()) -> HashSet<BlockState> {
        self.inner.block_palette_with(ctx)
    }
}

// Bus
//
// ++++++++++++============++++++++++++============++++++++++++============

/// The bus column a segment ends on: one cell, two lanes.
struct Bus {
    /// The lane ahead, `z = 0`: the pulse leaves here, toward the port.
    front: RedStoneTick,
    /// The lane behind, `z = 1`: the chain and the deeper taps feed here.
    back: RedStoneTick,
}

impl Bus {
    /// A column carrying these two lane delays; a lane holds 4 rt.
    fn new(front: RedStoneTick, back: RedStoneTick) -> Self {
        assert!(
            front <= 4 && back <= 4,
            "a bus lane holds at most 4 rt, got {front} and {back}"
        );
        Self { front, back }
    }

    /// The block a lane draws: wire at zero, else a repeater.
    fn lane(delay: RedStoneTick) -> BlockState {
        use WireConn::{Idle, Side};
        match delay {
            0 => wire_state(Idle, Idle, Side, Side, "0"),
            tick => repeater(tick.to_string(), Facing::North, false, false),
        }
    }
}

impl Layout<()> for Bus {
    fn block_at_with(&self, pos: BlockPos, _ctx: &()) -> Option<BlockState> {
        match (pos.z, pos.y) {
            (0, 2) => Some(Self::lane(self.front)),
            (1, 2) => Some(Self::lane(self.back)),
            (_, 1) => Some(chain_block()),
            _ => Option::None,
        }
    }

    fn size_with(&self, _ctx: &()) -> BlockSize {
        BlockSize::new(1, 4, 2)
    }

    /// Both lane states.
    fn block_palette_with(&self, _ctx: &()) -> HashSet<BlockState> {
        let mut palette = HashSet::from([chain_block()]);
        for lane in [self.front, self.back] {
            palette.insert(Self::lane(lane));
        }
        palette
    }
}

// Chain
//
// ++++++++++++============++++++++++++============++++++++++++============

/// A delay chain carrying exactly its redstone ticks.
struct Chain {
    delay: RedStoneTick,
}

impl Chain {
    /// A chain of `delay` ticks, the least being 3.
    fn new(delay: RedStoneTick) -> Self {
        assert!(delay >= 3, "a chain carries at least 3 rt, got {delay}");
        Self { delay }
    }

    /// The columns spanned: one per eight ticks, plus head and entry.
    fn columns(&self) -> i32 {
        ((self.delay - 2) / 8) as i32 + 3
    }
}

impl Layout<()> for Chain {
    fn block_at_with(&self, pos: BlockPos, _ctx: &()) -> Option<BlockState> {
        use WireConn::*;
        let (back, delay) = (self.columns() + 1 - pos.x, self.delay);
        let res = (delay + 6) % 8 + 1;
        let dust = Some(wire_state(Side, Side, Idle, Idle, "15"));
        let east = |d: u32| Some(repeater(d.to_string(), Facing::East, false, false));
        let west = |d: u32| Some(repeater(d.to_string(), Facing::West, true, false));

        match (back, pos.x, pos.z, pos.y) {
            (_, _, _, 1) | (_, 0, 0, 2) | (2, _, 1, 2) => Some(chain_block()),
            (_, 0, 1, 2) => Some(redstone_torch(Some("south"), false)),
            (2, _, 0, 2) => Some(redstone_torch(Some("south"), true)),
            (4.., 2.., 0, 2) => west(4),
            (4.., 1, 0, 2) => west(3),
            (3.., 2.., 1, 2) => east(4),
            (3, 1, 1, 2) => east(delay.saturating_sub(2).min(4)),
            (4.., 1, 1, 2) => east(res.min(4)),
            (3, 1, 0, 2) if delay < 7 => dust,
            (3, 1, 0, 2) => west(delay.saturating_sub(6).min(4)),
            (3, 2.., 0, 2) if res < 5 => dust,
            (3, 2.., 0, 2) => west(res - 4),
            _ => Option::None,
        }
    }

    fn size_with(&self, _ctx: &()) -> BlockSize {
        counts(self.columns(), 4, 2)
    }

    /// Every state the run draws.
    fn block_palette_with(&self, _ctx: &()) -> HashSet<BlockState> {
        use Facing::{East, West};
        use WireConn::*;
        let delay = self.delay;
        let mut palette = HashSet::from([
            chain_block(),
            redstone_torch(Some("south"), false),
            redstone_torch(Some("south"), true),
            repeater("3", West, true, false),
            repeater("4", West, true, false),
            repeater("4", East, false, false),
            repeater(
                delay.saturating_sub(2).min(4).to_string(),
                East,
                false,
                false,
            ),
            repeater(
                delay.saturating_sub(6).min(4).to_string(),
                East,
                false,
                false,
            ),
            wire_state(Side, Side, Idle, Idle, "15"),
        ]);
        for tick in 1..=4 {
            palette.insert(repeater(tick.to_string(), East, false, false));
            palette.insert(repeater(tick.to_string(), West, true, false));
        }
        palette
    }
}
