use clap::Parser;
use itertools::Either;
use nbs_fractal::analysis::reuse::{plan_to_tecs, reuse_flow};
use nbs_fractal::analysis::{BoundedTec, TePlane, TransEqClass};
use nbs_fractal::layout::Metadata;
use nbs_fractal::layout::compose::WithFloor;
use nbs_fractal::layout::linear::{MultiLinearLayout, StackedLinearLayout};
use nbs_fractal::layout::{LayoutExt, torrent::TorrentLayout};
#[allow(deprecated)]
use nbs_fractal::layout::{cascade::CascadeLayout, compact::MultiCompactLayout};
use nbs_fractal::layout::{fishbone::MultiFishboneLayout, tapped::TappedLayout};
use rsnbs::note::{Note, NoteStreamExt, Notes, Tone};
use rsnbs::song::{Layer, Song};
use rsnbs::types::{Position, Tick, TimeAnchor};
use std::{collections::BTreeSet, fs::File, num::NonZero, path::Path, str::FromStr};

// Cli
//
// ++++++++++++============++++++++++++============++++++++++++============

#[derive(Parser)]
#[command(
    name = "nbs-fractal",
    about = "Generate Minecraft projections from NBS songs"
)]
enum Cli {
    Compact(Compact),
    Linear(Linear),
    Fishbone(Fishbone),
    Cascade(Cascade),
    Torrent(Torrent),
    Decompose(Decompose),
    Match(Match),
    Tapped(Tapped),
}

fn main() {
    match Cli::parse() {
        Cli::Compact(cmd) => cmd.run(),
        Cli::Linear(cmd) => cmd.run(),
        Cli::Fishbone(cmd) => cmd.run(),
        Cli::Cascade(cmd) => cmd.run(),
        Cli::Torrent(cmd) => cmd.run(),
        Cli::Decompose(cmd) => cmd.run(),
        Cli::Match(cmd) => cmd.run(),
        Cli::Tapped(cmd) => cmd.run(),
    }
}

// Compact
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Layout (Compact): Compact variable-length packed structure.
///
/// Footprint first: the event stream is packed into densely stacked cells,
/// giving a variable-length structure with no wasted gaps. It supports periodic
/// signals and accommodates Java Edition's terminal-repeater behaviour.
#[derive(clap::Args)]
struct Compact {
    /// Path to input NBS file
    input: String,
    /// Path to output litematic file
    #[arg(default_value = "out/generated_compact.litematic")]
    output: String,
    /// Max columns per row before wrapping (0 = no wrap)
    #[arg(short, long, default_value_t = 16)]
    wrap: usize,
    /// Repeater delay coarseness 1-4 (0 = unlimited)
    #[arg(short, long, default_value_t = 0)]
    coarse: u32,
    /// Block spacing between adjacent tracks (0 = interlocked)
    #[arg(short, long, default_value_t = 0)]
    gap: u32,
    /// Add a full floor platform below the build
    #[arg(short, long)]
    full_floor: bool,
    /// Split even/odd game ticks into separate tracks
    #[arg(long)]
    halves: bool,
    /// Split the song into tracks on blank layers
    #[arg(short = 'b', long)]
    breaks: bool,
}

impl Compact {
    fn run(self) {
        let song = open_song(&self.input);
        let tempo = song.header.tempo;
        let coarse = NonZero::new(self.coarse);
        let groups = match self.breaks {
            true => song.notes.split_by_layer_gaps(),
            false => vec![song.notes],
        };
        let tracks = groups
            .into_iter()
            .flat_map(|group| group_lines(group, tempo, self.halves, coarse));
        let wrap_length = NonZero::new(self.wrap);
        let layout = MultiCompactLayout::new(tracks, wrap_length, self.gap, self.full_floor);
        let description = format!("Compact from {}", self.input);
        write_schematic(layout, Floor::None, description, &self.output);
    }
}

