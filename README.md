# opo-rs

Rust reference implementation and command-line tool for the [OPO format](https://github.com/opo-format/opo) (Open Parametric Object).

> Status: starting. The plan is in [opo/docs/migration.md](https://github.com/opo-format/opo/blob/main/docs/migration.md), phase 3.

Planned crates:

| Crate | Does |
|---|---|
| `opo-feel` | FEEL-subset evaluator (formulas) |
| `opo-model` | Types for every `opo::` attribute |
| `opo-ifcx` | Read, write and compose IFCX documents and layers |
| `opo-recipe` | Geometry kernel that runs recipes |
| `opo-build` | Compile YAML object sources to `.opo` packages; generate class templates |
| `opo-check` | Validate objects |
| `opo-render` | Previews, plans and sections with hatches |
| `opo-cli` | The `opo` command |

Every crate must pass the conformance suite of the spec version it supports.

Licence: MIT OR Apache-2.0, at your option.
