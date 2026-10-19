//! Cascade layout for NBS song projection.
//!
//! Deprecated in favour of the torrent layout.
#![allow(deprecated)]

use super::blocks::{Facing, ToneBlocks, WireConn, chain_block, inst_block};
use super::blocks::{note_block, repeater, wire_state};
use super::compose::{EvenlyArranged, Overlaid};
use super::{Layout, LayoutExt, air};
use crate::{BlockPos, BlockState, forward_layout};
use rsnbs::note::{Note, Tone};
use rsnbs::types::TimeAnchor;
use std::collections::HashSet;
use std::iter;

// Layout: CascadeLayout
//
// A cascade reads as one continuous sweep rather than many independent lines:
// every tick gets its own row, and silent ticks keep their row so the wall stays
// evenly spaced. The whole sweep fires from one fixed startup, which absorbs the
// timing differences between tracks so each row can simply step forward.
//
// ++++++++++++============++++++++++++============++++++++++++============

/// A note stream projected as a cascade: one row per tick off a fixed startup.
#[deprecated(note = "superseded by `TorrentLayout`")]
pub struct CascadeLayout(Overlaid<Linkage, EvenlyArranged<Template>>);

impl CascadeLayout {
    /// Projects a note stream, one row per redstone tick.
    pub fn new<Trk, A, T>(notes: Trk) -> Self
    where
        Trk: IntoIterator<Item = (A, T)>,
        A: TimeAnchor,
        T: Into<Note>,
    {
        let mut notes = notes
            .into_iter()
            .map(|(anchor, note)| (anchor.into_tick(), note.into()))
            .peekable();
        let mut rows: Vec<Template> = Vec::new();
        while let Some(&(next, _)) = notes.peek() {
            let len = (next as usize).max(rows.len());
            rows.resize_with(len, || Template::new(iter::empty()));
            let slice = iter::from_fn(|| notes.next_if(|&(t, _)| t == next).map(|(_, note)| note));
            rows.push(Template::new(slice));
        }
        if rows.is_empty() {
            rows.push(Template::new(iter::empty()));
        }

        let linkage = Linkage;
        let anchor = BlockPos::new(0, 0, linkage.size().z - 1);
        let rows = EvenlyArranged::new(rows, BlockPos::new(0, 0, 2), &());
        let size = BlockPos::new(rows.size().x, rows.size().y, anchor.z + rows.size().z);
        let overlay = Overlaid::new(BlockPos::ORIGIN, linkage, anchor, rows, size, &());
        Self(overlay)
    }
}

forward_layout!(CascadeLayout, ());

// Templates: Template & Linkage
//
// Linkage is the fixed front that sets every track running together; Template is
// one tick of the wall. Every track declares the loudness it projects, so a voice
// is placed by how it should be heard: loudness picks the track, panning the side.
//
// ++++++++++++============++++++++++++============++++++++++++============

// TODO: Template pending refactor. AI-generated; doesn't quite fit the design.

/// The track centres, leftmost outward track to rightmost; a track's three slots
/// sit at its centre and the cells either side.
const TRACK: [i32; 17] = [
    0, 3, 6, 10, 15, 22, 32, 42, 45, 48, 58, 68, 75, 80, 84, 87, 90,
];

/// One tick of the wall: the voices its tracks carry, three slots per track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Template {
    slots: [[Option<Tone>; 3]; 17],
}

impl Template {
    /// The three centre tracks, left inner to right inner.
    const CENTRE: [usize; 3] = [7, 8, 9];

    /// Each track's declared loudness, percent; the centre is full strength and
    /// the sides fall off a touch faster the further out they sit.
    const LOUDNESS: [u8; 17] = [
        0, 5, 15, 30, 50, 70, 90, 100, 100, 100, 90, 70, 50, 30, 15, 5, 0,
    ];

    /// Fills a row with the voices to sound on its tick, placing each on the
    /// track nearest its loudness; panning divides the sides, and the loudest
    /// balanced voices stack in the centre.
    fn new<I: IntoIterator<Item = Note>>(notes: I) -> Self {
        let mut template = Self {
            slots: [[None; 3]; 17],
        };
        // A voice the wall cannot sound never takes a slot; it would only push the
        // voices that do onto the sides and leave the centre bare.
        let voices = notes
            .into_iter()
            .filter(|note| note.tone.note_block_state().is_some());
        let mut centre = Vec::new();
        voices.for_each(|note| template.place(note, &mut centre));
        template.balance(&centre);
        template
    }

    /// Routes one voice: into the centre list, or onto its pair of side tracks.
    fn place(&mut self, note: Note, centre: &mut Vec<(Tone, u8)>) {
        let Some(tracks) = Self::route(note.velocity.get(), note.panning.get()) else {
            centre.push((note.tone, note.velocity.get()));
            return;
        };
        tracks
            .into_iter()
            .flatten()
            .for_each(|t| self.put(t, note.tone));
    }

