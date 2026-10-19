//! Note bamboo: the joint template of a compact line.
//!
//! A compact line is built from [`Joint`]s, each a bamboo joint whose stem carries
//! the repeaters and whose cap carries the note blocks. Joints stack end to end
//! into one long culm, and each is read within the row's south-bound flag, so it
//! learns the facing from its parent instead of storing it. [`Joint::next`] draws
//! them one at a time from a shared [`Events`] stream.

use super::blocks::{Facing, WireConn, chain_block, inst_block, note_block, repeater, wire_state};
use super::{Layout, air};
use crate::{BlockPos, BlockState, RedStoneTick};
use rsnbs::note::Tone;
use std::collections::HashSet;
use std::iter::{self, Peekable};

// Events
// ++++++++++++============++++++++++++============++++++++++++============

/// Notes and waits expanded from grouped input: the waits between groups and
/// the notes of each group.
///
/// Iterating yields each gap tick as `None` and each cached note as `Some` in
/// turn, so a consumer can spend waits and take notes without looking ahead.
pub struct Events<I: Iterator> {
    /// The groups still to read, as `(tick, note)` pairs in the order given.
    notes: Peekable<I>,
    /// Ticks left to wait before the next cached note is due.
    wait: RedStoneTick,
    /// Notes of the current group, yielded one per `Some` item.
    cache: Vec<Tone>,
    /// Tick of the group last read, from which the next wait is measured.
    last: RedStoneTick,
}

impl<I: Iterator<Item = (RedStoneTick, T)>, T: Into<Tone>> Events<I> {
    /// Reads the groups from `notes`, measuring the first wait from `last`.
    pub fn new(notes: Peekable<I>, last: RedStoneTick) -> Self {
        let mut this = Self {
            notes,
            last,
            wait: 0,
            cache: Vec::new(),
        };
        this.refresh();
        this
    }

    /// The wait before the cached group and how many notes it holds, if any
    /// are cached.
    pub fn pending(&self) -> Option<(RedStoneTick, usize)> {
        (!self.cache.is_empty()).then_some((self.wait, self.cache.len()))
    }

    /// Whether any group is still to read.
    pub fn has_next_group(&mut self) -> bool {
        self.notes.peek().is_some()
    }

    /// Spends `ticks` waits, advancing the stream past that many empty events.
    pub fn take_waits(&mut self, ticks: RedStoneTick) {
        for _ in 0..ticks {
            let event = self.next();
            debug_assert!(event == Some(None));
        }
    }

    /// Takes the next cached note, which must be due.
    pub fn take_note(&mut self) -> Tone {
        match self.next() {
            Some(Some(note)) => note,
            _ => unreachable!(),
        }
    }

    /// Takes up to `N` notes, leaving the rest in the stream.
    pub fn take_notes<const N: usize>(&mut self, count: usize) -> [Option<Tone>; N] {
        let mut cap = [None; N];
        for slot in cap.iter_mut().take(count.min(N)) {
            *slot = Some(self.take_note());
        }
        cap
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
    type Item = Option<Tone>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.wait > 0 {
            self.wait -= 1;
            return Some(None);
        }
        let Some(note) = self.cache.pop() else {
            return None;
        };
        if self.cache.is_empty() {
            self.refresh();
        }
        Some(Some(note))
    }
}

// Joint: the smallest unit of a row
// ++++++++++++============++++++++++++============++++++++++++============

/// One joint of a compact row: a stem carrying `stem` tick repeaters or bare
/// wire, and a cap of up to three notes.
#[derive(Clone, Copy)]
pub enum Joint {
    /// A stem topped by a repeater instead of notes, used to delay in place.
    Hold {
        /// Ticks in the stem.
        stem: RedStoneTick,
        /// Ticks the cap repeater adds.
        cap: RedStoneTick,
    },
    /// Bare wire sinking three notes into this row's chain, with no delay.
    Sink {
        /// Whether the previous cap raises this wire.
        raised: bool,
        /// Notes played on the cap.
        cap: [Option<Tone>; 3],
    },
    /// Bare wire riding upward and two notes on the cap, with no delay.
    Node {
        /// Whether the previous cap raises this wire.
        raised: bool,
        /// Notes played on the cap.
        cap: [Option<Tone>; 2],
    },
    /// A delayed stem sinking three notes into this row's chain.
    DelayedSink {
        /// Ticks in the stem.
        stem: RedStoneTick,
        /// Notes played on the cap.
        cap: [Option<Tone>; 3],
    },
    /// A delayed stem riding upward and two notes on the cap.
    DelayedNode {
        /// Ticks in the stem.
        stem: RedStoneTick,
        /// Notes played on the cap.
        cap: [Option<Tone>; 2],
    },
}

