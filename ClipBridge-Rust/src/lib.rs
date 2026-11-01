use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Cursor, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const MAX_BYTES: usize = 128 * 1024 * 1024;
const MAX_PIXELS: usize = 32_000_000;

pub enum Capture {
    Png(Vec<u8>),
    Dib(Vec<u8>),
}

fn u16_at(b: &[u8], at: usize) -> Result<u16, String> {
    Ok(u16::from_le_bytes(
        b.get(at..at + 2)
            .ok_or("DIB bị cắt ngắn")?
            .try_into()
            .unwrap(),
    ))
}
fn u32_at(b: &[u8], at: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(
        b.get(at..at + 4)
            .ok_or("DIB bị cắt ngắn")?
            .try_into()
            .unwrap(),
    ))
}

/// Decode packed Windows DIB without depending on GDI handles or a GUI runtime.
pub fn dib_rgba(b: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let header = u32_at(b, 0)? as usize;
    if header < 40 || header > b.len() {
        return Err("DIB header không hỗ trợ".into());
    }
    let w = u32_at(b, 4)? as i32;
    let signed_h = u32_at(b, 8)? as i32;
    if w <= 0 || signed_h == 0 || signed_h == i32::MIN {
        return Err("Kích thước ảnh không hợp lệ".into());
    }
    let h = signed_h.unsigned_abs();
    if w as u64 * h as u64 > MAX_PIXELS as u64 {
        return Err("Ảnh vượt giới hạn 32 megapixel".into());
    }
    if u16_at(b, 12)? != 1 {
        return Err("DIB planes không hợp lệ".into());
    }
    let depth = u16_at(b, 14)? as usize;
    let compression = u32_at(b, 16)?;
    if !(depth == 24 || depth == 32) || !(compression == 0 || (compression == 3 && depth == 32)) {
        return Err("Chỉ hỗ trợ DIB 24/32 bit không nén hoặc bitfields".into());
    }
    let mut offset = header;
    let mut masks = [0x00ff0000, 0x0000ff00, 0x000000ff, 0];
    if compression == 3 {
        let at = if header >= 52 { 40 } else { offset };
        for (i, mask) in masks.iter_mut().take(3).enumerate() {
            *mask = u32_at(b, at + i * 4)?;
        }
        if header == 40 {
            offset += 12;
        }
        if header >= 56 {
            masks[3] = u32_at(b, 52)?;
        }
        for i in 0..3 {
            if masks[i] == 0
                || masks[i].count_ones() != 8
                || masks[i] >> masks[i].trailing_zeros() != 255
            {
                return Err("DIB bitfields phải có kênh màu 8 bit".into());
            }
            for j in 0..i {
                if masks[i] & masks[j] != 0 {
                    return Err("DIB bitfields trùng nhau".into());
                }
            }
        }
        if masks[3] != 0
            && (masks[3].count_ones() != 8
                || masks[3] >> masks[3].trailing_zeros() != 255
                || masks[..3].iter().any(|m| m & masks[3] != 0))
        {
            return Err("DIB alpha mask không hợp lệ".into());
        }
    } else if header >= 56 {
        masks[3] = u32_at(b, 52)?;
        if masks[3] != 0 && masks[3] != 0xff000000 {
            return Err("DIB BI_RGB alpha mask không hỗ trợ".into());
        }
    }
    // A color table is permitted even with true-color DIBs.
    offset = offset
        .checked_add(
            (u32_at(b, 32)? as usize)
                .checked_mul(4)
                .ok_or("DIB overflow")?,
        )
        .ok_or("DIB overflow")?;
    let stride = ((w as usize * depth).div_ceil(32)) * 4;
    let length = stride.checked_mul(h as usize).ok_or("DIB overflow")?;
    if offset.checked_add(length).ok_or("DIB overflow")? > b.len() {
        return Err("DIB thiếu dữ liệu pixel".into());
    }
    let mut out = vec![0; w as usize * h as usize * 4];
    let explicit_alpha = masks[3] != 0;
    for y in 0..h as usize {
        let sy = if signed_h < 0 { y } else { h as usize - 1 - y };
        for x in 0..w as usize {
            let p = offset + sy * stride + x * (depth / 8);
            let q = (y * w as usize + x) * 4;
            if compression == 3 {
                let value = u32_at(b, p)?;
                for c in 0..3 {
                    out[q + c] = ((value & masks[c]) >> masks[c].trailing_zeros()) as u8;
                }
                out[q + 3] = if explicit_alpha {
                    ((value & masks[3]) >> masks[3].trailing_zeros()) as u8
                } else {
                    255
                };
            } else {
                out[q..q + 4].copy_from_slice(&[
                    b[p + 2],
                    b[p + 1],
                    b[p],
                    if depth == 32 { b[p + 3] } else { 255 },
                ]);
            }
        }
    }
    // CF_DIB BI_RGB often uses zero-filled reserved bytes, not transparency.
    if compression == 0 && depth == 32 && !explicit_alpha && out.chunks_exact(4).all(|p| p[3] == 0)
    {
        for p in out.chunks_exact_mut(4) {
            p[3] = 255;
        }
    }
    Ok((w as u32, h, out))
}

