// Xenify: INFO("release") gives the IronCalc release this copy is based on
// (upstream reads it from git, which here would be the Xenify repository).
fn main() {
    println!("cargo:rustc-env=GIT_VERSION=0.9.1-xenify");
}
