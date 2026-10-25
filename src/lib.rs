//! Fractal structures of NBS songs.
//!
//! This crate builds on [`rsnbs`]: [`analysis`] decomposes the note plane
//! into translation equivalence classes, and [`layout`] arranges the notes into
//! projection layouts that [`litematica`] writes out.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests;
use rsnbs::note::{Instrument, Tone};

pub mod analysis;
pub mod layout;
pub use litematica::knowledge::mc::{BlockPos, BlockState};

/// Redstone tick (10 t/s).
pub type RedStoneTick = rsnbs::types::Tick;
/// Game tick (20 t/s).
pub type GameTick = rsnbs::types::Tick;

// util
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Keeps only the notes that render into a note block, dropping the ones that
/// cannot. A note renders when it carries a vanilla instrument whose key lies
/// within the note block's f#3-f#5 range; mob head and custom instruments, and
/// keys outside that range, do not.
pub fn playable<I, T>(notes: I) -> impl Iterator<Item = T>
where
    I: IntoIterator<Item = T>,
    T: AsRef<Tone>,
{
    use Instrument::*;
    notes.into_iter().filter(|note| {
        let tone = note.as_ref();
        !matches!(tone.instrument, Imitate(_) | Custom(_)) && tone.key.minecraft_note().is_some()
    })
}
