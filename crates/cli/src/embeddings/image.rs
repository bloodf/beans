//! An image model's vision tower on the CPU (CLIP and its kind): an image, prepared exactly as
//! the model was trained, to a unit vector in the space its text tower shares. The text tower
//! is an ordinary pooled `LocalEmbedding`; this is only the half that reads pixels.

use super::assets::{verified_bytes, BoundAsset, ProtectedRuntime};
use super::local::{initialize_runtime, native_setup, session_from, TerminateOnDrop};
use super::{EmbeddingError, ImageSpec};
use image::imageops::FilterType;
use ort::session::{RunOptions, Session};
use ort::value::{Tensor, TensorElementType, ValueType};
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

/// The most images read per model call, and the largest vision model accepted.
const MAX_BATCH: usize = 16;
const MAX_MODEL_BYTES: u64 = 1024 * 1024 * 1024;

pub struct LocalImageEmbedding {
    spec: ImageSpec,
    dimensions: usize,
    session: Arc<tokio::sync::Mutex<Session>>,
    timeout: Duration,
}

impl LocalImageEmbedding {
    /// Loads the vision tower from verified bytes with the approved runtime, checking its
    /// tensors against `spec` before it is used.
    pub async fn load(spec: &ImageSpec, dimensions: usize, runtime: BoundAsset, model: BoundAsset, cancel: CancellationToken) -> Result<Self, EmbeddingError> {
        spec.validate()?;
        if dimensions == 0 || dimensions > 65536 {
            return Err(EmbeddingError::InvalidProfile);
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        let bytes = tokio::select! { biased;
            _ = cancel.cancelled() => return Err(EmbeddingError::Cancelled),
            bytes = verified_bytes(&model, MAX_MODEL_BYTES) => bytes?,
        };
        let checked = spec.clone();
        let session = native_setup(cancel, deadline, move |fenced| {
            initialize_runtime(ProtectedRuntime::prepare(runtime)?, fenced, deadline)?;
            let session = session_from(&bytes)?;
            validate_session(&session, &checked, dimensions)?;
            Ok(session)
        })
        .await?;
        Ok(LocalImageEmbedding { spec: spec.clone(), dimensions, session: Arc::new(tokio::sync::Mutex::new(session)), timeout: Duration::from_secs(30) })
    }

    /// One unit vector per image (encoded bytes: PNG, JPEG, WebP, GIF…), in order.
    pub async fn embed(&self, images: Vec<Vec<u8>>, cancel: CancellationToken) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        if images.is_empty() || images.len() > MAX_BATCH {
            return Err(EmbeddingError::InvalidInput);
        }
        let (spec, dimensions, session) = (self.spec.clone(), self.dimensions, self.session.clone());
        let work = async move {
            // Held until the native call ends, even when the caller gives up.
            let mut session = session.lock_owned().await;
            let options = Arc::new(RunOptions::new().map_err(|_| EmbeddingError::RuntimeUnavailable)?);
            let _terminate = TerminateOnDrop(options.clone());
            tokio::task::spawn_blocking(move || {
                let count = images.len();
                let side = spec.size as usize;
                let mut input = Vec::with_capacity(count * 3 * side * side);
                for bytes in &images {
                    input.extend(pixels(bytes, &spec)?);
                }
                let tensor = Tensor::from_array(([count, 3, side, side], input)).map_err(|_| EmbeddingError::ModelContract)?;
                let outputs = session.run_with_options(ort::inputs![spec.input.as_str() => tensor], &options).map_err(|_| EmbeddingError::ModelContract)?;
                let output = outputs.get(&spec.output).ok_or(EmbeddingError::ModelContract)?;
                let (shape, values) = output.try_extract_tensor::<f32>().map_err(|_| EmbeddingError::ModelContract)?;
                if **shape != [count as i64, dimensions as i64] {
                    return Err(EmbeddingError::ModelContract);
                }
                values.chunks_exact(dimensions).map(unit).collect()
            })
            .await
            .map_err(|_| EmbeddingError::RuntimeUnavailable)?
        };
        tokio::select! { biased;
            _ = cancel.cancelled() => Err(EmbeddingError::Cancelled),
            result = tokio::time::timeout(self.timeout, work) => result.map_err(|_| EmbeddingError::Timeout)?,
        }
    }
}

/// `vector` scaled to length 1, refusing one that is not finite or all zeros.
fn unit(vector: &[f32]) -> Result<Vec<f32>, EmbeddingError> {
    let norm = vector.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>().sqrt();
    if !norm.is_finite() || norm == 0.0 {
        return Err(EmbeddingError::InvalidResponse);
    }
    Ok(vector.iter().map(|v| (f64::from(*v) / norm) as f32).collect())
}

/// An image as the model takes it, channels first: RGB, its centered square (the shortest side
/// by itself) resized to `size` with a bicubic filter, scaled to 0..1, then `(x - mean) / std`
/// per channel. This frames the picture as resizing the shortest side and center-cropping
/// does, while never making an intermediate larger than the original: a 10000×10 panorama does
/// not become 224000×224.
pub fn pixels(bytes: &[u8], spec: &ImageSpec) -> Result<Vec<f32>, EmbeddingError> {
    let mut image = image::load_from_memory(bytes).map_err(|_| EmbeddingError::InvalidInput)?.to_rgb8();
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return Err(EmbeddingError::InvalidInput);
    }
    let square = width.min(height);
    let cropped = image::imageops::crop(&mut image, (width - square) / 2, (height - square) / 2, square, square).to_image();
    let side = spec.size;
    let resized = image::imageops::resize(&cropped, side, side, FilterType::CatmullRom);
    let area = (side * side) as usize;
    let mut out = vec![0f32; 3 * area];
    for y in 0..side {
        for x in 0..side {
            let pixel = resized.get_pixel(x, y);
            for channel in 0..3 {
                let value = f32::from(pixel[channel]) / 255.0;
                out[channel * area + (y * side + x) as usize] = (value - spec.mean[channel]) / spec.std[channel];
            }
        }
    }
    Ok(out)
}

