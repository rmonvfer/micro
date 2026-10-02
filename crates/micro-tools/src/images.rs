//! Fitting images to what a model accepts before they enter the conversation.
//!
//! An image is fitted once, when it first joins the history, and kept in its fitted form from
//! then on: changing models later never rewrites an image already sent, so a cached prompt prefix
//! stays byte-for-byte the same.

use base64::Engine as _;
use image::DynamicImage;
use image::ImageDecoder as _;
use image::ImageReader;
use micro_types::ContentBlock;
use micro_types::Message;
use std::io::Cursor;

/// The widest and tallest an image may be when nothing says otherwise.
pub const DEFAULT_MAX_DIMENSION: u32 = 2000;
/// The most base64 an image may take when nothing says otherwise, which leaves headroom below the
/// 5 MB limit the strictest provider sets.
pub const DEFAULT_MAX_BYTES: usize = 4_718_592;
/// The JPEG quality tried first when an image has to be re-encoded.
pub const DEFAULT_JPEG_QUALITY: u8 = 80;

/// The shrinking factor applied each time an image is still too large at its current size.
const SHRINK: f64 = 0.75;

/// How large an image a model may be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageLimits {
    pub max_width: u32,
    pub max_height: u32,
    /// The longest the base64 encoding may be.
    pub max_bytes: usize,
    pub jpeg_quality: u8,
}

impl Default for ImageLimits {
    fn default() -> Self {
        ImageLimits {
            max_width: DEFAULT_MAX_DIMENSION,
            max_height: DEFAULT_MAX_DIMENSION,
            max_bytes: DEFAULT_MAX_BYTES,
            jpeg_quality: DEFAULT_JPEG_QUALITY,
        }
    }
}

/// Image limits for every model: the ordinary ones, and those particular models set, keyed by
/// `provider/model`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImageLimitTable {
    pub ordinary: ImageLimits,
    pub models: std::collections::BTreeMap<String, ImageLimits>,
}

impl ImageLimitTable {
    /// The limits in force for one model.
    pub fn for_model(&self, qualified_id: &str) -> ImageLimits {
        self.models
            .get(qualified_id)
            .copied()
            .unwrap_or(self.ordinary)
    }
}

/// Fit the images in a message that carries content from outside the model: what the user
/// attached, and what a tool handed back. The decoding runs off the async threads.
pub async fn fit_message(message: Message, limits: ImageLimits) -> Message {
    let has_images = |content: &[ContentBlock]| {
        content
            .iter()
            .any(|block| matches!(block, ContentBlock::Image { .. }))
    };
    match message {
        Message::User { content, timestamp } if has_images(&content) => Message::User {
            content: fit_off_thread(content, limits).await,
            timestamp,
        },
        Message::ToolResult {
            tool_call_id,
            tool_name,
            content,
            is_error,
            timestamp,
        } if has_images(&content) => Message::ToolResult {
            tool_call_id,
            tool_name,
            content: fit_off_thread(content, limits).await,
            is_error,
            timestamp,
        },
        other => other,
    }
}

async fn fit_off_thread(content: Vec<ContentBlock>, limits: ImageLimits) -> Vec<ContentBlock> {
    let kept = content.clone();
    tokio::task::spawn_blocking(move || fit_content(content, &limits))
        .await
        .unwrap_or(kept)
}

/// An image as it will be sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FittedImage {
    /// Base64.
    pub data: String,
    pub mime_type: String,
    pub original_width: u32,
    pub original_height: u32,
    pub width: u32,
    pub height: u32,
}

impl FittedImage {
    pub fn was_resized(&self) -> bool {
        self.width != self.original_width || self.height != self.original_height
    }

    /// What the model is told about a resized image, so it can map coordinates back.
    pub fn dimension_note(&self) -> Option<String> {
        if !self.was_resized() {
            return None;
        }
        let scale = self.original_width as f64 / self.width as f64;
        Some(format!(
            "[Image: original {}x{}, displayed at {}x{}. Multiply coordinates by {scale:.2} to map \
             to original image.]",
            self.original_width, self.original_height, self.width, self.height
        ))
    }
}

/// Why an image could not be fitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FitError {
    /// The data is not base64.
    Unreadable,
    /// The image could not be decoded, and as it stands it breaks the limits.
    Undecodable,
    /// No size and encoding brought it within the byte limit.
    TooLarge,
}

