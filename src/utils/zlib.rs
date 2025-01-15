// todo:
// - introduce proper error reporting using enums?
struct ByteReader<'data> {
    data: &'data [u8],
    pos: usize,
}

impl<'data> ByteReader<'data> {
    pub fn new(data: &'data [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn read_u8(&mut self) -> Option<u8> {
        if self.pos >= self.data.len() {
            return None;
        } else {
            let val = self.data[self.pos];
            self.pos += 1;
            return Some(val);
        }
    }

    pub fn read_u32_be(&mut self) -> Option<u32> {
        if self.pos + 4 > self.data.len() {
            return None;
        }
        let bytes = &self.data[self.pos..self.pos + 4];
        self.pos += 4;
        Some(u32::from_be_bytes(bytes.try_into().unwrap()))
    }
}

struct BitReader<'reader, 'data> {
    reader: &'reader mut ByteReader<'data>,
    bit_buffer: u64,
    bits_in_buffer: u8,
}

impl<'reader, 'data> BitReader<'reader, 'data> {
    pub fn new(reader: &'reader mut ByteReader<'data>) -> Self {
        Self {
            reader,
            bit_buffer: 0,
            bits_in_buffer: 0,
        }
    }

    pub fn read_bits(&mut self, num_bits: u8) -> Result<u32, String> {
        if num_bits > 32 {
            return Err("Reading more than 32 bits at once is not supported".to_owned());
        }
        while self.bits_in_buffer < num_bits {
            let byte = self
                .reader
                .read_u8()
                .ok_or("Out of data while reading bits")?;
            self.bit_buffer |= (byte as u64) << self.bits_in_buffer;
            self.bits_in_buffer += 8;
        }
        let mask = (1u64 << num_bits) - 1;
        let result = (self.bit_buffer & mask) as u32;
        self.bit_buffer >>= num_bits;
        self.bits_in_buffer -= num_bits;

        Ok(result)
    }

    pub fn read_u16_le(&mut self) -> Result<u16, String> {
        let bits = self.read_bits(16)?;
        Ok(u16::from_le_bytes([bits as u8, (bits >> 8) as u8]))
    }

    pub fn read_bytes(&mut self, num_bytes: usize) -> Result<Vec<u8>, String> {
        if num_bytes > 4 {
            return Err("Reading more than 4 bytes at once is not supported".to_owned());
        }
        let mut bytes = Vec::with_capacity(num_bytes);
        for _ in 0..num_bytes {
            let byte = self.read_bits(8)? as u8;
            bytes.push(byte);
        }
        Ok(bytes)
    }

    pub fn byte_align(&mut self) -> Result<(), String> {
        let leftover = self.bits_in_buffer % 8;
        if leftover != 0 {
            self.read_bits(leftover)?;
        }
        Ok(())
    }
}

#[derive(Debug)]
struct ZlibHeader {
    pub compression_method: u8,
    pub compression_info: u8,
    pub dictionary_flag: bool,
    pub dictionary_id: Option<u32>,
}

fn parse_zlib_header<'data>(reader: &mut ByteReader<'data>) -> Result<ZlibHeader, String> {
    let cmf = reader
        .read_u8()
        .ok_or("Unexpected end of data while reading CMF")?;
    let flg = reader
        .read_u8()
        .ok_or("Unexpected end of data while reading FLG")?;

    let compression_method = cmf & 0x0F;
    let compression_info = (cmf >> 4) & 0x0F;

    if compression_method != 8 {
        return Err(format!(
            "Unsupported compression method: {}",
            compression_method
        ));
    }

    let dictionary_flag = (flg & 0b0010_0000) != 0;
    let dictionary_id = if dictionary_flag {
        Some(
            reader
                .read_u32_be()
                .ok_or("Unexpected end of data while reading dictionary ID")?,
        )
    } else {
        None
    };

    Ok(ZlibHeader {
        compression_method,
        compression_info,
        dictionary_flag,
        dictionary_id,
    })
}

