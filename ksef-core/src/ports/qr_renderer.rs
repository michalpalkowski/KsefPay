use crate::domain::qr::{QRCodeData, QRCodeOptions};
use crate::error::KSeFError;

/// Port: render QR codes to PNG or SVG.
pub trait QrRenderer: Send + Sync {
    fn render_png(&self, data: &QRCodeData, options: QRCodeOptions) -> Result<Vec<u8>, KSeFError>;
    fn render_svg(&self, data: &QRCodeData, options: QRCodeOptions) -> Result<String, KSeFError>;
}
