//! Conservative still-image conversion. No color management is performed: ICC,
//! EXIF, XMP, text and other ancillary metadata require explicit stripping consent.
//! Only 8-bit PNG/JPEG/static WebP are accepted. JPEG requires explicit quality;
//! alpha is never silently flattened. Publication uses checked, no-clobber renames.
use image::{DynamicImage, GenericImageView, ImageDecoder, ImageFormat};
use photo_core::rename::{self, SourceVersion};
use std::{
    fs,
    io::{Cursor, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_WORKING_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PIXELS: u64 = 32_000_000;

fn read_bounded(path: &Path) -> Result<Vec<u8>, ConversionFailure> {
    let file = fs::File::open(path).map_err(fail)?;
    let initial_size = file.metadata().map_err(fail)?.len();
    if initial_size > MAX_FILE_BYTES {
        return Err(fail("Conversion source exceeds 64 MiB"));
    }
    read_bounded_bytes(file.take(MAX_FILE_BYTES + 1), initial_size as usize)
}

fn read_bounded_bytes(
    mut reader: impl Read,
    initial_size: usize,
) -> Result<Vec<u8>, ConversionFailure> {
    if initial_size as u64 > MAX_FILE_BYTES {
        return Err(fail("Conversion source exceeds 64 MiB"));
    }
    let mut bytes = Vec::new();
    // Reserve the checked size once. Reserving every chunk exactly would copy
    // growing buffers repeatedly during source/stage/backup verification.
    bytes.try_reserve_exact(initial_size).map_err(fail)?;
    let mut chunk = [0u8; 65536];
    loop {
        let n = reader.read(&mut chunk).map_err(fail)?;
        if n == 0 {
            break;
        }
        if bytes
            .len()
            .checked_add(n)
            .is_none_or(|size| size > initial_size || size as u64 > MAX_FILE_BYTES)
        {
            return Err(fail("File changed while reading"));
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
    if bytes.len() != initial_size {
        return Err(fail("File changed while reading"));
    }
    Ok(bytes)
}

// Enforce the encoded-output limit during encoding, including seeking codecs.
struct BoundedOutput(Cursor<Vec<u8>>);
impl Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self
            .0
            .position()
            .checked_add(bytes.len() as u64)
            .is_none_or(|n| n > MAX_FILE_BYTES)
        {
            return Err(std::io::Error::other("Encoded output exceeds 64 MiB"));
        }
        self.0.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Seek for BoundedOutput {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        let old = self.0.position();
        let next = self.0.seek(position)?;
        if next > MAX_FILE_BYTES {
            self.0.set_position(old);
            return Err(std::io::Error::other("Encoded output exceeds 64 MiB"));
        }
        Ok(next)
    }
}

fn check_working_set(dimensions: (u32, u32), source_bytes: usize) -> Result<(), ConversionFailure> {
    let pixels = u64::from(dimensions.0) * u64::from(dimensions.1);
    if pixels == 0 || pixels > MAX_PIXELS {
        return Err(fail(
            "Conversion dimensions must contain 1–32 million pixels",
        ));
    }
    // Account for decoder/encoder working buffers, orientation copies, verified
    // pixels and both RGBA comparison buffers, plus bounded encoded output.
    if source_bytes as u64 + MAX_FILE_BYTES + pixels * 24 + 16 * 1024 * 1024 > MAX_WORKING_BYTES {
        return Err(fail("Conversion working memory estimate exceeds 256 MiB"));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub enum OutputFormat {
    Png,
    WebP,
    Jpeg { quality: u8 },
}
impl OutputFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::WebP => "webp",
            Self::Jpeg { .. } => "jpg",
        }
    }
    fn image_format(self) -> ImageFormat {
        match self {
            Self::Png => ImageFormat::Png,
            Self::WebP => ImageFormat::WebP,
            Self::Jpeg { .. } => ImageFormat::Jpeg,
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct ConversionOptions {
    /// Consent to discard all ancillary metadata, including ICC profiles. The
    /// pixel values are retained without transforming them into a color space.
    pub strip_metadata: bool,
}
pub struct PreparedConversion {
    source: PathBuf,
    target: PathBuf,
    original: Vec<u8>,
    version: SourceVersion,
    encoded: Vec<u8>,
    dimensions: (u32, u32),
}
impl PreparedConversion {
    pub fn target(&self) -> &Path {
        &self.target
    }
    pub fn dimensions(&self) -> (u32, u32) {
        self.dimensions
    }
}
#[derive(Debug)]
pub struct ConversionFailure {
    pub message: String,
    /// Files which may need recovery after an interrupted publication. Empty
    /// for a preparation failure. Never discard these paths without inspection.
    pub recovery_paths: Vec<PathBuf>,
}
impl std::fmt::Display for ConversionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)?;
        if !self.recovery_paths.is_empty() {
            write!(f, " (retained files: {:?})", self.recovery_paths)?;
        }
        Ok(())
    }
}
impl std::error::Error for ConversionFailure {}
fn fail(message: impl ToString) -> ConversionFailure {
    ConversionFailure {
        message: message.to_string(),
        recovery_paths: vec![],
    }
}

// Inspect the actual container rather than the filename. Unknown ancillary
// chunks/segments require consent too: decoders expose only some metadata.
fn inspect(bytes: &[u8], format: ImageFormat, strip: bool) -> Result<(), ConversionFailure> {
    match format {
        ImageFormat::Png => {
            let mut p = 8;
            while p + 12 <= bytes.len() {
                let n = u32::from_be_bytes(bytes[p..p + 4].try_into().unwrap()) as usize;
                let end = p
                    .checked_add(n)
                    .and_then(|v| v.checked_add(12))
                    .filter(|v| *v <= bytes.len())
                    .ok_or_else(|| fail("Invalid PNG chunk"))?;
                let kind = &bytes[p + 4..p + 8];
                if kind == b"acTL" {
                    return Err(fail("Animated PNG conversion is unsupported"));
                }
                if !strip && !matches!(kind, b"IHDR" | b"PLTE" | b"IDAT" | b"IEND" | b"tRNS") {
                    return Err(fail(
                        "PNG metadata/color information requires explicit strip_metadata consent",
                    ));
                }
                p = end;
            }
        }
        ImageFormat::WebP => {
            let mut p = 12;
            while p + 8 <= bytes.len() {
                let kind = &bytes[p..p + 4];
                let n = u32::from_le_bytes(bytes[p + 4..p + 8].try_into().unwrap()) as usize;
                let end = p
                    .checked_add(8)
                    .and_then(|v| v.checked_add(n))
                    .filter(|v| *v <= bytes.len())
                    .ok_or_else(|| fail("Invalid WebP chunk"))?;
                if kind == b"ANIM"
                    || kind == b"ANMF"
                    || (kind == b"VP8X" && n > 0 && bytes[p + 8] & 2 != 0)
                {
                    return Err(fail("Animated WebP conversion is unsupported"));
                }
                if !strip && !matches!(kind, b"VP8 " | b"VP8L" | b"VP8X" | b"ALPH") {
                    return Err(fail(
                        "WebP metadata requires explicit strip_metadata consent",
                    ));
                }
                p = end + n % 2;
            }
        }
        ImageFormat::Jpeg => {
            let mut p = 2;
            while p < bytes.len() {
                if bytes[p] != 0xff {
                    return Err(fail("Invalid JPEG marker"));
                }
                while p < bytes.len() && bytes[p] == 0xff {
                    p += 1;
                }
                if p >= bytes.len() {
                    break;
                }
                let marker = bytes[p];
                p += 1;
                if marker == 0xda || marker == 0xd9 {
                    break;
                }
                if marker == 0xd8 || (0xd0..=0xd7).contains(&marker) {
                    continue;
                }
                if p + 2 > bytes.len() {
                    return Err(fail("Truncated JPEG"));
                }
                let n = u16::from_be_bytes([bytes[p], bytes[p + 1]]) as usize;
                if n < 2 || p + n > bytes.len() {
                    return Err(fail("Invalid JPEG segment"));
                }
                if (0xc0..=0xcf).contains(&marker) && ![0xc4, 0xc8, 0xcc].contains(&marker) {
                    if n < 8 {
                        return Err(fail("Invalid JPEG frame"));
                    }
                    if bytes[p + 7] == 4 {
                        return Err(fail(
                            "CMYK/YCCK JPEG requires a color-managed conversion engine",
                        ));
                    }
                }
                // APP0 is ordinary JFIF framing; all other application data and
                // comments (including EXIF orientation) need stripping consent.
                if !strip && ((0xe1..=0xef).contains(&marker) || marker == 0xfe) {
                    return Err(fail(
                        "JPEG metadata requires explicit strip_metadata consent",
                    ));
                }
                p += n;
            }
        }
        _ => return Err(fail("Only PNG, JPEG and static WebP sources are supported")),
    }
    Ok(())
}

pub fn prepare(
    source: &Path,
    output: OutputFormat,
    options: ConversionOptions,
) -> Result<PreparedConversion, ConversionFailure> {
    let version = rename::capture(source).map_err(fail)?;
    let original = read_bounded(source)?;
    if rename::capture(source).map_err(fail)? != version {
        return Err(fail("Source changed while reading"));
    }
    let format = image::guess_format(&original).map_err(fail)?;
    inspect(&original, format, options.strip_metadata)?;
    let target = source.with_extension(output.extension());
    if target == source || target.try_exists().map_err(fail)? {
        return Err(fail("Destination already exists or is the source"));
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(&original), format);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_WORKING_BYTES);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(fail)?;
    check_working_set(decoder.dimensions(), original.len())?;
    if decoder.color_type().bits_per_pixel() / decoder.color_type().channel_count() as u16 != 8 {
        return Err(fail(
            "Only 8-bit images are supported; conversion would lose precision",
        ));
    }
    let orientation = decoder.orientation().map_err(fail)?;
    let mut pixels = DynamicImage::from_decoder(decoder).map_err(fail)?;
    if pixels.color().bits_per_pixel() / pixels.color().channel_count() as u16 != 8 {
        return Err(fail(
            "Only 8-bit images are supported; conversion would lose precision",
        ));
    }
    pixels.apply_orientation(orientation);
    let dimensions = pixels.dimensions();
    // Reserve the bound once: Cursor's geometric growth could otherwise leave
    // capacity above the encoded-byte budget even while its length is bounded.
    let mut encoded = BoundedOutput(Cursor::new(Vec::with_capacity(MAX_FILE_BYTES as usize)));
    match output {
        OutputFormat::Jpeg { quality } => {
            if !(1..=100).contains(&quality) {
                return Err(fail("JPEG quality must be 1..=100"));
            }
            if pixels.color().has_alpha() {
                return Err(fail("JPEG cannot preserve alpha; choose PNG or WebP"));
            }
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, quality)
                .encode_image(&pixels)
                .map_err(fail)?;
        }
        _ => pixels
            .write_to(&mut encoded, output.image_format())
            .map_err(fail)?,
    }
    let encoded = encoded.0.into_inner();
    let verified =
        image::load_from_memory_with_format(&encoded, output.image_format()).map_err(fail)?;
    if verified.dimensions() != dimensions {
        return Err(fail("Encoded dimensions failed verification"));
    }
    if !matches!(output, OutputFormat::Jpeg { .. }) && verified.to_rgba8() != pixels.to_rgba8() {
        return Err(fail("Lossless pixel verification failed"));
    }
    Ok(PreparedConversion {
        source: source.to_owned(),
        target,
        original,
        version,
        encoded,
        dimensions,
    })
}