fn parse_deflate_stream<'reader, 'data>(
    reader: &'reader mut ByteReader<'data>,
) -> Result<Vec<u8>, String> {
    let mut bit_reader = BitReader::new(reader);
    let mut output = Vec::new();

    loop {
        let bfinal = bit_reader.read_bits(1)?;
        let btype = bit_reader.read_bits(2)?;

        match btype {
            0 => parse_uncompressed_block(&mut bit_reader, &mut output)?,
            // 1 => parse_fixed_huffman_block(&mut bit_reader, &mut output)?,
            // 2 => parse_dynamic_huffman_block(&mut bit_reader, &mut output)?,
            3 => return Err("Reserved BTYPE=3 encountered".to_string()),
            _ => unreachable!(),
        }

        if bfinal == 1 {
            break;
        }
    }

    Ok(output)
}
pub fn deflate(data: Vec<u8>) -> Result<Vec<u8>, String> {
    let mut reader = ByteReader::new(&data);

    // 1) Parse zlib header
    let header = parse_zlib_header(&mut reader)?;
    if header.dictionary_flag {
        // If you have a real dictionary, you'd check or load it here
        // For now, we might just acknowledge it
        println!("Dictionary ID = {:08X}", header.dictionary_id.unwrap());
    }

    // 2) Parse the DEFLATE stream
    let uncompressed_data = parse_deflate_stream(&mut reader)?;

    // 3) Read & validate Adler-32
    let adler_from_stream = reader.read_u32_be().ok_or("Missing Adler-32")?;
    let computed_adler = compute_adler32(&uncompressed_data);

    if adler_from_stream != computed_adler {
        return Err(format!(
            "Adler-32 mismatch. Stream={:08X}, Computed={:08X}",
            adler_from_stream, computed_adler
        ));
    }

    Ok(uncompressed_data)
}

fn compute_adler32(data: &[u8]) -> u32 {
    let mut s1: u32 = 1;
    let mut s2: u32 = 0;

    for &b in data {
        s1 = (s1 + (b as u32)) % 65521;
        s2 = (s2 + s1) % 65521;
    }

    (s2 << 16) | s1
}

fn parse_uncompressed_block(
    bit_reader: &mut BitReader,
    output: &mut Vec<u8>,
) -> Result<(), String> {
    bit_reader.byte_align()?;

    let len = bit_reader.read_u16_le()?;
    let nlen = bit_reader.read_u16_le()?;

    if nlen != !len {
        return Err("Uncompressed block LEN / NLEN mismatch".to_string());
    }

    let block_data = bit_reader.read_bytes(len as usize)?;
    output.extend_from_slice(&block_data);

    Ok(())
}

// Uncomment and implement these when ready
// fn parse_dynamic_huffman_block(bit_reader: &mut BitReader, output: &mut Vec<u8>) -> Result<(), String> { /* ... */ }
// fn parse_fixed_huffman_block(
//     bit_reader: &mut BitReader,
//     output: &mut Vec<u8>,
// ) -> Result<(), String> {
//     bit_reader.byte_align();
// }

enum HuffManNode {
    Leaf(u16),
    Branch(Box<HuffManNode>, Box<HuffManNode>),
}
#[cfg(test)]
const HELLO_ZLIB: [u8; 19] = [
    0x78, 0x9C, // zlib header (CMF=0x78, FLG=0x9C)
    0xF3, 0x48, 0xCD, 0xC9, 0xC9, 0x57, 0x08, 0xCF, 0x2F, 0xCA, 0x49, 0x01, 0x00, 0x1A, 0xEB, 0x03,
    0x1D,
];
#[cfg(test)]
const DICT_ZLIB: [u8; 15] = [
    0x78, 0x20, // CMF/FLG with dictionary bit set
    0x12, 0x34, 0x56, 0x78, // dictionary_id
    0x01, // DEFLATE block header (BFINAL=1, BTYPE=00 -> uncompressed)
    0x00, 0x00, // LEN = 0
    0xFF, 0xFF, // NLEN = one's complement of LEN
    0x00, 0x00, 0x00, 0x01, // Adler-32 of empty data
];

#[test]
fn test_byte_reader_new() {
    let reader = ByteReader::new(&HELLO_ZLIB);
    assert_eq!(reader.pos, 0);
}

#[test]
fn test_byte_reader_read_u8() -> Result<(), String> {
    let mut reader = ByteReader::new(&HELLO_ZLIB);
    assert_eq!(reader.pos, 0);
    let byte = reader.read_u8().ok_or("Can't read byte")?;
    assert_eq!(byte, 0x78);
    Ok(())
}

