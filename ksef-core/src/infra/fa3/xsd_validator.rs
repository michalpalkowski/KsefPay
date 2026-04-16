use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use uppsala::xsd::XsdValidator;

use crate::domain::xml::InvoiceXml;
use crate::error::XmlError;
use crate::ports::invoice_xml_validator::InvoiceXmlValidator;

const BUNDLE_DIR: &str = "fa3-2025-06-25-13775";
const SCHEMAT_XSD: &str = include_str!("../../../schemas/fa3/2025-06-25-13775/schemat.xsd");
const STRUKTURY_XSD: &str =
    include_str!("../../../schemas/fa3/2025-06-25-13775/StrukturyDanych_v10-0E.xsd");
const ELEMENTARNE_XSD: &str =
    include_str!("../../../schemas/fa3/2025-06-25-13775/ElementarneTypyDanych_v10-0E.xsd");
const KODY_KRAJOW_XSD: &str =
    include_str!("../../../schemas/fa3/2025-06-25-13775/KodyKrajow_v10-0E.xsd");

static VALIDATOR: OnceLock<Result<XsdValidator, String>> = OnceLock::new();

/// FA(3) XSD preflight validator backed by the pinned, vendored schema bundle.
pub struct Fa3XsdValidator;

impl Fa3XsdValidator {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Compile and cache the underlying XSD validator.
    pub fn warm_up(&self) -> Result<(), XmlError> {
        let _ = Self::validator()?;
        Ok(())
    }

    fn validator() -> Result<&'static XsdValidator, XmlError> {
        let entry = VALIDATOR.get_or_init(|| compile_validator().map_err(|e| e.to_string()));
        match entry {
            Ok(validator) => Ok(validator),
            Err(err) => Err(XmlError::ValidationFailed(format!(
                "failed to initialize FA(3) XSD validator: {err}"
            ))),
        }
    }
}

impl Default for Fa3XsdValidator {
    fn default() -> Self {
        Self::new()
    }
}

impl InvoiceXmlValidator for Fa3XsdValidator {
    fn validate(&self, xml: &InvoiceXml) -> Result<(), XmlError> {
        let validator = Self::validator()?;
        let instance = uppsala::parse(xml.as_str())
            .map_err(|e| XmlError::ParseFailed(format!("cannot parse generated XML: {e}")))?;
        let errors = validator.validate(&instance);
        if errors.is_empty() {
            return Ok(());
        }

        let mut details = errors
            .iter()
            .take(3)
            .map(|err| err.to_string())
            .collect::<Vec<_>>();
        if errors.len() > 3 {
            details.push(format!("... and {} more", errors.len() - 3));
        }

        Err(XmlError::ValidationFailed(format!(
            "FA(3) XSD validation failed: {}",
            details.join(" | ")
        )))
    }
}

fn compile_validator() -> Result<XsdValidator, XmlError> {
    let schema_path = write_bundle_to_temp_dir()?;
    let schema_doc = uppsala::parse(SCHEMAT_XSD).map_err(|e| {
        XmlError::ValidationFailed(format!("cannot parse bundled schemat.xsd: {e}"))
    })?;
    XsdValidator::from_schema_with_base_path(&schema_doc, Some(&schema_path)).map_err(|e| {
        XmlError::ValidationFailed(format!("cannot compile bundled FA(3) XSD schema: {e}"))
    })
}

fn write_bundle_to_temp_dir() -> Result<PathBuf, XmlError> {
    let dir = std::env::temp_dir()
        .join("ksef-paymoney")
        .join("schemas")
        .join(BUNDLE_DIR);
    fs::create_dir_all(&dir).map_err(|e| {
        XmlError::ValidationFailed(format!("cannot create schema cache directory: {e}"))
    })?;

    write_if_changed(&dir.join("schemat.xsd"), SCHEMAT_XSD)?;
    write_if_changed(&dir.join("StrukturyDanych_v10-0E.xsd"), STRUKTURY_XSD)?;
    write_if_changed(
        &dir.join("ElementarneTypyDanych_v10-0E.xsd"),
        ELEMENTARNE_XSD,
    )?;
    write_if_changed(&dir.join("KodyKrajow_v10-0E.xsd"), KODY_KRAJOW_XSD)?;

    Ok(dir.join("schemat.xsd"))
}

fn write_if_changed(path: &Path, content: &str) -> Result<(), XmlError> {
    let existing = fs::read_to_string(path).ok();
    if existing.as_deref() == Some(content) {
        return Ok(());
    }

    fs::write(path, content).map_err(|e| {
        XmlError::ValidationFailed(format!(
            "cannot write bundled schema '{}': {e}",
            path.display()
        ))
    })
}