impl std::fmt::Display for FitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FitError::Unreadable => formatter.write_str("its data is not base64"),
            FitError::Undecodable => formatter.write_str("it could not be decoded"),
            FitError::TooLarge => formatter.write_str("it could not be made small enough"),
        }
    }
}

/// Bring a base64 image within `limits`: kept as it is when it already fits, otherwise scaled
/// down and re-encoded as whichever of PNG and JPEG is smaller.
pub fn fit_image(
    data: &str,
    mime_type: &str,
    limits: &ImageLimits,
) -> Result<FittedImage, FitError> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.trim())
        .map_err(|_| FitError::Unreadable)?;

    // A format this build cannot decode is passed on as it is while it is small enough, since the
    // provider may still read it; its size is unknown, so it is reported as unchanged.
    let Some(image) = decode(&bytes) else {
        if data.len() > limits.max_bytes {
            return Err(FitError::Undecodable);
        }
        return Ok(FittedImage {
            data: data.to_string(),
            mime_type: mime_type.to_string(),
            original_width: 0,
            original_height: 0,
            width: 0,
            height: 0,
        });
    };

    let (original_width, original_height) = (image.width(), image.height());
    let within_dimensions =
        original_width <= limits.max_width && original_height <= limits.max_height;
    if within_dimensions && data.len() <= limits.max_bytes {
        return Ok(FittedImage {
            data: data.to_string(),
            mime_type: mime_type.to_string(),
            original_width,
            original_height,
            width: original_width,
            height: original_height,
        });
    }

    let (mut width, mut height) = bounded(
        original_width,
        original_height,
        limits.max_width.max(1),
        limits.max_height.max(1),
    );
    let mut qualities = vec![limits.jpeg_quality.clamp(1, 100)];
    for quality in [85, 70, 55, 40] {
        if !qualities.contains(&quality) {
            qualities.push(quality);
        }
    }

    loop {
        let resized = match (width, height) == (original_width, original_height) {
            true => image.clone(),
            false => image.resize_exact(width, height, image::imageops::FilterType::Lanczos3),
        };
        if let Some((data, mime_type)) = smallest_within(&resized, &qualities, limits.max_bytes) {
            return Ok(FittedImage {
                data,
                mime_type,
                original_width,
                original_height,
                width,
                height,
            });
        }
        if width == 1 && height == 1 {
            return Err(FitError::TooLarge);
        }
        width = shrink(width);
        height = shrink(height);
    }
}

/// Fit every image in a run of content blocks, adding a note where one was resized and a plain
/// note in place of one that could not be fitted.
pub fn fit_content(content: Vec<ContentBlock>, limits: &ImageLimits) -> Vec<ContentBlock> {
    let mut fitted = Vec::with_capacity(content.len());
    for block in content {
        let ContentBlock::Image { data, mime_type } = block else {
            fitted.push(block);
            continue;
        };
        match fit_image(&data, &mime_type, limits) {
            Ok(image) => {
                let note = image.dimension_note();
                fitted.push(ContentBlock::Image {
                    data: image.data,
                    mime_type: image.mime_type,
                });
                if let Some(note) = note {
                    fitted.push(ContentBlock::text(note));
                }
            }
            Err(error) => fitted.push(ContentBlock::text(format!(
                "[Image omitted: {error} within {} bytes at most {}x{}.]",
                limits.max_bytes, limits.max_width, limits.max_height
            ))),
        }
    }
    fitted
}

/// The image the bytes hold, turned upright as its EXIF orientation says.
fn decode(bytes: &[u8]) -> Option<DynamicImage> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut decoder = reader.into_decoder().ok()?;
    let orientation = decoder.orientation().ok();
    let mut image = DynamicImage::from_decoder(decoder).ok()?;
    if let Some(orientation) = orientation {
        image.apply_orientation(orientation);
    }
    Some(image)
}

/// The largest size within the bounds that keeps the image's proportions.
fn bounded(width: u32, height: u32, max_width: u32, max_height: u32) -> (u32, u32) {
    let (mut width, mut height) = (width.max(1) as f64, height.max(1) as f64);
    if width > max_width as f64 {
        height = (height * max_width as f64 / width).round();
        width = max_width as f64;
    }
    if height > max_height as f64 {
        width = (width * max_height as f64 / height).round();
        height = max_height as f64;
    }
    ((width as u32).max(1), (height as u32).max(1))
}