fn validate_session(session: &Session, spec: &ImageSpec, dimensions: usize) -> Result<(), EmbeddingError> {
    let side = i64::from(spec.size);
    let [input] = session.inputs() else { return Err(EmbeddingError::ModelContract) };
    let fits = |shape: &[i64], want: &[i64]| shape.len() == want.len() && shape.iter().zip(want).all(|(s, w)| *s == -1 || s == w);
    match input.dtype() {
        ValueType::Tensor { ty: TensorElementType::Float32, shape, .. } if input.name() == spec.input && fits(shape, &[-1, 3, side, side]) => {}
        _ => return Err(EmbeddingError::ModelContract),
    }
    let output = session.outputs().iter().find(|o| o.name() == spec.output).ok_or(EmbeddingError::ModelContract)?;
    match output.dtype() {
        ValueType::Tensor { ty: TensorElementType::Float32, shape, .. } if fits(shape, &[-1, dimensions as i64]) => Ok(()),
        _ => Err(EmbeddingError::ModelContract),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(size: u32) -> ImageSpec {
        ImageSpec { input: "pixel_values".into(), output: "image_embeds".into(), size, mean: [0.5, 0.5, 0.5], std: [0.5, 0.5, 0.5] }
    }

    fn png(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
        let image = image::RgbImage::from_fn(width, height, |x, y| image::Rgb(pixel(x, y)));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    #[test]
    fn pixels_are_resized_cropped_normalized_and_channels_first() {
        // A wide image: left half red, right half blue. The center crop keeps the middle square.
        let wide = png(8, 4, |x, _| if x < 4 { [255, 0, 0] } else { [0, 0, 255] });
        let out = pixels(&wide, &spec(4)).unwrap();
        assert_eq!(out.len(), 3 * 4 * 4);
        let (red, blue) = (&out[0..16], &out[32..48]);
        // Left column of the crop is red (1.0 after (1 - 0.5) / 0.5), right column blue.
        assert!((red[0] - 1.0).abs() < 0.05 && (blue[0] + 1.0).abs() < 0.05, "{red:?} {blue:?}");
        assert!((red[3] + 1.0).abs() < 0.05 && (blue[3] - 1.0).abs() < 0.05, "{red:?} {blue:?}");
        assert!(pixels(b"not an image", &spec(4)).is_err());
        let tall = pixels(&png(3, 9, |_, _| [0, 255, 0]), &spec(6)).unwrap();
        assert!(tall[36..72].iter().all(|v| (v - 1.0).abs() < 1e-5), "green everywhere in the green channel");
    }

    fn encoded(image: image::DynamicImage) -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    #[test]
    fn odd_pictures_become_the_same_input_shape() {
        let spec = spec(16);
        let shape = 3 * 16 * 16;
        // A panorama is cropped before it is resized: its middle, never a 160000-pixel-wide copy.
        let panorama = png(10_000, 1, |x, _| if x < 5_000 { [255, 0, 0] } else { [0, 0, 255] });
        assert_eq!(pixels(&panorama, &spec).unwrap().len(), shape);
        let tall = png(1, 4_000, |_, _| [0, 255, 0]);
        assert_eq!(pixels(&tall, &spec).unwrap().len(), shape);
        assert_eq!(pixels(&png(1, 1, |_, _| [0, 0, 0]), &spec).unwrap(), vec![-1.0; shape], "one black pixel fills the square");
        // Grayscale, alpha, and 16-bit pictures read as RGB.
        let gray = encoded(image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(20, 20, image::Luma([255]))));
        assert!(pixels(&gray, &spec).unwrap().iter().all(|v| (v - 1.0).abs() < 1e-5));
        let clear = encoded(image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(20, 20, image::Rgba([255, 0, 0, 0]))));
        assert!((pixels(&clear, &spec).unwrap()[0] - 1.0).abs() < 1e-5, "colour is kept where alpha is zero");
        let deep = encoded(image::DynamicImage::ImageRgb16(image::ImageBuffer::from_pixel(20, 20, image::Rgb([65535u16, 0, 0]))));
        assert!((pixels(&deep, &spec).unwrap()[0] - 1.0).abs() < 1e-3);
        for broken in [&b""[..], b"\x89PNG\r\n\x1a\n", &panorama[..panorama.len() / 2]] {
            assert!(pixels(broken, &spec).is_err(), "{} bytes", broken.len());
        }
    }

    #[test]
    fn vectors_are_unit_and_empty_ones_refused() {
        assert_eq!(unit(&[3.0, 4.0]).unwrap(), vec![0.6, 0.8]);
        assert!(unit(&[0.0, 0.0]).is_err());
        assert!(unit(&[f32::NAN, 1.0]).is_err());
        assert!(spec(224).validate().is_ok());
        assert!(spec(4).validate().is_err(), "too small to be a model's input");
        assert!(ImageSpec { std: [0.0, 1.0, 1.0], ..spec(224) }.validate().is_err());
        assert!(ImageSpec { output: "pixel_values".into(), ..spec(224) }.validate().is_err());
    }
}
