# Run `just check` before every pull request.

# Format, lint, test and run the conformance suite.
check: fmt-check clippy test conformance

# Check formatting.
fmt-check:
    cargo fmt --all -- --check

# Format the code.
fmt:
    cargo fmt --all

# Lint; warnings are errors.
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Unit and doc tests (integration tests, which read the spec, run in `conformance`).
test:
    cargo test --workspace --lib --bins
    cargo test --workspace --doc

# Run the conformance suite and the other integration tests against the pinned spec
# version (submodule `opo-spec`) and the IFC data of `just ifc`.
conformance: ifc
    git submodule update --init opo-spec
    cargo test --workspace --tests

# IFC is published under CC BY-ND 4.0, so no IFC data is committed: fetch the UML sources of
# buildingSMART/IFC4.x-development at the commit in ifc-version into target/ifc-src and let
# `opo ifc` write target/ifc/elements.json and psets.json (the tests read them).
ifc:
    #!/usr/bin/env bash
    set -euo pipefail
    rev=$(cat ifc-version)
    src=target/ifc-src/$rev
    if [ ! -d "$src/schemas" ]; then
        rm -rf "$src" && mkdir -p "$src"
        git -C "$src" init -q
        git -C "$src" fetch -q --depth 1 --filter=blob:none https://github.com/buildingSMART/IFC4.x-development "$rev"
        git -C "$src" checkout -q FETCH_HEAD -- schemas
    fi
    if [ ! -f target/ifc/psets.json ] || [ ifc-version -nt target/ifc/psets.json ]; then
        cargo run -q -p opo-cli --bin opo -- ifc "$src/schemas" -o target/ifc
    fi

# Move the pinned spec version to a commit of the standard repository.
pin-spec commit:
    git -C opo-spec fetch origin
    git -C opo-spec checkout {{commit}}
    @echo "Now run 'just check' and commit opo-spec."
