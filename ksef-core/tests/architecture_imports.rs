use std::fs;
use std::path::{Path, PathBuf};

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| {
        panic!("failed to read directory '{}': {e}", dir.display());
    });
    for entry in entries {
        let entry = entry.unwrap_or_else(|e| panic!("failed to read directory entry: {e}"));
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn services_and_workers_do_not_depend_on_infra_in_production_code() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    collect_rs_files(&root.join("src/services"), &mut files);
    collect_rs_files(&root.join("src/workers"), &mut files);

    let mut violations = Vec::new();
    for file in files {
        let content = fs::read_to_string(&file).unwrap_or_else(|e| {
            panic!("failed to read '{}': {e}", file.display());
        });
        let production_slice = content.split("#[cfg(test)]").next().unwrap_or(&content);
        for (line_no, line) in production_slice.lines().enumerate() {
            if line.contains("crate::infra::") {
                violations.push(format!(
                    "{}:{} -> {}",
                    file.strip_prefix(root).unwrap_or(&file).display(),
                    line_no + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "found forbidden infra imports in services/workers production code:\n{}",
        violations.join("\n")
    );
}