impl Joint {
    // Constructors...

    /// Draws the next joint from `events`, carrying `state` across, or `None`
    /// when the stream has no notes left.
    ///
    /// `coarse` bounds how many stem ticks one repeater chain may hold, and
    /// `closing` marks the last joint of a culm, which may settle on a sink
    /// rather than feed on.
    pub fn next<I: Iterator<Item = (RedStoneTick, T)>, T: Into<Tone>>(
        events: &mut Events<I>,
        state: &mut State,
        coarse: RedStoneTick,
        closing: bool,
    ) -> Option<Self> {
        debug_assert!(!(coarse == 1 && state.chained));

        let (wait, count) = events.pending()?;
        let bought = match (coarse, state.chained, closing) {
            (c @ 2..=4, false, _) if wait == c * 3 => Some((c, 0)),
            (c @ 2..=4, _, false) if wait > c * 2 => Some((c, c)),
            (c @ 2..=4, true, _) if wait == c * 2 => Some((c, 1)),
            (c @ 2..=4, true, _) if wait >= c => Some((c - 1, 0)),
            (c @ 1..=4, false, _) if wait > c => Some((c, 0)),
            (_, false, _) if wait > 8 => Some((4, 4)),
            (_, false, _) if wait > 4 => Some((4, 0)),
            _ => None,
        };

        let feeds = closing && (bought.is_some() || count > 3 || events.has_next_group());
        let terminal = !feeds && (wait > 0 || (count <= 3 && !state.terminal));
        let (stem, cap) = bought.unwrap_or((wait, 0));
        events.take_waits(stem + cap);
        let placed = bought.map_or(count, |_| 0);
        let joint = match (cap > 0, terminal) {
            (true, _) => Self::hold(stem, cap),
            (false, true) => Self::sink(stem, events.take_notes(placed), state.raised),
            (false, false) => Self::node(stem, events.take_notes(placed), state.raised),
        };

        state.terminal = matches!(joint, Self::Sink { .. } | Self::DelayedSink { .. });
        state.chained = matches!(joint, Self::Hold { cap, .. } if cap == coarse);
        state.raised = matches!(joint, Self::Node { .. } | Self::DelayedNode { .. });
        Some(joint)
    }

    /// A joint holding its cap repeater for `cap` extra ticks.
    pub fn hold(stem: RedStoneTick, cap: RedStoneTick) -> Self {
        Self::Hold { stem, cap }
    }

    /// A joint sinking three notes into the row's chain.
    ///
    /// A delayed stem is all repeaters, so `raised` only bears on a bare wire.
    pub fn sink(stem: RedStoneTick, cap: [Option<Tone>; 3], raised: bool) -> Self {
        match stem {
            0 => Self::Sink { raised, cap },
            stem => Self::DelayedSink { stem, cap },
        }
    }

    /// A joint riding the wire into the next row.
    ///
    /// A delayed stem is all repeaters, so `raised` only bears on a bare wire.
    pub fn node(stem: RedStoneTick, cap: [Option<Tone>; 2], raised: bool) -> Self {
        match stem {
            0 => Self::Node { raised, cap },
            stem => Self::DelayedNode { stem, cap },
        }
    }

    // Query methods...

    /// Ticks the stem delays.
    pub fn stem(&self) -> RedStoneTick {
        match *self {
            Joint::Sink { .. } | Joint::Node { .. } => 0,
            Joint::Hold { stem, .. }
            | Joint::DelayedSink { stem, .. }
            | Joint::DelayedNode { stem, .. } => stem,
        }
    }

