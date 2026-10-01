//! 按源图格式读写图片：除格式转换外，处理结果沿用源文件的实际编码格式，且不降低画质。

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::codecs::tiff::TiffEncoder;
use image::codecs::webp::WebPEncoder;
use image::{DynamicImage, ImageDecoder, ImageEncoder, ImageFormat, ImageReader};
use std::io::Cursor;
use std::path::Path;
use std::sync::OnceLock;

/// 重新编码 JPEG 的最低质量：低质量源图若按原质量再压一次，损失会叠加一代
const JPEG_MIN_QUALITY: u8 = 95;

pub(crate) fn read_dimensions(path: &Path) -> image::ImageResult<(u32, u32)> {
    ImageReader::open(path)?
        .with_guessed_format()?
        .into_dimensions()
}

pub(crate) fn probe_has_alpha_channel(path: &Path) -> Result<bool, String> {
    let reader = ImageReader::open(path).map_err(|e| format!("无法打开图片: {}", e))?;
    let decoder = reader
        .with_guessed_format()
        .map_err(|e| format!("无法识别图片格式: {}", e))?
        .into_decoder()
        .map_err(|e| format!("无法解码图片: {}", e))?;
    Ok(decoder.color_type().has_alpha())
}

pub(crate) fn flatten_preserving_depth(img: DynamicImage, bg: [u8; 3]) -> DynamicImage {
    match img.color() {
        image::ColorType::La16 | image::ColorType::Rgba16 => {
            let rgba = img.to_rgba16();
            let rgb = image::ImageBuffer::from_fn(rgba.width(), rgba.height(), |x, y| {
                let pixel = rgba.get_pixel(x, y);
                let alpha = u64::from(pixel[3]);
                image::Rgb(std::array::from_fn(|c| {
                    ((u64::from(pixel[c]) * alpha + u64::from(bg[c]) * 257 * (65535 - alpha))
                        / 65535) as u16
                }))
            });
            DynamicImage::ImageRgb16(rgb)
        }
        image::ColorType::Rgba32F => {
            let rgba = img.to_rgba32f();
            DynamicImage::ImageRgb32F(image::ImageBuffer::from_fn(
                rgba.width(),
                rgba.height(),
                |x, y| {
                    let p = rgba.get_pixel(x, y);
                    image::Rgb(std::array::from_fn(|c| {
                        p[c] * p[3] + f32::from(bg[c]) / 255.0 * (1.0 - p[3])
                    }))
                },
            ))
        }
        _ => super::flatten_onto(img, bg),
    }
}

/// JPEG 量化表，按表号（0-3）存放，数值为文件内的 zigzag 顺序
type JpegTables = [Option<[u16; 64]>; 4];

/// 源图的编码信息，写回时据此选择格式与参数
pub(crate) struct SourceInfo {
    /// magic bytes 识别出的实际格式（扩展名可能与之不符）
    pub format: ImageFormat,
    icc_profile: Option<Vec<u8>>,
    jpeg_tables: Option<JpegTables>,
}

fn read_source(path: &Path) -> Result<(Vec<u8>, ImageFormat), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("无法打开图片: {}", e))?;
    let format = image::guess_format(&bytes)
        .ok()
        .or_else(|| ImageFormat::from_path(path).ok())
        .ok_or("无法识别图片格式")?;
    Ok((bytes, format))
}

fn decoder_for(bytes: &[u8], format: ImageFormat) -> Result<impl ImageDecoder + '_, String> {
    let mut reader = ImageReader::new(Cursor::new(bytes));
    reader.set_format(format);
    reader
        .into_decoder()
        .map_err(|e| format!("无法解码图片: {}", e))
}

fn source_info(bytes: &[u8], format: ImageFormat, icc_profile: Option<Vec<u8>>) -> SourceInfo {
    SourceInfo {
        format,
        icc_profile,
        jpeg_tables: (format == ImageFormat::Jpeg).then(|| parse_dqt(bytes)),
    }
}

