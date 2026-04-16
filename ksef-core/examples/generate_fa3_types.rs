use std::fs;
use std::path::PathBuf;

use anyhow::Context;
use xsd_parser::config::{
    Config, GeneratorFlags, InterpreterFlags, OptimizerFlags, ParserFlags, Resolver, Schema,
};
use xsd_parser::generate;

fn main() -> anyhow::Result<()> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let default_input = manifest_dir.join("schemas/fa3/2025-06-25-13775/schemat.xsd");
    let default_output = manifest_dir.join("src/infra/fa3/generated/v2025_06_25_13775.rs");

    let input = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or(default_input);
    let output = std::env::args()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or(default_output);

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
        .with_schema(Schema::File(input.clone()));
    cfg.parser.resolver = vec![Resolver::File];

    let code = generate(cfg)
        .context("failed to generate Rust bindings from XSD")?
        .to_string();

    fs::write(&output, code).with_context(|| {
        format!(
            "failed to write generated bindings to '{}'",
            output.display()
        )
    })?;

    println!("Generated FA(3) Rust types: {}", output.display());
    Ok(())
}
