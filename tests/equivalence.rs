//! The adapted writer must round-trip through `litematica`'s reader.

use itertools::iproduct;
use litematica::model::document::Document;
use litematica::task::decompress::gzip_with_limit;
use nbs_fractal::layout::{Layout, LayoutExt, Metadata};
use nbs_fractal::{BlockPos, BlockState};

const NAME: &str = "Note Block Track Schematic";

/// A block state from a name and a list of properties.
fn block(name: &'static str, props: &[(&'static str, &'static str)]) -> BlockState {
    BlockState {
        name: name.into(),
        properties: props
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect(),
    }
}

/// A tiny layout exercising note blocks, instruments, and circuit blocks.
struct Sample;

const SIZE: BlockPos = BlockPos::new(4, 3, 3);

impl Layout<()> for Sample {
    fn block_at_with(&self, pos: BlockPos, _ctx: &()) -> Option<BlockState> {
        let harp = [("instrument", "harp"), ("note", "12"), ("powered", "false")];
        let repeater = [
            ("delay", "1"),
            ("facing", "north"),
            ("locked", "false"),
            ("powered", "false"),
        ];
        match (pos.x, pos.y, pos.z) {
            (1, 1, 1) => Some(block("minecraft:note_block", &harp)),
            (1, 0, 1) => Some(block("minecraft:dirt", &[])),
            (2, 0, 2) => Some(block("minecraft:repeater", &repeater)),
            (0, 2, 0) => Some(block("minecraft:barrier", &[("waterlogged", "false")])),
            _ => None,
        }
    }

    fn size_with(&self, _ctx: &()) -> BlockPos {
        SIZE
    }
}

/// The metadata the writer is given.
fn metadata() -> Metadata<'static> {
    Metadata {
        name: NAME,
        description: "Equivalence",
        author: "nbs-fractal",
    }
}

/// The bytes the adapted writer produces.
fn written(layout: &Sample) -> Vec<u8> {
    let mut bytes = Vec::new();
    layout.as_litematic(metadata(), &mut bytes).unwrap();
    bytes
}

/// Every cell's block, read back through `litematica`, in writing order.
fn cells(bytes: &[u8]) -> Vec<BlockState> {
    let nbt = gzip_with_limit(bytes, 1 << 20).unwrap();
    let document = Document::parse(&nbt).unwrap();
    let region = &document.regions[0];
    let size = region.size();
    iproduct!(0..size.y, 0..size.z, 0..size.x)
        .map(|(y, z, x)| {
            let pos = BlockPos::new(x as i32, y as i32, z as i32);
            region
                .state_at(pos)
                .cloned()
                .expect("position is in bounds")
        })
        .collect()
}

#[test]
fn round_trips_through_litematica() {
    let layout = Sample;
    let expected: Vec<BlockState> = iproduct!(0..SIZE.y, 0..SIZE.z, 0..SIZE.x)
        .map(|(y, z, x)| layout.get_block_or_air(BlockPos::new(x, y, z)))
        .collect();
    assert_eq!(cells(&written(&layout)), expected);
}