pub fn encode(capture: Capture) -> Result<Vec<u8>, String> {
    match capture {
        Capture::Png(bytes) => {
            if bytes.len() > MAX_BYTES {
                return Err("PNG quá lớn".into());
            }
            let mut decoder = png::Decoder::new(Cursor::new(&bytes));
            decoder.set_limits(png::Limits { bytes: MAX_BYTES });
            let reader = decoder.read_info().map_err(|e| e.to_string())?;
            let info = reader.info();
            if info.width == 0
                || info.height == 0
                || info.width as u64 * info.height as u64 > MAX_PIXELS as u64
            {
                return Err("PNG vượt giới hạn kích thước".into());
            }
            // Validate complete framing/CRC and compressed pixel stream before saving.
            let mut reader = reader;
            let mut row_count = 0;
            while reader.next_row().map_err(|e| e.to_string())?.is_some() {
                row_count += 1;
            }
            if row_count == 0 {
                return Err("PNG không có pixel".into());
            }
            reader.finish().map_err(|e| e.to_string())?;
            drop(reader);
            Ok(bytes) // Preserve original PNG; no second compression or loss of metadata/alpha.
        }
        Capture::Dib(bytes) => {
            if bytes.len() > MAX_BYTES {
                return Err("DIB quá lớn".into());
            }
            let (w, h, rgba) = dib_rgba(&bytes)?;
            let mut out = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut out, w, h);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                encoder.set_compression(png::Compression::Fast);
                let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
                writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
            }
            Ok(out)
        }
    }
}

fn nonce() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}

pub fn prepare_folder(folder: &Path) -> Result<PathBuf, String> {
    if !folder.is_absolute() {
        return Err("Hãy nhập đường dẫn thư mục đầy đủ".into());
    }
    fs::create_dir_all(folder).map_err(|e| format!("Không tạo được thư mục: {e}"))?;
    let probe = folder.join(format!(".clipbridge-check-{}", nonce()));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map_err(|e| format!("Không ghi được vào thư mục: {e}"))?;
    drop(file);
    fs::remove_file(&probe).map_err(|e| e.to_string())?;
    Ok(folder.to_path_buf())
}

