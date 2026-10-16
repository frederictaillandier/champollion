//! Compiles the C++ wrapper around TensorRT and links NVIDIA's libraries.
//!
//! `CHAMPOLLION_GPU_ROOT` is where NVIDIA's Jetson packages are: `/` when
//! installed with apt, or a folder they were unpacked into with `dpkg -x`.

use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=src/tensorrt.cpp");
    println!("cargo:rerun-if-env-changed=CHAMPOLLION_GPU_ROOT");
    let root = PathBuf::from(env::var("CHAMPOLLION_GPU_ROOT").unwrap_or_else(|_| "/".into()));
    let cuda = root.join("usr/local/cuda-11.4/targets/aarch64-linux");
    let includes = [
        root.join("usr/include/aarch64-linux-gnu"),
        cuda.join("include"),
    ];
    let libs = [root.join("usr/lib/aarch64-linux-gnu"), cuda.join("lib")];

    let mut build = cc::Build::new();
    build.cpp(true).std("c++14").file("src/tensorrt.cpp");
    // As system headers, so their own warnings are not shown.
    for dir in &includes {
        build
            .flag("-isystem")
            .flag(dir.to_str().expect("UTF-8 path"));
    }
    build.compile("champollion_tensorrt");

    for dir in &libs {
        println!("cargo:rustc-link-search=native={}", dir.display());
        // TensorRT's own dependencies (cuDNN, cuBLAS) are found there too,
        // when linking and when running, without LD_LIBRARY_PATH.
        println!("cargo:rustc-link-arg=-Wl,-rpath-link,{}", dir.display());
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", dir.display());
    }
    // RPATH rather than RUNPATH: it also applies to the libraries' own
    // dependencies.
    println!("cargo:rustc-link-arg=-Wl,--disable-new-dtags");
    println!("cargo:rustc-link-lib=dylib=nvinfer");
    println!("cargo:rustc-link-lib=dylib=cudart");
}
