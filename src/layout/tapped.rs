//! Tapped delay line layout for NBS song projection.

use super::blocks::{Facing, ToneBlocks, WireConn, chain_block, observer};
use super::blocks::{redstone_torch, repeater, wire_state};
use super::compact::{CompactLayout, RawCompact};
use super::compose::{Arranged, EdgeArranged, Overlaid, WithFloor};
use super::{Axis, Layout, LayoutExt};
use super::{counts, signed, size_of};
use crate::{BlockPos, BlockSize, BlockState, RedStoneTick, analysis::BoundedTec};
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
    /// The control box's signed extent: its width splits a position between the
    /// two units, and it bounds the control side.
    control_size: BlockPos,
    /// The playing box's signed extent, bounding the playing side.
    playing_size: BlockPos,
    size: BlockSize,
}

impl TappedLayout {
    /// Builds the composite layout from bounded TEC data.
    ///
    /// Each TEC's offsets drive the tapped delay line (control unit); its bounded
    /// kernel drives the playing unit. With `balanced`, the playing lines stretch
    /// their row intervals so they fit within the longest line's span, flooring
    /// the interval to whole blocks; otherwise every line keeps the base pitch.
    /// `max_gain` caps one tap's length; a larger gap is carried by several taps
    /// in a row, so no single tap grows past the cap.
    pub fn new<I: IntoIterator<Item = BoundedTec<Tone>>>(
        tecs: I,
        wrap_length: Option<NonZero<usize>>,
        gap: u32,
        max_gain: Option<u32>,
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
            tap_lines.push(TapLine::new(tec.scatter, repeater_coarse, switch, max_gain));
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
        let control_size = signed(control.size());
        let playing_size = signed(playing.size());
        let size = BlockPos::new(
            control_size.x + playing_size.x,
            control_size.y.max(playing_size.y),
            control_size.z.max(playing_size.z),
        );

        Self {
            control,
            playing,
            control_size,
            playing_size,
            size: size_of(size),
        }
    }
}

impl Layout<()> for TappedLayout {
    fn block_at_with(&self, pos: BlockPos, _ctx: &()) -> Option<BlockState> {
        let control = self.control_size;
        let side = pos.x < control.x;

        let (local, size) = match side {
            true => (pos, control),
            false => (pos - BlockPos::new(control.x, 0, 0), self.playing_size),
        };
        let dispatch = || match side {
            true => self.control.block_at(local),
            false => self.playing.block_at(local),
        };
        (local.y < size.y && local.z < size.z).then(dispatch)?
    }

    fn size_with(&self, _ctx: &()) -> BlockSize {
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
    size: BlockSize,
}

impl TapLine {
    /// Builds a TEC's line of taps, one per gap between its offsets.
    pub fn new<I: IntoIterator<Item = NonZero<RedStoneTick>>>(
        ticks: I,
        repeater_coarse: Option<NonZero<RedStoneTick>>,
        switch: bool,
        max_gain: Option<u32>,
    ) -> Self {
        let mut taps = Vec::new();
        let mut prev = 0;
        for tick in ticks {
            let gap = NonZero::new(tick.get() - prev).expect("a gap is positive");
            prev = tick.get();
            taps.extend(Tap::row(gap, repeater_coarse, max_gain));
        }

        let delays = EdgeArranged::new(taps, Axis::Southing, 0, Axis::Easting.unit(), &());
        let inner = signed(delays.size());
        let width = inner.x.max(if switch { 5 } else { 2 });

        // an empty line still keeps the port's height
        let size = BlockPos::new(width, inner.y.max(4), inner.z + 1);
        let anchor = BlockPos::new(width - inner.x, 0, 1);

        Self {
            anchor,
            switch,
            size: size_of(size),
            delays,
        }
    }

    fn port(&self, pos: BlockPos) -> Option<BlockState> {
        use WireConn::*;
        let local_easting = signed(self.size).x - pos.x - 1;
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
            (4, 2) if self.switch => Some(Self::switch_button()),
            (1, 0) if !self.switch => Some(chain_block()),
            (1, 1) if !self.switch => Some(redstone_torch(Option::None::<&'static str>, false)),
            _ => Option::None,
        }
    }

    /// The switch's source button: a bass-drum note block.
    fn switch_button() -> BlockState {
        let tone = Tone::new(Instrument::BassDrum, Key::from_minecraft_note(0).unwrap());
        tone.note_block_state().unwrap_or_else(chain_block)
    }
}

impl Layout<()> for TapLine {
    fn block_at_with(&self, pos: BlockPos, ctx: &()) -> Option<BlockState> {
        match pos.z == 0 {
            true => self.port(pos),
            false => self.delays.get_block_with(pos - self.anchor, ctx),
        }
    }

    fn size_with(&self, _ctx: &()) -> BlockSize {
        self.size
    }

    /// The port's fixed hardware, plus every delay element's palette.
    fn block_palette_with(&self, ctx: &()) -> HashSet<BlockState> {
        use WireConn::*;
        let (switch_west, switch_south) = match self.switch {
            true => (Side, Idle),
            false => (Idle, Side),
        };
        let mut palette = HashSet::from([
            chain_block(),
            redstone_torch(Option::None::<&'static str>, true),
            redstone_torch(Option::None::<&'static str>, false),
            wire_state(switch_west, Side, Idle, switch_south, "0"),
            repeater("4", Facing::East, false, false),
            observer("west"),
            Self::switch_button(),
        ]);
        palette.extend(self.delays.block_palette_with(ctx));
        palette
    }
}

// Tap
//
// ++++++++++++============++++++++++++============++++++++++++============

/// One segment: a chain feeding the bus column east of it.
struct Tap {
    /// The chain and the column, overlaid without overlap.
    inner: Overlaid<Chain, Bus>,
}

impl Tap {
    /// The taps consuming `delay`, none over `gain`.
    fn row(
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
