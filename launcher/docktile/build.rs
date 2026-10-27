fn main() {
    // The dylib is only ever loaded by path as a bundle's executable, so the
    // install name is never resolved; this keeps the build directory's
    // absolute path out of the shipped binary.
    println!("cargo:rustc-cdylib-link-arg=-Wl,-install_name,@rpath/GameLauncherDockTile");
}