    /// Whether the previous cap raises the bare wire.
    pub fn raised(&self) -> bool {
        match *self {
            Joint::Sink { raised, .. } | Joint::Node { raised, .. } => raised,
            _ => unreachable!(),
        }
    }

    /// The bare wire a node's cap climbs onto, one block above its stem.
    fn lifted_wire() -> BlockState {
        use WireConn::{Idle, Side};
        wire_state(Idle, Idle, Side, Side, "0")
    }

    /// The bare wire a joint rides: a node's cap climbs onto its own wire, while
    /// a sink's cap meets it flat.
    fn wire(&self, south_bound: bool) -> BlockState {
        use WireConn::*;
        let forward = match *self {
            Self::Node { .. } => Up,
            Self::Sink { .. } => Side,
            _ => unreachable!(),
        };
        let backward = if self.raised() { Up } else { Side };
        let (north, south) = match south_bound {
            true => (backward, forward),
            false => (forward, backward),
        };
        wire_state(Idle, Idle, north, south, "0")
    }
}

impl Layout<bool> for Joint {
    fn block_at_with(&self, pos: BlockPos, &south_bound: &bool) -> Option<BlockState> {
        use {Facing::*, Joint::*};
        let local_z = if south_bound { pos.z } else { 1 - pos.z };
        let facing = || if south_bound { South } else { North };
        let repeater = |delay: RedStoneTick| repeater(delay.to_string(), facing(), false, false);
        debug_assert!(!matches!(self, Hold { stem: 0, .. } | Hold { cap: 0, .. }));

        let stem = || match (pos.x, pos.y) {
            (1, 0) => Some(chain_block()),
            (1, 1) => Some(match self.stem() {
                0 => self.wire(south_bound),
                stem => repeater(stem),
            }),
            _ => None,
        };
        let hold = |cap: RedStoneTick| match (pos.x, pos.y) {
            (1, 0) => Some(chain_block()),
            (1, 1) => Some(repeater(cap)),
            _ => None,
        };
        let node = |cap: [Option<Tone>; 2]| match (pos.x, pos.y) {
            (1, 0 | 1) => Some(chain_block()),
            (1, 2) => Some(Self::lifted_wire()),
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
            (1, Node { cap, .. } | DelayedNode { cap, .. }) => node(cap),
            (1, Sink { cap, .. } | DelayedSink { cap, .. }) => sink(cap),
            _ => unreachable!(),
        }
    }

    fn size_with(&self, _ctx: &bool) -> BlockPos {
        BlockPos::new(3, 3, 2)
    }

    /// The blocks a joint can draw: its chain stem, the repeater or bare wire its
    /// stem rides, the wire a node's cap climbs onto, and one note and instrument
    /// block per stored tone.
    fn block_palette_with(&self, &south_bound: &bool) -> HashSet<BlockState> {
        use Facing::{North, South};
        let facing = if south_bound { South } else { North };
        let repeater = |delay: RedStoneTick| repeater(delay.to_string(), facing, false, false);
        let mut palette = HashSet::from([chain_block()]);
        match self.stem() {
            0 => palette.insert(self.wire(south_bound)),
            stem => palette.insert(repeater(stem)),
        };
        if let Joint::Hold { cap, .. } = *self {
            palette.insert(repeater(cap));
        }
        if matches!(self, Joint::Node { .. } | Joint::DelayedNode { .. }) {
            palette.insert(Self::lifted_wire());
        }
        let caps: &[Option<Tone>] = match self {
            Joint::Hold { .. } => &[],
            Joint::Node { cap, .. } | Joint::DelayedNode { cap, .. } => cap,
            Joint::Sink { cap, .. } | Joint::DelayedSink { cap, .. } => cap,
        };
        for tone in caps.iter().flatten() {
            palette.insert(note_block(Some(tone), air));
            palette.insert(inst_block(Some(tone), air));
        }
        palette
    }
}

/// The input conditions a joint must be given before it can be built.
#[derive(Clone, Copy, Default)]
pub struct State {
    /// Whether the input needs lifting onto the joint's wire.
    pub raised: bool,
    /// Whether the input arrives as a block charge.
    pub terminal: bool,
    /// Whether the input repeater's scheduled-tick priority is too low.
    pub chained: bool,
}
