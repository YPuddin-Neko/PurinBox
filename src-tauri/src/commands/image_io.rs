//! 按源图格式读写图片：除格式转换外，处理结果沿用源文件的实际编码格式，且不降低画质。

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::codecs::tiff::TiffEncoder;
use image::codecs::webp::WebPEncoder;
use image::{
    DynamicImage, ImageBuffer, ImageDecoder, ImageEncoder, ImageFormat, ImageReader, Pixel,
};
use std::io::{Cursor, Read};
use std::path::Path;
use std::sync::OnceLock;

/// 重新编码 JPEG 的最低质量：低质量源图若按原质量再压一次，损失会叠加一代
const JPEG_MIN_QUALITY: u8 = 95;

pub(crate) fn read_dimensions(path: &Path) -> image::ImageResult<(u32, u32)> {
    ImageReader::open(path)?
        .with_guessed_format()?
        .into_dimensions()
}

/// 按文件头的魔数识别实际编码格式，不看扩展名；读不到或识别不出时为 None
pub(crate) fn sniff_format(path: &Path) -> Option<ImageFormat> {
    let mut head = Vec::with_capacity(64);
    std::fs::File::open(path)
        .ok()?
        .take(64)
        .read_to_end(&mut head)
        .ok()?;
    image::guess_format(&head).ok()
}

/// 按内容识别格式（识别不出时按扩展名）并解码，供缩略图、指纹和格式转换使用。
/// 保留 image 库默认的 512 MiB 上限：解码前按整张图的字节数预留，超限直接报错，
/// 不会为一张预览图先申请几个 GB。按源格式写回的处理用 `load_image`，它不做整图预留
pub(crate) fn decode_image(path: &Path) -> Result<DynamicImage, String> {
    ImageReader::open(path)
        .map_err(|e| format!("无法打开图片: {}", e))?
        .with_guessed_format()
        .map_err(|e| format!("无法识别图片格式: {}", e))?
        .decode()
        .map_err(|e| format!("无法解码图片: {}", e))
}

/// 逐像素运算的通道类型（u8 / u16 / f32）：运算在 f64 上做，写回时整数通道四舍五入并钳位到取值范围，
/// 浮点通道原样写回
pub(crate) trait Channel: image::Primitive + Into<f64> {
    fn from_f64(value: f64) -> Self;
}

impl Channel for u8 {
    fn from_f64(value: f64) -> Self {
        value.round().clamp(0.0, 255.0) as u8
    }
}

impl Channel for u16 {
    fn from_f64(value: f64) -> Self {
        value.round().clamp(0.0, 65535.0) as u16
    }
}

impl Channel for f32 {
    fn from_f64(value: f64) -> Self {
        value as f32
    }
}

/// 不改变像素类型的逐像素处理，由 [`map_pixels`] 按图片自身的像素类型实例化
pub(crate) trait PixelMap {
    fn map<P>(&self, img: ImageBuffer<P, Vec<P::Subpixel>>) -> ImageBuffer<P, Vec<P::Subpixel>>
    where
        P: Pixel,
        P::Subpixel: Channel;
}

/// 按图片自身的颜色类型和位深处理：灰度仍是灰度，没有 alpha 的不会多出 alpha，16 位、浮点不降位深
pub(crate) fn map_pixels(img: DynamicImage, op: &impl PixelMap) -> DynamicImage {
    use DynamicImage as D;
    match img {
        D::ImageLuma8(b) => D::ImageLuma8(op.map(b)),
        D::ImageLumaA8(b) => D::ImageLumaA8(op.map(b)),
        D::ImageRgb8(b) => D::ImageRgb8(op.map(b)),
        D::ImageRgba8(b) => D::ImageRgba8(op.map(b)),
        D::ImageLuma16(b) => D::ImageLuma16(op.map(b)),
        D::ImageLumaA16(b) => D::ImageLumaA16(op.map(b)),
        D::ImageRgb16(b) => D::ImageRgb16(op.map(b)),
        D::ImageRgba16(b) => D::ImageRgba16(op.map(b)),
        D::ImageRgb32F(b) => D::ImageRgb32F(op.map(b)),
        D::ImageRgba32F(b) => D::ImageRgba32F(op.map(b)),
        other => D::ImageRgba32F(op.map(other.into_rgba32f())),
    }
}

