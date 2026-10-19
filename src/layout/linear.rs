//! Linear time-proportional layout for NBS song projection.

use super::blocks::{Facing, WireConn, chain_block, inst_block, note_block, repeater};
use super::blocks::{redstone_block, sticky_piston, wire_state};
use super::compose::{Clipped, EvenlyArranged, Overlaid, Provided, WithFloor};
use super::{Layout, air};
use crate::{BlockPos, BlockState, forward_layout};
use rsnbs::note::Tone;
use rsnbs::types::{Tick, TimeAnchor};
use std::{collections::HashSet, iter::from_fn, num::NonZero, vec::IntoIter as VecIter};

// Shell: MultiLinearLayout & StackedLinearLayout
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Multi-line linear noteblocks layout.
pub struct MultiLinearLayout(EvenlyArranged<LinearLayout>);

impl MultiLinearLayout {
    /// Create a linear layout from per-track notes.
    pub fn new<Trks, Trk, A, T>(tracks: Trks, gap: u32, song_length: Tick) -> Self
    where
        Trks: IntoIterator<Item = Trk>,
        Trk: IntoIterator<Item = (A, T)>,
        A: TimeAnchor,
        T: Into<Tone>,
        for<'a> &'a Trks: IntoIterator<Item = &'a Trk>,
        for<'a> &'a Trk: IntoIterator<Item = (&'a A, &'a T)>,
    {
        let scale = ScaleMode::from_tracks(&tracks);
        let layouts = tracks.into_iter().flat_map(|notes| {
            let cells = Cells::new(notes, scale, song_length);
            LinearLayout::from_cells(cells, scale, None, 0)
        });
        let pitch = BlockPos::new(scale.width() + gap as i32, 0, 0);
        Self(EvenlyArranged::new(layouts, pitch, &()))
    }
}

forward_layout!(MultiLinearLayout, ());

/// Multi-track linear layout stacked vertically, each with a floor platform below.
pub struct StackedLinearLayout(EvenlyArranged<WithFloor<LinearLayout>>);

impl StackedLinearLayout {
    /// Create a stacked linear layout from per-track notes.
    pub fn new<Trks, Trk, A, T>(
        tracks: Trks,
        wrap_length: Option<NonZero<usize>>,
        gap: u32,
        full: bool,
        song_length: Tick,
    ) -> Self
    where
        Trks: IntoIterator<Item = Trk>,
        Trk: IntoIterator<Item = (A, T)>,
        A: TimeAnchor,
        T: Into<Tone>,
        for<'a> &'a Trks: IntoIterator<Item = &'a Trk>,
        for<'a> &'a Trk: IntoIterator<Item = (&'a A, &'a T)>,
    {
        let scale = ScaleMode::from_tracks(&tracks);
        let layouts = tracks.into_iter().flat_map(|notes| {
            let cells = Cells::new(notes, scale, song_length);
            let layer = LinearLayout::from_cells(cells, scale, wrap_length, gap);
            layer.map(|layout| WithFloor::new(layout, full))
        });
        let pitch = BlockPos::new(0, 4, 0);
        Self(EvenlyArranged::new(layouts, pitch, &()))
    }
}

forward_layout!(StackedLinearLayout, ());

// Layout: LinearLayout & Row
//
// The structural layout layer is the hierarchy that generates and organizes
// layouts; it produces no blocks itself and only performs layout.
//
// ++++++++++++============++++++++++++============++++++++++++============

/// A foldable timeline with side branches for one track: each backbone carries
/// one unit of branch, played evenly along time.
pub struct LinearLayout(Clipped<EvenlyArranged<Row>>);

impl LinearLayout {
    /// Builds the foldable timeline for one track from its notes.
    pub fn new<Trk, A, T>(
        notes: Trk,
        wrap_length: Option<NonZero<usize>>,
        gap: u32,
        song_length: Tick,
    ) -> impl Iterator<Item = Self>
    where
        Trk: IntoIterator<Item = (A, T)>,
        A: TimeAnchor,
        T: Into<Tone>,
        for<'a> &'a Trk: IntoIterator<Item = (&'a A, &'a T)>,
    {
        let ticks = IntoIterator::into_iter(&notes).map(|(anchor, _)| (*anchor).into_tick());
        let scale = ScaleMode::new(ticks);
        let cells = Cells::new(notes, scale, song_length);
        Self::from_cells(cells, scale, wrap_length, gap)
    }

