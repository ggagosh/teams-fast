fn main() {
    if std::env::var_os("CARGO_FEATURE_AUTO_UPDATE").is_some() {
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
    }
}
