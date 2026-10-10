fn main() {
    // Morrowind.exe with the 4 GB patch can use 4 GB of address space. The test program gets
    // the same limit.
    println!("cargo:rustc-link-arg=/LARGEADDRESSAWARE");
}
