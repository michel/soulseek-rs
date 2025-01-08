pub struct ByteReader<'a> {
    data: &'a [u8],
    pos: usize,
}
impl<'a> ByteReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn read_u8(self) -> Option<u8> {
        if self.pos >= self.data.len() {
            return None;
        } else {
            let val = self.data[self.pos];
            let pos = self.pos + 1;
            return Some(val);
        }
    }
}

fn parse_zlib_header(mut reader: ByteReader) -> Result<(), String> {
    let cmf = reader.read_u8();
    Ok(())
}

#[cfg(test)]
pub const HELLO_ZLIB: [u8; 19] = [
    0x78, 0x9C, // zlib header (CMF=0x78, FLG=0x9C)
    0xF3, 0x48, 0xCD, 0xC9, 0xC9, 0x57, 0x08, 0xCF, 0x2F, 0xCA, 0x49, 0x01, 0x00, 0x1A, 0xEB, 0x03,
    0x1D,
];
#[test]
fn test_byte_reader_new() {
    let reader = ByteReader::new(&HELLO_ZLIB);
    assert_eq!(reader.pos, 0);
}
#[test]
fn test_byte_reader_read_u8() -> Result<(), String> {
    let reader = ByteReader::new(&HELLO_ZLIB);
    assert_eq!(reader.pos, 0);
    let byte = reader.read_u8().ok_or("Can't read byte")?;
    assert_eq!(byte, 0x78);
    Ok(())
}

#[test]
fn test_byte_reader_read_u8_error() -> Result<(), String> {
    let reader = ByteReader::new(&HELLO_ZLIB);
    assert_eq!(reader.pos, 0);
    let byte = reader.read_u8().ok_or("Can't read byte")?;
    assert_eq!(byte, 0x78);
    Ok(())
}
