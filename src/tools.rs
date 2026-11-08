const CRC32_TABLE: [u32; 16] = [
    0x00000000, 0x1DB71064, 0x3B6E20C8, 0x26D930AC, 0x76DC4190, 0x6B6B51F4, 0x4DB26158, 0x5005713C,
    0xEDB88320, 0xF00F9344, 0xD6D6A3E8, 0xCB61B38C, 0x9B64C2B0, 0x86D3D2D4, 0xA00AE278, 0xBDBDF21C,
];

pub fn crc32(data: &[u8], size: usize, start_value: u32) -> u32 {
    let mut crc: u32 = !start_value;
    for i in 0..size {
        let byte = data[i];
        crc = CRC32_TABLE[((crc ^ (byte as u32)) & 0x0F) as usize] ^ (crc >> 4);
        crc = CRC32_TABLE[((crc ^ (byte >> 4) as u32) & 0x0F) as usize] ^ (crc >> 4);
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crc32() {
        let data: [u8; 10] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        assert_eq!(crc32(&data, 10, 0), 622_876_539u32);
    }

    #[test]
    fn test_crc32_incremental() {
        // Splitting the input and chaining must yield the same result as a single pass.
        let data: &[u8] = &[1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let crc = crc32(&data[..5], 5, 0);
        let crc = crc32(&data[5..], 5, crc);
        assert_eq!(crc, 622_876_539u32);
    }
}
