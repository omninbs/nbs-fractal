---
name: tapped-layout
description: Make litematic projections from NBS songs with nbs-fractal. Use for the Tapped layout, match rules, TEC layers, and checking the result.
---

# Flow

Copy the song into `out/`. Drive the CLI, no code edits.

```bash
cargo build --release
cp SONG.nbs out/NAME.nbs
./target/release/nbs-fractal-tec scan out/NAME.nbs --layers 2,3
./target/release/nbs-fractal match out/NAME.nbs out/NAME_2l.tec.nbs -r A -r B
./target/release/nbs-fractal tapped out/NAME_2l.tec.nbs out/NAME_tapped_2l.litematic -w 18
./target/release/nbs-fractal-tec verify out/NAME_2l.tec.nbs
```

One `-r` per rule layer, in `scan` order.

# Layers

`N`L counts every layer in the TEC NBS, the residual included: `N-1` rules
plus one leftover group. So ask `scan` for `--layers N-1`, and expect `N`
groups.

The CLI counts rules, the name counts layers. Only the name shifts.

# Score

Reuse `Σ|K|·|S|` up. Volume down. Tie goes to the smaller volume.

# Optimal

`scan` is a beam, not a proof. Widen `--top` and `--max-arity` until the
numbers stop moving. A small `--max-arity` hides wide scatters, which is the
usual wrong answer.

No settings, no "optimal".

# Deliver

Song note count, search settings, then per layer: rules, notes, reuse,
volume.

# Check

`verify` is the truth. `scan` only estimates. Keep scatter gaps at 8 redstone
ticks or more, and quote `-r` in raw ticks.
