# nbs-fractal

[![Crates.io Version](https://img.shields.io/crates/v/nbs-fractal?style=flat-square&logo=rust&labelColor=363a4f&color=f5a97f)](https://crates.io/crates/nbs-fractal)
[![Crates.io Dependents](https://img.shields.io/crates/dependents/nbs-fractal?style=flat-square&logo=rust&labelColor=363a4f&color=ee99a0)](https://crates.io/crates/nbs-fractal/reverse_dependencies)
[![GitHub last commit](https://img.shields.io/github/last-commit/omninbs/nbs-fractal?style=flat-square&logo=github&labelColor=363a4f&color=f5bde6)](https://github.com/omninbs/nbs-fractal/activity)
[![GitHub Repo stars](https://img.shields.io/github/stars/omninbs/nbs-fractal?style=flat-square&logo=github&labelColor=363a4f&color=c6a0f6)](https://github.com/omninbs/nbs-fractal)

It is a concrete implementation of the "_formal reduction of redstone music_", pioneeringly validating the feasibility of its formal logic and layout structures.


```bash
# run the following cargo command in your project directory:
cargo install nbs-fractal --version ">=0.1.2, <0.2.0"
```

## Scope of this work

The main part of this crate is the **analysis and reconstruction of repetitive structures**. As a research branch of formal reduction, the formal rule of repetition has a relatively intuitive representation. But as one kind of formal rule, it does not represent the whole of formal reduction.

Since the crate already handles the generation of structures, a few simple structural layouts for traditional note block music are implemented along the way, purely to test how broadly the framework applies. These are not a focus of ongoing maintenance or development.

## Contributors

See [CONTRIBUTORS.md](CONTRIBUTORS.md) for the full list of contributors to this project.
