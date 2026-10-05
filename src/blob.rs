use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

pub fn path_for(data_dir: &Path, hash: &str, ext: &str) -> PathBuf {
    data_dir.join("blobs").join(&hash[..2]).join(format!("{hash}.{ext}"))
}

/// Encodes RGBA as PNG, stores it content-addressed (crash-safe: tmp + fsync + rename).
/// Returns the blake3 hex hash.
pub fn store_rgba(data_dir: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<String, String> {
    let mut px = rgba.to_vec();
    opaque_if_alpha_missing(&mut px);
    let img = image::RgbaImage::from_raw(w, h, px).ok_or("invalid image buffer")?;
    let mut png = Vec::new();
    img.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    // 80-bit prefix of BLAKE3: collision-free in practice and keeps `![](blob:…)` refs short.
    let hash = blake3::hash(&png).to_hex()[..20].to_string();
    let path = path_for(data_dir, &hash, "png");
    if path.exists() {
        return Ok(hash);
    }
    let dir = path.parent().ok_or("invalid path")?;
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

/// Many Windows apps put 32-bit bitmaps on the clipboard with every alpha byte = 0.
/// Taken literally that is a fully transparent image; treat it as opaque instead.
fn opaque_if_alpha_missing(px: &mut [u8]) {
    if px.chunks_exact(4).all(|p| p[3] == 0) {
        px.chunks_exact_mut(4).for_each(|p| p[3] = 255);
    }
}

/// One-time repair of images stored before the alpha fix (marker file prevents re-runs).
pub fn repair_transparent(data_dir: &Path) {
    let marker = data_dir.join("blobs").join(".alpha-repaired");
    if marker.exists() || !data_dir.join("blobs").exists() {
        return;
    }
    let walk = std::fs::read_dir(data_dir.join("blobs"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|d| d.path().is_dir())
        .flat_map(|d| std::fs::read_dir(d.path()).into_iter().flatten().flatten());
    for f in walk {
        let p = f.path();
        if p.extension().is_none_or(|e| e != "png") {
            continue;
        }
        let Ok(img) = image::open(&p) else { continue };
        let mut rgba = img.to_rgba8();
        if rgba.pixels().all(|q| q[3] == 0) {
            rgba.pixels_mut().for_each(|q| q[3] = 255);
            let tmp = p.with_extension("tmp");
            if rgba.save_with_format(&tmp, image::ImageFormat::Png).is_ok() {
                let _ = std::fs::rename(&tmp, &p);
            }
        }
    }
    let _ = std::fs::write(marker, "");
}

#[cfg(test)]
mod tests {
    #[test]
    fn alpha_fix() {
        let mut a = vec![1, 2, 3, 0, 4, 5, 6, 0];
        super::opaque_if_alpha_missing(&mut a);
        assert_eq!(a, vec![1, 2, 3, 255, 4, 5, 6, 255]);
        let mut b = vec![1, 2, 3, 0, 4, 5, 6, 9];
        super::opaque_if_alpha_missing(&mut b);
        assert_eq!(b[3], 0);
    }
}
