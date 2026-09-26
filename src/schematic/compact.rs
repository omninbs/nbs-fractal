//! Compact note block layouts for NBS song projection.

use super::{AsLayout, EvenlyArranged, Facing, Layout, Overlaid, WithFloor};
use super::{air, chain_block, inst_block, note_block, redstone_wire, repeater};
use crate::{GameTick, RedStoneTick};
use mcdata::{GenericBlockState, util::BlockPos};
use rsnbs::note::Tone;
use rsnbs::types::Tick;
use std::iter::{self, Peekable};
use std::num::NonZero;

// Shell: MultiCompactLayout
//
// ++++++++++++============++++++++++++============++++++++++++============

/// One [`CompactLayout`] line per redstone sub-track, stacked vertically.
pub struct MultiCompactLayout(EvenlyArranged<WithFloor<CompactLayout>>);

impl MultiCompactLayout {
    /// Create a multi-track compact layout from multiple note groups.
    pub fn new<Trks, Trk, T>(
        tracks: Trks,
        wrap_length: Option<NonZero<usize>>,
        gap: u32,
        full: bool,
    ) -> Self
    where
        Trks: IntoIterator<Item = (Trk, Option<NonZero<RedStoneTick>>)>,
        Trk: IntoIterator<Item = (GameTick, T)>,
        T: Into<Tone>,
    {
        let lines = tracks
            .into_iter()
            .flat_map(|(notes, coarse)| split_even_odd(notes, coarse));
        let layers = lines.map(|(notes, coarse)| {
            let line = CompactLayout::new(notes, coarse, wrap_length, gap);
            WithFloor::new(line, full)
        });
        Self(EvenlyArranged::new(layers, BlockPos::new(0, 4, 0)))
    }
}

impl AsLayout for MultiCompactLayout {
    fn as_layout(&self) -> &impl Layout {
        &self.0
    }
}

/// Splits game tick notes into even and odd redstone tick lines.
fn split_even_odd<I: IntoIterator<Item = (GameTick, T)>, T: Into<Tone>>(
    notes: I,
    coarse: Option<NonZero<Tick>>,
) -> impl Iterator<Item = (Vec<(RedStoneTick, Tone)>, Option<NonZero<Tick>>)> {
    let mut lines: [Vec<(RedStoneTick, Tone)>; 2] = Default::default();
    for (game_tick, note) in notes {
        lines[(game_tick % 2) as usize].push((game_tick / 2, note.into()));
    }
    lines
        .into_iter()
        .filter(|line| !line.is_empty())
        .map(move |line| (line, coarse))
}

// Layout: CompactLayout & Row
//
// ++++++++++++============++++++++++++============++++++++++++============

pub struct CompactLayout(EvenlyArranged<Row>);

impl CompactLayout {
    pub fn new<I: IntoIterator<Item = (RedStoneTick, T)>, T: Into<Tone>>(
        notes: I,
        repeater_coarse: Option<NonZero<RedStoneTick>>,
        wrap_length: Option<NonZero<usize>>,
        gap: u32,
    ) -> Self {
        let width = 4 + gap as i32;
        let coarse = repeater_coarse.map_or(Tick::MAX, NonZero::get);
        let notes = notes.into_iter();
        let mut events = Events::new(notes);
        let mut south_bound = false;
        let rows = iter::from_fn(move || {
            south_bound = !south_bound;
            Row::new(&mut events, width, coarse, wrap_length, south_bound)
        });
        Self(EvenlyArranged::new(rows, BlockPos::new(width - 2, 0, 0)))
    }
}

impl AsLayout for CompactLayout {
    fn as_layout(&self) -> &impl Layout {
        &self.0
    }
}

struct Row(Overlaid<Turn, EvenlyArranged<Tile>>);

impl Row {
    fn new<I: Iterator<Item = (RedStoneTick, T)>, T: Into<Tone>>(
        events: &mut Events<I>,
        width: i32,
        coarse: Tick,
        wrap_length: Option<NonZero<usize>>,
        south_bound: bool,
    ) -> Option<Self> {
        let len = wrap_length.map_or(usize::MAX, NonZero::get);

        let turn: Turn = todo!();

        let mut column = 1;
        let mut tiles = iter::from_fn(|| {
            if column == len {
                return None;
            }
            let closing = column + 1 == len;
            column += 1;
            let tile: Tile = todo!();
            Some(tile)
        });

        let pitch = if south_bound { 2 } else { -2 };
        let rest = EvenlyArranged::new(tiles, BlockPos::new(0, 0, pitch));
        let depth = wrap_length.map_or(column, NonZero::get);
        let southing = 2 * depth as i32;
        let size = BlockPos::new(width, 3, southing);
        let turn_at = BlockPos::new(0, 0, if south_bound { 0 } else { -1 });
        let tiles_at = BlockPos::new(-1, 0, if south_bound { 1 } else { -2 });
        Some(Self(Overlaid::new(turn_at, turn, tiles_at, rest, size)))
    }
}

impl AsLayout for Row {
    fn as_layout(&self) -> &impl Layout {
        &self.0
    }
}

// Containers: Events
//
// ++++++++++++============++++++++++++============++++++++++++============