    /// Lays a pre-filed cell container out into lanes.
    pub(crate) fn from_cells(
        mut cells: Cells,
        scale: ScaleMode,
        wrap_length: Option<NonZero<usize>>,
        gap: u32,
    ) -> impl Iterator<Item = Self> {
        let width = scale.width() + gap as i32 + 1;
        let row_length = wrap_length.map_or(cells.len(), NonZero::get);
        let pitch = BlockPos::new(width - 2, 0, 0);
        let clip = BlockPos::new(width - scale.width(), 0, 0);
        from_fn(move || {
            if !cells.has_notes() {
                return None;
            }
            let rows = (0..cells.len()).step_by(row_length).map(|start| {
                let index = start / row_length;
                let region = cells.window(start, row_length);
                Row::new(region, scale, width, index > 0, index % 2 == 0, row_length)
            });
            Some(Self(Clipped::new(
                EvenlyArranged::new(rows, pitch, &()),
                clip,
                &(),
            )))
        })
    }
}

forward_layout!(LinearLayout, ());

/// One directional row of template cells, read within the [`Frame`] it holds.
struct Row(Provided<Frame, Overlaid<Turn, EvenlyArranged<Template>>>);

impl Row {
    /// Arranges its region of cells in the row direction, draining them.
    pub(crate) fn new(
        cells: &mut [Cell],
        scale: ScaleMode,
        width: i32,
        leading_turn: bool,
        south_bound: bool,
        row_length: usize,
    ) -> Self {
        let templates: Vec<Template> = cells
            .iter_mut()
            .map(|cell| Template::new(cell, scale))
            .collect();
        // every row spans the full `row_length`
        let depth = 2 * row_length as i32 + 2;
        let pitch = BlockPos::new(0, 0, if south_bound { 2 } else { -2 });
        let frame = Frame {
            scale,
            width,
            south_bound,
        };
        let cells = EvenlyArranged::new(templates, pitch, &frame);
        let turn = Turn::new(leading_turn);
        let size = BlockPos::new(width, cells.size_with(&frame).y, depth);
        let turn_anchor = BlockPos::new(0, 0, if south_bound { 0 } else { -1 });
        let cells_anchor = BlockPos::new(-1, 0, if south_bound { 1 } else { -2 });
        let inner = Overlaid::new(turn_anchor, turn, cells_anchor, cells, size, &frame);
        Self(Provided::new(frame, inner))
    }
}

forward_layout!(Row, ());

// Containers: Cells & ScaleMode
//
// Data containers are a parallel layer to the structural layout, declaring a
// data container suitable for consumption by the structural layout.
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Cell-domain note container: the time stream is filed here once, and
/// every lane consumes cells by capacity afterwards.
pub(crate) struct Cells {
    slots: Vec<Cell>,
}

impl Cells {
    /// Files notes into their cells, keeping every cell up to `song_length`
    /// alive even when silent.
    fn new<Trk, A, T>(notes: Trk, scale: ScaleMode, song_length: Tick) -> Self
    where
        Trk: IntoIterator<Item = (A, T)>,
        A: TimeAnchor,
        T: Into<Tone>,
    {
        let min = song_length
            .checked_sub(1)
            .map_or(0, |tick| scale.cell_slot(tick).0 + 1);
        let mut slots: Vec<(Vec<Tone>, Vec<Tone>)> = vec![Default::default(); min];
        for (anchor, note) in notes {
            let (cell, is_branch) = scale.cell_slot(anchor.into_tick());
            slots.resize_with(slots.len().max(cell + 1), Default::default);
            let (main, branch) = &mut slots[cell];
            match is_branch {
                true => branch.push(note.into()),
                false => main.push(note.into()),
            }
        }
        let slots = slots.into_iter().map(|(main, branch)| Cell {
            main: main.into_iter(),
            branch: branch.into_iter(),
        });
        let slots = slots.collect();
        Self { slots }
    }

    /// Number of cells, including trailing silent ones.
    fn len(&self) -> usize {
        self.slots.len()
    }

    /// The `len` cells starting at `start`, clamped to what exists.
    fn window(&mut self, start: usize, len: usize) -> &mut [Cell] {
        let end = start.saturating_add(len).min(self.slots.len());
        &mut self.slots[start..end]
    }

    /// Whether the container still holds unconsumed notes.
    fn has_notes(&self) -> bool {
        self.slots
            .iter()
            .any(|cell| cell.main.len() + cell.branch.len() > 0)
    }
}

/// One cell's note queues, main and branch.
pub(crate) struct Cell {
    main: VecIter<Tone>,
    branch: VecIter<Tone>,
}

/// Time scale used by linear cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaleMode {
    /// Four-game-tick scale.
    Scale4,
    /// Two-game-tick scale.
    Scale2,
    /// Three-game-tick scale.
    Scale3,
    /// One-game-tick scale.
    Scale1,
}

impl ScaleMode {
    const SCALE_MODES: [Tick; 3] = [4, 3, 2];

