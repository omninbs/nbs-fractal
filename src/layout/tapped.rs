//! Tapped delay line layout for NBS song projection.

use super::blocks::{Facing, ToneBlocks, WireConn, chain_block, observer};
use super::blocks::{redstone_torch, repeater, wire_state};
use super::compact::{CompactLayout, RawCompact};
use super::compose::{Arranged, EdgeArranged, Overlaid, WithFloor};
use super::{Axis, Layout, LayoutExt};
use crate::{BlockPos, BlockState, RedStoneTick, analysis::BoundedTec};
use rsnbs::note::{Instrument, Key, Tone};
use std::collections::HashSet;
use std::num::NonZero;

// TappedLayout
//
// ++++++++++++============++++++++++++============++++++++++++============

/// A direct descriptive structure for the translation equivalence class design:
/// a tapped delay line controls the repetition pattern, using the Compact layout
/// as its playing unit.
pub struct TappedLayout {
    control: EdgeArranged<TapLine>,
    playing: Arranged<WithFloor<CompactLayout>>,
    size: BlockPos,
}

impl TappedLayout {
    /// Builds the composite layout from bounded TEC data.
    ///
    /// Each TEC's offsets drive the tapped delay line (control unit); its bounded
    /// kernel drives the playing unit. With `balanced`, the playing lines stretch
    /// their row intervals so they fit within the longest line's span, flooring
    /// the interval to whole blocks; otherwise every line keeps the base pitch.
    /// `max_tap` bounds one tap's delay; a larger gap is carried by several taps
    /// in a row, so no single tap grows past the bound.
    pub fn new<I: IntoIterator<Item = BoundedTec<Tone>>>(
        tecs: I,
        wrap_length: Option<NonZero<usize>>,
        gap: u32,
        max_tap: Option<MaxTap>,
        full: bool,
        balanced: bool,
    ) -> Self {
        let tecs: Vec<BoundedTec<Tone>> = tecs.into_iter().collect();
        let count = tecs.len();
        let mut tap_lines = Vec::new();
        let mut raws = Vec::new();

        // kernel -> raw compact rows
        for (lyr, tec) in tecs.into_iter().enumerate() {
            let tec = tec.into_inner();
            let repeater_coarse = tec.min_gap().and_then(|gap| NonZero::new(gap / 2));
            let notes = tec
                .kernel
                .into_notes()
                .into_iter()
                .map(|(tick, tones)| (tick + 2 * lyr as u32, tones))
                .flat_map(|(tick, tones)| tones.into_iter().map(move |tone| (tick, tone)));
            let raw = CompactLayout::raw(notes, repeater_coarse, wrap_length);

            // the bottom line carries the switch
            let switch = lyr + 1 == count;
            tap_lines.push(TapLine::new(tec.scatter, repeater_coarse, switch, max_tap));
            raws.push(raw);
        }

        // rows -> compact layouts, balanced across the longest line
        let most = match balanced {
            true => raws.iter().map(RawCompact::rows).max().unwrap_or(0),
            false => 0,
        };
        let mut layouts: Vec<_> = raws
            .into_iter()
            .map(|raw| {
                let width = raw.balanced_width(most, gap);
                WithFloor::new(raw.into_layout(width), full)
            })
            .collect();

        // signal propagates upward, so lower layers need more delay
        tap_lines.reverse();
        layouts.reverse();

        // arrange both sides
        let control = EdgeArranged::new(tap_lines, Axis::Elevation, 0, Axis::Easting.unit(), &());
        let playing = Arranged::new(layouts, Axis::Elevation, 0, &());

        // composite bounding box
        let ctrl_size = control.size();
        let play_size = playing.size();
        let size = BlockPos::new(
            ctrl_size.x + play_size.x,
            ctrl_size.y.max(play_size.y),
            ctrl_size.z.max(play_size.z),
        );

        Self {
            control,
            playing,
            size,
        }
    }
}

impl Layout<()> for TappedLayout {
    fn block_at_with(&self, pos: BlockPos, _ctx: &()) -> Option<BlockState> {
        let divide = self.control.size().x;
        let side = pos.x < divide;

        let (local, size) = match side {
            true => (pos, self.control.size()),
            false => (pos - BlockPos::new(divide, 0, 0), self.playing.size()),
        };
        let dispatch = || match side {
            true => self.control.block_at(local),
            false => self.playing.block_at(local),
        };
        (local.y < size.y && local.z < size.z).then(dispatch)?
    }

    fn size_with(&self, _ctx: &()) -> BlockPos {
        self.size
    }