enum Event {
    Wait,
    Note(Tone),
}

struct Events<I: Iterator> {
    notes: Peekable<I>,
    last: RedStoneTick,
    wait: RedStoneTick,
    cache: Vec<Tone>,
}

impl<I: Iterator> Events<I> {
    fn new(notes: I) -> Self {
        Self {
            notes: notes.peekable(),
            last: RedStoneTick::MAX,
            wait: 0,
            cache: Vec::new(),
        }
    }
}

impl<I: Iterator<Item = (RedStoneTick, T)>, T: Into<Tone>> Events<I> {
    fn current(&self) -> Option<(RedStoneTick, &[Tone])> {
        (!self.cache.is_empty()).then(|| (self.wait, self.cache.as_slice()))
    }

    fn load(notes: &mut Peekable<I>) -> Option<(RedStoneTick, impl Iterator<Item = Tone> + '_)> {
        let (tick, note) = notes.next()?;
        let rest = iter::from_fn(move || notes.next_if(|&(t, _)| t == tick).map(|(_, n)| n.into()));
        Some((tick, iter::once(note.into()).chain(rest)))
    }
}

impl<I: Iterator<Item = (RedStoneTick, T)>, T: Into<Tone>> Iterator for Events<I> {
    type Item = Event;

    fn next(&mut self) -> Option<Event> {
        if self.cache.is_empty() {
            debug_assert_eq!(self.wait, 0);
            let (tick, notes) = Self::load(&mut self.notes)?;
            self.wait = tick.wrapping_sub(self.last);
            self.last = tick;
            self.cache.extend(notes);
        }
        if self.wait == 0 {
            self.cache.pop().map(Event::Note)
        } else {
            self.wait -= 1;
            Some(Event::Wait)
        }
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
            | Tile::Node { south_bound, .. } => south_bound,
        }
    }

    fn stem(&self) -> RedStoneTick {
        match *self {
            Tile::Hold { stem, .. } | Tile::Sink { stem, .. } | Tile::Node { stem, .. } => stem,
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
        debug_assert!(!matches!(self, Hold { stem: 0, .. } | Hold { cap: 0, .. }));

        let stem = || match (pos.x, pos.y) {
            (1, 0) => Some(chain_block()),
            (1, 1) if self.stem() == 0 => Some(redstone_wire()),
            (1, 1) => Some(repeater(self.stem())),
            _ => None,
        };
        let hold = |cap: RedStoneTick| match (pos.x, pos.y) {
            (1, 0) => Some(chain_block()),
            (1, 1) => Some(repeater(cap)),
            _ => None,
        };
        let node = |cap: [Option<Tone>; 2]| match (pos.x, pos.y) {
            (1, 0 | 1) => Some(chain_block()),
            (1, 2) => Some(redstone_wire()),
            (0, 0) => Some(inst_block(cap[0], air)),
            (0, 1) => Some(note_block(cap[0], air)),
            (2, 0) => Some(inst_block(cap[1], air)),
            (2, 1) => Some(note_block(cap[1], air)),
            _ => None,
        };
        let sink = |cap: [Option<Tone>; 3]| match (pos.x, pos.y) {
            (1, 0) => Some(inst_block(cap[0], chain_block)),
            (1, 1) => Some(note_block(cap[0], chain_block)),
            (0, 0) => Some(inst_block(cap[1], air)),
            (0, 1) => Some(note_block(cap[1], air)),
            (2, 0) => Some(inst_block(cap[2], air)),
            (2, 1) => Some(note_block(cap[2], air)),
            _ => None,
        };
        match (local_z, *self) {
            (0, _) => stem(),
            (1, Hold { cap, .. }) => hold(cap),
            (1, Node { cap, .. }) => node(cap),
            (1, Sink { cap, .. }) => sink(cap),
            _ => unreachable!(),
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
        let repeater = |delay: RedStoneTick| repeater(delay.to_string(), East, false, false);

        let stem = || match (local_x, pos.y) {
            (2.., 0) => Some(chain_block()),
            (2, 1) if self.stem() > 0 => Some(repeater(self.stem())),
            (2.., 1) => Some(redstone_wire()),
            _ => None,
        };
        let sink = |cap: [Option<Tone>; 2]| match (local_x, pos.y) {
            (1, 0) => Some(inst_block(cap[0], chain_block)),
            (1, 1) => Some(note_block(cap[0], chain_block)),
            (0, 0) => Some(inst_block(cap[1], air)),
            (0, 1) => Some(note_block(cap[1], air)),
            _ => None,
        };
        let node = |cap: Option<Tone>| match (local_x, pos.y) {
            (1, 0 | 1) => Some(chain_block()),
            (1, 2) => Some(redstone_wire()),
            (0, 0) => Some(inst_block(cap, air)),
            (0, 1) => Some(note_block(cap, air)),
            _ => None,
        };
        match (local_x, *self) {
            (2.., _) => stem(),
            (0 | 1, Sink { cap, .. }) => sink(cap),
            (0 | 1, Node { cap, .. }) => node(cap),
            _ => unreachable!(),
        }
    }

    fn size(&self) -> BlockPos {
        BlockPos::new(self.width(), 3, 1)
    }
}
