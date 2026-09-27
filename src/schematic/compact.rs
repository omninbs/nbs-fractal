//! Compact note block layouts for NBS song projection.

use super::{AsLayout, Clipped, EvenlyArranged, Facing, Layout, Overlaid, WithFloor};
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

pub struct CompactLayout(Clipped<EvenlyArranged<Row>>);

impl CompactLayout {
    pub fn new<I: IntoIterator<Item = (RedStoneTick, T)>, T: Into<Tone>>(
        notes: I,
        repeater_coarse: Option<NonZero<RedStoneTick>>,
        wrap_length: Option<NonZero<usize>>,
        gap: u32,
    ) -> Self {
        let width = 4 + gap as i32;
        let coarse = repeater_coarse.map_or(Tick::MAX, NonZero::get);
        let mut events = Events::new(notes.into_iter());
        let mut south_bound = false;
        let rows = iter::from_fn(move || {
            south_bound = !south_bound;
            Row::new(&mut events, width, coarse, wrap_length, south_bound)
        });
        Self(Clipped::new(
            EvenlyArranged::new(rows, BlockPos::new(width - 2, 0, 0)),
            BlockPos::new(width - 3, 0, 0),
        ))
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
        let (turn, mut terminal) = turn(events, width, coarse, len == 1)?;

        let mut chained = false;
        let mut tiles = Vec::new();
        for column in 1..len {
            let closing = column + 1 == len;
            let Some((tile, state)) = tile(events, coarse, closing, chained, terminal, south_bound)
            else {
                break;
            };
            chained = matches!(tile, Tile::Hold { cap, .. } if cap == coarse);
            terminal = state;
            tiles.push(tile);
        }

        let depth = wrap_length.map_or(tiles.len() + 1, NonZero::get);
        let southing = 2 * depth as i32;
        let size = BlockPos::new(width, 3, southing);
        let pitch = if south_bound { 2 } else { -2 };
        let rest = EvenlyArranged::new(tiles, BlockPos::new(0, 0, pitch));
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

// Generation: Turn & Tile
//
// ++++++++++++============++++++++++++============++++++++++++============

fn turn<I: Iterator<Item = (RedStoneTick, T)>, T: Into<Tone>>(
    events: &mut Events<I>,
    width: i32,
    coarse: Tick,
    closing: bool,
) -> Option<(Turn, bool)> {
    let (wait, count) = events.pending()?;
    let terminal = !closing && (wait > 0 || count <= 2);
    let bought = match (coarse, wait) {
        (c @ 2..=4, w) if w > c => Some(c),
        (1, w) if w > 1 => Some(1),
        (_, w) if w > 4 => Some(4),
        _ => None,
    };
    let stem = bought.unwrap_or(wait);
    for _ in 0..stem {
        let event = events.next();
        debug_assert!(matches!(event, Some(Event::Wait)));
    }
    let turn = if bought.is_some() {
        if terminal {
            Turn::Sink {
                width,
                stem,
                cap: [None; 2],
            }
        } else {
            Turn::Node {
                width,
                stem,
                cap: None,
            }
        }
    } else if terminal {
        let mut cap = [None; 2];
        for slot in cap.iter_mut().take(count.min(2)) {
            *slot = Some(match events.next() {
                Some(Event::Note(note)) => note,
                _ => unreachable!(),
            });
        }
        Turn::Sink { width, stem, cap }
    } else {
        Turn::Node {
            width,
            stem,
            cap: Some(match events.next() {
                Some(Event::Note(note)) => note,
                _ => unreachable!(),
            }),
        }
    };
    let terminal = matches!(turn, Turn::Sink { .. });
    Some((turn, terminal))
}

fn tile<I: Iterator<Item = (RedStoneTick, T)>, T: Into<Tone>>(
    events: &mut Events<I>,
    coarse: Tick,
    closing: bool,
    chained: bool,
    terminal: bool,
    south_bound: bool,
) -> Option<(Tile, bool)> {
    debug_assert!(!(coarse == 1 && chained));

    let (wait, count) = events.pending()?;
    let fed = !terminal;
    let terminal = !closing && (wait > 0 || (count <= 3 && fed));
    let bought = match (coarse, chained, closing, wait) {
        (c @ 2..=4, false, _, w) if w == c * 3 => Some((c, 0)),
        (c @ 2..=4, _, false, w) if w > c * 2 => Some((c, c)),
        (c @ 2..=4, true, _, w) if w == c * 2 => Some((c, 1)),
        (c @ 2..=4, true, _, w) if w >= c => Some((c - 1, 0)),
        (c @ 2..=4, false, _, w) if w > c => Some((c, 0)),
        (1, false, _, w) if w > 1 => Some((1, 0)),
        (_, false, _, w) if w > 8 => Some((4, 4)),
        (_, false, _, w) if w > 4 => Some((4, 0)),
        _ => None,
    };
    let (stem, delay) = bought.unwrap_or((wait, 0));
    for _ in 0..stem + delay {
        let event = events.next();
        debug_assert!(matches!(event, Some(Event::Wait)));
    }
    let tile = if bought.is_some() {
        match (delay > 0, closing) {
            (true, _) => Tile::Hold {
                stem,
                cap: delay,
                south_bound,
            },
            (false, true) => Tile::Node {
                stem,
                cap: [None; 2],
                south_bound,
            },
            (false, false) => Tile::Sink {
                stem,
                cap: [None; 3],
                south_bound,
            },
        }
    } else if terminal {
        let mut cap = [None; 3];
        for slot in cap.iter_mut().take(count.min(3)) {
            *slot = Some(match events.next() {
                Some(Event::Note(note)) => note,
                _ => unreachable!(),
            });
        }
        Tile::Sink {
            stem,
            cap,
            south_bound,
        }
    } else {
        let mut cap = [None; 2];
        for slot in cap.iter_mut().take(count.min(2)) {
            *slot = Some(match events.next() {
                Some(Event::Note(note)) => note,
                _ => unreachable!(),
            });
        }
        Tile::Node {
            stem,
            cap,
            south_bound,
        }
    };
    let terminal = matches!(tile, Tile::Sink { .. });
    Some((tile, terminal))
}

// Containers: Events
//
// ++++++++++++============++++++++++++============++++++++++++============

struct Events<I: Iterator> {
    notes: Peekable<I>,
    last: RedStoneTick,
    wait: RedStoneTick,
    cache: Vec<Tone>,
}

enum Event {
    Wait,
    Note(Tone),
}

impl<I: Iterator<Item = (RedStoneTick, T)>, T: Into<Tone>> Events<I> {
    fn new(notes: I) -> Self {
        let mut this = Self {
            notes: notes.peekable(),
            last: RedStoneTick::MAX,
            wait: 0,
            cache: Vec::new(),
        };
        this.refresh();
        this
    }
}

impl<I: Iterator<Item = (RedStoneTick, T)>, T: Into<Tone>> Events<I> {
    fn pending(&self) -> Option<(RedStoneTick, usize)> {
        (!self.cache.is_empty()).then_some((self.wait, self.cache.len()))
    }

    fn refresh(&mut self) -> Option<()> {
        debug_assert!(self.cache.is_empty() && self.wait == 0);
        let (tick, notes) = Self::load(&mut self.notes)?;
        self.wait = tick.wrapping_sub(self.last);
        self.last = tick;
        self.cache.extend(notes);
        Some(())
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
        if self.wait > 0 {
            self.wait -= 1;
            return Some(Event::Wait);
        }
        let Some(note) = self.cache.pop() else {
            return None;
        };
        if self.cache.is_empty() {
            self.refresh();
        }
        Some(Event::Note(note))
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
        stem: RedStoneTick,
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
            Turn::Sink { stem, .. } | Turn::Node { stem, .. } => stem,
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