/// No database/index scan; the content hash is the persistent filename.
pub fn store_png(bytes: &[u8], folder: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let hash = format!("{:x}", Sha256::digest(bytes));
    let path = folder.join(format!("image_{hash}.png"));
    if path.is_file() {
        if fs::metadata(&path).map_err(|e| e.to_string())?.len() != bytes.len() as u64
            || fs::read(&path).map_err(|e| e.to_string())? != bytes
        {
            return Err("Tệp ảnh cùng tên đã bị thay đổi; không ghi đè".into());
        }
        return Ok(path);
    }
    let temp = folder.join(format!(".clipbridge-{}.tmp", nonce()));
    let result = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        f.write_all(bytes).map_err(|e| e.to_string())?;
        // The temp file is renamed only after the write succeeds. Avoid a
        // forced disk flush for every clipboard image: it can stall on HDDs,
        // network shares and antivirus-scanned directories.
        drop(f);
        fs::rename(&temp, &path).map_err(|e| e.to_string())?;
        Ok(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

pub fn save_setting(config: &Path, folder: &Path) -> Result<(), String> {
    let text = folder
        .to_str()
        .ok_or("Đường dẫn không phải Unicode hợp lệ")?;
    if text.contains(['\r', '\n']) {
        return Err("Đường dẫn có ký tự xuống dòng".into());
    }
    let temp = config.with_extension(format!("{}.tmp", nonce()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Storage::FileSystem::{
                MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
            };
            let from: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
            let to: Vec<u16> = config.as_os_str().encode_wide().chain(Some(0)).collect();
            if unsafe {
                MoveFileExW(
                    from.as_ptr(),
                    to.as_ptr(),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            } == 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        #[cfg(not(windows))]
        fs::rename(&temp, config).map_err(|e| e.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temp() -> PathBuf {
        let p = std::env::temp_dir().join(format!("clipbridge-test-{}", nonce()));
        fs::create_dir_all(&p).unwrap();
        p
    }
    fn dib(depth: u16, top_down: bool) -> Vec<u8> {
        let mut b = vec![0u8; 48];
        b[0..4].copy_from_slice(&40u32.to_le_bytes());
        b[4..8].copy_from_slice(&1i32.to_le_bytes());
        b[8..12].copy_from_slice(&(if top_down { -2i32 } else { 2i32 }).to_le_bytes());
        b[12..14].copy_from_slice(&1u16.to_le_bytes());
        b[14..16].copy_from_slice(&depth.to_le_bytes());
        b[40..44].copy_from_slice(&[0, 0, 255, 0]);
        b[44..48].copy_from_slice(&[255, 0, 0, 0]);
        b
    }
    #[test]
    fn dib_orientation_padding_and_alpha() {
        let (_, _, p) = dib_rgba(&dib(24, false)).unwrap();
        assert_eq!(&p[..4], &[0, 0, 255, 255]);
        let (_, _, p) = dib_rgba(&dib(24, true)).unwrap();
        assert_eq!(&p[..4], &[255, 0, 0, 255]);
        let (_, _, p) = dib_rgba(&dib(32, true)).unwrap();
        assert_eq!(p[3], 255);
        let mut b = dib(32, true);
        b[43] = 88;
        let (_, _, p) = dib_rgba(&b).unwrap();
        assert_eq!(p[3], 88);
        assert_eq!(p[7], 0);
    }
    #[test]
    fn fully_transparent_dibv5_stays_transparent() {
        for compression in [0u32, 3] {
            let original = dib(32, true);
            let mut b = vec![0u8; 132];
            b[..40].copy_from_slice(&original[..40]);
            b[0..4].copy_from_slice(&124u32.to_le_bytes());
            b[16..20].copy_from_slice(&compression.to_le_bytes());
            for (i, mask) in [0x00ff0000u32, 0x0000ff00, 0x000000ff, 0xff000000]
                .iter()
                .enumerate()
            {
                b[40 + i * 4..44 + i * 4].copy_from_slice(&mask.to_le_bytes());
            }
            b[124..].copy_from_slice(&original[40..]);
            let (_, _, rgba) = dib_rgba(&b).unwrap();
            assert!(rgba.chunks_exact(4).all(|p| p[3] == 0));
        }
    }
    #[test]
    fn reject_truncated_and_oversized_dib() {
        let mut b = dib(32, true);
        b.truncate(43);
        assert!(dib_rgba(&b).is_err());
        let mut b = dib(32, true);
        b[4..8].copy_from_slice(&100000i32.to_le_bytes());
        b[8..12].copy_from_slice(&100000i32.to_le_bytes());
        assert!(dib_rgba(&b).is_err());
    }
    #[test]
    fn png_roundtrip_and_truncation() {
        let png = encode(Capture::Dib(dib(32, true))).unwrap();
        assert_eq!(encode(Capture::Png(png.clone())).unwrap(), png);
        assert!(encode(Capture::Png(png[..png.len() / 2].to_vec())).is_err());
    }
    #[test]
    fn switching_folders_preserves_assets_and_config() {
        let root = temp();
        let first = prepare_folder(&root.join("Ảnh cũ")).unwrap();
        let second = prepare_folder(&root.join("Ảnh mới")).unwrap();
        let data = encode(Capture::Dib(dib(24, false))).unwrap();
        let old = store_png(&data, &first).unwrap();
        assert_eq!(store_png(&data, &first).unwrap(), old);
        let config = root.join("settings.txt");
        save_setting(&config, &first).unwrap();
        save_setting(&config, &second).unwrap();
        assert_eq!(
            fs::read_to_string(config).unwrap(),
            second.to_str().unwrap()
        );
        let new = store_png(&data, &second).unwrap();
        assert!(old.exists() && new.exists());
        assert_eq!(fs::read(old).unwrap(), data);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn corrupted_existing_file_is_never_overwritten() {
        let root = temp();
        let data = b"sample";
        let path = store_png(data, &root).unwrap();
        fs::write(&path, b"edited").unwrap();
        assert!(store_png(data, &root).is_err());
        assert_eq!(fs::read(path).unwrap(), b"edited");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn relative_storage_path_rejected() {
        assert!(prepare_folder(Path::new("relative")).is_err());
    }
}
