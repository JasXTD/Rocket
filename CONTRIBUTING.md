# Contributing to Rocket

Thanks for helping. Rocket is a small project with one maintainer, so agreeing on a change
early saves everyone time.

## Before you start

- **Bugs:** search the existing issues, then open one with the bug report form.
- **Features and UI changes:** open an issue first. Describe the problem, ideally with a
  screenshot or sketch, before you write code.
- **Security problems:** don't open an issue. Report them privately as
  [SECURITY.md](SECURITY.md) describes.

## Build

Follow [Build and run](README.md#build-and-run) for your platform. The build scripts bind
Rocket to its Lua worker; plain `cargo build` works too, but turns extensions off.

## Make the change

- Keep a pull request to one change, and match the style of the code around it. Don't
  reformat code you aren't changing.
- Add or update tests for behaviour you change, and run the tests for that area.
  [tools/README.md](tools/README.md) lists the focused tests, including the localhost MCP
  runners that drive the real window.
- Discuss a new dependency in an issue first. Every crate and bundled runtime is reviewed
  and recorded in [DEPENDENCIES.md](DEPENDENCIES.md).
- Never commit real tokens, keys or private repository content, including in test fixtures.

## Documentation

- Record user-visible changes in the top entry of [CHANGELOG.md](CHANGELOG.md).
- Update the [user guide](crates/rocket-ui/src/guide.md) when a workflow changes, and the
  [CLI reference](CLI.md) or [HOSTING.md](HOSTING.md) when those change.
- Keep app text plain: short sentences, the names shown in the app, and no trailing "…" on
  command labels.

## Commits and pull requests

- Write commit subjects in the imperative and in sentence case, without a full stop, for
  example "Add Fetch all to the Repository manager".
- In the pull request, say what you changed, why, and how you tested it.

## License

Rocket is licensed under either [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your
option. Unless you say otherwise, any contribution you submit is licensed the same way,
without additional terms or conditions.

By taking part, you agree to follow the [code of conduct](CODE_OF_CONDUCT.md).