    /// Selects the coarsest scale compatible with every event timestamp.
    pub fn from_tracks<'a, Trks, Trk: 'a, A: 'a, T: 'a>(tracks: &'a Trks) -> Self
    where
        &'a Trks: IntoIterator<Item = &'a Trk>,
        &'a Trk: IntoIterator<Item = (&'a A, &'a T)>,
        A: TimeAnchor,
    {
        let ticks = tracks
            .into_iter()
            .flat_map(|track| track.into_iter().map(|(anchor, _)| (*anchor).into_tick()));
        Self::new(ticks)
    }

    /// Selects the coarsest scale compatible with all timestamps.
    ///
    /// The fallback is [`ScaleMode::Scale1`].
    pub fn new<I: IntoIterator<Item = Tick>>(ticks: I) -> Self {
        let applicable = ticks.into_iter().fold([true; 3], |applicable, tick| {
            let divisible = Self::SCALE_MODES.map(|scale| tick % scale == 0);
            std::array::from_fn(|index| applicable[index] && divisible[index])
        });
        match applicable.iter().position(|&is_applicable| is_applicable) {
            Some(0) => Self::Scale4,
            Some(1) => Self::Scale3,
            Some(2) => Self::Scale2,
            _ => Self::Scale1,
        }
    }

    /// Returns the scale in game ticks.
    pub const fn scale(self) -> Tick {
        match self {
            Self::Scale4 => 4,
            Self::Scale2 => 2,
            Self::Scale3 => 3,
            Self::Scale1 => 1,
        }
    }

    /// Returns the cell width in blocks.
    pub const fn width(self) -> i32 {
        match self {
            Self::Scale4 | Self::Scale2 => 5,
            Self::Scale3 | Self::Scale1 => 6,
        }
    }

    /// Whether the branch region is driven by a piston press.
    const fn is_piston(self) -> bool {
        matches!(self, Self::Scale3 | Self::Scale1)
    }

    /// Maps a tick to its cell index and whether it lands on the branch line.
    pub fn cell_slot(self, tick: Tick) -> (usize, bool) {
        let tick = tick / self.scale();
        let branch = tick % 2 == 1;
        let cell = tick as usize / 2 + usize::from(self == Self::Scale1 && !branch);
        (cell, branch)
    }
}

// Templates: Template & Turn
//
// The template layer only produces placeable structures; it does not depend on
// any layout structure and can be fully constructed on its own.
//
// ++++++++++++============++++++++++++============++++++++++++============

/// The row-level context every template and turn in a row is read within: the
/// scale its cells are cut at, the width it spans, and which way it faces.
struct Frame {
    scale: ScaleMode,
    width: i32,
    south_bound: bool,
}

/// Template cell: the notes a cell carries, cut at its row's scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Template {
    main: [Option<Tone>; 2],
    branch: Branch,
}

/// Notes carried past the two fixed main-line slots.
///
/// A repeater branch carries three notes, a piston branch two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Branch {
    /// No branch-line notes: the branch region carries the third main note.
    Unbranched(Option<Tone>),
    /// Repeater branch line: up to three notes.
    Repeater([Option<Tone>; 3]),
    /// Piston branch line: up to two notes.
    Piston([Option<Tone>; 2]),
}

impl Branch {
    /// The tones this branch carries, in order.
    fn tones(&self) -> impl Iterator<Item = &Tone> {
        let slots: &[Option<Tone>] = match self {
            Self::Unbranched(tone) => std::slice::from_ref(tone),
            Self::Repeater(tones) => tones,
            Self::Piston(tones) => tones,
        };
        slots.iter().flatten()
    }
}

impl Template {
    /// Takes one cell out of the region, at the `scale` its row is cut at.
    fn new(cell: &mut Cell, scale: ScaleMode) -> Self {
        let main = [cell.main.next(), cell.main.next()];
        let branch = match cell.branch.next() {
            None => Branch::Unbranched(cell.main.next()),
            Some(first) if scale.is_piston() => Branch::Piston([Some(first), cell.branch.next()]),
            Some(first) => Branch::Repeater([Some(first), cell.branch.next(), cell.branch.next()]),
        };

        Self { main, branch }
    }
}