/// 读取并解码图片，同时记录写回所需的源图信息
pub(crate) fn load_image(path: &Path) -> Result<(DynamicImage, SourceInfo), String> {
    let (bytes, format) = read_source(path)?;
    let mut decoder = decoder_for(&bytes, format)?;
    let icc_profile = decoder.icc_profile().ok().flatten();
    let image = DynamicImage::from_decoder(decoder).map_err(|e| format!("无法解码图片: {}", e))?;
    Ok((image, source_info(&bytes, format, icc_profile)))
}

/// 只读取源图信息、不解码像素（外部程序产出结果后按源图格式写回时用）
pub(crate) fn probe_image(path: &Path) -> Result<SourceInfo, String> {
    let (bytes, format) = read_source(path)?;
    let icc_profile = decoder_for(&bytes, format)
        .ok()
        .and_then(|mut d| d.icc_profile().ok().flatten());
    Ok(source_info(&bytes, format, icc_profile))
}

/// 按源图格式编码并写到 `path`：JPEG 的量化不比源图粗且质量不低于 `JPEG_MIN_QUALITY`，
/// 不做色度抽样；WebP 用无损编码；PNG/BMP/TIFF/GIF 本身无损。源图的 ICC 配置文件一并写回。
/// 先在内存里编码完再写文件，编码失败不会截断已有的同名文件。
pub(crate) fn save_like_source(
    img: DynamicImage,
    path: &Path,
    source: &SourceInfo,
) -> Result<(), String> {
    let img = if source.format == ImageFormat::Jpeg && img.color().has_alpha() {
        super::flatten_to_rgb_white(img)
    } else {
        img
    };
    let color = img.color().has_color();
    let icc = source.icc_profile.clone().filter(|p| icc_matches(p, color));
    let mut buf = Cursor::new(Vec::new());
    let encoded = match source.format {
        ImageFormat::Jpeg => {
            let quality = source
                .jpeg_tables
                .as_ref()
                .map_or(100, |t| jpeg_quality_for(t, color));
            encode_with_icc(&img, JpegEncoder::new_with_quality(&mut buf, quality), icc)
        }
        ImageFormat::Png => encode_with_icc(&img, PngEncoder::new(&mut buf), icc),
        ImageFormat::WebP => encode_with_icc(&img, WebPEncoder::new_lossless(&mut buf), icc),
        ImageFormat::Tiff => encode_with_icc(&img, TiffEncoder::new(&mut buf), icc),
        other => img.write_to(&mut buf, other),
    };
    encoded.map_err(|e| format!("无法编码图片: {}", e))?;
    std::fs::write(path, buf.into_inner()).map_err(|e| format!("无法保存图片: {}", e))
}

/// 有 ICC 配置文件时先交给编码器（编码器不支持就不写），再编码
fn encode_with_icc(
    img: &DynamicImage,
    mut encoder: impl ImageEncoder,
    icc: Option<Vec<u8>>,
) -> image::ImageResult<()> {
    if let Some(p) = icc {
        let _ = encoder.set_icc_profile(p);
    }
    img.write_with_encoder(encoder)
}

/// 配置文件的色彩空间与输出一致才写回（CMYK JPEG 解码成 RGB 后，原配置文件已不适用）
fn icc_matches(profile: &[u8], color: bool) -> bool {
    let expected: &[u8] = if color { b"RGB " } else { b"GRAY" };
    profile.get(16..20) == Some(expected)
}

/// 解析 JPEG 的 DQT 段，同一表号以最后一次定义为准
fn parse_dqt(bytes: &[u8]) -> JpegTables {
    let mut tables: JpegTables = [None; 4];
    if bytes.len() < 4 || bytes[..2] != [0xFF, 0xD8] {
        return tables;
    }
    let mut i = 2;
    while i + 4 <= bytes.len() && bytes[i] == 0xFF {
        let marker = bytes[i + 1];
        if marker == 0xFF {
            i += 1;
            continue;
        }
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            i += 2;
            continue;
        }
        // 量化表都在扫描数据之前
        if marker == 0xDA || marker == 0xD9 {
            break;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if len < 2 || i + 2 + len > bytes.len() {
            break;
        }
        if marker == 0xDB {
            let seg = &bytes[i + 4..i + 2 + len];
            let mut p = 0;
            while p < seg.len() {
                let wide = seg[p] >> 4 != 0;
                let id = (seg[p] & 0x0F) as usize;
                p += 1;
                let size = if wide { 128 } else { 64 };
                if p + size > seg.len() {
                    break;
                }
                let mut table = [0u16; 64];
                for (k, v) in table.iter_mut().enumerate() {
                    *v = if wide {
                        u16::from_be_bytes([seg[p + 2 * k], seg[p + 2 * k + 1]])
                    } else {
                        seg[p + k] as u16
                    };
                }
                if id < tables.len() {
                    tables[id] = Some(table);
                }
                p += size;
            }
        }
        i += 2 + len;
    }
    tables
}

