fn main() {
    let default_server = std::env::var("LWID_DEFAULT_SERVER")
        .unwrap_or_else(|_| "https://lookwhatidid.xyz".to_string());

    println!("cargo:rustc-env=LWID_DEFAULT_SERVER={default_server}");
    println!("cargo:rerun-if-env-changed=LWID_DEFAULT_SERVER");
}