impl Layout<Frame> for Template {
    fn block_at_with(&self, pos: BlockPos, frame: &Frame) -> Option<BlockState> {
        use self::{Branch::*, Facing::*};
        let south_bound = frame.south_bound;
        let local_z = if south_bound { pos.z } else { 2 - pos.z };
        let local_x = frame.scale.width() - pos.x - 1;
        let facing = if south_bound { South } else { North };
        let main_repeater = || repeater(frame.scale.scale().to_string(), facing, false, false);
        let branch_repeater =
            || repeater((frame.scale.scale() / 2).to_string(), West, false, false);
        match (self.branch, local_x, pos.y, local_z) {
            (_any_branch, 1, 0, 0) => Some(chain_block()),
            (_any_branch, 1, 1, 0) => Some(main_repeater()),
            (_any_branch, 1, 0, 1) => Some(inst_block(self.main[0], chain_block)),
            (_any_branch, 1, 1, 1) => Some(note_block(self.main[0], chain_block)),
            (Unbranched(note), 0, 0, 1) => Some(inst_block(note, air)),
            (Unbranched(note), 0, 1, 1) => Some(note_block(note, air)),
            (_any_branch, 0, 0, 1) => Some(inst_block(self.main[1], air)),
            (_any_branch, 0, 1, 1) => Some(note_block(self.main[1], air)),
            (Unbranched(_), 2, 0, 1) => Some(inst_block(self.main[1], air)),
            (Unbranched(_), 2, 1, 1) => Some(note_block(self.main[1], air)),

            (Repeater(_), 2, 0, 1) => Some(chain_block()),
            (Repeater(_), 2, 1, 1) => Some(branch_repeater()),
            (Repeater(b), 3, 0, 1) => Some(inst_block(b[0], chain_block)),
            (Repeater(b), 3, 1, 1) => Some(note_block(b[0], chain_block)),
            (Repeater(b), 4, 0, 1) => Some(inst_block(b[1], air)),
            (Repeater(b), 4, 1, 1) => Some(note_block(b[1], air)),
            (Repeater(b), 3, 0, 2) => Some(inst_block(b[2], air)),
            (Repeater(b), 3, 1, 2) => Some(note_block(b[2], air)),

            (Piston(_), 2, 1, 1) => Some(sticky_piston("west")),
            (Piston(_), 3, 1, 1) => Some(redstone_block()),
            (Piston(b), 5, 0, 1) => Some(inst_block(b[0], air)),
            (Piston(b), 5, 1, 1) => Some(note_block(b[0], air)),
            (Piston(b), 4, 0, 2) => Some(inst_block(b[1], air)),
            (Piston(b), 4, 1, 2) => Some(note_block(b[1], air)),

            _ => None,
        }
    }

    fn size_with(&self, frame: &Frame) -> BlockPos {
        BlockPos::new(frame.scale.width(), 2, 3)
    }

    /// The blocks a cell can draw: its fixed circuit row, the branch hardware its
    /// shape calls for, and one note and instrument block per stored tone.
    fn block_palette_with(&self, frame: &Frame) -> HashSet<BlockState> {
        use Facing::{North, South};
        let facing = match frame.south_bound {
            true => South,
            false => North,
        };
        let scale = frame.scale.scale();
        let main = repeater(scale.to_string(), facing, false, false);
        let mut palette = HashSet::from([chain_block(), main]);
        if matches!(self.branch, Branch::Repeater(_)) {
            let half = (scale / 2).to_string();
            palette.insert(repeater(half, Facing::West, false, false));
        }
        if matches!(self.branch, Branch::Piston(_)) {
            palette.insert(sticky_piston("west"));
            palette.insert(redstone_block());
        }
        for tone in self.main.iter().flatten().chain(self.branch.tones()) {
            palette.insert(note_block(Some(tone), air));
            palette.insert(inst_block(Some(tone), air));
        }
        palette
    }
}

/// The connector turn at a row's far x-end, spanning a single z layer.
///
/// A leaf layout built from the row width and orientation alone, independent
/// of the surrounding row and cell arrangement. An unconnected turn leaves its
/// layer empty, preserving the row's fixed depth.
struct Turn {
    connected: bool,
}

impl Turn {
    /// A turn facing the row's travel direction, wired up only when connected.
    fn new(connected: bool) -> Self {
        Self { connected }
    }
}

impl Layout<Frame> for Turn {
    fn block_at_with(&self, pos: BlockPos, frame: &Frame) -> Option<BlockState> {
        use self::WireConn::*;
        if !self.connected {
            return Option::None;
        }
        match (pos.y, frame.width - 1 - pos.x, frame.south_bound) {
            (1, 1, true) => Some(wire_state(Side, Idle, Idle, Side, "0")),
            (1, 1, false) => Some(wire_state(Side, Idle, Side, Idle, "0")),
            (1, 2.., _) => Some(wire_state(Side, Side, Idle, Idle, "0")),
            (0, 1.., _) => Some(chain_block()),
            _ => Option::None,
        }
    }

    fn size_with(&self, frame: &Frame) -> BlockPos {
        BlockPos::new(frame.width, 2, 1)
    }

    /// The blocks a turn can draw: its connector wires and the chain it rides on.
    fn block_palette_with(&self, _frame: &Frame) -> HashSet<BlockState> {
        if !self.connected {
            return HashSet::new();
        }
        use WireConn::*;
        HashSet::from([
            chain_block(),
            wire_state(Side, Idle, Idle, Side, "0"),
            wire_state(Side, Idle, Side, Idle, "0"),
            wire_state(Side, Side, Idle, Idle, "0"),
        ])
    }
}