/// 转成 RGBA，位深不低于源图：16 位（含 16 位灰度）→ Rgba16，32 位浮点 → Rgba32F，其余 → Rgba8
pub(crate) fn into_rgba_keeping_depth(img: DynamicImage) -> DynamicImage {
    use image::ColorType as C;
    match img.color() {
        C::L16 | C::La16 | C::Rgb16 | C::Rgba16 => DynamicImage::ImageRgba16(img.into_rgba16()),
        C::Rgb32F | C::Rgba32F => DynamicImage::ImageRgba32F(img.into_rgba32f()),
        _ => DynamicImage::ImageRgba8(img.into_rgba8()),
    }
}

/// 文件头里的颜色信息（`probe_header` 的结果）
#[derive(Debug)]
pub(crate) struct ImageHeader {
    pub color: image::ColorType,
    pub has_icc: bool,
}

impl ImageHeader {
    /// 每个通道超过 8 位（16 位整数或 32 位浮点）
    pub(crate) fn high_bit_depth(&self) -> bool {
        self.color.bytes_per_pixel() > self.color.channel_count()
    }
}

/// 按内容识别格式（识别不出时按扩展名）后读文件头，不解码像素。
/// JPEG 的解码器一建好就把整个文件读进内存：只关心某几种格式时先用 `sniff_format` 筛掉别的
pub(crate) fn probe_header(path: &Path) -> Result<ImageHeader, String> {
    let mut decoder = ImageReader::open(path)
        .map_err(|e| format!("无法打开图片: {}", e))?
        .with_guessed_format()
        .map_err(|e| format!("无法识别图片格式: {}", e))?
        .into_decoder()
        .map_err(|e| format!("无法解码图片: {}", e))?;
    Ok(ImageHeader {
        color: decoder.color_type(),
        has_icc: decoder.icc_profile().ok().flatten().is_some(),
    })
}

