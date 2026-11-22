//! Analysis CLI: optimal Tapped match rules, and TEC group verification.

use clap::Parser;
use nbs_fractal::analysis::reuse::autocorrelation;
use nbs_fractal::analysis::{BoundedTec, TePlane, TransEqClass};
use rsnbs::note::{Note, NoteStreamExt, Notes, Tone};
use rsnbs::song::Song;
use rsnbs::types::{Position, Tick, TimeAnchor};
use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZero;

/// Default smallest scatter gap, in redstone ticks.
const DEFAULT_MIN_GAP: Tick = 8;

// Cli
//
// ++++++++++++============++++++++++++============++++++++++++============

#[derive(Parser)]
#[command(
    name = "nbs-fractal-tec",
    about = "Analyze NBS songs for Tapped match rules and TEC groups"
)]
enum Cli {
    Scan(Scan),
    Verify(Verify),
}

fn main() {
    match Cli::parse() {
        Cli::Scan(cmd) => cmd.run(),
        Cli::Verify(cmd) => cmd.run(),
    }
}

// Scan
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Report the match rules that maximize reuse for each layer budget.
///
/// Each layer grows its scatter greedily out of the strongest spacings of the
/// plane, keeping only scatters that `tapped` can realize; the rules are
/// printed in raw ticks for `match -r`.
#[derive(clap::Args)]
struct Scan {
    /// Path to input NBS file
    input: String,
    /// Layer budgets to solve, comma separated
    #[arg(long, default_value = "3,4")]
    layers: String,
    /// Number of spacings kept in the offset pool
    #[arg(long, default_value_t = 24)]
    top: usize,
    /// Largest scatter arity considered
    #[arg(long = "max-arity", default_value_t = 12)]
    max_arity: usize,
    /// Smallest scatter gap, in redstone ticks
    #[arg(long = "min-gap", default_value_t = DEFAULT_MIN_GAP)]
    min_gap: Tick,
}

impl Scan {
    fn run(self) {
        let song = open_song(&self.input);
        let plane = plane_of(&song.notes, song.header.tempo);
        let factor = rescale_factor(song.header.tempo);
        let pool = offset_pool(&autocorrelation(&plane), self.top);

        println!(
            "# {}: notes={} tempo={}",
            self.input,
            plane.values().sum::<usize>(),
            song.header.tempo
        );
        for budget in self.layers.split(',') {
            let Ok(budget) = budget.trim().parse::<usize>() else {
                continue;
            };
            let (gain, plan) = plan_layers(&plane, &pool, budget, self.max_arity, self.min_gap);
            let first = plan.first().map(|tec| tec.offsets().nth(1).unwrap_or(0));
            println!(
                "layers={budget} reuse={gain} d_rs={}",
                first.map_or(0, |d| d)
            );
            for (index, tec) in plan.iter().enumerate() {
                let offsets: Vec<String> = tec
                    .offsets()
                    .filter(|&t| t > 0)
                    .map(|t| raw_offset(t, factor).to_string())
                    .collect();
                println!("  layer{}: -r {}", index + 1, offsets.join(","));
            }
        }
    }
}

// Verify
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Report each TEC group's restored scatter and realized reuse.
///
/// Groups are the runs of notes separated by an empty layer, matching how
/// `tapped` splits a `match` output.
#[derive(clap::Args)]
struct Verify {
    /// Path to input TEC NBS file
    input: String,
}

impl Verify {
    fn run(self) {
        let song = open_song(&self.input);
        let tempo = song.header.tempo;
        let groups = song.notes.split_by_layer_gaps();
        println!("# {}: {} group(s)", self.input, groups.len());
        let mut total = 0;
        for (index, group) in groups.iter().enumerate() {
            let plane = plane_of(group, tempo);
            let tec = BoundedTec::restore(&plane).into_inner();
            let offsets: Vec<Tick> = tec.offsets().collect();
            let gain = tec.reuse();
            total += gain;
            println!(
                "  group{}: notes={} scatter={offsets:?} reuse={gain}",
                index + 1,
                plane.values().sum::<usize>()
            );
        }
        println!("  total reuse={total}");
    }
}

// Utils
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Builds the redstone-tick plane of a note stream, keeping multiplicities.
fn plane_of(notes: &Notes<Position, Note>, tempo: f32) -> TePlane<Tone> {
    let points = notes.iter().map(|(pos, note)| (*pos, note.tone));
    points
        .rescale_to_redstone_tick(tempo)
        .map(|(pos, tone)| (pos.into_tick(), tone))
        .collect()
}

