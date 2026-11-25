---
name: tapped-layout
description: Make litematic projections from NBS songs with nbs-fractal. Use for the Tapped layout, match rules, TEC layers, and checking the result.
---

# Flow

Copy the song into `out/`. Drive the CLI, no code edits.

```bash
cargo build --release
NB=./target/release/nbs-fractal
TE=./target/release/nbs-fractal-tec
cp SONG.nbs out/NAME.nbs
$TE scan out/NAME.nbs --layers 2 --max-gap 102
$NB match out/NAME.nbs out/NAME_3l.tec.nbs -r A -r B
$NB tapped out/NAME_3l.tec.nbs out/tapped_3l.litematic -w 18 --max-gain 7
$TE verify out/NAME_3l.tec.nbs
```

One `-r` per rule layer, in `scan` order. `tapped` prints the box and each
layer's volume.

Run one layer per `scan`, all at once: a plan re-solves its prefix at every
layer. Background them and poll the files.

# Layers

`N`L counts the residual: `N-1` rules plus one leftover group. Ask `scan` for
`--layers N-1` and expect `N` groups.

The CLI counts rules, the name counts layers.

# Score

Reuse `Σ|K|·|S|` up. Volume down. Ties go to the smaller volume.

# Tap length

A tap's x length follows its delay, and one oversized tap spoils the build.

```
width = 4 + gain             // gain = columns past the bus column
delay <= 8 * gain + 13       // an exposing tap
delay <= 8 * gain + 17       // a middle tap, its back lane 4 rt deeper
```

The lanes take 4 rt each first, so an 11 rt tap still fits width 4.

Always pass `--max-gain 7`: width 11 holds 69 rt, 73 for a middle tap, so a
`-r 128` rule's 64 rt stays one tap. A wider gap rides a row of taps, timing
unchanged. Never omit it: unbounded, a scatter took 688 rt at 89 blocks wide.

Gaps are rt, rules are raw ticks. At tempo 18 two raw ticks make one rt, so
`-r 128` is 64 rt.

# Wrap

`-w` only reshapes the box; the TEC NBS fixes notes and reuse. Choose it last
and keep the smaller volume. Here `-w 18` gave 86400, `-w 24` 88704, both at
reuse 5658.

# Optimal

`scan` is a beam, not a proof. Widen `--top` and `--max-arity` until the
numbers stop moving; a small `--max-arity` hides wide scatters.

Per layer on 12 cores: `8/6` ~15 s, `16/10` ~55 s, `24/12` ~70 s, `32/16`
~110 s. Nearby tiers agreeing is a stop rule, not a proof.

No settings, no "optimal".

# Deliver

One line per projection: file, notes, reuse, volume. Then one row per layer:
rules, notes, reuse, volume.

Layer volumes are rebuilt alone and need not sum to the box: 18432 + 14208 +
28032 against 88704.

# Check

`verify` is the truth; `scan` runs a few notes high (5661/5658, 6040/6032,
6179/6169). Deliver `verify`.

Keep scatter gaps at 8 rt or more.