static SERIAL: AtomicU64 = AtomicU64::new(0);
fn unique_name(label: &str) -> String {
    format!(
        ".photo-convert-{}-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        SERIAL.fetch_add(1, Ordering::Relaxed),
        label
    )
}
/// Saved-file identity, captured from the verified stage before publication.
#[derive(Debug)]
pub struct PublishedFile {
    pub path: PathBuf,
    pub version: SourceVersion,
}
impl PublishedFile {
    pub fn is_current(&self) -> bool {
        rename::capture(&self.path).is_ok_and(|version| version == self.version)
    }
}

/// Reuse the checked staging/backup/rollback transaction for same-path edits.
pub fn replace_same_file(
    source: &Path,
    version: &SourceVersion,
    original: Vec<u8>,
    encoded: Vec<u8>,
) -> Result<PublishedFile, ConversionFailure> {
    commit_checked(PreparedConversion {
        source: source.to_path_buf(),
        target: source.to_path_buf(),
        original,
        version: version.clone(),
        encoded,
        dimensions: (0, 0),
    })
}

/// Commits on the same volume. A crash can leave a staging/backup file; errors
/// list every retained path. Success removes the original only after verified
/// output has been published. Filesystems must support no-clobber rename.
pub fn commit(prepared: PreparedConversion) -> Result<PathBuf, ConversionFailure> {
    commit_checked(prepared).map(|published| published.path)
}