/// Spends up to `budget` layers on the plane, greedily taking the best scatter
/// each time.
fn plan_layers(
    plane: &TePlane<Tone>,
    pool: &[Tick],
    budget: usize,
    max_arity: usize,
    min_gap: Tick,
) -> (usize, Vec<TransEqClass<Tone>>) {
    let mut work = plane.clone();
    let mut plan = Vec::new();
    let mut gain = 0;
    while plan.len() < budget {
        let Some((reuse, scatter)) = grow(&work, pool, max_arity, min_gap) else {
            break;
        };
        let offsets = to_nonzero(&scatter);
        plan.push(BoundedTec::extract_from(&mut work, offsets).into_inner());
        gain += reuse;
    }
    (gain, plan)
}

/// Grows scatters over `pool` with a beam, then keeps the best realization
/// `tapped` derives from them.
///
/// Returns the realized reuse and the scatter, or `None` when nothing pairs.
fn grow(
    plane: &TePlane<Tone>,
    pool: &[Tick],
    max_arity: usize,
    min_gap: Tick,
) -> Option<(usize, BTreeSet<Tick>)> {
    const WIDTH: usize = 16;
    let mut beam = vec![(BTreeSet::new(), 0)];
    let mut seen = Vec::new();
    for _ in 0..max_arity.saturating_sub(1) {
        let mut next = Vec::new();
        for (set, _) in &beam {
            for &offset in pool {
                if set.contains(&offset) {
                    continue;
                }
                let mut candidate = set.clone();
                candidate.insert(offset);
                let mut work = plane.clone();
                let tec = BoundedTec::extract_from(&mut work, to_nonzero(&candidate)).into_inner();
                if tec.reuse() == 0 {
                    continue;
                }
                next.push((candidate, tec.reuse()));
            }
        }
        if next.is_empty() {
            break;
        }
        next.sort_by(|a, b| b.1.cmp(&a.1));
        next.truncate(WIDTH);
        seen.extend(next.iter().map(|(set, _)| set.clone()));
        beam = next;
    }
    seen.into_iter()
        .filter_map(|set| realized_reuse(plane, &set, min_gap).map(|reuse| (reuse, set)))
        .max_by_key(|(reuse, _)| *reuse)
}

/// Extracts `scatter` from a copy of `plane` and scores the scatter `restore`
/// ends up with, dropping candidates with too small a gap.
fn realized_reuse(plane: &TePlane<Tone>, scatter: &BTreeSet<Tick>, min_gap: Tick) -> Option<usize> {
    let mut work = plane.clone();
    let tec = BoundedTec::extract_from(&mut work, to_nonzero(scatter)).into_inner();
    if tec.reuse() == 0 {
        return None;
    }
    let realized = BoundedTec::restore(&tec.expand()).into_inner();
    let gap = realized.min_gap()?;
    (gap >= min_gap).then(|| realized.reuse())
}

/// The offsets worth trying, strongest autocorrelations first, plus their
/// pair sums so non-uniform scatters stay reachable.
fn offset_pool(support: &BTreeMap<Tick, usize>, top: usize) -> Vec<Tick> {
    let mut ranked: Vec<(usize, Tick)> = support.iter().map(|(&d, &c)| (c, d)).collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    ranked.truncate(top);
    let mut pool: BTreeSet<Tick> = ranked.iter().map(|&(_, d)| d).collect();
    let small: Vec<Tick> = ranked.iter().take(10).map(|&(_, d)| d).collect();
    for (i, &a) in small.iter().enumerate() {
        for &b in &small[i + 1..] {
            pool.insert(a + b);
        }
    }
    pool.into_iter().filter(|&d| d >= DEFAULT_MIN_GAP).collect()
}

/// Maps offsets onto `NonZero`, dropping zeros.
fn to_nonzero(scatter: &BTreeSet<Tick>) -> BTreeSet<NonZero<Tick>> {
    scatter.iter().filter_map(|&t| NonZero::new(t)).collect()
}

/// The `(num, den)` pair `rsnbs` uses to rescale raw ticks to redstone ticks.
fn rescale_factor(tempo: f32) -> (Tick, Tick) {
    let scale = match (0.0..30.0).contains(&tempo) {
        true => 10.0 / tempo,
        false => 10.0 / 20.0,
    };
    match scale >= 1.0 {
        true => (scale.round() as Tick, 1),
        false => (1, (1.0 / scale).round() as Tick),
    }
}

/// The smallest raw tick whose redstone rescale reaches `rs`.
fn raw_offset(rs: Tick, (num, den): (Tick, Tick)) -> Tick {
    (rs * den).div_ceil(num)
}

/// Loads the input song.
fn open_song(input: &str) -> Song {
    Song::open_nbs(input).unwrap()
}
