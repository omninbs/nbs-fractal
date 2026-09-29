use clap::Parser;
use nbs_fractal::analysis::reuse::reuse_flow;
use nbs_fractal::analysis::{BoundedTec, TePlane, TransEqClass};
use nbs_fractal::schematic::{Layout, MultiCompactLayout, MultiLinearLayout};
use nbs_fractal::schematic::{StackedLinearLayout, TappedLayout, WithFloor};
use rsnbs::note::{Note, Notes, Tone};
use rsnbs::song::{Layer, Song};
use rsnbs::types::{LayerAnchor, Position, Tick, TimeAnchor};
use rustmatica::Litematic;
use std::collections::BTreeSet;
use std::num::NonZero;
use std::path::Path;
use std::str::FromStr;

// Cli
//
// ++++++++++++============++++++++++++============++++++++++++============

#[derive(Parser)]
#[command(
    name = "nbs-fractal",
    about = "Generate Minecraft litematic projections from NBS songs"
)]
enum Cli {
    Compact(Compact),
    Linear(Linear),
    Decompose(Decompose),
    Match(Match),
    Tapped(Tapped),
}

fn main() {
    match Cli::parse() {
        Cli::Compact(cmd) => cmd.run(),
        Cli::Linear(cmd) => cmd.run(),
        Cli::Decompose(cmd) => cmd.run(),
        Cli::Match(cmd) => cmd.run(),
        Cli::Tapped(cmd) => cmd.run(),
    }
}

// Compact
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Compact layout
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
    /// Block spacing between adjacent rows (0 = interlocked)
    #[arg(short, long, default_value_t = 0)]
    gap: u32,
    /// Add a full floor platform below the build
    #[arg(short, long)]
    full_floor: bool,
}

impl Compact {
    fn run(self) {
        let song = open_song(&self.input);
        let notes = song
            .notes
            .rescale_to_game_tick(song.header.tempo)
            .map(|(pos, note)| (pos.into_tick(), note));
        let tracks = std::iter::once((notes, NonZero::new(self.coarse)));
        let layout =
            MultiCompactLayout::new(tracks, NonZero::new(self.wrap), self.gap, self.full_floor);
        let description = format!("Compact from {}", self.input);
        let litematic = build_schematic(layout, Floor::None, description);
        write_output(&self.output, litematic);
    }
}

// Linear
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Linear layout
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
}

impl Linear {
    fn run(self) {
        let song = open_song(&self.input);
        let notes: Notes = song.notes.rescale_to_game_tick(song.header.tempo).collect();
        let song_length = notes
            .last_key_value()
            .map(|(pos, _)| pos.into_tick() + 1)
            .unwrap_or(0);
        let tracks: Vec<Notes> = notes.split_by_layer_gaps();
        let description = format!("Linear from {}", self.input);

        let litematic = if let Some(wrap) = NonZero::new(self.wrap) {
            let full = self.floor.full();
            let layout = StackedLinearLayout::new(tracks, Some(wrap), self.gap, full, song_length);
            build_schematic(layout, Floor::None, description)
        } else {
            let layout = MultiLinearLayout::new(tracks, self.gap, song_length);
            build_schematic(layout, self.floor, description)
        };
        write_output(&self.output, litematic);
    }
}

// Decompose
//
// ++++++++++++============++++++++++++============++++++++++++============

// **Experimental**: output may change.

/// Decompose an NBS song into automatically matched TEC groups.
#[derive(clap::Args)]
struct Decompose {
    /// Path to input NBS file
    input: String,
    /// Path to output NBS file
    #[arg(default_value = "out/generated_decompose.nbs")]
    output: String,
    /// Max number of layers (TECs) to generate; 0 = no budget
    #[arg(short, long, default_value_t = 2)]
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

        // 层数预算：分解在预算耗尽时停止；0 = 无预算（残差无任何同音色配对）。
        let max_layers = match self.layers {
            0 => usize::MAX,
            n => n,
        };
        let (plan, _, residual) = reuse_flow(&all_plane, 6, max_layers);