/// Rescales one note group and breaks it into coarseness-tagged redstone lines.
///
/// With `halves` the group is rescaled to game ticks (20 t/s) and the two game
/// ticks sharing a redstone tick go to separate lines; otherwise it is rescaled
/// to redstone ticks (10 t/s) and collapses onto a single line.
fn group_lines(
    group: Notes,
    tempo: f32,
    halves: bool,
    coarse: Option<NonZero<Tick>>,
) -> impl Iterator<Item = (Vec<(Tick, Note)>, Option<NonZero<Tick>>)> {
    let rate = if halves { 20 } else { 10 };
    let notes = group.rescale_to_tick_rate(tempo, rate);
    let notes = notes.map(|(pos, note)| (pos.into_tick(), note));
    let lines = match halves {
        true => split_halves(notes).to_vec(),
        false => vec![notes.collect()],
    };
    lines
        .into_iter()
        .filter(|line| !line.is_empty())
        .map(move |line| (line, coarse))
}

/// Splits a game-tick note stream into two redstone lines by tick parity.
fn split_halves<I, T>(notes: I) -> [Vec<(Tick, T)>; 2]
where
    I: IntoIterator<Item = (Tick, T)>,
{
    let mut lines: [Vec<(Tick, T)>; 2] = Default::default();
    for (tick, note) in notes {
        lines[(tick % 2) as usize].push((tick / 2, note));
    }
    lines
}

// Linear
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Layout (Linear): Foldable timeline with side branches.
///
/// Each backbone can carry one unit of branch, played evenly along time,
/// supporting 20 t/s. The wrap option folds it.
#[derive(clap::Args)]
struct Linear {
    /// Path to input NBS file
    input: String,
    /// Path to output litematic file
    #[arg(default_value = "out/generated_linear.litematic")]
    output: String,
    /// Block spacing between adjacent tracks (0 = interlocked)
    #[arg(short, long, default_value_t = 0)]
    gap: u32,
    /// Max columns per row before wrapping (0 = no wrap)
    #[arg(short, long, default_value_t = 0)]
    wrap: usize,
    /// Floor platform mode
    #[arg(short = 'F', long, value_enum, default_value_t)]
    floor: Floor,
    /// Keep the song on one track, ignoring blank layers
    #[arg(short = 'B', long = "no-breaks")]
    no_breaks: bool,
}

impl Linear {
    fn run(self) {
        let song = open_song(&self.input);
        let notes: Notes = song.notes.rescale_to_game_tick(song.header.tempo).collect();
        let song_length = notes.tick_count();
        let tracks: Vec<Notes> = match self.no_breaks {
            true => vec![notes],
            false => notes.split_by_layer_gaps(),
        };
        let description = format!("Linear from {}", self.input);

        if let Some(wrap) = NonZero::new(self.wrap) {
            let full = self.floor.full();
            let layout = StackedLinearLayout::new(tracks, Some(wrap), self.gap, full, song_length);
            write_schematic(layout, Floor::None, description, &self.output);
        } else {
            let layout = MultiLinearLayout::new(tracks, self.gap, song_length);
            write_schematic(layout, self.floor, description, &self.output);
        }
    }
}

// Fishbone
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Layout (Fishbone): Linear timeline with side branches.
///
/// Each backbone can carry three units of branches, played evenly along time.
#[derive(clap::Args)]
struct Fishbone {
    /// Path to input NBS file
    input: String,
    /// Path to output litematic file
    #[arg(default_value = "out/generated_fishbone.litematic")]
    output: String,
    /// Block spacing between adjacent tracks (0 = interlocked)
    #[arg(short, long, default_value_t = 0)]
    gap: u32,
    /// Floor platform mode
    #[arg(short = 'F', long, value_enum, default_value_t)]
    floor: Floor,
    /// Keep the song on one track, ignoring blank layers
    #[arg(short = 'B', long = "no-breaks")]
    no_breaks: bool,
}

impl Fishbone {
    fn run(self) {
        let song = open_song(&self.input);
        let notes: Notes = song.notes;
        let song_length = notes.tick_count();
        let tracks: Vec<Notes> = match self.no_breaks {
            true => vec![notes],
            false => notes.split_by_layer_gaps(),
        };
        let layout = MultiFishboneLayout::new(tracks, self.gap, song_length);
        let description = format!("Fishbone from {}", self.input);
        write_schematic(layout, self.floor, description, &self.output);
    }
}

