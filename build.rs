//! Puts the commit of this build into the environment of the compiler as BUILD_COMMIT, for
//! the build id of the plugin (`BUILD_ID` in `src/lib.rs`).

#[path = "build_support/build_id.rs"]
mod build_id;

fn main() {
    println!("cargo:rustc-env=BUILD_COMMIT={}", build_id::commit());
    build_id::watch_commit();
}