        // one layer group per TEC; the residual is kept as an offset-free TEC.
        let mut tecs: Vec<BoundedTec<Tone>> = plan.into_iter().map(BoundedTec::new).collect();
        if !residual.is_empty() {
            let rest = TransEqClass::new(BTreeSet::new(), residual);
            tecs.push(BoundedTec::new(rest));
        }

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

/// Decompose the song into TEC groups and re-emit it as an NBS.
///
/// Rule offsets are interpreted in the song's own (file) tick, so each TEC
/// group is written on its own layers, separated from the next by a blank
/// layer. Original layer assignments are discarded.
#[derive(clap::Args)]
struct Match {
    /// Path to input NBS file
    input: String,
    /// Path to output NBS file
    #[arg(default_value = "out/generated_match.nbs")]
    output: String,
    /// Match rule offsets, slash-separated; multiple rules in order
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
    Notes::<Position, Note>::pack_layers(points)
        .into_iter()
        .collect()
}

/// Rewrites `song` with one layer group per TEC and writes it as an NBS.
///
/// Each TEC's expansion is packed onto its own layers; `concat` leaves a
/// blank layer between groups. Original layer assignments are discarded.
fn write_tec_groups(mut song: Song, tecs: Vec<BoundedTec<Tone>>, output: &str, name: String) {
    let notes: Notes<Position, Note> = Notes::concat(tecs.into_iter().map(tec_notes)).collect();
    let last = notes.keys().map(|pos| pos.into_layer()).max();
    song.notes = notes;
    song.layers = vec![Layer::default(); last.map_or(0, |l| l as usize + 1)];
    song.header.song_name = name;
    write_song(output, song);
}

/// One match rule's offsets, slash-separated, e.g. "4/8".
#[derive(Clone, Debug)]
struct Rule(Vec<Tick>);

impl FromStr for Rule {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parse = |t: &str| t.parse().map_err(|_| format!("invalid tick: {t}"));
        let tokens = s.split(|c| matches!(c, '/' | ',' | ' '));
        let offsets: Result<_, _> = tokens.filter(|t| !t.is_empty()).map(parse).collect();
        Ok(Rule(offsets?))
    }
}

// Tapped
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Build a Tapped layout from a grouped NBS, as emitted by `match`/`decompose`.
///
/// The input's layers are TEC groups separated by blank layers.
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
    /// Block spacing between adjacent rows (0 = interlocked)
    #[arg(short, long, default_value_t = 0)]
    gap: u32,
    /// Add a full floor platform below the build
    #[arg(short, long)]
    full_floor: bool,
}

impl Tapped {
    fn run(self) {
        let song = open_song(&self.input);
        let notes: Notes = song
            .notes
            .rescale_to_redstone_tick(song.header.tempo)
            .collect();

        // one layer group per TEC; restore each group's widest scatter.
        let tecs: Vec<BoundedTec<Tone>> = notes
            .split_by_layer_gaps()
            .into_iter()
            .map(|group| BoundedTec::restore(&group.into_iter().collect()))
            .collect();

        let layout = TappedLayout::new(tecs, NonZero::new(self.wrap), self.gap, self.full_floor);
        let description = format!("Tapped from {}", self.input);
        let litematic = build_schematic(layout, Floor::None, description);
        write_output(&self.output, litematic);
    }
}

// Utils
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Builds the schematic, wrapping the layout in a floor platform when requested.
fn build_schematic<L: Layout>(layout: L, floor: Floor, description: String) -> Litematic {
    const AUTHOR: &str = "nbs-fractal";
    match floor {
        Floor::None => layout.as_litematic(description, AUTHOR),
        _ => WithFloor::new(layout, floor.full()).as_litematic(description, AUTHOR),
    }
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

/// Ensures the parent directory exists, writes the litematic, and reports it.
fn write_output(output: &str, litematic: Litematic) {
    ensure_parent(output);
    litematic.write_file(output).unwrap();
    eprintln!("Wrote {output}");
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
