# AGENTS.md

This is the **Rust reference implementation** of the OPO format. The rules of the whole project live in the standard repository: read [opo/AGENTS.md](https://github.com/opo-format/opo/blob/main/AGENTS.md), [docs/architecture.md](https://github.com/opo-format/opo/blob/main/docs/architecture.md) and [docs/workflow.md](https://github.com/opo-format/opo/blob/main/docs/workflow.md) first. They apply here too.

## Specific to this repository
- Rust only; stable toolchain; no C or C++ dependencies (decision 0015).
- No Google libraries or services.
- Behaviour is defined by the spec and its conformance suite, not by this code. If the code and the spec disagree, fix the code or propose a spec change in `opo`; never change behaviour silently.
- Every public function has a doc comment; every behaviour has a test.
- `just check` (format, clippy with warnings as errors, tests, conformance) must pass before a pull request.
- Conventional Commits; one concern per pull request.