#[test]
fn test_byte_reader_read_u8_error() -> Result<(), String> {
    let mut reader = ByteReader::new(&[]);
    assert_eq!(reader.pos, 0);
    assert_eq!(reader.read_u8(), None);
    Ok(())
}

#[test]
fn test_parse_zlib_header() -> Result<(), String> {
    let mut reader = ByteReader::new(&HELLO_ZLIB);
    let result = parse_zlib_header(&mut reader)?;
    assert_eq!(result.compression_method, 8);
    assert_eq!(result.compression_info, 7);
    assert_eq!(result.dictionary_flag, false);
    assert_eq!(result.dictionary_id, None);
    Ok(())
}

#[test]
fn test_parse_zlib_header_with_dictionary() -> Result<(), String> {
    let mut reader = ByteReader::new(&DICT_ZLIB);
    let result = parse_zlib_header(&mut reader)?;
    assert_eq!(result.compression_method, 8);
    assert_eq!(result.compression_info, 7);
    assert_eq!(result.dictionary_flag, true);
    assert_eq!(result.dictionary_id, Some(0x12345678));
    Ok(())
}

#[test]
fn test_bit_reader_bits_read() {
    let mut reader = ByteReader::new(&HELLO_ZLIB);
    let _header = parse_zlib_header(&mut reader).unwrap();
    let mut bit_reader = BitReader::new(&mut reader);
    let result = bit_reader.read_bits(32).expect("can't read bits");
    assert_eq!(result, 0xC9CD48F3);
}

#[test]
fn test_bit_reader_too_many_bits_read() {
    let mut reader = ByteReader::new(&HELLO_ZLIB);
    let mut bit_reader = BitReader::new(&mut reader);
    let result = bit_reader.read_bits(33).is_err();
    assert!(result)
}

#[test]
fn test_bit_reader_read_u16_le() {
    let mut reader = ByteReader::new(&HELLO_ZLIB);

    let _header = parse_zlib_header(&mut reader).unwrap();
    let mut bit_reader = BitReader::new(&mut reader);
    let result = bit_reader.read_u16_le();
    assert_eq!(result, Ok(0x48F3));
}

#[test]
fn test_bit_reader_read_bytes_single() -> Result<(), String> {
    let data = [0xAB];
    let mut byte_reader = ByteReader::new(&data);
    let mut bit_reader = BitReader::new(&mut byte_reader);

    let bytes = bit_reader.read_bytes(1)?;
    assert_eq!(bytes, vec![0xAB]);

    Ok(())
}

#[test]
fn test_bit_reader_read_bytes_multiple() -> Result<(), String> {
    let data = [0x34, 0x12, 0x78, 0x56];
    let mut byte_reader = ByteReader::new(&data);
    let mut bit_reader = BitReader::new(&mut byte_reader);

    let bytes = bit_reader.read_bytes(2)?;
    assert_eq!(bytes, vec![0x34, 0x12]);

    let bytes = bit_reader.read_bytes(2)?;
    assert_eq!(bytes, vec![0x78, 0x56]);

    Ok(())
}

#[test]
fn test_bit_reader_read_bytes_insufficient_data() {
    let data = [0x34];
    let mut byte_reader = ByteReader::new(&data);
    let mut bit_reader = BitReader::new(&mut byte_reader);

    let result = bit_reader.read_bytes(2);
    assert!(result.is_err());
}

#[test]
fn test_bit_reader_read_bytes_exceeds_limit() {
    let data = [0x34, 0x12, 0x78, 0x56, 0x9A];
    let mut byte_reader = ByteReader::new(&data);
    let mut bit_reader = BitReader::new(&mut byte_reader);

    let result = bit_reader.read_bytes(5);
    assert!(result.is_err());
}

