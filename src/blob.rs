use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

pub fn path_for(data_dir: &Path, hash: &str, ext: &str) -> PathBuf {
    data_dir.join("blobs").join(&hash[..2]).join(format!("{hash}.{ext}"))
}

/// Encodes RGBA as PNG, stores it content-addressed (crash-safe: tmp + fsync + rename).
/// Returns the blake3 hex hash.
pub fn store_rgba(data_dir: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<String, String> {
    let img = image::RgbaImage::from_raw(w, h, rgba.to_vec()).ok_or("ogiltig bildbuffert")?;
    let mut png = Vec::new();
    img.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    let hash = blake3::hash(&png).to_hex().to_string();
    let path = path_for(data_dir, &hash, "png");
    if path.exists() {
        return Ok(hash);
    }
    let dir = path.parent().ok_or("ogiltig sökväg")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("tmp");
    let write = || -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&png)?;
        f.sync_all()?;
        std::fs::rename(&tmp, &path)
    };
    write().map_err(|e| e.to_string())?;
    Ok(hash)
}
