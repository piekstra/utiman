fn main() {
    // Bake in the target triple so `utiman self-update` picks the right release
    // asset (`utiman-<triple>.tar.gz`), matching the family updater's contract.
    println!(
        "cargo:rustc-env=BUILD_TARGET={}",
        std::env::var("TARGET").unwrap()
    );
}
