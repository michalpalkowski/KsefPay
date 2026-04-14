use std::io::Cursor;

use image::ImageFormat;
use qrcode::QrCode;
use qrcode::render::svg;

use crate::domain::qr::{QRCodeData, QRCodeOptions};
use crate::error::KSeFError;

pub struct QRCodeGenerator;

impl QRCodeGenerator {
    pub fn generate_png(data: &QRCodeData, options: QRCodeOptions) -> Result<Vec<u8>, KSeFError> {
        data.validate().map_err(|e| {
            KSeFError::InvoiceSubmissionFailed(format!("invalid QR payload URL: {e}"))
        })?;

        let qr = QrCode::new(data.url.as_bytes()).map_err(|e| {
            KSeFError::InvoiceSubmissionFailed(format!("failed to generate QR matrix: {e}"))
        })?;
        let image = qr
            .render::<image::Luma<u8>>()
            .max_dimensions(u32::from(options.size), u32::from(options.size))
            .quiet_zone(options.margin > 0)
            .build();

        let mut out = Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(image)
            .write_to(&mut out, ImageFormat::Png)
            .map_err(|e| {
                KSeFError::InvoiceSubmissionFailed(format!("failed to encode QR PNG: {e}"))
            })?;
        Ok(out.into_inner())
    }

    pub fn generate_svg(data: &QRCodeData, options: QRCodeOptions) -> Result<String, KSeFError> {
        data.validate().map_err(|e| {
            KSeFError::InvoiceSubmissionFailed(format!("invalid QR payload URL: {e}"))
        })?;

        let qr = QrCode::new(data.url.as_bytes()).map_err(|e| {
            KSeFError::InvoiceSubmissionFailed(format!("failed to generate QR matrix: {e}"))
        })?;
        let svg = qr
            .render::<svg::Color<'_>>()
            .min_dimensions(u32::from(options.size), u32::from(options.size))
            .quiet_zone(options.margin > 0)
            .build();
        Ok(svg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> QRCodeData {
        QRCodeData {
            url: "https://qr-test.ksef.mf.gov.pl/invoice/5260250274/13-04-2026/hash".to_string(),
        }
    }

    #[test]
    fn png_is_valid_image_bytes() {
        let png = QRCodeGenerator::generate_png(&data(), QRCodeOptions::default()).unwrap();
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
        let decoded = image::load_from_memory(&png).unwrap();
        assert!(decoded.width() > 0);
        assert!(decoded.height() > 0);
    }

    #[test]
    fn svg_contains_svg_root() {
        let svg = QRCodeGenerator::generate_svg(&data(), QRCodeOptions::default()).unwrap();
        assert!(svg.contains("<svg"));
        assert!(svg.contains("</svg>"));
    }
}