    /// The side tracks a voice falls between, or `None` when it belongs in the centre.
    /// Each ear rounds up, so it leans to the heavier side rather than the lighter.
    fn route(volume: u8, panning: i8) -> Option<[Option<usize>; 2]> {
        let (v, p) = (i32::from(volume), i32::from(panning));
        let left = (v * (100 - p.max(0)) + 50) / 100;
        let right = (v * (100 + p.min(0)) + 50) / 100;
        const FULL: i32 = 95;
        if left >= FULL && right >= FULL {
            return None;
        }
        Some([Self::side(left, false), Self::side(right, true)])
    }

    /// The track on one side nearest a loudness; `None` when the ear is silent.
    fn side(loudness: i32, east: bool) -> Option<usize> {
        if loudness <= 0 {
            return None;
        }
        let [west, mid, east_edge] = Self::CENTRE;
        let band = match east {
            true => east_edge + 1..Self::LOUDNESS.len(),
            false => 1..west,
        };
        band.min_by_key(|&t| {
            let near = (i32::from(Self::LOUDNESS[t]) - loudness).abs();
            (near, (t as i32 - mid as i32).abs())
        })
    }

    /// Routes the centre voices over the three middle tracks. The centre track
    /// takes as many as it can while staying balanced and the outer two mirror it;
    /// any overflow splits in two at half loudness, each voice leaving a half on
    /// the matching tracks to either side of the middle stack.
    fn balance(&mut self, centre: &[(Tone, u8)]) {
        let count = centre.len();
        // The middle track takes the centre share; an even share above three skips
        // its heart slot, which the relay bridges, so the centre stays balanced.
        let middle = match count > 3 && count % 2 == 0 {
            true => 2,
            false => count.min(3),
        };
        let outer = ((count - middle) / 2).min(3);
        let [west, mid, east] = Self::CENTRE;
        let mut rest = centre;
        for (track, share) in [(mid, middle), (west, outer), (east, outer)] {
            let take = share.min(rest.len());
            let order = match track == mid && take == 2 {
                true => [1, 2, 0],
                false => Self::order(track),
            };
            let (chunk, tail) = rest.split_at(take);
            Self::spread(&mut self.slots[track], chunk, order);
            rest = tail;
        }
        for &(tone, level) in rest {
            let half = i32::from(level) / 2;
            let sides = [Self::side(half, false), Self::side(half, true)];
            sides.into_iter().flatten().for_each(|t| self.put(t, tone));
        }
    }

    /// A track's slot fill order: its own middle first, then the slot toward the
    /// centre, then the one away from it, so a track reads as centred on itself
    /// while leaning in with the wall.
    fn order(track: usize) -> [usize; 3] {
        match track < Self::CENTRE[1] {
            true => [0, 2, 1],
            false => [0, 1, 2],
        }
    }

    /// Lays a track's voices onto its slots, in the given order.
    fn spread(slots: &mut [Option<Tone>; 3], tones: &[(Tone, u8)], order: [usize; 3]) {
        for (&slot, &(tone, _)) in order.iter().zip(tones) {
            slots[slot] = Some(tone);
        }
    }

    /// Keeps a track's middle slot powered when it carries no voice, so the row's
    /// signal still runs through.
    fn relay(&self, y: i32, track: usize, x: i32) -> Option<BlockState> {
        use WireConn::*;
        let slots = &self.slots[track];
        let split = slots[1].is_some() || slots[2].is_some();
        let bare_heart = x == TRACK[Self::CENTRE[1]] && slots[0].is_none() && split;
        if y == 0 || bare_heart {
            return Some(chain_block());
        }
        Some(wire_state(Idle, Idle, Side, Side, "0"))
    }

    /// Records a voice, spilling outward onto the quieter tracks once its own is full.
    fn put(&mut self, track: usize, tone: Tone) {
        let step: i32 = if track < Self::CENTRE[1] { -1 } else { 1 };
        let mut outward = iter::successors(Some(track as i32), move |&t| {
            (0..Self::LOUDNESS.len() as i32)
                .contains(&(t + step))
                .then(|| t + step)
        });
        let _ = outward.any(|t| self.try_put(t as usize, tone));
    }

    /// Spills a voice onto a track's first free slot, in that track's fill order.
    fn try_put(&mut self, track: usize, tone: Tone) -> bool {
        let order = Self::order(track);
        let slots = &mut self.slots[track];
        let Some(&slot) = order.iter().find(|&&slot| slots[slot].is_none()) else {
            return false;
        };
        slots[slot] = Some(tone);
        true
    }
}

