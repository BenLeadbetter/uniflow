# Development entry points. `just` with no arguments lists them.

default:
    @just --list

# Run every test, with all features enabled
test:
    cargo test --all-features

# Check formatting and run clippy over every target, as CI does
lint:
    cargo fmt --all --check
    cargo clippy --all-targets --all-features -- -D warnings

# Run an example; e.g. `just run-example counter`
run-example name *args:
    cargo run --example {{name}} -- {{args}}

# Install the git hooks (see CONTRIBUTING.md)
install-hooks:
    pre-commit install --install-hooks
    pre-commit install --install-hooks -t commit-msg

# Remove the git hooks
uninstall-hooks:
    pre-commit uninstall
    pre-commit uninstall -t commit-msg

# Run every git hook against every file
check:
    pre-commit run --all-files