// Cascade
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Layout (Cascade): Multi-track timeline with pan and volume.
///
/// Deprecated in favour of the torrent layout.
///
/// Multiple tracks run in parallel, played evenly along time. The design can
/// reproduce volume and panning.
#[derive(clap::Args)]
struct Cascade {
    /// Path to input NBS file
    input: String,
    /// Path to output litematic file
    #[arg(default_value = "out/generated_cascade.litematic")]
    output: String,
    /// Floor platform mode
    #[arg(short = 'F', long, value_enum, default_value_t)]
    floor: Floor,
}

impl Cascade {
    #[allow(deprecated)]
    fn run(self) {
        let song = open_song(&self.input);
        let notes = song.resolved_notes();
        let layout = CascadeLayout::new(notes);
        let description = format!("Cascade from {}", self.input);
        write_schematic(layout, self.floor, description, &self.output);
    }
}

// Torrent
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Layout (Torrent): Tunnel of slices with pan and volume.
///
/// A tunnel of whole slices laid tick by tick, read from one fixed viewpoint, so
/// a voice lands where it is heard. The design can reproduce volume and panning
/// across a wide field.
#[derive(clap::Args)]
struct Torrent {
    /// Path to input NBS file
    input: String,
    /// Path to output litematic file
    #[arg(default_value = "out/generated_torrent.litematic")]
    output: String,
    /// Keep only the notes before this many ticks
    #[arg(short = 't', long)]
    ticks: Option<u32>,
    /// Load every track, even ones that never sound
    #[arg(long)]
    all_tracks: bool,
    /// Track centres relative to the axis, e.g. `--track "-8,12"`; replaces the
    /// built-in set. East is negative x and up is positive y.
    #[arg(long = "track", value_name = "x,y")]
    tracks: Vec<Track>,
}

impl Torrent {
    fn run(self) {
        let song = open_song(&self.input);
        let limit = self.ticks.unwrap_or(u32::MAX);
        let notes = song
            .resolved_notes()
            .filter(|(pos, _)| pos.into_tick() < limit);
        let tracks = match self.tracks.is_empty() {
            true => Either::Left(TorrentLayout::DEFAULT_TRACKS.into_iter()),
            false => Either::Right(self.tracks.iter().map(|t| (t.0, t.1))),
        };
        let layout = TorrentLayout::new(notes, tracks, !self.all_tracks);
        let description = format!("Torrent from {}", self.input);
        write_schematic(layout, Floor::None, description, &self.output);
    }
}

/// One track centre relative to the torrent axis, two whole numbers split by any
/// non-digit but the sign, e.g. "-8/12". East is negative x and up is positive y.
#[derive(Clone, Debug)]
struct Track(i32, i32);

impl FromStr for Track {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use itertools::Itertools;
        let parse = |t: &str| t.parse().map_err(|_| format!("invalid number: {t}"));
        let (x, y) = s
            .split(|c: char| !c.is_ascii_digit() && c != '-')
            .filter(|t| !t.is_empty())
            .collect_tuple()
            .ok_or(format!("expected x,y: {s}"))?;
        Ok(Track(parse(x)?, parse(y)?))
    }
}

// Decompose
//
// ++++++++++++============++++++++++++============++++++++++++============

// **Experimental**: output may change.

/// Automatically decompose NBS files into TEC groups.
///
/// Decomposes translation equivalence classes automatically, and saves them in
/// NBS format.
#[derive(clap::Args)]
struct Decompose {
    /// Path to input NBS file
    input: String,
    /// Path to output NBS file
    #[arg(default_value = "out/generated_decompose.tec.nbs")]
    output: String,
    /// Max number of layers (TECs) to generate (0 = no budget)
    #[arg(short, long, default_value_t = 3)]
    layers: usize,
}