impl<Ctx> Layout<Ctx> for Template {
    fn block_at_with(&self, pos: BlockPos, _ctx: &Ctx) -> Option<BlockState> {
        // Each track's three slots as x offsets from its centre: middle, left, right.
        const SLOT_DX: [i32; 3] = [0, -1, 1];
        let cell = TRACK.iter().enumerate().find_map(|(track, &c)| {
            let slot = SLOT_DX.iter().position(|&dx| c + dx == pos.x)?;
            Some((track, slot))
        });
        match (pos.z, pos.y, cell) {
            // z = 0: the drive row, one repeater on each track's middle slot.
            (0, 0, Some((_, 0))) => Some(chain_block()),
            (0, 1, Some((_, 0))) => Some(repeater("1", Facing::South, false, false)),
            // z = 1: the note row. Every stored voice can sound (filtered in
            // `new`), so a bare middle slot relays rather than meeting air.
            (1, _, Some((t, 0))) if self.slots[t][0].is_none() => self.relay(pos.y, t, pos.x),
            (1, 0, Some((t, s))) => self.slots[t][s].map(|tone| inst_block(Some(tone), air)),
            (1, 1, Some((t, s))) => self.slots[t][s].map(|tone| note_block(Some(tone), air)),
            _ => None,
        }
    }

    fn size_with(&self, _ctx: &Ctx) -> BlockPos {
        BlockPos::new(TRACK[TRACK.len() - 1] + 2, 2, 2)
    }

    /// The blocks a row can draw: the drive row, the relay, and one note and
    /// instrument block per stored voice.
    fn block_palette_with(&self, _ctx: &Ctx) -> HashSet<BlockState> {
        use WireConn::*;
        let mut palette = HashSet::from([
            chain_block(),
            repeater("1", Facing::South, false, false),
            wire_state(Idle, Idle, Side, Side, "0"),
        ]);
        for tone in self.slots.iter().flatten().flatten() {
            palette.insert(note_block(Some(tone), air));
            palette.insert(inst_block(Some(tone), air));
        }
        palette
    }
}

/// The fixed z0..=z4 startup.
struct Linkage;

impl<Ctx> Layout<Ctx> for Linkage {
    fn block_at_with(&self, pos: BlockPos, _ctx: &Ctx) -> Option<BlockState> {
        use WireConn::*;
        match (pos.z, pos.y, pos.x) {
            // z0..=z2
            (0..=2, 0, 45) => Some(chain_block()),
            (0, 1, 45) => Some(button()),
            (1..=2, 1, 45) => Some(repeater("4", Facing::South, false, false)),
            // z3
            (3, 1, 1 | 16 | 30) => Some(repeater("1", Facing::West, false, false)),
            (3, 1, 60 | 74 | 89) => Some(repeater("1", Facing::East, false, false)),
            (3, 1, 0) => Some(wire_state(Idle, Side, Idle, Side, "0")),
            (3, 1, 90) => Some(wire_state(Side, Idle, Idle, Side, "0")),
            (3, 1, 45) => Some(wire_state(Side, Side, Side, Side, "0")),
            (3, 1, x) if TRACK.contains(&x) => Some(wire_state(Side, Side, Idle, Side, "0")),
            (3, 0, 0..=90) => Some(chain_block()),
            (3, 1, 0..=90) => Some(wire_state(Side, Side, Idle, Idle, "0")),
            // z4
            (4, 0, x) if TRACK.contains(&x) => Some(chain_block()),
            (4, 1, 0 | 90) => Some(repeater("1", Facing::South, false, false)),
            (4, 1, 3 | 6 | 10 | 15) => Some(repeater("2", Facing::South, false, false)),
            (4, 1, 75 | 80 | 84 | 87) => Some(repeater("2", Facing::South, false, false)),
            (4, 1, 22 | 68) => Some(repeater("3", Facing::South, false, false)),
            (4, 1, 32 | 42 | 45 | 48 | 58) => Some(repeater("4", Facing::South, false, false)),
            _ => None,
        }
    }

    fn size_with(&self, _ctx: &Ctx) -> BlockPos {
        BlockPos::new(92, 2, 5)
    }

    /// The whole fixed startup: its chain, button, repeaters and wire links.
    fn block_palette_with(&self, _ctx: &Ctx) -> HashSet<BlockState> {
        use Facing::{East, South, West};
        use WireConn::*;
        HashSet::from([
            chain_block(),
            button(),
            repeater("1", South, false, false),
            repeater("1", West, false, false),
            repeater("1", East, false, false),
            repeater("2", South, false, false),
            repeater("3", South, false, false),
            repeater("4", South, false, false),
            wire_state(Idle, Side, Idle, Side, "0"),
            wire_state(Side, Idle, Idle, Side, "0"),
            wire_state(Side, Side, Side, Side, "0"),
            wire_state(Side, Side, Idle, Side, "0"),
            wire_state(Side, Side, Idle, Idle, "0"),
        ])
    }
}

/// The centre source button.
fn button() -> BlockState {
    let properties = From::from([
        ("face".into(), "floor".into()),
        ("facing".into(), "south".into()),
        ("powered".into(), "false".into()),
    ]);
    BlockState {
        name: "minecraft:stone_button".into(),
        properties,
    }
}
