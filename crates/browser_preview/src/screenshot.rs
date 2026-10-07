pub fn png_dimensions(png: &[u8]) -> Result<(u32, u32), String> {
    if png.len() > 50 * 1024 * 1024
        || png.get(..8) != Some(b"\x89PNG\r\n\x1a\n")
        || png.get(8..12) != Some(b"\0\0\0\r")
        || png.get(12..16) != Some(b"IHDR")
    {
        return Err("Browser did not return a valid PNG screenshot".into());
    }
    let width = u32::from_be_bytes(
        png.get(16..20)
            .ok_or("Truncated PNG width")?
            .try_into()
            .map_err(|_| "Invalid PNG width")?,
    );
    let height = u32::from_be_bytes(
        png.get(20..24)
            .ok_or("Truncated PNG height")?
            .try_into()
            .map_err(|_| "Invalid PNG height")?,
    );
    if width == 0 || height == 0 {
        return Err("Browser returned an empty screenshot".into());
    }
    Ok((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_dimensions_and_truncated_capture() -> Result<(), String> {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(png_dimensions(&png)?, (640, 480));
        assert!(png_dimensions(png.get(..20).ok_or("Invalid fixture")?).is_err());
        assert!(png_dimensions(b"not a screenshot").is_err());
        Ok(())
    }

    #[test]
    fn empty_capture_dimensions_are_rejected() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&0u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        assert!(png_dimensions(&png).is_err());
    }
}
