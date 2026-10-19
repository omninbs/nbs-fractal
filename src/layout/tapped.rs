//! Tapped delay line layout for NBS song projection.

use super::blocks::{Facing, ToneBlocks, WireConn, chain_block, observer};
use super::blocks::{redstone_torch, redstone_wire, repeater, wire_state};
use super::compact::{CompactLayout, RawCompact};
use super::compose::{Arranged, EdgeArranged, WithFloor};
use super::{Axis, Layout, LayoutExt};
use crate::{BlockPos, BlockState, RedStoneTick, analysis::BoundedTec};
use rsnbs::note::{Instrument, Key, Tone};
use rsnbs::types::Tick;
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
    pub fn new<I: IntoIterator<Item = BoundedTec<Tone>>>(
        tecs: I,
        wrap_length: Option<NonZero<usize>>,
        gap: u32,
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
            tap_lines.push(TapLine::new(tec.scatter, repeater_coarse, switch));
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
    /// driven by the gap to the previous offset; `switch` adds the closing tap.
    pub fn new<I: IntoIterator<Item = NonZero<RedStoneTick>>>(
        ticks: I,
        repeater_coarse: Option<NonZero<RedStoneTick>>,
        switch: bool,
    ) -> Self {
        let taps = ticks.into_iter().scan(0, |prev, tick| {
            let diff = tick.get() - *prev;
            *prev = tick.get();
            Some(Tap::new(NonZero::new(diff).unwrap(), repeater_coarse))
        });
        let delays = EdgeArranged::new(taps, Axis::Southing, 0, Axis::Easting.unit(), &());
        let inner = delays.size();
        let width = inner.x.max(if switch { 5 } else { 2 });
        let size = BlockPos::new(width, Tap::ELEVATION, inner.z + 1);
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

impl<Ctx> Layout<Ctx> for TapLine {
    fn block_at_with(&self, pos: BlockPos, ctx: &Ctx) -> Option<BlockState> {
        match pos.z == 0 {
            true => self.port(pos),
            false => self.delays.get_block_with(pos - self.anchor, ctx),
        }
    }

    fn size_with(&self, _ctx: &Ctx) -> BlockPos {
        self.size
    }

    /// The port's fixed hardware, plus every delay element's palette.
    fn block_palette_with(&self, ctx: &Ctx) -> HashSet<BlockState> {
        let mut palette = self.port_palette();
        palette.extend(self.delays.block_palette_with(ctx));
        palette
    }
}

/// The switch's source button: a bass-drum note block.
fn switch_button() -> BlockState {
    let tone = Tone::new(Instrument::BassDrum, Key::from_minecraft_note(0).unwrap());
    tone.note_block_state().unwrap_or_else(chain_block)
}

// Tap
//
// ++++++++++++============++++++++++++============++++++++++++============

/// A single delay element in the tapped delay line.
struct Tap {
    delay: NonZero<RedStoneTick>,
    size: BlockPos,
}

impl Tap {
    const ELEVATION: i32 = 4;
    const SOUTHING: i32 = 2;

    fn new(delay: NonZero<RedStoneTick>, repeater_coarse: Option<NonZero<RedStoneTick>>) -> Self {
        assert!(
            repeater_coarse.is_none_or(|c| c.get() >= 4),
            "coarse < 4 is not supported, adjust the step"
        );

        let width = (delay.get() as i32 - 6) / 8 + 4;
        let size = BlockPos::new(width, Self::ELEVATION, Self::SOUTHING);
        Self { delay, size }
    }
}

impl<Ctx> Layout<Ctx> for Tap {
    fn block_at_with(&self, pos: BlockPos, _ctx: &Ctx) -> Option<BlockState> {
        use WireConn::*;
        let rev_easting = self.size.x - pos.x - 1;
        let delay = self.delay.get();
        let clamp = |tick: Tick| tick.min(4).to_string();
        let cycle = |off: Tick, modu: Tick| clamp((delay + off) % modu);
        let decay = |sub: Tick| clamp(delay.saturating_sub(sub));
        let phase = || (delay + 3) % 8;

        match (rev_easting, pos.x, pos.z, pos.y) {
            (_, _, _, 1) | (_, 0, 0, 2) | (1, _, 1, 2) => Some(chain_block()),
            (_, 0, 1, 2) => Some(redstone_torch(Some("south"), false)),
            (1, _, 0, 2) => Some(redstone_torch(Some("south"), true)),
            (3.., 2.., 0, 2) => Some(repeater("4", Facing::West, true, false)),
            (2.., 2.., 1, 2) => Some(repeater("4", Facing::East, false, false)),
            (2, 1, 1, 2) => Some(repeater(decay(6), Facing::East, false, false)),
            (3.., 1, 1, 2) => Some(repeater(cycle(3, 8), Facing::East, false, false)),
            (2, 1, 0, 2) if delay < 11 => Some(redstone_wire()),
            (2, 1, 0, 2) => Some(repeater(decay(10), Facing::East, false, false)),
            (2, 2.., 0, 2) if phase() < 4 => Some(wire_state(Side, Side, Idle, Idle, "15")),
            (2, 2.., 0, 2) => Some(repeater(cycle(3, 8), Facing::West, true, false)),
            (3.., 1, 0, 2) => Some(repeater("3", Facing::West, true, false)),
            (0, _, 1, 2) => Some(wire_state(Idle, Idle, Side, Side, "0")),
            (0, _, 0, 2) => Some(repeater(decay(3), Facing::North, false, false)),
            _ => Option::None,
        }
    }

    fn size_with(&self, _ctx: &Ctx) -> BlockPos {
        self.size
    }

    /// The blocks a delay element can draw: its chain, torches, repeaters and the
    /// wire links that carry its tapped delay.
    fn block_palette_with(&self, _ctx: &Ctx) -> HashSet<BlockState> {
        use Facing::{East, North, West};
        use WireConn::*;
        let delay = self.delay.get();
        let clamp = |tick: Tick| tick.min(4).to_string();
        let cycle = |off: Tick, modu: Tick| clamp((delay + off) % modu);
        let decay = |sub: Tick| clamp(delay.saturating_sub(sub));
        HashSet::from([
            chain_block(),
            redstone_torch(Some("south"), false),
            redstone_torch(Some("south"), true),
            repeater("3", West, true, false),
            repeater("4", West, true, false),
            repeater("4", East, false, false),
            repeater(decay(3), North, false, false),
            repeater(decay(6), East, false, false),
            repeater(decay(10), East, false, false),
            repeater(cycle(3, 8), East, false, false),
            repeater(cycle(3, 8), West, true, false),
            redstone_wire(),
            wire_state(Idle, Idle, Side, Side, "0"),
            wire_state(Side, Side, Idle, Idle, "15"),
        ])
    }
}
