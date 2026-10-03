//! Fishbone layout for NBS song projection.

use super::{AsLayout, EvenlyArranged, Facing, Layout};
use super::{air, chain_block, inst_block, note_block, repeater};
use crate::schematic::{WireConn, wire_state};
use mcdata::GenericBlockState;
use mcdata::util::BlockPos;
use rsnbs::note::Tone;
use rsnbs::types::{Tick, TimeAnchor};
use std::iter::from_fn;

// Shell: MultiFishboneLayout
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Multi-line fishbone noteblocks layout.
pub struct MultiFishboneLayout(EvenlyArranged<FishboneLayout>);

impl MultiFishboneLayout {
    /// Create a fishbone layout from per-track notes.
    pub fn new<Trks, Trk, A, T>(tracks: Trks, gap: u32, song_length: Tick) -> Self
    where
        Trks: IntoIterator<Item = Trk>,
        Trk: IntoIterator<Item = (A, T)>,
        A: TimeAnchor,
        T: Into<Tone>,
        for<'a> &'a Trks: IntoIterator<Item = &'a Trk>,
        for<'a> &'a Trk: IntoIterator<Item = (&'a A, &'a T)>,
    {
        let layouts = tracks
            .into_iter()
            .flat_map(|notes| FishboneLayout::new(notes, song_length));
        let pitch = BlockPos::new(8 + gap as i32, 0, 0);
        Self(EvenlyArranged::new(layouts, pitch))
    }
}

impl AsLayout for MultiFishboneLayout {
    fn as_layout(&self) -> &impl Layout {
        &self.0
    }
}

// Layout: FishboneLayout
//
// ++++++++++++============++++++++++++============++++++++++++============

/// A fishbone layout for one track.
pub struct FishboneLayout(EvenlyArranged<Template>);

impl FishboneLayout {
    /// Splits a track's notes into as many fishbone lanes as needed, each
    /// group consuming up to two notes, keeping every cell up to `song_length`
    /// alive even when silent.
    pub(crate) fn new<Trk, A, T>(notes: Trk, song_length: Tick) -> impl Iterator<Item = Self>
    where
        Trk: IntoIterator<Item = (A, T)>,
        A: TimeAnchor,
        T: Into<Tone>,
    {
        // File each note into its cell's tick group, keeping the track long
        // enough to match the song even where it stays silent.
        let min = song_length
            .checked_sub(1)
            .map_or(0, |tick| (tick / 4 + 1) as usize);
        let mut cells: Vec<[Vec<Tone>; 4]> = Vec::new();
        for (anchor, note) in notes {
            let tick = anchor.into_tick() as usize;
            cells.resize_with(cells.len().max(tick / 4 + 1), Default::default);
            cells[tick / 4][tick % 4].push(note.into());
        }
        cells.resize_with(cells.len().max(min), Default::default);

        // Emit one lane per pass, each group draining up to two notes.
        from_fn(move || {
            let live = cells.iter().flatten().any(|queue| !queue.is_empty());
            live.then(|| {
                let templates = cells.iter_mut().map(Template::take);
                Self(EvenlyArranged::new(templates, BlockPos::new(0, 0, 2)))
            })
        })
    }
}

impl AsLayout for FishboneLayout {
    fn as_layout(&self) -> &impl Layout {
        &self.0
    }
}

// Templates: Template
//
// The template layer only produces placeable structures; it does not depend on
// any layout structure and can be fully constructed on its own.
//
// ++++++++++++============++++++++++++============++++++++++++============

/// One cell's structure: four group slots down the primary row plus three
/// carry-over slots on the trailing row, over an instrument layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Template {
    groups: [[Option<Tone>; 2]; 4],
}

impl Template {
    /// Drains up to two queued notes per group into a fresh template.
    fn take(cell: &mut [Vec<Tone>; 4]) -> Self {
        let groups = std::array::from_fn(|g| {
            let count = cell[g].len().min(2);
            let mut drain = cell[g].drain(..count);
            [drain.next(), drain.next()]
        });
        Self { groups }
    }

    /// Whether group `g` carries any tone.
    fn filled(&self, g: usize) -> bool {
        self.groups[g].iter().any(Option::is_some)
    }

    /// Whether any group after `g` carries a tone, so the line must run on.
    fn continues(&self, g: usize) -> bool {
        ((g + 1)..4).any(|g| self.filled(g))
    }

    /// Repeater delay for a filled group: one tick per crossed empty group.
    fn delay(&self, g: usize) -> usize {
        let skipped = (2..=g).rev().take_while(|&n| !self.filled(n - 1)).count();
        1 + skipped
    }
}

impl Layout for Template {
    fn block_at(&self, pos: BlockPos) -> Option<GenericBlockState> {
        use Facing::{East, South};
        use WireConn::{None as Air, Side};
        let g = pos.x as usize / 2;
        let [first, second] = self.groups[g];
        let wire = || wire_state(Side, Side, Air, Air, "0");
        let drive = || repeater(self.delay(g).to_string(), East, false, false);
        let east = if self.continues(0) { Side } else { Air };
        let dust = wire_state(Air, east, Side, Side, "0");

        // Main line: the spine on columns 0 and 1, plus the clock pass-through.
        let main = |y: i32| match (pos.z, pos.x, y) {
            (0, 1, 0) => Some(chain_block()),
            (0, 1, 1) => Some(repeater("4", South, false, false)),
            (1, 0, 0) => second.map(|t| inst_block(Some(t), air)),
            (1, 0, 1) => second.map(|t| note_block(Some(t), air)),
            (1, 1, 0) => Some(first.map_or_else(chain_block, |t| inst_block(Some(t), air))),
            (1, 1, 1) => Some(first.map_or(dust, |t| note_block(Some(t), air))),
            _ => None,
        };
        // Branch line: one rib per group on columns 2..7. Each rib has a drive
        // column, a note on the primary row and its carry on the trailing row.
        let branch = |y: i32| match (pos.z, pos.x, y) {
            (1, 2 | 4 | 6, 0) if self.filled(g) || self.continues(g) => Some(chain_block()),
            (1, 2 | 4 | 6, 1) if self.filled(g) => Some(drive()),
            (1, 2 | 4 | 6, 1) if self.continues(g) => Some(wire()),
            (1, 3 | 5 | 7, 0) if first.is_some() => Some(inst_block(first, air)),
            (1, 3 | 5 | 7, 0) if self.continues(g) => Some(chain_block()),
            (1, 3 | 5 | 7, 1) if first.is_some() => Some(note_block(first, air)),
            (1, 3 | 5 | 7, 1) if self.continues(g) => Some(wire()),
            (2, 3 | 5 | 7, 0) => Some(inst_block(second, air)),
            (2, 3 | 5 | 7, 1) => Some(note_block(second, air)),
            _ => None,
        };

        main(pos.y).or_else(|| branch(pos.y))
    }

    fn size(&self) -> BlockPos {
        BlockPos::new(8, 2, 3)
    }
}
