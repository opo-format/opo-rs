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

# Unit and doc tests (the conformance tests run in `conformance`).
test:
    cargo test --workspace --lib
    cargo test --workspace --doc

# Run the conformance suite of the pinned spec version (submodule `opo-spec`).
conformance:
    git submodule update --init opo-spec
    cargo test --workspace --test conformance

# Move the pinned spec version to a commit of the standard repository.
pin-spec commit:
    git -C opo-spec fetch origin
    git -C opo-spec checkout {{commit}}
    @echo "Now run 'just check' and commit opo-spec."
