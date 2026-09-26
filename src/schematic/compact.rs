//! Compact note block layouts for NBS song projection.

use super::{AsLayout, EvenlyArranged, Facing, Layout, WithFloor};
use super::{air, chain_block, inst_block, note_block, redstone_wire, repeater};
use crate::{GameTick, RedStoneTick};
use mcdata::{GenericBlockState, util::BlockPos};
use rsnbs::note::{Notes, Tone};
use rsnbs::types::Tick;
use std::num::NonZero;

// Shell: MultiCompactLayout
//
// ++++++++++++============++++++++++++============++++++++++++============

/// One [`CompactLayout`] line per redstone sub-track, stacked vertically.
pub struct MultiCompactLayout(EvenlyArranged<WithFloor<CompactLayout>>);

impl MultiCompactLayout {
    /// Create a multi-track compact layout from multiple note groups.
    pub fn new<Trks, Trk, Chord>(
        tracks: Trks,
        wrap_length: Option<NonZero<usize>>,
        gap: u32,
        full: bool,
    ) -> Self
    where
        Trks: IntoIterator<Item = (Trk, Option<NonZero<RedStoneTick>>)>,
        Trk: IntoIterator<Item = (GameTick, Chord)>,
        Chord: IntoIterator,
        Chord::Item: Into<Tone>,
    {
        let lines = tracks
            .into_iter()
            .flat_map(|(notes, coarse)| split_even_odd(notes, coarse))
            .filter(|(notes, _)| !notes.is_empty())
            .map(|(notes, coarse)| {
                let line = CompactLayout::new(notes, coarse, wrap_length, gap);
                WithFloor::new(line, full)
            });
        // A line is four tall, plus one block of headroom.
        Self(EvenlyArranged::new(lines, BlockPos::new(0, 4, 0)))
    }
}

impl AsLayout for MultiCompactLayout {
    fn as_layout(&self) -> &impl Layout {
        &self.0
    }
}

/// Splits game tick notes into even and odd redstone tick lines.
fn split_even_odd<Trks, Chord>(
    tracks: Trks,
    coarse: Option<NonZero<Tick>>,
) -> impl Iterator<Item = (Notes<RedStoneTick, Vec<Tone>>, Option<NonZero<Tick>>)>
where
    Trks: IntoIterator<Item = (GameTick, Chord)>,
    Chord: IntoIterator,
    Chord::Item: Into<Tone>,
{
    let mut lines: [Notes<RedStoneTick, Vec<Tone>>; 2] = Default::default();
    for (game_tick, chord) in tracks {
        lines[(game_tick % 2) as usize]
            .entry(game_tick / 2)
            .or_default()
            .extend(chord.into_iter().map(Into::into));
    }
    lines.into_iter().map(move |line| (line, coarse))
}

// CompactLayout
//
// ++++++++++++============++++++++++++============++++++++++++============

/// A single compact note block track: one redstone line folded into a serpentine
/// stack of rows.
pub struct CompactLayout(());

impl CompactLayout {
    /// Create a compact layout from redstone-tick-grouped notes.
    pub fn new<Trk, Chord>(
        _notes: Trk,
        _repeater_coarse: Option<NonZero<RedStoneTick>>,
        _wrap_length: Option<NonZero<usize>>,
        _gap: u32,
    ) -> Self
    where
        Trk: IntoIterator<Item = (RedStoneTick, Chord)>,
        Chord: IntoIterator,
        Chord::Item: Into<Tone>,
    {
        todo!()
    }
}

impl AsLayout for CompactLayout {
    fn as_layout(&self) -> &impl Layout {
        &self.0
    }
}

// Templates: Tile & Turn
//
// ++++++++++++============++++++++++++============++++++++++++============

#[derive(Clone, Copy)]
enum Tile {
    Hold {
        stem: RedStoneTick,
        cap: RedStoneTick,
        south_bound: bool,
    },
    Sink {
        stem: RedStoneTick,
        cap: [Option<Tone>; 3],
        south_bound: bool,
    },
    Node {
        cap: [Option<Tone>; 2],
        south_bound: bool,
    },
    Port {
        stem: RedStoneTick,
        cap: [Option<Tone>; 2],
        south_bound: bool,
    },
}

impl Tile {
    fn south_bound(&self) -> bool {
        match *self {
            Tile::Hold { south_bound, .. }
            | Tile::Sink { south_bound, .. }
            | Tile::Node { south_bound, .. }
            | Tile::Port { south_bound, .. } => south_bound,
        }
    }

