//! The program `nirender-host`: the renderer of the crate, from the command line.

fn main() {
    nirender_wgpu::run(std::env::args().collect());
}
