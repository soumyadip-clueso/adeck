# adeck's entry points. `just` lists them.

default:
    @just --list

# The app with live editing: UI edits patch into the open window (scripts/rune-hot).
hot *ARGS:
    scripts/rune-hot -- {{ARGS}}

# The app, plain.
run *ARGS:
    cargo run --release -- {{ARGS}}

# Clippy on every save, in its own pane.
check:
    bacon clippy

# Unit and UI tests (rune's in-process Runtime), then every headless scene.
test:
    cargo nextest run
    just snaps

# One headless scene to a PNG: just snap compare
snap SCENE OUT="target/snaps":
    mkdir -p {{OUT}}
    ADECK_HOME=target/snap-home ADECK_CACHE=target/snap-cache cargo run -q -- --snap {{OUT}}/{{SCENE}}.png --scene {{SCENE}} --size 1280x800 --scale 1

# Every scene, into target/snaps/.
snaps:
    for s in empty single compare blind run board pads palette; do just snap $s; done

# Move rune to the tip of its default branch (or a given commit), then lint, test and snap: just bump-rune [sha]
bump-rune *SHA:
    scripts/bump-rune {{SHA}}

# Lint once, as CI would.
lint:
    cargo clippy --all-targets -- -D warnings
    cargo fmt --check

# Build a release and install it as ~/Applications/adeck.app (run again to update)
install:
    scripts/install

# Free the dev build (keeps the release build and the dependency cache)
clean-dev:
    rm -rf target/debug target/snap-cache target/hot.json target/hot.png