    /// The control lines and the playing units together.
    fn block_palette_with(&self, _ctx: &()) -> HashSet<BlockState> {
        let mut palette = self.control.block_palette();
        palette.extend(self.playing.block_palette());
        palette
    }
}

// TapLine
//
// ++++++++++++============++++++++++++============++++++++++++============

/// A single TEC's tapped delay line.
pub struct TapLine {
    delays: EdgeArranged<Tap>,
    anchor: BlockPos,
    switch: bool,
    size: BlockPos,
}

impl TapLine {
    /// Builds a TEC's tapped delay line from its ascending offsets, each tap
    /// driven by the gap to the previous offset; `switch` adds the closing tap,
    /// and `max_tap` bounds one tap's delay by spreading a wider gap over a row
    /// of taps whose last one exposes the pulse.
    pub fn new<I: IntoIterator<Item = NonZero<RedStoneTick>>>(
        ticks: I,
        repeater_coarse: Option<NonZero<RedStoneTick>>,
        switch: bool,
        max_tap: Option<MaxTap>,
    ) -> Self {
        let mut taps = Vec::new();
        let mut prev = 0;
        let mut carried = 0;
        for tick in ticks {
            let mut parts = split_gap(tick.get() - prev, max_tap);
            prev = tick.get();
            let head = parts.pop().expect("a gap is positive");
            for part in parts {
                carried += part.get();
                taps.push(Tap::new(part, repeater_coarse, false));
            }
            carried += head.get();
            taps.push(Tap::new(head, repeater_coarse, true));
        }
        debug_assert_eq!(carried, prev, "the taps carry exactly the line's span");
        let delays = EdgeArranged::new(taps, Axis::Southing, 0, Axis::Easting.unit(), &());
        let inner = delays.size();
        let width = inner.x.max(if switch { 5 } else { 2 });
        // an empty line still keeps the port's height
        let height = inner.y.max(Bus::SIZE.y);
        let size = BlockPos::new(width, height, inner.z + 1);
        let anchor = BlockPos::new(width - inner.x, 0, 1);

        Self {
            anchor,
            switch,
            size,
            delays,
        }
    }

