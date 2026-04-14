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

fn brace_delta(line: &str) -> isize {
    let opens = line.chars().filter(|&c| c == '{').count() as isize;
    let closes = line.chars().filter(|&c| c == '}').count() as isize;
    opens - closes
}

fn strip_cfg_test_modules(content: &str) -> String {
    let mut out = String::new();
    let mut pending_cfg_test = false;
    let mut skipping_test_module = false;
    let mut depth = 0isize;

    for line in content.lines() {
        let trimmed = line.trim_start();

        if skipping_test_module {
            depth += brace_delta(line);
            if depth <= 0 {
                skipping_test_module = false;
            }
            continue;
        }

        if pending_cfg_test {
            if trimmed.starts_with("mod ") && line.contains('{') {
                skipping_test_module = true;
                depth = brace_delta(line);
                pending_cfg_test = false;
                continue;
            }
            pending_cfg_test = false;
        }

        if trimmed.contains("#[cfg(test)]") {
            pending_cfg_test = true;
            continue;
        }

        out.push_str(line);
        out.push('\n');
    }

    out
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
        let production_code = strip_cfg_test_modules(&content);
        for (line_no, line) in production_code.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            let uses_infra_path = trimmed.starts_with("use ") && trimmed.contains("infra::");
            let directly_references_infra = line.contains("crate::infra::");
            if uses_infra_path || directly_references_infra {
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
