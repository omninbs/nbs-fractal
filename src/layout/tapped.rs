//! Tapped delay line layout for NBS song projection.

use super::blocks::{Facing, ToneBlocks, WireConn, chain_block, observer};
use super::blocks::{redstone_torch, repeater, wire_state};
use super::compact::{CompactLayout, RawCompact};
use super::compose::{Arranged, EdgeArranged, WithFloor};
use super::tap::Tap;
use super::{Axis, Layout, LayoutExt};
use super::{signed, size_of};
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