    fn port(&self, pos: BlockPos) -> Option<BlockState> {
        use WireConn::*;
        let local_easting = self.size.x - pos.x - 1;
        let local_elevation = pos.y;
        let port_wire = || {
            let switch_south = if self.switch { Idle } else { Side };
            let switch_west = if self.switch { Side } else { Idle };
            wire_state(switch_west, Side, Idle, switch_south, "0")
        };

        match (local_easting, local_elevation) {
            (1, 3) => Some(redstone_torch(Option::None::<&'static str>, true)),
            (0, 1) | (1, 2) => Some(chain_block()),
            (0, 2) => Some(port_wire()),
            (1..=4, 1) if self.switch => Some(chain_block()),
            (2, 2) if self.switch => Some(repeater("4", Facing::East, false, false)),
            (3, 2) if self.switch => Some(observer("west")),
            (4, 2) if self.switch => Some(switch_button()),
            (1, 0) if !self.switch => Some(chain_block()),
            (1, 1) if !self.switch => Some(redstone_torch(Option::None::<&'static str>, false)),
            _ => Option::None,
        }
    }

    /// The blocks the port can draw: its torches, its switch hardware, and the
    /// wire link that closes the line.
    fn port_palette(&self) -> HashSet<BlockState> {
        use WireConn::*;
        let (switch_west, switch_south) = match self.switch {
            true => (Side, Idle),
            false => (Idle, Side),
        };
        HashSet::from([
            chain_block(),
            redstone_torch(Option::None::<&'static str>, true),
            redstone_torch(Option::None::<&'static str>, false),
            wire_state(switch_west, Side, Idle, switch_south, "0"),
            repeater("4", Facing::East, false, false),
            observer("west"),
            switch_button(),
        ])
    }
}

impl Layout<()> for TapLine {
    fn block_at_with(&self, pos: BlockPos, ctx: &()) -> Option<BlockState> {
        match pos.z == 0 {
            true => self.port(pos),
            false => self.delays.get_block_with(pos - self.anchor, ctx),
        }
    }

    fn size_with(&self, _ctx: &()) -> BlockPos {
        self.size
    }

    /// The port's fixed hardware, plus every delay element's palette.
    fn block_palette_with(&self, ctx: &()) -> HashSet<BlockState> {
        let mut palette = self.port_palette();
        palette.extend(self.delays.block_palette_with(ctx));
        palette
    }
}

// Split
//
// ++++++++++++============++++++++++++============++++++++++++============

/// The delays one gap's segments carry, none above `max`.
///
/// A gap within `max` stays a single segment. A longer one is spread over
/// `ceil(gap / max)` segments of near-equal delay, so the widest shrinks with the
/// count while the group still carries exactly `gap`.
fn split_gap(gap: RedStoneTick, max: Option<MaxTap>) -> Vec<NonZero<RedStoneTick>> {
    let max = max.map_or(gap, MaxTap::get).max(1);
    let count = gap.div_ceil(max);
    let base = gap / count;
    let rest = gap % count;
    (0..count)
        .map(|index| if index < rest { base + 1 } else { base })
        .map(|part| NonZero::new(part).expect("a split segment is positive"))
        .collect()
}

/// The switch's source button: a bass-drum note block.
fn switch_button() -> BlockState {
    let tone = Tone::new(Instrument::BassDrum, Key::from_minecraft_note(0).unwrap());
    tone.note_block_state().unwrap_or_else(chain_block)
}

// Tap
//
// ++++++++++++============++++++++++++============++++++++++++============

/// One segment of the tapped delay line: a delay chain at the origin, with the
/// bus column it ends on laid east of it.
struct Tap {
    /// The chain and the column, overlaid without overlap.
    inner: Overlaid<Chain, Bus>,
}

impl Tap {
    fn new(
        delay: NonZero<RedStoneTick>,
        repeater_coarse: Option<NonZero<RedStoneTick>>,
        out: bool,
    ) -> Self {
        assert!(
            repeater_coarse.is_none_or(|c| c.get() >= 4),
            "coarse < 4 is not supported, adjust the step"
        );

        // one tick per lane, plus the chain's floor
        let lanes = if out { 1 } else { 2 };
        let least = Chain::MIN_DELAY + lanes * Bus::MIN_LANE_DELAY;
        assert!(
            delay.get() >= least,
            "a segment carries at least {least} rt, got {}",
            delay.get()
        );

        // the bus takes what it can, the chain the rest
        let absorbed = (delay.get() - Chain::MIN_DELAY).min(lanes * Bus::MAX_LANE_DELAY);
        let back = if out { 0 } else { absorbed / 2 };
        let bus = Bus::new(absorbed - back, back);

        let chain = Chain::new(delay.get() - absorbed);
        let (chain_size, bus_size) = (chain.size(), bus.size());
        let size = BlockPos::new(
            chain_size.x + bus_size.x,
            chain_size.y.max(bus_size.y),
            chain_size.z.max(bus_size.z),
        );
        let anchor = BlockPos::new(chain_size.x, 0, 0);
        let inner = Overlaid::new(BlockPos::ORIGIN, chain, anchor, bus, size, &());
        Self { inner }
    }
}

impl Layout<()> for Tap {
    fn block_at_with(&self, pos: BlockPos, ctx: &()) -> Option<BlockState> {
        self.inner.block_at_with(pos, ctx)
    }

    fn size_with(&self, ctx: &()) -> BlockPos {
        self.inner.size_with(ctx)
    }

    fn block_palette_with(&self, ctx: &()) -> HashSet<BlockState> {
        self.inner.block_palette_with(ctx)
    }
}

// Bus
//
// ++++++++++++============++++++++++++============++++++++++++============

/// The bus column a segment ends on: one cell, two lanes of delay.
struct Bus {
    /// The lane ahead, `z = 0`: the pulse leaves here, toward the port.
    front: RedStoneTick,
    /// The lane behind, `z = 1`: the chain and the deeper taps feed here.
    back: RedStoneTick,
}

impl Bus {
    /// The shortest delay one lane's repeater holds.
    const MIN_LANE_DELAY: RedStoneTick = 1;

    /// The longest delay one lane's repeater holds.
    const MAX_LANE_DELAY: RedStoneTick = 4;

    /// The column's own box.
    const SIZE: BlockPos = BlockPos::new(1, 4, 2);

    /// A column carrying these two lane delays; panics above [`Bus::MAX_LANE_DELAY`].
    fn new(front: RedStoneTick, back: RedStoneTick) -> Self {
        assert!(
            front <= Self::MAX_LANE_DELAY && back <= Self::MAX_LANE_DELAY,
            "a bus lane holds at most {} rt, got {front} and {back}",
            Self::MAX_LANE_DELAY
        );
        Self { front, back }
    }

    /// The block a lane draws: wire at zero, else a repeater of that many ticks.
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

    fn size_with(&self, _ctx: &()) -> BlockPos {
        Self::SIZE
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

/// A delay chain carrying exactly the redstone ticks it was built with.
struct Chain {
    delay: RedStoneTick,
}

impl Chain {
    /// The least delay one chain carries: a repeater between its two torches.
    const MIN_DELAY: RedStoneTick = 3;

    /// The run's depth, `z = 0..2`.
    const SOUTHING: i32 = 2;

    /// The run's height, its block row included.
    const ELEVATION: i32 = 4;

    /// A chain carrying `delay` ticks; panics under [`Chain::MIN_DELAY`].
    fn new(delay: RedStoneTick) -> Self {
        assert!(
            delay >= Self::MIN_DELAY,
            "a chain carries at least {} rt, got {delay}",
            Self::MIN_DELAY
        );
        Self { delay }
    }

    /// The columns spanned: one per eight ticks, plus head and entry.
    fn columns(&self) -> i32 {
        (self.delay as i32 - 2) / 8 + 3
    }
}

impl Layout<()> for Chain {
    fn block_at_with(&self, pos: BlockPos, _ctx: &()) -> Option<BlockState> {
        use WireConn::*;
        let back = self.columns() + 1 - pos.x;
        let delay = self.delay;
        let clamp = |tick: RedStoneTick| tick.min(4).to_string();
        let cycle = || clamp((delay + 7) % 8);
        let decay = |sub: RedStoneTick| clamp(delay.saturating_sub(sub));

        match (back, pos.x, pos.z, pos.y) {
            (_, _, _, 1) | (_, 0, 0, 2) | (2, _, 1, 2) => Some(chain_block()),
            (_, 0, 1, 2) => Some(redstone_torch(Some("south"), false)),
            (2, _, 0, 2) => Some(redstone_torch(Some("south"), true)),
            (4.., 2.., 0, 2) => Some(repeater("4", Facing::West, true, false)),
            (3.., 2.., 1, 2) => Some(repeater("4", Facing::East, false, false)),
            (3, 1, 1, 2) => Some(repeater(decay(2), Facing::East, false, false)),
            (4.., 1, 1, 2) => Some(repeater(cycle(), Facing::East, false, false)),
            (3, 1, 0, 2) if delay < 7 => Some(wire_state(Side, Side, Idle, Idle, "15")),
            (3, 1, 0, 2) => Some(repeater(decay(6), Facing::East, false, false)),
            (3, 2.., 0, 2) if (delay + 7) % 8 < 4 => Some(wire_state(Side, Side, Idle, Idle, "15")),
            (3, 2.., 0, 2) => Some(repeater(cycle(), Facing::West, true, false)),
            (4.., 1, 0, 2) => Some(repeater("3", Facing::West, true, false)),
            _ => Option::None,
        }
    }

    fn size_with(&self, _ctx: &()) -> BlockPos {
        BlockPos::new(self.columns(), Self::ELEVATION, Self::SOUTHING)
    }

    /// Every state the run draws.
    fn block_palette_with(&self, _ctx: &()) -> HashSet<BlockState> {
        use Facing::{East, West};
        use WireConn::*;
        let delay = self.delay;
        let clamp = |tick: RedStoneTick| tick.min(4).to_string();
        let cycle = clamp((delay + 7) % 8);
        HashSet::from([
            chain_block(),
            redstone_torch(Some("south"), false),
            redstone_torch(Some("south"), true),
            repeater("3", West, true, false),
            repeater("4", West, true, false),
            repeater("4", East, false, false),
            repeater(clamp(delay.saturating_sub(2)), East, false, false),
            repeater(clamp(delay.saturating_sub(6)), East, false, false),
            repeater(cycle.clone(), East, false, false),
            repeater(cycle, West, true, false),
            wire_state(Side, Side, Idle, Idle, "15"),
        ])
    }
}

// MaxTap
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Upper bound on one segment's delay, in redstone ticks.
///
/// A bound under [`MaxTap::MIN`] cannot be split into segments that each outlast
/// a bus seam, so the constructor reads it as no bound, just as [`NonZero::new`]
/// reads zero.
#[derive(Clone, Copy)]
pub struct MaxTap(NonZero<RedStoneTick>);

impl MaxTap {
    /// Smallest realizable bound: the shortest relay a split can yield still
    /// keeps its chain floor and a tick per lane.
    pub const MIN: RedStoneTick = 8;

    /// The bound, or `None` when `ticks` cannot carry a split.
    pub fn new(ticks: RedStoneTick) -> Option<Self> {
        NonZero::new(ticks)
            .filter(|ticks| ticks.get() >= Self::MIN)
            .map(Self)
    }

    /// The bound in redstone ticks.
    pub fn get(self) -> RedStoneTick {
        self.0.get()
    }
}