impl Decompose {
    fn run(self) {
        let song = open_song(&self.input);

        // match on the raw file ticks: the output NBS keeps the original timing.
        let points = song
            .notes
            .iter()
            .map(|(pos, note)| (pos.into_tick(), note.tone));
        let all_plane: TePlane<Tone> = points.collect();

        // Layer budget: decomposition stops once the budget runs out; 0 = no
        // budget (the residual keeps no same-tone pairing).
        let max_layers = match self.layers {
            0 => usize::MAX,
            n => n,
        };
        // A tapped layer needs a scatter gap of at least 8 redstone ticks
        // (repeater coarse >= 4), and the layers are rescaled before that
        // layout reads them, so the budget is spent in the rescaled domain.
        // Layers still under that gap are absorbed back into the residual.
        let min_step = redstone_step(&song, 8);
        let (plan, _, residual) = reuse_flow(&all_plane, 6, max_layers, min_step);
        let (tecs, skipped) = plan_to_tecs(plan, residual, min_step);
        if skipped > 0 {
            eprintln!("decompose: skipped {skipped} layer(s) below tapped granularity");
        }
        let tecs: Vec<BoundedTec<Tone>> = tecs.into_iter().map(BoundedTec::new).collect();

        write_tec_groups(
            song,
            tecs,
            &self.output,
            format!("Decompose from {}", self.input),
        );
    }
}

// Match
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Manually decompose NBS files into TEC groups.
///
/// Decomposes translation equivalence classes from manually entered match
/// rules, and saves them in NBS format.
#[derive(clap::Args)]
struct Match {
    /// Path to input NBS file
    input: String,
    /// Path to output NBS file
    #[arg(default_value = "out/generated_match.tec.nbs")]
    output: String,
    /// Match rule offsets, split by any non-digit; multiple rules in order
    #[arg(short, long, num_args = 1..)]
    rules: Vec<Rule>,
}

impl Match {
    fn run(self) {
        let song = open_song(&self.input);

        // match on the raw file ticks: the output NBS keeps the original timing.
        let points = song
            .notes
            .iter()
            .map(|(pos, note)| (pos.into_tick(), note.tone));
        let mut residual: TePlane<Tone> = points.collect();

        // one TEC per rule, extracting from the shared residual in order.
        let mut tecs: Vec<BoundedTec<Tone>> = self
            .rules
            .into_iter()
            .map(|rule| extract_tec(&mut residual, rule))
            .collect();

        // keep the rest.
        if !residual.is_empty() {
            let rest = TransEqClass::new(BTreeSet::new(), residual);
            tecs.push(BoundedTec::new(rest));
        }

        // one layer group per TEC; `concat` leaves a blank layer between groups.
        write_tec_groups(
            song,
            tecs,
            &self.output,
            format!("Match from {}", self.input),
        );
    }
}

/// Extracts one TEC from `residual` under a rule's offsets.
fn extract_tec(residual: &mut TePlane<Tone>, Rule(scatter): Rule) -> BoundedTec<Tone> {
    let scatter = scatter.into_iter().filter_map(NonZero::new).collect();
    BoundedTec::extract_from(residual, scatter)
}

/// Packs a TEC's expansion onto its own layers.
fn tec_notes(tec: BoundedTec<Tone>) -> Notes<Position, Note> {
    let points = tec
        .into_inner()
        .expand()
        .into_points()
        .map(|(tick, tone)| (tick, Note::from(tone)));
    points.pack_layers().collect()
}

/// Rewrites `song` with one layer group per TEC and writes it as an NBS.
///
/// Each TEC's expansion is packed onto its own layers; `concat` leaves a
/// blank layer between groups. Original layer assignments are discarded.
fn write_tec_groups(mut song: Song, tecs: Vec<BoundedTec<Tone>>, output: &str, name: String) {
    let notes: Notes<Position, Note> = tecs.into_iter().map(tec_notes).concat().collect();
    let layers = notes.layer_count() as usize;
    song.notes = notes;
    song.layers = vec![Layer::default(); layers];
    song.header.song_name = name;
    write_song(output, song);
}

/// One match rule's offsets, split by any non-digit, e.g. "4,8".
#[derive(Clone, Debug)]
struct Rule(Vec<Tick>);

impl FromStr for Rule {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parse = |t: &str| t.parse().map_err(|_| format!("invalid tick: {t}"));
        let tokens = s.split(|c: char| !c.is_ascii_digit());
        let offsets: Result<_, _> = tokens.filter(|t| !t.is_empty()).map(parse).collect();
        Ok(Rule(offsets?))
    }
}

