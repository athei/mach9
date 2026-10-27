//! Links the Sparkle framework when `SPARKLE_DIR` names the directory that
//! holds `Sparkle.framework`, and gives the binary the rpath that resolves
//! the copy an installed bundle has in `Contents/Frameworks`.
//!
//! Without `SPARKLE_DIR` the launcher builds and tests without Sparkle: the
//! update check finds no Sparkle classes at run time and skips itself
//! (`update::check`). A binary linked with Sparkle does not start without
//! it, and only a bundle has it next to the binary, so run the tests
//! without `SPARKLE_DIR`.

use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=SPARKLE_DIR");

    let Some(dir) = env::var_os("SPARKLE_DIR").filter(|dir| !dir.is_empty()) else {
        if env::var("PROFILE").is_ok_and(|profile| profile == "release") {
            println!(
                "cargo:warning=SPARKLE_DIR is not set, so this build of the app has no updater"
            );
        }
        return;
    };
    let sparkle = PathBuf::from(dir);
    // This script runs in launcher/app and the linker at the workspace root,
    // so a relative path would name two different directories.
    assert!(
        sparkle.is_absolute(),
        "SPARKLE_DIR is {}, a relative path; give the absolute path of the directory that \
         holds Sparkle.framework",
        sparkle.display()
    );
    let framework = sparkle.join("Sparkle.framework");
    assert!(
        framework.is_dir(),
        "SPARKLE_DIR is {}, which holds no Sparkle.framework",
        sparkle.display()
    );

    println!("cargo:rerun-if-changed={}", framework.display());
    println!("cargo:rustc-link-search=framework={}", sparkle.display());
    println!("cargo:rustc-link-lib=framework=Sparkle");
    // Contents/MacOS/gamelauncher -> Contents/Frameworks/Sparkle.framework.
    println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
}
