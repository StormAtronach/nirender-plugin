// The game and the dinput8.dll of MGE XE find the two functions by their plain names. A
// function with the calling convention of the Windows API has a name with decoration.
fn main() {
    println!("cargo:rustc-cdylib-link-arg=/EXPORT:Direct3DCreate8=_Direct3DCreate8@4");
    println!("cargo:rustc-cdylib-link-arg=/EXPORT:DirectInput8Create=_DirectInput8Create@20");
    println!("cargo:rustc-cdylib-link-arg=/EXPORT:NirenderD3D8=_NirenderD3D8@0");
}
