use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use xsd_parser::config::{
    Config, GeneratorFlags, InterpreterFlags, OptimizerFlags, ParserFlags, Resolver, Schema,
};
use xsd_parser::generate;

const SCHEMA_DIR: &str = "schemas/fa3/2025-06-25-13775";
const MAIN_SCHEMA_FILE: &str = "schemat.xsd";
const GENERATED_BINDINGS_FILE: &str = "src/infra/fa3/generated/v2025_06_25_13775.rs";

fn main() {
    if env::var_os("KSEF_FA3_SKIP_BINDINGS_CHECK").is_some() {
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let schema_dir = manifest_dir.join(SCHEMA_DIR);
    let schema_main = schema_dir.join(MAIN_SCHEMA_FILE);
    let bindings_file = manifest_dir.join(GENERATED_BINDINGS_FILE);

    for file in [
        "schemat.xsd",
        "StrukturyDanych_v10-0E.xsd",
        "ElementarneTypyDanych_v10-0E.xsd",
        "KodyKrajow_v10-0E.xsd",
        "wyroznik.xml",
    ] {
        println!("cargo:rerun-if-changed={}", schema_dir.join(file).display());
    }
    println!("cargo:rerun-if-changed={}", bindings_file.display());

    if let Err(err) = verify_bindings_are_fresh(&schema_main, &bindings_file) {
        panic!("{err}");
    }
}

fn verify_bindings_are_fresh(schema_main: &Path, bindings_file: &Path) -> Result<(), String> {
    let generated = generate_bindings(schema_main)?;
    let current = fs::read_to_string(bindings_file).map_err(|e| {
        format!(
            "failed to read generated bindings '{}': {e}",
            bindings_file.display()
        )
    })?;

    if normalize(&generated) == normalize(&current) {
        return Ok(());
    }

    Err(format!(
        "FA(3) generated bindings are stale.\n\
         Regenerate with:\n\
         KSEF_FA3_SKIP_BINDINGS_CHECK=1 cargo run -p ksef-core --example generate_fa3_types\n\
         Expected file: {}",
        bindings_file.display()
    ))
}

fn generate_bindings(schema_main: &Path) -> Result<String, String> {
    let mut cfg = Config::default()
        .with_quick_xml_config(
            xsd_parser::pipeline::renderer::NamespaceSerialization::Global,
            None,
            false,
        )
        .with_advanced_enums()
        .with_parser_flags(ParserFlags::all())
        .with_interpreter_flags(InterpreterFlags::all() - InterpreterFlags::WITH_NUM_BIG_INT)
        .with_optimizer_flags(OptimizerFlags::all() - OptimizerFlags::REMOVE_DUPLICATES)
        .with_generator_flags(GeneratorFlags::all())
        .with_schema(Schema::File(schema_main.to_path_buf()));

    cfg.parser.resolver = vec![Resolver::File];

    generate(cfg)
        .map(|tokens| tokens.to_string())
        .map_err(|e| format!("failed to generate FA(3) bindings from XSD: {e}"))
}

fn normalize(input: &str) -> String {
    input.replace("\r\n", "\n").trim().to_string()
}
