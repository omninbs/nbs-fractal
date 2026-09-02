//! Fractal structures of NBS songs.
//!
//! This crate builds on [`rsnbs`]: [`analysis`] decomposes the note plane
//! into translation equivalence classes, and [`schematic`] reconstructs
//! Minecraft litematic projections from them.

pub mod analysis;
pub mod schematic;

#[cfg(test)]
mod tests;

/// Redstone tick (10 t/s).
pub(crate) type RedStoneTick = rsnbs::types::Tick;
/// Game tick (20 t/s).
pub(crate) type GameTick = rsnbs::types::Tick;
