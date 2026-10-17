include!("../../scripts/windows-resource.rs");

fn main() {
    if let Some(output) = compile_windows_resource("unbloat-shell.rc") {
        println!("cargo:rustc-link-arg-cdylib={}", output.display());
    }
}
