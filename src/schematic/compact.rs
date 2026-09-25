//! Compact note block layouts for NBS song projection.

use super::{AsLayout, EvenlyArranged, Layout, WithFloor};
use crate::{GameTick, RedStoneTick};
use mcdata::util::BlockPos;
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
        delay: RedStoneTick,
        south_bound: bool,
    },
    Sink {
        delay: RedStoneTick,
        notes: [Option<Tone>; 3],
        south_bound: bool,
    },
    Node {
        notes: [Option<Tone>; 2],
        south_bound: bool,
    },
    Port {
        delay: RedStoneTick,
        notes: [Option<Tone>; 2],
        south_bound: bool,
    },
}

#[derive(Clone, Copy)]
enum Turn {
    Sink {
        width: i32,
        handle: RedStoneTick,
        notes: [Option<Tone>; 2],
    },
    Node {
        width: i32,
        note: Option<Tone>,
    },
}
