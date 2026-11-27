# Contributing to uniflow

[DESIGN.md](DESIGN.md) describes the architecture and [ROADMAP.md](ROADMAP.md) what is
done and what is planned.

## Setting up

The repo has a Nix flake whose dev shell has everything you need: the stable Rust
toolchain (with `rust-src` and `rust-analyzer`), [`just`](https://github.com/casey/just),
and [uv](https://docs.astral.sh/uv/) with Python for the git hooks. On entry, the shell runs
`uv sync` to build `.venv/` from `uv.lock` and puts its tools on `PATH`.

```sh
nix develop
```

If you use [direnv](https://direnv.net/), the `.envrc` enters that shell for you whenever
you `cd` into the repo. Approve it once:

```sh
direnv allow
```

[nix-direnv](https://github.com/nix-community/nix-direnv) is optional, but it caches the
shell so entering the directory stays fast. Put any personal settings in `.envrc.local`,
which git ignores.

You don't need Nix. A stable Rust toolchain and `just` are enough to build and test. For
the hooks, also install uv and run `uv sync`, then `source .venv/bin/activate`.

`just` with no arguments lists the development commands.

## Git hooks

The repo uses [pre-commit](https://pre-commit.com) to run checks on every commit. The
configuration is in [.pre-commit-config.yaml](.pre-commit-config.yaml). The Nix dev shell
installs the hooks when you enter it, if they are missing. The first commit afterwards
takes a minute while pre-commit builds the hook environments.

To manage the hooks yourself, set `UNIFLOW_SKIP_HOOKS=1` (for example in `.envrc.local`)
and use `just install-hooks` and `just uninstall-hooks`. Without Nix, run
`just install-hooks` once.

There are two hooks:

- **On commit**, the hooks check the staged files:
  - the standard [pre-commit-hooks](https://github.com/pre-commit/pre-commit-hooks)
    checks: merge conflict markers, large files, private keys, case conflicts, symlinks,
    shebangs, valid TOML and YAML, line endings, trailing whitespace and final newlines;
  - `cargo fmt` for Rust;
  - [Prettier](https://prettier.io) for Markdown;
  - [taplo](https://taplo.tamasfe.dev) for TOML, configured in [.taplo.toml](.taplo.toml);
  - [codespell](https://github.com/codespell-project/codespell) for spelling. It fixes
    what it finds. Add any word it gets wrong for this repo to
    [codespell-ignore](codespell-ignore), one per line, in lowercase.
- **On the commit message**, a hook checks that it follows
  [Conventional Commits](https://www.conventionalcommits.org): `type(scope): description`,
  for example `feat(reader): adds lens zoom`. codespell also fixes typos in the
  message. [COMMIT_MESSAGE_TEMPLATE](COMMIT_MESSAGE_TEMPLATE) lists the types. The Nix dev
  shell makes it this clone's commit template (`commit.template` in `.git/config`), so git
  opens your editor with it. It leaves a template you have already set for this clone
  alone. Without Nix, run `git config commit.template COMMIT_MESSAGE_TEMPLATE` once.

The formatters rewrite files in place. When a hook changes something, the commit stops:
look at the changes, stage them and commit again. `just check` runs every hook over the
whole repo, and `just uninstall-hooks` removes them.

## Continuous integration

[The Rust workflow](.github/workflows/rust.yml) runs on every push to `main` and every pull
request. It runs `cargo test` on the latest Ubuntu, macOS and Windows runners, and checks
`cargo fmt` and `cargo clippy` (with warnings as errors) on Ubuntu. `just test` and
`just lint` run the same checks locally.