/// image 库在质量 1..=100 下写出的 [亮度, 色度] 量化表
fn encoder_tables() -> &'static [[[u16; 64]; 2]; 100] {
    static TABLES: OnceLock<[[[u16; 64]; 2]; 100]> = OnceLock::new();
    TABLES.get_or_init(|| {
        let probe = DynamicImage::ImageRgb8(image::RgbImage::new(8, 8));
        let mut out = [[[0u16; 64]; 2]; 100];
        for (i, slot) in out.iter_mut().enumerate() {
            let mut buf = Cursor::new(Vec::new());
            probe
                .write_with_encoder(JpegEncoder::new_with_quality(&mut buf, i as u8 + 1))
                .expect("8x8 RGB 图编码不会失败");
            let t = parse_dqt(buf.get_ref());
            *slot = [
                t[0].expect("image 库总会写出亮度量化表"),
                t[1].expect("彩色图总会写出色度量化表"),
            ];
        }
        out
    })
}

/// 选出每个量化步长都不比源图粗的最低质量，再与 `JPEG_MIN_QUALITY` 取大
fn jpeg_quality_for(tables: &JpegTables, color: bool) -> u8 {
    let Some(luma) = tables[0] else {
        return 100;
    };
    let chroma = tables[1..]
        .iter()
        .flatten()
        .copied()
        .reduce(|a, b| std::array::from_fn(|k| a[k].min(b[k])))
        .unwrap_or(luma);
    let fits = |enc: &[u16; 64], src: &[u16; 64]| enc.iter().zip(src).all(|(e, s)| e <= s);
    encoder_tables()
        .iter()
        .position(|[enc_luma, enc_chroma]| {
            fits(enc_luma, &luma) && (!color || fits(enc_chroma, &chroma))
        })
        .map_or(100, |i| (i as u8 + 1).max(JPEG_MIN_QUALITY))
}

