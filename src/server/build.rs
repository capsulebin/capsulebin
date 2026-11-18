//! Makes embedding the frontend assets work reliably.
//!
//! Two jobs:
//!
//! 1. Create `web/dist` when it is missing. The directory is
//!    gitignored, so a fresh clone does not have it - without this step even
//!    `cargo build` would fail in `#[derive(Embed)]`. The server reports
//!    missing assets at runtime through `/health/ready` instead.
//! 2. Tell Cargo that a changed asset must trigger a rebuild. Without that hint
//!    a freshly built frontend would stay invisible inside an old binary,
//!    because embedding happens at compile time.

use std::path::Path;

fn main() {
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("web/dist");

    if !dist.exists()
        && let Err(error) = std::fs::create_dir_all(&dist)
    {
        println!(
            "cargo::warning=could not create web/dist ({error}); \
             run `npm run build` in src/server/web/ first"
        );
        return;
    }

    println!("cargo::rerun-if-changed={}", dist.display());
}