fn shrink(side: u32) -> u32 {
    ((side as f64 * SHRINK).floor() as u32).max(1)
}

/// The smallest of the PNG and JPEG encodings whose base64 fits within `max_bytes`.
fn smallest_within(
    image: &DynamicImage,
    qualities: &[u8],
    max_bytes: usize,
) -> Option<(String, String)> {
    let mut candidates = Vec::new();
    if let Some(png) = encode_png(image) {
        candidates.push((png, "image/png"));
    }
    for quality in qualities {
        if let Some(jpeg) = encode_jpeg(image, *quality) {
            candidates.push((jpeg, "image/jpeg"));
        }
    }
    candidates
        .into_iter()
        .map(|(bytes, mime_type)| {
            (
                base64::engine::general_purpose::STANDARD.encode(bytes),
                mime_type,
            )
        })
        .filter(|(data, _)| data.len() <= max_bytes)
        .min_by_key(|(data, _)| data.len())
        .map(|(data, mime_type)| (data, mime_type.to_string()))
}

fn encode_png(image: &DynamicImage) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .ok()?;
    Some(bytes)
}

fn encode_jpeg(image: &DynamicImage, quality: u8) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality);
    image.to_rgb8().write_with_encoder(encoder).ok()?;
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> String {
        let image = image::RgbImage::from_fn(width, height, |x, y| {
            image::Rgb([
                (x * 7 % 256) as u8,
                (y * 13 % 256) as u8,
                ((x ^ y) % 256) as u8,
            ])
        });
        let mut bytes = Vec::new();
        DynamicImage::ImageRgb8(image)
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn an_image_within_the_limits_is_kept_byte_for_byte() {
        let data = png(40, 30);
        let fitted = fit_image(&data, "image/png", &ImageLimits::default()).unwrap();
        assert_eq!(fitted.data, data);
        assert_eq!(fitted.mime_type, "image/png");
        assert!(!fitted.was_resized());
        assert_eq!(fitted.dimension_note(), None);
    }

    #[test]
    fn a_large_image_is_scaled_into_the_box_keeping_its_shape() {
        let limits = ImageLimits {
            max_width: 100,
            max_height: 100,
            ..ImageLimits::default()
        };
        let fitted = fit_image(&png(400, 200), "image/png", &limits).unwrap();
        assert_eq!((fitted.width, fitted.height), (100, 50));
        assert_eq!((fitted.original_width, fitted.original_height), (400, 200));
        assert!(fitted
            .dimension_note()
            .unwrap()
            .contains("Multiply coordinates by 4.00"));
    }

    #[test]
    fn a_byte_limit_shrinks_the_image_until_it_fits() {
        let limits = ImageLimits {
            max_bytes: 4_000,
            ..ImageLimits::default()
        };
        let fitted = fit_image(&png(300, 300), "image/png", &limits).unwrap();
        assert!(fitted.data.len() <= 4_000);
        assert!(fitted.width < 300);
    }

    #[test]
    fn fitting_is_deterministic() {
        let limits = ImageLimits {
            max_width: 64,
            max_height: 64,
            ..ImageLimits::default()
        };
        let data = png(256, 128);
        assert_eq!(
            fit_image(&data, "image/png", &limits),
            fit_image(&data, "image/png", &limits)
        );
    }

    #[test]
    fn an_image_that_cannot_fit_is_replaced_by_a_note() {
        let limits = ImageLimits {
            max_bytes: 10,
            ..ImageLimits::default()
        };
        let content = fit_content(
            vec![ContentBlock::Image {
                data: png(50, 50),
                mime_type: "image/png".into(),
            }],
            &limits,
        );
        assert_eq!(content.len(), 1);
        assert!(content[0].as_text().contains("Image omitted"));
    }

    #[test]
    fn undecodable_data_passes_when_small_and_is_dropped_when_large() {
        let data = base64::engine::general_purpose::STANDARD.encode(b"not an image at all");
        let small = fit_image(&data, "image/png", &ImageLimits::default()).unwrap();
        assert_eq!(small.data, data);

        let limits = ImageLimits {
            max_bytes: 4,
            ..ImageLimits::default()
        };
        assert_eq!(
            fit_image(&data, "image/png", &limits),
            Err(FitError::Undecodable)
        );
    }
}
