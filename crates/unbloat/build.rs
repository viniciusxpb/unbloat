include!("../../scripts/windows-resource.rs");

fn main() {
    if let Some(output) = compile_windows_resource("unbloat.rc") {
        println!("cargo:rustc-link-arg-bin=unbloat={}", output.display());
    }
}