// Tapped
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Layout (Tapped): Tapped delay line simulating TEC.
///
/// A direct descriptive structure for the translation equivalence class design:
/// a tapped delay line controls the repetition pattern, using the Compact layout
/// as its playing unit.
#[derive(clap::Args)]
struct Tapped {
    /// Path to input NBS file
    input: String,
    /// Path to output litematic file
    #[arg(default_value = "out/generated_tapped.litematic")]
    output: String,
    /// Max columns per row before wrapping (0 = no wrap)
    #[arg(short, long, default_value_t = 16)]
    wrap: usize,
    /// Block spacing between adjacent tracks (0 = interlocked)
    #[arg(short, long, default_value_t = 0)]
    gap: u32,
    /// Add a full floor platform below the build
    #[arg(short, long)]
    full_floor: bool,
    /// Keep every playing line at the base pitch instead of aligning rows
    #[arg(long = "no-balance")]
    no_balance: bool,
}

impl Tapped {
    fn run(self) {
        let song = open_song(&self.input);

        // Split into TEC groups on the native ticks, then rescale each group as a
        // `(Tick, Event)` stream into redstone ticks before restoring. Rescaling
        // first is fine as long as multiplicities are kept: folding into a
        // position-keyed `Notes` map would overwrite the notes that collapse
        // onto one redstone tick and so erase the `K (+) S` factor.
        let restore = |group: Notes| {
            let notes = group.into_iter().map(|(p, n)| (p, n.tone));
            let notes: Notes<Position, Tone> = notes.collect();
            let ticks = notes.rescale_to_redstone_tick(song.header.tempo);
            let plane: TePlane<Tone> = ticks.map(|(p, t)| (p.into_tick(), t)).collect();
            BoundedTec::restore(&plane)
        };
        let tecs: Vec<BoundedTec<Tone>> = song
            .notes
            .split_by_layer_gaps()
            .into_iter()
            .map(restore)
            .collect();

        let layout = TappedLayout::new(
            tecs,
            NonZero::new(self.wrap),
            self.gap,
            self.full_floor,
            !self.no_balance,
        );
        let description = format!("Tapped from {}", self.input);
        write_schematic(layout, Floor::None, description, &self.output);
    }
}

// Utils
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Writes the layout as a streaming litematic projection, wrapping the layout in
/// a floor platform when requested, and reports it.
fn write_schematic<L: LayoutExt>(layout: L, floor: Floor, description: String, output: &str) {
    const NAME: &str = "Note Block Track Schematic";
    const AUTHOR: &str = "nbs-fractal";
    let metadata = Metadata {
        name: NAME,
        description: &description,
        author: AUTHOR,
    };
    ensure_parent(output);
    let file = File::create(output).unwrap();
    match floor {
        Floor::None => layout.as_litematic(metadata, file),
        _ => WithFloor::new(layout, floor.full()).as_litematic(metadata, file),
    }
    .unwrap();
    eprintln!("Wrote {output}");
}

/// Floor platform mode.
#[derive(Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
enum Floor {
    /// No floor platform.
    #[default]
    None,
    /// Full coverage platform.
    Full,
    /// Platform only below gravity blocks.
    Gravity,
}

impl Floor {
    /// Full-coverage flag for layouts that always carry a floor.
    fn full(self) -> bool {
        matches!(self, Floor::Full)
    }
}

/// Loads the input song.
fn open_song(input: &str) -> Song {
    Song::open_nbs(input).unwrap()
}

/// The smallest raw tick step that spans at least `min` redstone ticks once
/// `song` is rescaled, so a tapped layer built on it stays realizable.
fn redstone_step(song: &Song, min: Tick) -> Tick {
    let tempo = song.header.tempo;
    let gap = |raw: Tick| {
        let mut ticks = [(0, ()), (raw, ())]
            .into_iter()
            .rescale_to_redstone_tick(tempo);
        let first = ticks.next().expect("probe has two ticks").0;
        ticks.next().expect("probe has two ticks").0 - first
    };
    (min..)
        .find(|&raw| gap(raw) >= min)
        .expect("step grows without bound")
}

/// Ensures the parent directory exists, writes the song as NBS, and reports it.
fn write_song(output: &str, mut song: Song) {
    ensure_parent(output);
    song.save_nbs(output).unwrap();
    eprintln!("Wrote {output}");
}

/// Creates the output's parent directory when it has one.
fn ensure_parent(output: &str) {
    let parent = Path::new(output)
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty());
    if let Some(dir) = parent {
        std::fs::create_dir_all(dir).unwrap();
    }
}