pub(crate) fn probe_has_alpha_channel(path: &Path) -> Result<bool, String> {
    probe_header(path).map(|header| header.color.has_alpha())
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

/// 测试用：写一张文件头完好、像素数据损坏的 32x32 PNG——只读尺寸能成功，解码像素会失败
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
    use crate::commands::test_support::TempDir;
    use image::GenericImageView;

    #[test]
    fn dimensions_follow_content_not_extension() {
        let root = TempDir::new("image_io_dimensions");
        let path = root.join("webp.png");
        image::RgbImage::new(17, 23)
            .save_with_format(&path, ImageFormat::WebP)
            .unwrap();
        assert_eq!(read_dimensions(&path).unwrap(), (17, 23));
        assert!(!probe_has_alpha_channel(&path).unwrap());
    }

    #[test]
    fn header_reports_color_depth_and_icc() {
        let root = TempDir::new("image_io_header");
        let deep = root.join("deep.png");
        DynamicImage::ImageRgb16(ImageBuffer::new(4, 4))
            .save(&deep)
            .unwrap();
        let header = probe_header(&deep).unwrap();
        assert_eq!(header.color, image::ColorType::Rgb16);
        assert!(header.high_bit_depth() && !header.has_icc);
        let gray = root.join("gray16.png");
        DynamicImage::ImageLuma16(ImageBuffer::new(4, 4))
            .save(&gray)
            .unwrap();
        assert!(probe_header(&gray).unwrap().high_bit_depth());

        let with_icc = root.join("icc.png");
        let mut buf = Cursor::new(Vec::new());
        let mut encoder = PngEncoder::new(&mut buf);
        encoder.set_icc_profile(fake_icc(b"RGB ")).unwrap();
        noisy_rgb(4, 4).write_with_encoder(encoder).unwrap();
        std::fs::write(&with_icc, buf.into_inner()).unwrap();
        let header = probe_header(&with_icc).unwrap();
        assert!(header.has_icc && !header.high_bit_depth());

        // 按内容而不是扩展名选解码器
        let jpeg_named_png = root.join("photo.png");
        noisy_rgb(4, 4)
            .save_with_format(&jpeg_named_png, ImageFormat::Jpeg)
            .unwrap();
        assert_eq!(
            probe_header(&jpeg_named_png).unwrap().color,
            image::ColorType::Rgb8
        );

        let text = root.join("broken.png");
        std::fs::write(&text, b"plain text").unwrap();
        assert!(probe_header(&text).unwrap_err().starts_with("无法解码图片"));
        assert!(probe_header(&root.join("missing.png"))
            .unwrap_err()
            .starts_with("无法打开图片"));
    }

    #[test]
    fn sniff_format_reads_magic_bytes_only() {
        let root = TempDir::new("image_io_sniff");
        let jpeg_named_png = root.join("photo.png");
        noisy_rgb(8, 8)
            .save_with_format(&jpeg_named_png, ImageFormat::Jpeg)
            .unwrap();
        assert_eq!(sniff_format(&jpeg_named_png), Some(ImageFormat::Jpeg));
        let webp = root.join("a.webp");
        noisy_rgb(8, 8).save(&webp).unwrap();
        assert_eq!(sniff_format(&webp), Some(ImageFormat::WebP));
        // 扩展名不能代替内容
        let text = root.join("fake.jpg");
        std::fs::write(&text, b"plain text").unwrap();
        assert_eq!(sniff_format(&text), None);
        assert_eq!(sniff_format(&root.join("missing.png")), None);
    }

    /// 只写 PNG 文件头声明的尺寸，像素数据为空
    fn png_header_only(path: &Path, width: u32, height: u32) {
        fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
            out.extend((data.len() as u32).to_be_bytes());
            out.extend(kind);
            out.extend(data);
            let mut crc = flate2::Crc::new();
            crc.update(kind);
            crc.update(data);
            out.extend(crc.sum().to_be_bytes());
        }
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend(width.to_be_bytes());
        ihdr.extend(height.to_be_bytes());
        ihdr.extend([8, 6, 0, 0, 0]);
        chunk(&mut bytes, b"IHDR", &ihdr);
        chunk(&mut bytes, b"IDAT", &[]);
        chunk(&mut bytes, b"IEND", &[]);
        std::fs::write(path, bytes).unwrap();
    }

    /// 缩略图、指纹走的解码要先按整张图预留内存：声明 20000x20000 RGBA（约 1.5 GB）的图在分配前就被拒绝
    #[test]
    fn decode_image_keeps_the_default_memory_limit() {
        let root = TempDir::new("image_io_decode_limit");
        let huge = root.join("huge.png");
        png_header_only(&huge, 20_000, 20_000);
        assert_eq!(read_dimensions(&huge).unwrap(), (20_000, 20_000));
        let err = decode_image(&huge).unwrap_err();
        assert!(err.contains("limit"), "{}", err);

        let jpeg_named_png = root.join("a.png");
        noisy_rgb(9, 7)
            .save_with_format(&jpeg_named_png, ImageFormat::Jpeg)
            .unwrap();
        assert_eq!(decode_image(&jpeg_named_png).unwrap().dimensions(), (9, 7));
        let text = root.join("b.png");
        std::fs::write(&text, b"plain text").unwrap();
        assert!(decode_image(&text).unwrap_err().starts_with("无法解码图片"));
    }

    #[test]
    fn channels_round_and_clamp_integers_but_keep_floats() {
        assert_eq!(u8::from_f64(254.6), 255);
        assert_eq!(u8::from_f64(300.0), 255);
        assert_eq!(u8::from_f64(-3.0), 0);
        assert_eq!(u16::from_f64(1000.49), 1000);
        assert_eq!(u16::from_f64(70000.0), 65535);
        assert_eq!(f32::from_f64(1.25), 1.25);
        assert_eq!(f32::from_f64(-0.5), -0.5);
    }

    /// 每个通道加 1（整数通道钳位），用来检查 map_pixels 不改变像素类型
    struct AddOne;

    impl PixelMap for AddOne {
        fn map<P>(
            &self,
            mut img: ImageBuffer<P, Vec<P::Subpixel>>,
        ) -> ImageBuffer<P, Vec<P::Subpixel>>
        where
            P: Pixel,
            P::Subpixel: Channel,
        {
            for pixel in img.pixels_mut() {
                for c in pixel.channels_mut() {
                    *c = P::Subpixel::from_f64((*c).into() + 1.0);
                }
            }
            img
        }
    }

    #[test]
    fn map_pixels_keeps_color_type_and_depth() {
        let images = [
            DynamicImage::ImageLuma8(image::GrayImage::from_pixel(2, 2, image::Luma([255]))),
            DynamicImage::ImageLumaA8(image::GrayAlphaImage::new(2, 2)),
            DynamicImage::ImageRgb8(image::RgbImage::new(2, 2)),
            DynamicImage::ImageRgba8(image::RgbaImage::new(2, 2)),
            DynamicImage::ImageLuma16(ImageBuffer::new(2, 2)),
            DynamicImage::ImageLumaA16(ImageBuffer::new(2, 2)),
            DynamicImage::ImageRgb16(ImageBuffer::new(2, 2)),
            DynamicImage::ImageRgba16(ImageBuffer::new(2, 2)),
            DynamicImage::ImageRgb32F(ImageBuffer::new(2, 2)),
            DynamicImage::ImageRgba32F(ImageBuffer::new(2, 2)),
        ];
        for img in images {
            let color = img.color();
            let mapped = map_pixels(img, &AddOne);
            assert_eq!(mapped.color(), color);
        }
        let luma = map_pixels(
            DynamicImage::ImageLuma8(image::GrayImage::from_pixel(1, 1, image::Luma([255]))),
            &AddOne,
        );
        assert_eq!(luma.to_luma8().get_pixel(0, 0).0, [255]);
        let float = map_pixels(DynamicImage::ImageRgb32F(ImageBuffer::new(1, 1)), &AddOne);
        assert_eq!(float.to_rgb32f().get_pixel(0, 0).0, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn rgba_conversion_keeps_depth() {
        use image::ColorType as C;
        for (img, expected) in [
            (DynamicImage::ImageLuma16(ImageBuffer::new(1, 1)), C::Rgba16),
            (DynamicImage::ImageRgb16(ImageBuffer::new(1, 1)), C::Rgba16),
            (
                DynamicImage::ImageRgb32F(ImageBuffer::new(1, 1)),
                C::Rgba32F,
            ),
            (
                DynamicImage::ImageLuma8(image::GrayImage::new(1, 1)),
                C::Rgba8,
            ),
            (
                DynamicImage::ImageRgb8(image::RgbImage::new(1, 1)),
                C::Rgba8,
            ),
        ] {
            assert_eq!(into_rgba_keeping_depth(img).color(), expected);
        }
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
        let dir = TempDir::new("image_io_jpeg");
        let src = dir.join("src.jpg");
        std::fs::write(&src, jpeg_bytes(&noisy_rgb(64, 64), 98)).unwrap();

        let (img, info) = load_image(&src).unwrap();
        let out = dir.join("out.jpg");
        save_like_source(img.crop_imm(3, 5, 40, 40), &out, &info).unwrap();

        assert_eq!(
            parse_dqt(&std::fs::read(&out).unwrap()),
            parse_dqt(&std::fs::read(&src).unwrap())
        );
    }

    #[test]
    fn keeps_actual_format_when_extension_differs() {
        let dir = TempDir::new("image_io_mismatch");
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
    }

    #[test]
    fn webp_is_rewritten_lossless() {
        let dir = TempDir::new("image_io_webp");
        let src = dir.join("a.webp");
        let img = noisy_rgb(24, 24);
        img.save(&src).unwrap();

        let (decoded, info) = load_image(&src).unwrap();
        let out = dir.join("out.webp");
        save_like_source(decoded, &out, &info).unwrap();

        assert_eq!(image::open(&out).unwrap().to_rgb8(), img.to_rgb8());
    }

    #[test]
    fn icc_profile_round_trips_only_when_color_space_matches() {
        let dir = TempDir::new("image_io_icc");
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
    }

    #[test]
    fn transparent_pixels_flatten_to_white_for_jpeg() {
        let dir = TempDir::new("image_io_alpha_jpeg");
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
    }
}