#[test]
fn test_deflate() {
    let data: Vec<u8> = [
        207, 143, 219, 84, 16, 199, 29, 10, 109, 5, 18, 42, 90, 1, 226, 130, 230, 132, 18, 32, 81,
        236, 164, 249, 33, 113, 136, 227, 132, 77, 104, 178, 142, 226, 116, 203, 106, 189, 149, 76,
        252, 54, 121, 187, 182, 95, 120, 118, 216, 13, 23, 42, 33, 164, 30, 57, 244, 86, 132, 16,
        72, 28, 16, 61, 33, 64, 72, 8, 56, 149, 11, 255, 3, 18, 85, 165, 30, 56, 149, 35, 32, 152,
        231, 252, 216, 100, 147, 108, 87, 93, 22, 136, 228, 120, 215, 121, 126, 223, 207, 204, 124,
        61, 158, 243, 146, 36, 181, 212, 102, 67, 95, 203, 100, 220, 151, 110, 164, 241, 95, 41,
        114, 235, 222, 147, 210, 149, 136, 20, 126, 158, 21, 23, 174, 227, 87, 161, 192, 120, 215,
        115, 204, 70, 83, 95, 109, 234, 218, 37, 83, 227, 196, 247, 169, 109, 153, 90, 179, 108,
        24, 213, 146, 106, 42, 73, 89, 129, 56, 180, 186, 4, 214, 9, 15, 104, 135, 193, 6, 177,
        184, 15, 170, 23, 116, 153, 195, 58, 3, 144, 243, 153, 124, 92, 206, 103, 101, 216, 44,
        107, 181, 178, 6, 138, 146, 74, 231, 182, 76, 173, 4, 138, 153, 76, 37, 192, 232, 187, 46,
        225, 112, 133, 144, 93, 226, 217, 192, 182, 193, 130, 26, 221, 38, 1, 117, 73, 98, 219,
        177, 218, 31, 126, 113, 113, 132, 38, 73, 103, 4, 28, 30, 63, 225, 241, 40, 30, 165, 207,
        37, 233, 49, 60, 95, 88, 10, 253, 26, 9, 186, 156, 65, 171, 239, 56, 166, 86, 169, 106,
        151, 202, 107, 96, 84, 84, 17, 78, 151, 182, 81, 18, 140, 174, 213, 222, 29, 69, 161, 49,
        183, 231, 144, 128, 64, 209, 233, 19, 168, 48, 78, 223, 102, 184, 66, 196, 205, 60, 31,
        161, 21, 51, 42, 167, 99, 176, 57, 115, 243, 214, 132, 190, 198, 222, 26, 66, 127, 252,
        201, 217, 57, 232, 119, 23, 65, 123, 139, 160, 141, 0, 119, 229, 212, 238, 16, 211, 104,
        33, 107, 179, 90, 90, 45, 199, 33, 107, 26, 204, 197, 120, 168, 215, 129, 87, 25, 15, 137,
        71, 202, 83, 183, 96, 40, 75, 151, 37, 218, 125, 178, 115, 78, 154, 249, 68, 216, 191, 139,
        32, 210, 115, 231, 220, 87, 43, 135, 211, 179, 241, 196, 127, 159, 30, 180, 236, 202, 133,
        67, 233, 209, 255, 33, 132, 54, 122, 131, 155, 201, 196, 78, 175, 211, 248, 230, 252, 105,
        106, 200, 66, 227, 236, 115, 43, 167, 169, 161, 8, 141, 107, 245, 167, 79, 83, 35, 37, 52,
        220, 199, 159, 63, 77, 141, 180, 208, 248, 229, 211, 167, 102, 53, 242, 139, 52, 90, 149,
        50, 212, 245, 245, 178, 137, 61, 45, 23, 110, 85, 199, 45, 204, 100, 14, 45, 53, 109, 238,
        187, 191, 126, 55, 217, 105, 108, 238, 221, 69, 207, 254, 230, 145, 42, 98, 115, 220, 185,
        68, 156, 254, 62, 129, 178, 77, 3, 236, 65, 120, 65, 209, 74, 162, 123, 202, 102, 50, 125,
        32, 12, 155, 117, 230, 49, 209, 133, 69, 167, 218, 10, 57, 110, 191, 18, 204, 113, 184,
        139, 56, 106, 39, 225, 192, 46, 158, 61, 148, 128, 194, 213, 159, 231, 132, 247, 22, 9,
        191, 249, 64, 225, 177, 186, 77, 253, 54, 235, 112, 171, 215, 29, 128, 219, 75, 153, 50,
        24, 65, 223, 166, 12, 84, 231, 141, 190, 235, 15, 75, 18, 15, 11, 2, 81, 50, 98, 148, 243,
        121, 37, 54, 147, 166, 4, 222, 251, 98, 180, 56, 161, 123, 100, 116, 46, 68, 134, 148, 142,
        160, 250, 241, 161, 169, 148, 49, 85, 147, 184, 150, 31, 16, 78, 236, 133, 128, 152, 58,
        176, 135, 249, 36, 251, 61, 203, 179, 113, 29, 63, 184, 101, 204, 175, 36, 147, 217, 88,
        88, 107, 136, 226, 171, 168, 67, 61, 203, 1, 87, 212, 121, 180, 34, 118, 164, 7, 144, 232,
        247, 219, 203, 99, 21, 86, 136, 124, 249, 191, 139, 85, 129, 168, 71, 246, 32, 252, 121,
        58, 208, 236, 108, 21, 159, 185, 85, 89, 26, 153, 240, 90, 228, 218, 67, 71, 150, 10, 199,
        0, 234, 88, 66, 217, 135, 23, 112, 28, 193, 197, 104, 38, 121, 52, 37, 20, 137, 31, 128,
        190, 61, 105, 1, 98, 200, 201, 197, 179, 73, 83, 206, 204, 66, 94, 255, 161, 176, 20, 114,
        71, 64, 118, 143, 132, 156, 236, 63, 204, 165, 75, 188, 192, 199, 191, 83, 201, 160, 139,
        19, 150, 135, 84, 220, 183, 248, 96, 106, 218, 138, 11, 207, 103, 49, 141, 242, 124, 91,
        186, 243, 217, 183, 199, 107, 75, 254, 66, 168, 144, 133, 217, 131, 112, 52, 242, 77, 181,
        166, 106, 112, 248, 162, 62, 54, 233, 196, 8, 185, 28, 14, 120, 28, 106, 22, 212, 197, 203,
        239, 34, 84, 176, 172, 34, 191, 196, 135, 96, 234, 197, 235, 166, 173, 27, 171, 247, 35,
        135, 147, 245, 199, 153, 33, 231, 95, 130, 235, 163, 227, 112, 53, 89, 7, 167, 201, 98,
        223, 243, 194, 100, 100, 242, 120, 106, 80, 210, 38, 162, 96, 117, 138, 79, 73, 84, 165,
        220, 181, 40, 66, 242, 118, 23, 115, 40, 174, 82, 168, 53, 64, 152, 47, 41, 167, 98, 163,
        17, 239, 96, 167, 45, 80, 39, 207, 24, 245, 160, 142, 38, 230, 214, 62, 181, 112, 235, 34,
        243, 250, 62, 180, 56, 190, 125, 194, 36, 123, 87, 239, 207, 205, 125, 189, 200, 130, 36,
        127, 125, 156, 96, 134, 78, 208, 75, 27, 80, 172, 93, 46, 27, 162, 11, 87, 13, 13, 23, 171,
        141, 202, 134, 89, 212, 95, 7, 163, 220, 50, 196, 24, 158, 30, 89, 179, 193, 156, 129, 141,
        47, 186, 225, 24, 142, 249, 207, 196, 69, 23, 132, 168, 209, 239, 97, 52, 179, 77, 60, 246,
        50, 100, 176, 137, 135, 85, 138, 207, 214, 41, 49, 93, 168, 233, 9, 9, 93, 125, 243, 183,
        15, 150, 186, 90, 124, 69, 222, 57, 121, 108, 122, 179, 186, 90, 93, 83, 107, 96, 180, 46,
        151, 170, 250, 131, 25, 131, 89, 198, 239, 7, 203, 25, 67, 51, 189, 119, 114, 198, 245,
        234, 218, 70, 45, 222, 172, 54, 230, 233, 32, 186, 78, 189, 129, 19, 231, 180, 23, 91, 148,
        205, 189, 3, 210, 187, 239, 223, 92, 74, 250, 39, 30, 127, 3, 4, 86, 176, 208,
    ]
    .to_vec();
    let res = deflate(data);
    dbg!(res);
}
