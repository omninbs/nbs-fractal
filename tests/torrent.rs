//! Placement of panning and volume in the Torrent layout.

use nbs_fractal::BlockPos;
use nbs_fractal::layout::{LayoutExt, torrent::TorrentLayout};
use rsnbs::note::{Instrument, Key, Note, Tone};
use rsnbs::types::{Panning, Volume};

/// A note of `tone` at the given panning and volume.
fn voiced(tone: Tone, panning: i8, volume: u8) -> Note {
    Note {
        tone,
        velocity: Volume::new(volume).unwrap(),
        panning: Panning::new(panning).unwrap(),
        pitch: 0,
    }
}

/// A harp note at the given panning and volume.
fn note(panning: i8, volume: u8) -> Note {
    voiced(Tone::new(Instrument::Harp, Key::FS3), panning, volume)
}

/// The pivot the slice is heard from.
const AXIS: i32 = 49;
const AXIS_Y: i32 = 46;

/// Every note block in `layout`, ordered by ascending z.
fn placed(layout: &impl LayoutExt) -> Vec<BlockPos> {
    let size = layout.size();
    let mut found = Vec::new();
    for z in 0..size.z {
        for y in 0..size.y {
            for x in 0..size.x {
                let pos = BlockPos::new(x, y, z);
                let hit = layout
                    .get_block(pos)
                    .is_some_and(|b| b.name == "minecraft:note_block");
                if hit {
                    found.push(pos);
                }
            }
        }
    }
    found
}

#[test]
fn louder_lands_nearer_the_axis() {
    let layout = TorrentLayout::new(
        [(0u32, note(0, 100)), (1, note(0, 20))],
        TorrentLayout::DEFAULT_TRACKS,
        false,
    );
    let placed = placed(&layout);
    let distance = |p: &BlockPos| (p.x - AXIS).pow(2) + (p.y - AXIS_Y).pow(2);
    assert!(distance(&placed[0]) < distance(&placed[1]));
}

/// NBS velocity is a linear playback gain, so half volume is heard halfway down
/// the 48-block falloff rather than at its sensory midpoint.
#[test]
fn volume_falls_off_linearly() {
    let layout = TorrentLayout::new([(0u32, note(0, 50))], TorrentLayout::DEFAULT_TRACKS, false);
    let placed = placed(&layout);
    let distance = |p: &BlockPos| (p.x - AXIS).pow(2) + (p.y - AXIS_Y).pow(2);
    assert_eq!(distance(&placed[0]), 24 * 24);
}

#[test]
fn panning_spreads_across_the_axis() {
    let layout = TorrentLayout::new(
        [(0u32, note(-100, 100)), (1, note(100, 100))],
        TorrentLayout::DEFAULT_TRACKS,
        false,
    );
    let placed = placed(&layout);
    assert!(placed[0].x > AXIS);
    assert!(placed[1].x < AXIS);
}

/// A point stepping through the panning and volume plane, springing back off the
/// panning and volume borders like a logo drifting across a screen.
fn bounce(start: (i32, i32), step: (i32, i32)) -> impl Iterator<Item = (i32, i32)> {
    let (mut pos, mut step) = (start, step);
    std::iter::from_fn(move || {
        let here = pos;
        pos.0 += step.0;
        pos.1 += step.1;
        if !(-100..=100).contains(&pos.0) {
            pos.0 = pos.0.clamp(-100, 100);
            step.0 = -step.0;
        }
        if !(0..=100).contains(&pos.1) {
            pos.1 = pos.1.clamp(0, 100);
            step.1 = -step.1;
        }
        Some(here)
    })
}

/// Writes a 300-tick audition of panning and volume to
/// `out/torrent_pan_volume.litematic`: voices in distinct registers, each drifting
/// about the panning-volume plane at its own pace so any one can be picked by ear.
#[test]
#[ignore = "writes an artifact; run with --ignored"]
fn sweep_all_panning_and_volume() {
    const TICKS: usize = 300;
    let voices = [
        (Tone::new(Instrument::Harp, Key::C4), (-100, 100), (7, -3)),
        (Tone::new(Instrument::Bell, Key::C5), (0, 50), (11, 7)),
        (Tone::new(Instrument::Flute, Key::G4), (100, 0), (-9, 5)),
    ];
    let paths: Vec<Vec<(i32, i32)>> = voices
        .iter()
        .map(|&(_, start, step)| bounce(start, step).take(TICKS).collect())
        .collect();

    let mut notes: Vec<(u32, Note)> = Vec::new();
    for t in 0..TICKS {
        for ((tone, _, _), path) in voices.iter().zip(&paths) {
            let (pan, vol) = path[t];
            notes.push((t as u32, voiced(*tone, pan as i8, vol as u8)));
        }
    }

    let layout = TorrentLayout::new(notes, TorrentLayout::DEFAULT_TRACKS, false);
    assert_eq!(placed(&layout).len(), TICKS * voices.len());

    let metadata = nbs_fractal::layout::Metadata {
        name: "Note Block Track Schematic",
        description: "Torrent pan/volume audition",
        author: "nbs-fractal",
    };
    std::fs::create_dir_all("out").ok();
    let file = std::fs::File::create("out/torrent_pan_volume.litematic").unwrap();
    layout.as_litematic(metadata, file).unwrap();
}

#[test]
fn every_voice_is_placed() {
    let notes: Vec<(u32, Note)> = (0..24)
        .map(|i| (i, note((i as i32 * 8 - 96) as i8, (i * 4) as u8)))
        .collect();
    let layout = TorrentLayout::new(notes, TorrentLayout::DEFAULT_TRACKS, false);
    assert_eq!(placed(&layout).len(), 24);
}
