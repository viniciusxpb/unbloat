// Shared build-script step: embeds the UTF-8 catalogs used by shared/i18n.rs.

/// Embed sorted static catalogs, with no parsing or heap cache inside Explorer.
fn compile_localizations() {
    use std::collections::BTreeMap;
    println!("cargo:rerun-if-changed=../../scripts/localization-build.rs");
    let mut generated = String::new();
    for (name, constant) in [("en", "EN"), ("pt-BR", "PT")] {
        let path = format!("../../locales/{name}.txt");
        println!("cargo:rerun-if-changed={path}");
        let source = std::fs::read_to_string(path).expect("Read UTF-8 localization catalog");
        let mut entries = BTreeMap::new();
        for line in source
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
        {
            let (key, value) = line.split_once('=').expect("Invalid localization entry");
            assert!(
                entries
                    .insert(key, value.replace("\\r", "\r").replace("\\n", "\n"))
                    .is_none(),
                "Duplicate localization key: {key}"
            );
        }
        generated.push_str(&format!("const {constant}: &[(&str, &str)] = &[\n"));
        for (key, value) in entries {
            generated.push_str(&format!("({key:?}, {value:?}),\n"));
        }
        generated.push_str("];\n");
    }
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("locales.rs");
    std::fs::write(output, generated).expect("Generate embedded localization catalogs");
}
