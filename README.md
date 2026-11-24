# opo-rs

Rust reference implementation and command-line tool for the [OPO format](https://github.com/opo-format/opo) (Open Parametric Object).

> Status: phase 3 of [opo/docs/migration.md](https://github.com/opo-format/opo/blob/main/docs/migration.md). `opo-feel`, `opo-model`, `opo-ifcx` and `opo-recipe` pass the conformance suite; `opo-check`, `opo-build` and `opo-render` are next.

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

## Building and testing

You need stable Rust and [`just`](https://github.com/casey/just).

```sh
git clone --recurse-submodules https://github.com/opo-format/opo-rs
cd opo-rs
just check        # format, clippy (warnings are errors), tests, conformance
```

The standard repository is pinned as the submodule `opo-spec`; the conformance suite is read from it. To move to a newer spec version: `just pin-spec <commit>`, then `just check`, then commit `opo-spec`. Set `OPO_SPEC_DIR` to test against another checkout.

Licence: MIT OR Apache-2.0, at your option.