#[cfg(test)]
pub(crate) fn test_dir(label: &str) -> std::path::PathBuf {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "purin_imageops_{}_{}_{}",
        label,
        std::process::id(),
        SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[cfg(test)]
pub(crate) fn write_broken_pixels(path: &Path) {
    image::RgbImage::new(32, 32).save(path).unwrap();
    let mut bytes = std::fs::read(path).unwrap();
    let idat = bytes.windows(4).position(|chunk| chunk == b"IDAT").unwrap();
    bytes[idat + 4] ^= 0xff;
    std::fs::write(path, bytes).unwrap();
    assert_eq!(read_dimensions(path).unwrap(), (32, 32));
    assert!(load_image(path).is_err());
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;
    use std::path::PathBuf;

    #[test]
    fn dimensions_follow_content_not_extension() {
        let root = test_dir("dimensions");
        let path = root.join("webp.png");
        image::RgbImage::new(17, 23)
            .save_with_format(&path, ImageFormat::WebP)
            .unwrap();
        assert_eq!(read_dimensions(&path).unwrap(), (17, 23));
        assert!(!probe_has_alpha_channel(&path).unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("purinbox_image_io_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn noisy_rgb(w: u32, h: u32) -> DynamicImage {
        let mut seed = 7u32;
        DynamicImage::ImageRgb8(image::RgbImage::from_fn(w, h, |_, _| {
            let mut next = || {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                (seed >> 16) as u8
            };
            image::Rgb([next(), next(), next()])
        }))
    }

    fn jpeg_bytes(img: &DynamicImage, quality: u8) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        img.write_with_encoder(JpegEncoder::new_with_quality(&mut buf, quality))
            .unwrap();
        buf.into_inner()
    }

    fn fake_icc(space: &[u8; 4]) -> Vec<u8> {
        let mut p = vec![0u8; 128];
        p[16..20].copy_from_slice(space);
        p
    }

    #[test]
    fn jpeg_quality_never_below_source_or_floor() {
        let img = noisy_rgb(16, 16);
        for (src_q, expected) in [(100, 100), (98, 98), (96, 96), (95, 95), (80, 95), (40, 95)] {
            let tables = parse_dqt(&jpeg_bytes(&img, src_q));
            assert_eq!(
                jpeg_quality_for(&tables, true),
                expected,
                "源质量 {}",
                src_q
            );
        }
        assert_eq!(jpeg_quality_for(&[None; 4], true), 100);
    }

    #[test]
    fn saved_jpeg_keeps_source_quantization() {
        let dir = temp_dir("jpeg");
        let src = dir.join("src.jpg");
        std::fs::write(&src, jpeg_bytes(&noisy_rgb(64, 64), 98)).unwrap();

        let (img, info) = load_image(&src).unwrap();
        let out = dir.join("out.jpg");
        save_like_source(img.crop_imm(3, 5, 40, 40), &out, &info).unwrap();

        assert_eq!(
            parse_dqt(&std::fs::read(&out).unwrap()),
            parse_dqt(&std::fs::read(&src).unwrap())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeps_actual_format_when_extension_differs() {
        let dir = temp_dir("mismatch");
        let src = dir.join("png_inside.jpg");
        noisy_rgb(32, 32)
            .save_with_format(&src, ImageFormat::Png)
            .unwrap();

        let (img, info) = load_image(&src).unwrap();
        assert_eq!(info.format, ImageFormat::Png);
        let out = dir.join("out.jpg");
        save_like_source(img.crop_imm(0, 0, 16, 16), &out, &info).unwrap();

        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(image::guess_format(&bytes).unwrap(), ImageFormat::Png);
        assert_eq!(
            image::load_from_memory(&bytes).unwrap().dimensions(),
            (16, 16)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn webp_is_rewritten_lossless() {
        let dir = temp_dir("webp");
        let src = dir.join("a.webp");
        let img = noisy_rgb(24, 24);
        img.save(&src).unwrap();

        let (decoded, info) = load_image(&src).unwrap();
        let out = dir.join("out.webp");
        save_like_source(decoded, &out, &info).unwrap();

        assert_eq!(image::open(&out).unwrap().to_rgb8(), img.to_rgb8());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn icc_profile_round_trips_only_when_color_space_matches() {
        let dir = temp_dir("icc");
        let src = dir.join("a.png");
        let img = noisy_rgb(8, 8);
        let mut buf = Cursor::new(Vec::new());
        let mut encoder = PngEncoder::new(&mut buf);
        encoder.set_icc_profile(fake_icc(b"RGB ")).unwrap();
        img.write_with_encoder(encoder).unwrap();
        std::fs::write(&src, buf.into_inner()).unwrap();

        let (decoded, info) = load_image(&src).unwrap();
        let out = dir.join("out.png");
        save_like_source(decoded.clone(), &out, &info).unwrap();
        assert_eq!(
            probe_image(&out).unwrap().icc_profile,
            Some(fake_icc(b"RGB "))
        );

        // 灰度输出不写 RGB 配置文件
        let gray = dir.join("gray.png");
        save_like_source(DynamicImage::ImageLuma8(decoded.to_luma8()), &gray, &info).unwrap();
        assert_eq!(probe_image(&gray).unwrap().icc_profile, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn transparent_pixels_flatten_to_white_for_jpeg() {
        let dir = temp_dir("alpha_jpeg");
        let src = dir.join("a.jpg");
        std::fs::write(&src, jpeg_bytes(&noisy_rgb(8, 8), 95)).unwrap();
        let info = probe_image(&src).unwrap();

        let rgba = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            8,
            8,
            image::Rgba([0, 0, 0, 0]),
        ));
        let out = dir.join("out.jpg");
        save_like_source(rgba, &out, &info).unwrap();
        let px = image::open(&out).unwrap().to_rgb8().get_pixel(4, 4).0;
        assert!(px.iter().all(|&c| c > 240), "{:?}", px);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