fn commit_checked(prepared: PreparedConversion) -> Result<PublishedFile, ConversionFailure> {
    let p = prepared;
    if rename::capture(&p.source).map_err(fail)? != p.version
        || read_bounded(&p.source)? != p.original
    {
        return Err(fail("Source changed since preparation"));
    }
    if p.target != p.source && p.target.try_exists().map_err(fail)? {
        return Err(fail("Destination already exists"));
    }
    let parent = p
        .source
        .parent()
        .ok_or_else(|| fail("Source has no parent"))?;
    let temp = parent.join(unique_name("output"));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(fail)?;
    if let Err(e) = file.write_all(&p.encoded).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&temp);
        return Err(fail(e));
    }
    drop(file);
    if read_bounded(&temp).ok().as_deref() != Some(p.encoded.as_slice()) {
        return Err(ConversionFailure {
            message: "Staged bytes failed verification".into(),
            recovery_paths: vec![p.source, temp],
        });
    }
    let backup = match rename::rename_checked(&p.source, &unique_name("original"), &p.version) {
        Ok(path) => path,
        Err(e) => {
            let _ = fs::remove_file(&temp);
            return Err(fail(e));
        }
    };
    // Re-read the captured original to catch edits during staging even on
    // filesystems with coarse modification timestamps.
    let unchanged = read_bounded(&backup).ok().as_deref() == Some(p.original.as_slice());
    let publish = if !unchanged {
        Err(std::io::Error::other(
            "Source content changed while staging",
        ))
    } else {
        rename::capture(&temp).and_then(|version| {
            rename::rename_checked(
                &temp,
                p.target.file_name().and_then(|s| s.to_str()).unwrap_or(""),
                &version,
            )
            .map(|path| PublishedFile { path, version })
        })
    };
    let published = match publish {
        Ok(published) => published,
        Err(e) => {
            let restore = rename::capture(&backup).and_then(|v| {
                rename::rename_checked(
                    &backup,
                    p.source.file_name().and_then(|s| s.to_str()).unwrap_or(""),
                    &v,
                )
            });
            let _ = fs::remove_file(&temp);
            return Err(ConversionFailure {
                message: format!("Publication failed: {e}; restoration: {restore:?}"),
                recovery_paths: if restore.is_ok() {
                    vec![p.source]
                } else {
                    vec![backup, p.source]
                },
            });
        }
    };
    if let Err(e) = fs::remove_file(&backup) {
        return Err(ConversionFailure {
            message: format!("Output published but original backup cleanup failed: {e}"),
            recovery_paths: vec![p.target, backup],
        });
    }
    Ok(published)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_reader_preserves_large_payload_and_rejects_size_changes() {
        let payload: Vec<u8> = (0..4 * 1024 * 1024 + 57)
            .map(|index| (index % 251) as u8)
            .collect();
        assert_eq!(
            read_bounded_bytes(Cursor::new(&payload), payload.len()).unwrap(),
            payload
        );
        for expected_size in [payload.len() - 1, payload.len() + 1] {
            assert!(
                read_bounded_bytes(Cursor::new(&payload), expected_size)
                    .unwrap_err()
                    .message
                    .contains("changed while reading")
            );
        }
        assert!(read_bounded_bytes(Cursor::new([]), MAX_FILE_BYTES as usize + 1).is_err());
    }

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(unique_name("test"));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn source(&self) -> PathBuf {
            let path = self.0.join("original.png");
            image::RgbaImage::from_fn(7, 5, |x, y| {
                image::Rgba([
                    x as u8 * 20,
                    y as u8 * 30,
                    100,
                    if x == 0 { 0 } else { 255 },
                ])
            })
            .save(&path)
            .unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn publication_receipt_survives_rename_and_rejects_external_replacement() {
        let dir = Fixture::new();
        let source = dir.source();
        let prepared = prepare(&source, OutputFormat::WebP, ConversionOptions::default()).unwrap();
        let published = commit_checked(prepared).unwrap();
        assert_eq!(published.version, rename::capture(&published.path).unwrap());
        assert!(published.is_current());

        // Identical bytes at the same path still belong to a different file.
        let bytes = fs::read(&published.path).unwrap();
        fs::rename(&published.path, dir.0.join("old.webp")).unwrap();
        fs::write(&published.path, bytes).unwrap();
        assert!(!published.is_current());
    }

    #[test]
    fn lossless_roundtrip_replaces_original() {
        let dir = Fixture::new();
        let source = dir.source();
        let original = image::open(&source).unwrap().to_rgba8();
        let prepared = prepare(&source, OutputFormat::WebP, ConversionOptions::default()).unwrap();
        assert_eq!(prepared.dimensions(), (7, 5));
        let target = commit(prepared).unwrap();
        assert!(!source.exists());
        assert_eq!(image::open(&target).unwrap().to_rgba8(), original);
        let result =
            commit(prepare(&target, OutputFormat::Png, ConversionOptions::default()).unwrap())
                .unwrap();
        assert_eq!(image::open(result).unwrap().to_rgba8(), original);
    }
    #[test]
    fn alpha_and_unsupported_preserve_original() {
        let dir = Fixture::new();
        let source = dir.source();
        let bytes = fs::read(&source).unwrap();
        assert!(
            prepare(
                &source,
                OutputFormat::Jpeg { quality: 90 },
                ConversionOptions::default()
            )
            .is_err()
        );
        assert_eq!(fs::read(&source).unwrap(), bytes);
        let gif = dir.0.join("still.gif");
        image::RgbImage::new(2, 2).save(&gif).unwrap();
        let bytes = fs::read(&gif).unwrap();
        assert!(prepare(&gif, OutputFormat::Png, ConversionOptions::default()).is_err());
        assert_eq!(fs::read(gif).unwrap(), bytes);
    }
    #[test]
    fn conflict_and_changed_source_are_not_overwritten() {
        let dir = Fixture::new();
        let source = dir.source();
        let bytes = fs::read(&source).unwrap();
        let prepared = prepare(&source, OutputFormat::WebP, ConversionOptions::default()).unwrap();
        fs::write(prepared.target(), b"keep target").unwrap();
        assert!(commit(prepared).is_err());
        assert_eq!(
            fs::read(source.with_extension("webp")).unwrap(),
            b"keep target"
        );
        assert_eq!(fs::read(&source).unwrap(), bytes);
        fs::remove_file(source.with_extension("webp")).unwrap();
        let prepared = prepare(&source, OutputFormat::WebP, ConversionOptions::default()).unwrap();
        fs::write(&source, b"replacement").unwrap();
        assert!(commit(prepared).is_err());
        assert_eq!(fs::read(&source).unwrap(), b"replacement");
        assert!(!source.with_extension("webp").exists());
    }
    #[test]
    fn animation_and_metadata_require_safe_handling() {
        // Container-level rejection occurs even before a decoder can accept a frame.
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend(0u32.to_be_bytes());
        png.extend(b"acTL");
        png.extend([0; 4]);
        assert!(inspect(&png, ImageFormat::Png, true).is_err());
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend(0u32.to_be_bytes());
        png.extend(b"iCCP");
        png.extend([0; 4]);
        assert!(inspect(&png, ImageFormat::Png, false).is_err());
        assert!(inspect(&png, ImageFormat::Png, true).is_ok());
    }

    #[test]
    fn exif_orientation_is_applied_only_with_metadata_consent() {
        let dir = Fixture::new();
        let source = dir.0.join("rotated.jpg");
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode_image(&image::RgbImage::new(7, 5))
            .unwrap();
        // Little-endian TIFF, one SHORT orientation entry with value 6.
        let exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
        let mut tagged = jpeg[..2].to_vec();
        tagged.extend([0xff, 0xe1]);
        tagged.extend(((exif.len() + 2) as u16).to_be_bytes());
        tagged.extend(exif);
        tagged.extend(&jpeg[2..]);
        fs::write(&source, tagged).unwrap();
        assert!(prepare(&source, OutputFormat::Png, ConversionOptions::default()).is_err());
        let prepared = prepare(
            &source,
            OutputFormat::Png,
            ConversionOptions {
                strip_metadata: true,
            },
        )
        .unwrap();
        assert_eq!(prepared.dimensions(), (5, 7));
        let target = commit(prepared).unwrap();
        assert_eq!(image::open(target).unwrap().dimensions(), (5, 7));
    }

    #[test]
    fn rejects_high_bit_depth_without_altering_source() {
        let dir = Fixture::new();
        let source = dir.0.join("deep.png");
        image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::new(2, 3)
            .save(&source)
            .unwrap();
        let bytes = fs::read(&source).unwrap();
        assert!(prepare(&source, OutputFormat::WebP, ConversionOptions::default()).is_err());
        assert_eq!(fs::read(source).unwrap(), bytes);
    }

    #[test]
    fn resource_bounds_reject_large_sources_headers_and_output() {
        let dir = Fixture::new();
        let large = dir.0.join("large.png");
        fs::File::create(&large)
            .unwrap()
            .set_len(MAX_FILE_BYTES + 1)
            .unwrap();
        assert!(prepare(&large, OutputFormat::WebP, ConversionOptions::default()).is_err());
        assert_eq!(fs::metadata(&large).unwrap().len(), MAX_FILE_BYTES + 1);
        assert!(check_working_set((100_000, 100_000), 1).is_err());
        assert!(check_working_set((4000, 4000), 1).is_err());
        let mut writer = BoundedOutput(Cursor::new(Vec::new()));
        writer.seek(SeekFrom::Start(MAX_FILE_BYTES)).unwrap();
        assert!(writer.write_all(&[1]).is_err());
        assert!(writer.seek(SeekFrom::Start(MAX_FILE_BYTES + 1)).is_err());
        // A truncated PNG advertises a chunk larger than the available input.
        let mut malformed = b"\x89PNG\r\n\x1a\n".to_vec();
        malformed.extend(u32::MAX.to_be_bytes());
        malformed.extend(b"IHDR");
        malformed.extend([0; 4]);
        assert!(inspect(&malformed, ImageFormat::Png, true).is_err());
    }

    #[test]
    fn cmyk_jpeg_is_rejected_even_with_strip_consent() {
        let frame = [0xff, 0xd8, 0xff, 0xc0, 0, 8, 8, 0, 1, 0, 1, 4];
        assert!(
            inspect(&frame, ImageFormat::Jpeg, true)
                .unwrap_err()
                .message
                .contains("CMYK")
        );
    }
}
