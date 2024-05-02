use std::env;

fn main() {
    let path = match env::var("SGX_SDK") {
        Ok(p) => p,
        Err(_) => panic!("SGX_SDK env var not set"),
    };
    println!(r"cargo:rustc-link-search={}/lib64", path);
}