    fn stem(&self) -> RedStoneTick {
        match *self {
            Tile::Hold { stem, .. } | Tile::Sink { stem, .. } | Tile::Port { stem, .. } => stem,
            Tile::Node { .. } => 0,
        }
    }
}

impl Layout for Tile {
    fn block_at(&self, pos: BlockPos) -> Option<GenericBlockState> {
        use self::{Facing::*, Tile::*};
        let south_bound = self.south_bound();
        let local_z = if south_bound { pos.z } else { 1 - pos.z };
        let facing = if south_bound { South } else { North };
        let repeater = |delay: RedStoneTick| repeater(delay.to_string(), facing, false, false);
        match (*self, local_z, pos.x, pos.y) {
            // stem
            (_, 0, 1, 0) => Some(chain_block()),
            (_, 0, 1, 1) if self.stem() == 0 => Some(redstone_wire()),
            (_, 0, 1, 1) => Some(repeater(self.stem())),

            // cap: hold
            (Hold { .. }, 1, 1, 0) => Some(chain_block()),
            (Hold { cap, .. }, 1, 1, 1) => Some(repeater(cap)),

            // cap: node
            (Node { .. } | Port { .. }, 1, 1, 0 | 1) => Some(chain_block()),
            (Node { .. } | Port { .. }, 1, 1, 2) => Some(redstone_wire()),
            (Node { cap, .. } | Port { cap, .. }, 1, 0, 0) => Some(inst_block(cap[0], air)),
            (Node { cap, .. } | Port { cap, .. }, 1, 0, 1) => Some(note_block(cap[0], air)),
            (Node { cap, .. } | Port { cap, .. }, 1, 2, 0) => Some(inst_block(cap[1], air)),
            (Node { cap, .. } | Port { cap, .. }, 1, 2, 1) => Some(note_block(cap[1], air)),

            // cap: sink
            (Sink { cap, .. }, 1, 1, 0) => Some(inst_block(cap[0], chain_block)),
            (Sink { cap, .. }, 1, 1, 1) => Some(note_block(cap[0], chain_block)),
            (Sink { cap, .. }, 1, 0, 0) => Some(inst_block(cap[1], air)),
            (Sink { cap, .. }, 1, 0, 1) => Some(note_block(cap[1], air)),
            (Sink { cap, .. }, 1, 2, 0) => Some(inst_block(cap[2], air)),
            (Sink { cap, .. }, 1, 2, 1) => Some(note_block(cap[2], air)),

            _ => None,
        }
    }

    fn size(&self) -> BlockPos {
        BlockPos::new(3, 3, 2)
    }
}

#[derive(Clone, Copy)]
enum Turn {
    Sink {
        width: i32,
        stem: RedStoneTick,
        cap: [Option<Tone>; 2],
    },
    Node {
        width: i32,
        cap: Option<Tone>,
    },
}

impl Turn {
    fn width(&self) -> i32 {
        match *self {
            Turn::Sink { width, .. } | Turn::Node { width, .. } => width,
        }
    }

    fn stem(&self) -> RedStoneTick {
        match *self {
            Turn::Sink { stem, .. } => stem,
            Turn::Node { .. } => 0,
        }
    }
}

impl Layout for Turn {
    fn block_at(&self, pos: BlockPos) -> Option<GenericBlockState> {
        use self::{Facing::*, Turn::*};
        let local_x = self.width() - 1 - pos.x;
        let repeater = || repeater(self.stem().to_string(), East, false, false);
        match (*self, local_x, pos.y) {
            // stem
            (_, 2, 1) if self.stem() > 0 => Some(repeater()),
            (_, 2.., 0) => Some(chain_block()),
            (_, 2.., 1) => Some(redstone_wire()),

            // cap: sink
            (Sink { cap, .. }, 1, 0) => Some(inst_block(cap[0], chain_block)),
            (Sink { cap, .. }, 1, 1) => Some(note_block(cap[0], chain_block)),
            (Sink { cap, .. }, 0, 0) => Some(inst_block(cap[1], air)),
            (Sink { cap, .. }, 0, 1) => Some(note_block(cap[1], air)),

            // cap: node
            (Node { .. }, 1, 0 | 1) => Some(chain_block()),
            (Node { .. }, 1, 2) => Some(redstone_wire()),
            (Node { cap, .. }, 0, 0) => Some(inst_block(cap, air)),
            (Node { cap, .. }, 0, 1) => Some(note_block(cap, air)),

            _ => None,
        }
    }

    fn size(&self) -> BlockPos {
        BlockPos::new(self.width(), 3, 1)
    }
}
