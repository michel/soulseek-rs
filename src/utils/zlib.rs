//https://www.rfc-editor.org/rfc/rfc1950

use flate2::bufread::ZlibDecoder;

#[derive(Debug)]
struct ZlibHeader {
    cm: u8,
    cinfo: u8,
    fcheck: u8,
    fdict: u8,
    flevel: u8,
}

struct BlockHeader {
    final_block: bool,
    btype: u8,
}

pub fn extract_header(data: &Vec<u8>) -> Result<ZlibHeader, String> {
    if data.len() < 2 {
        Err("Invalid ZLIB header")?
    }

    let cmf = data[0];
    let cm = cmf & 0b0000_1111; // mask bits 0..3 (lower nibble)
    let cinfo = (cmf & 0b1111_0000) >> 4; // mask bits 4..7 (upper nibble) and shift down
    let lfg = data[1];
    let fcheck = lfg & 0b0001_1111; // bits 0..4 (mask lower 5 bits)
    let fdict = (lfg >> 5) & 0b0000_0001; // bit 5 (shift right 5, mask 1 bit)
    let flevel = (lfg >> 6) & 0b0000_0011; // bits 6..7 (shift right 6, mask 2 bits)

    if cm != 8 {
        return Err(format!("ZLIB compression method not supported: {}", cmf));
    }
    return Ok(ZlibHeader {
        cm,
        cinfo,
        fcheck,
        fdict,
        flevel,
    });
}

pub fn extract_block_header(data: &Vec<u8>) -> BlockHeader {
    let bfinal = data[0] & 0b1000_0000;
    let btype = data[0] & 0b0011_1111;

    BlockHeader {
        final_block: bfinal != 0,
        btype,
    }
}
struct BitReader {
    data: Vec<u8>,
    pos: usize,
}

impl BitReader {
    pub fn new(data: Vec<u8>) -> Self {
        Self { data, pos: 0 }
    }
    pub fn read_bits(&mut self, n: usize) -> Result<u32, &'static str> {
        if n > 32 {
            return Err("Cannot read more than 32 bits at a time");
        }

        if (self.data.len() * 8) < self.pos + n {
            return Err("Not enough bits left to read in stream");
        }

        let mut result = 0u32;
        for i in 0..n {
            let byte_index = (self.pos + i) / 8;
            let bit_index = (self.pos + i) % 8;

            let bit = (self.data[byte_index] >> bit_index) & 1;
            result |= (bit as u32) << i;
        }

        self.pos += n;
        Ok(result)
    }

    pub fn read_bytes(&mut self, n: usize) -> Result<Vec<u8>, &'static str> {
        if self.data.len() < self.pos + n {
            return Err("Not enough bytes left to read in stream");
        }
        let result = self.data[self.pos..self.pos + n].to_vec();
        self.pos += n;
        Ok(result)
    }

    pub fn align_to_byte(&mut self) {
        self.pos = (self.pos + 7) & !7;
    }
    pub fn reset(&mut self) {
        self.pos = 0;
    }
}

fn reverse_bits(n: u32, len: u32) -> u32 {
    let mut result = 0;
    for i in 0..len {
        if (n >> i) & 1 == 1 {
            result |= 1 << (len - 1 - i);
        }
    }
    result
}
fn find_symbol(current_code: u32, code_len: u32) -> Option<u32> {
    match code_len {
        7 => {
            let reversed_code = reverse_bits(current_code, 7);
            // Range for symbols 256-279: 0b0000000 to 0b0010111
            if (0..=0b0010111).contains(&reversed_code) {
                // Symbol = base_symbol + (code - base_code)
                let symbol = 256 + (reversed_code - 0b0000000);
                return Some(symbol);
            }
        }
        8 => {
            let reversed_code = reverse_bits(current_code, 8);
            // Range for symbols 0-143: 0b00110000 to 0b10111111
            if (0b00110000..=0b10111111).contains(&reversed_code) {
                let symbol = 0 + (reversed_code - 0b00110000);
                return Some(symbol);
            }
            // Range for symbols 280-287: 0b11000000 to 0b11000111
            if (0b11000000..=0b11000111).contains(&reversed_code) {
                let symbol = 280 + (reversed_code - 0b11000000);
                return Some(symbol);
            }
        }
        9 => {
            let reversed_code = reverse_bits(current_code, 9);
            // Range for symbols 144-255: 0b110010000 to 0b111111111
            if (0b110010000..=0b111111111).contains(&reversed_code) {
                let symbol = 144 + (reversed_code - 0b110010000);
                return Some(symbol);
            }
        }
        _ => {
            // Any other length is not a valid code in the fixed table.
            return None;
        }
    }

    // If we checked a valid length but the code wasn't in any range for that length.
    None
}
pub fn decode_fixed_huffman_symbol(reader: &mut BitReader) -> Result<u32, &'static str> {
    let mut current_code = 0u32;
    let mut code_len = 0u32;
    loop {
        let bit = reader.read_bits(1)?;
        current_code |= bit << code_len;
        code_len += 1;

        if let Some(symbol) = find_symbol(current_code, code_len) {
            return Ok(symbol);
        }

        // To prevent an infinite loop on corrupt data.
        if code_len >= 10 {
            return Err("Invalid code sequence in stream");
        }
    }
}

fn decode_distance(code: u32, reader: &mut BitReader) -> Result<u32, &'static str> {
    match code {
        0..=3 => {
            // 0 extra bits
            Ok(code + 1)
        }
        4..=5 => {
            // 1 extra bit
            let extra = reader.read_bits(1)?;
            let base = [5, 7];
            Ok(base[(code - 4) as usize] + extra)
        }
        6..=7 => {
            // 2 extra bits
            let extra = reader.read_bits(2)?;
            let base = [9, 13];
            Ok(base[(code - 6) as usize] + extra)
        }
        8..=9 => {
            // 3 extra bits
            let extra = reader.read_bits(3)?;
            let base = [17, 25];
            Ok(base[(code - 8) as usize] + extra)
        }
        10..=11 => {
            // 4 extra bits
            let extra = reader.read_bits(4)?;
            let base = [33, 49];
            Ok(base[(code - 10) as usize] + extra)
        }
        12..=13 => {
            // 5 extra bits
            let extra = reader.read_bits(5)?;
            let base = [65, 97];
            Ok(base[(code - 12) as usize] + extra)
        }
        14..=15 => {
            // 6 extra bits
            let extra = reader.read_bits(6)?;
            let base = [129, 193];
            Ok(base[(code - 14) as usize] + extra)
        }
        16..=17 => {
            // 7 extra bits
            let extra = reader.read_bits(7)?;
            let base = [257, 385];
            Ok(base[(code - 16) as usize] + extra)
        }
        18..=19 => {
            // 8 extra bits
            let extra = reader.read_bits(8)?;
            let base = [513, 769];
            Ok(base[(code - 18) as usize] + extra)
        }
        20..=21 => {
            // 9 extra bits
            let extra = reader.read_bits(9)?;
            let base = [1025, 1537];
            Ok(base[(code - 20) as usize] + extra)
        }
        22..=23 => {
            // 10 extra bits
            let extra = reader.read_bits(10)?;
            let base = [2049, 3073];
            Ok(base[(code - 22) as usize] + extra)
        }
        24..=25 => {
            // 11 extra bits
            let extra = reader.read_bits(11)?;
            let base = [4097, 6145];
            Ok(base[(code - 24) as usize] + extra)
        }
        26..=27 => {
            // 12 extra bits
            let extra = reader.read_bits(12)?;
            let base = [8193, 12289];
            Ok(base[(code - 26) as usize] + extra)
        }
        28..=29 => {
            // 13 extra bits
            let extra = reader.read_bits(13)?;
            let base = [16385, 24577];
            Ok(base[(code - 28) as usize] + extra)
        }
        _ => Err("Invalid distance code"),
    }
}
fn decode_length(symbol: u32, reader: &mut BitReader) -> Result<u32, &'static str> {
    match symbol {
        257..=264 => {
            // 0 extra bits
            Ok(symbol - 257 + 3)
        }
        265..=268 => {
            // 1 extra bit
            let extra = reader.read_bits(1)?;
            let base = [11, 13, 15, 17];
            Ok(base[(symbol - 265) as usize] + extra)
        }
        269..=272 => {
            // 2 extra bits
            let extra = reader.read_bits(2)?;
            let base = [19, 23, 27, 31];
            Ok(base[(symbol - 269) as usize] + extra)
        }
        273..=276 => {
            // 3 extra bits
            let extra = reader.read_bits(3)?;
            let base = [35, 43, 51, 59];
            Ok(base[(symbol - 273) as usize] + extra)
        }
        277..=280 => {
            // 4 extra bits
            let extra = reader.read_bits(4)?;
            let base = [67, 83, 99, 115];
            Ok(base[(symbol - 277) as usize] + extra)
        }
        281..=284 => {
            // 5 extra bits
            let extra = reader.read_bits(5)?;
            let base = [131, 163, 195, 227];
            Ok(base[(symbol - 281) as usize] + extra)
        }
        285 => {
            // 0 extra bits
            Ok(258)
        }
        _ => Err("Invalid length symbol"),
    }
}
#[derive(Debug)]
struct HuffmanTable {
    // Each entry stores (symbol, length)
    table: Vec<(u32, u32)>,
}

impl HuffmanTable {
    /// Creates a new HuffmanTable from a slice of code lengths.
    /// This implements the canonical Huffman code generation algorithm from RFC 1951.
    pub fn new(code_lengths: &[u32]) -> Result<Self, &'static str> {
        let max_len = *code_lengths.iter().max().unwrap_or(&0) as usize;
        if max_len > 15 {
            return Err("Max code length exceeds 15 bits");
        }

        // 1. Count the number of codes for each length.
        let mut bl_count = vec![0; max_len + 1];
        for &len in code_lengths {
            if len > 0 {
                bl_count[len as usize] += 1;
            }
        }

        // 2. Find the starting numerical value for each code length.
        let mut next_code = vec![0; max_len + 1];
        let mut code = 0;
        for bits in 1..=max_len {
            code = (code + bl_count[bits - 1]) << 1;
            next_code[bits] = code;
        }

        // 3. Assign numerical values to all codes and build the table.
        // Our table will map a reversed code to its symbol and length.
        // The table size is 2^max_len for quick lookups.
        let table_size = 1 << max_len;
        let mut table = vec![(0, 0); table_size];

        for (symbol, &len) in code_lengths.iter().enumerate() {
            if len > 0 {
                let code = next_code[len as usize];
                next_code[len as usize] += 1;

                // The code read from the stream is LSB-first, so we store
                // the reversed code as the key in our lookup table.
                let reversed_code = reverse_bits(code, len);

                // For fast lookups, we populate all entries that start with this prefix.
                let step = 1 << len;
                for i in (reversed_code as usize..table_size).step_by(step) {
                    table[i] = (symbol as u32, len);
                }
            }
        }

        Ok(Self { table })
    }
}

// --- New Generic Symbol Decoder ---
/// Decodes one symbol from the stream using the provided Huffman table.
fn decode_symbol(
    reader: &mut BitReader,
    huffman_table: &HuffmanTable,
) -> Result<u32, &'static str> {
    let mut code = 0;
    let mut len = 0;

    // The maximum length for DEFLATE codes is 15 bits.
    for i in 0..15 {
        let bit = reader.read_bits(1)?;
        code |= bit << i;

        let (symbol, length) = huffman_table.table[code as usize];
        if length > 0 {
            // We need to confirm that the code we read is of the correct length.
            if length == i + 1 {
                return Ok(symbol);
            }
        }
    }
    Err("Invalid Huffman code found in stream")
}

/// Decompresses a raw DEFLATE data stream.
pub fn deflate(data: Vec<u8>) -> Result<Vec<u8>, String> {
    // move this out of deflate, deflate should only handle the decompression logic.
    let mut reader = BitReader::new(data);
    let mut output: Vec<u8> = vec![];
    let header_data = reader.read_bytes(16).unwrap();
    let header = extract_header(&header_data).unwrap();

    // Main loop to process all blocks in the stream.
    loop {
        // Read the 3-bit block header.
        let bfinal = reader.read_bits(1).map_err(|e| e.to_string())?;
        let btype = reader.read_bits(2).map_err(|e| e.to_string())?;

        match btype {
            // --- Uncompressed Block ---
            0b00 => {
                println!("Uncompressed block");
                // FIX: Align to the next byte boundary as per RFC 1951.
                reader.align_to_byte();

                let len = reader.read_bits(16).map_err(|e| e.to_string())?;
                let nlen = reader.read_bits(16).map_err(|e| e.to_string())?;

                // As a data integrity check.
                if len != !nlen {
                    return Err("Invalid len/nlen in uncompressed block".to_string());
                }

                // Read the literal data.
                for _ in 0..len {
                    let byte = reader.read_bits(8).map_err(|e| e.to_string())?;
                    output.push(byte as u8);
                }
            }
            // --- Fixed Huffman Block ---
            0b01 => {
                println!("Fixed Huffman block");
                loop {
                    let symbol = decode_fixed_huffman_symbol(&mut reader)?;
                    match symbol {
                        0..=255 => output.push(symbol as u8),
                        256 => break, // End of block.
                        257..=285 => {
                            let length = decode_length(symbol, &mut reader)?;
                            let distance_code = reader.read_bits(5)?;
                            let distance = decode_distance(distance_code, &mut reader)?;

                            let start = output.len() - distance as usize;
                            for _ in 0..length {
                                let byte =
                                    output[start + (output.len() - start) - distance as usize];
                                output.push(byte);
                            }
                        }
                        _ => return Err("Invalid symbol decoded".to_string()),
                    }
                }
            }
            // --- Dynamic Huffman Block ---
            0b10 => {
                println!("Dynamic Huffman block");

                // 1. Read table size headers.
                let hlit = reader.read_bits(5)? as usize + 257;
                let hdist = reader.read_bits(5)? as usize + 1;
                let hclen = reader.read_bits(4)? as usize + 4;

                // 2. Decode the code lengths for the "code length" alphabet.
                const CODE_LENGTH_ORDER: [usize; 19] = [
                    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
                ];
                let mut code_len_code_lengths = vec![0; 19];
                for i in 0..hclen {
                    let len = reader.read_bits(3)?;
                    code_len_code_lengths[CODE_LENGTH_ORDER[i]] = len;
                }

                // 3. Build the Huffman table for decoding the main code lengths.
                let code_len_table = HuffmanTable::new(&code_len_code_lengths)?;

                // 4. Decode the main literal/length and distance code lengths.
                let mut main_code_lengths = Vec::with_capacity(hlit + hdist);
                while main_code_lengths.len() < (hlit + hdist) {
                    let symbol = decode_symbol(&mut reader, &code_len_table)?;
                    match symbol {
                        0..=15 => main_code_lengths.push(symbol),
                        16 => {
                            // Repeat the previous code length
                            let prev = *main_code_lengths
                                .last()
                                .ok_or("No previous code length to repeat")?;
                            let repeat_count = reader.read_bits(2)? + 3;
                            for _ in 0..repeat_count {
                                main_code_lengths.push(prev);
                            }
                        }
                        17 => {
                            // Repeat a code length of 0
                            let repeat_count = reader.read_bits(3)? + 3;
                            for _ in 0..repeat_count {
                                main_code_lengths.push(0);
                            }
                        }
                        18 => {
                            // Repeat a code length of 0 for a longer run
                            let repeat_count = reader.read_bits(7)? + 11;
                            for _ in 0..repeat_count {
                                main_code_lengths.push(0);
                            }
                        }
                        _ => return Err("Invalid symbol in code length alphabet".to_string()),
                    }
                }

                // 5. Build the final Huffman tables for data decompression.
                let (lit_len_lengths, dist_lengths) = main_code_lengths.split_at(hlit);
                let lit_len_table = HuffmanTable::new(lit_len_lengths)?;
                let dist_table = HuffmanTable::new(dist_lengths)?;

                // 6. Decompress the actual data using the new tables.
                loop {
                    let symbol = decode_symbol(&mut reader, &lit_len_table)?;
                    match symbol {
                        0..=255 => output.push(symbol as u8),
                        256 => break, // End of block.
                        257..=285 => {
                            let length = decode_length(symbol, &mut reader)?;
                            let dist_symbol = decode_symbol(&mut reader, &dist_table)?;
                            let distance = decode_distance(dist_symbol, &mut reader)?;

                            let start = output.len() - distance as usize;
                            for i in 0..length {
                                let byte = output[start + i as usize];
                                output.push(byte);
                            }
                        }
                        _ => return Err("Invalid data symbol decoded".to_string()),
                    }
                }
            }
            // --- Error ---
            _ => return Err("Invalid block type (11)".to_string()),
        }

        // FIX: Check the bfinal flag to see if this was the last block.
        if bfinal == 1 {
            break; // Exit the main loop.
        }
    }

    Ok(output)
}

#[test]
fn test_bitreader_read_bytes() {
    let data: Vec<u8> = vec![0b10101010, 0b11001100];
    let mut reader = BitReader::new(data);

    let bytes = reader.read_bytes(1).unwrap();
    assert_eq!(bytes, vec![0b10101010]);

    let bytes = reader.read_bytes(1).unwrap();
    assert_eq!(bytes, vec![0b11001100]);

    let bytes = reader.read_bytes(3);
    assert_eq!(
        bytes.err().unwrap(),
        "Not enough bytes left to read in stream"
    );
    reader.reset();

    let bytes = reader.read_bytes(2).unwrap();
    assert_eq!(bytes, vec![0b10101010, 0b11001100]);
}

#[test]
fn test_bitreader_read_bits() {
    let data: Vec<u8> = vec![0b10101010, 0b11001100];
    let mut reader = BitReader::new(data);

    let bits = reader.read_bits(8).unwrap();
    assert_eq!(bits, 0b10101010);

    let bits = reader.read_bits(4).unwrap();
    assert_eq!(bits, 0b1100);

    let bits = reader.read_bits(4).unwrap();
    assert_eq!(bits, 0b1100);

    // end of bitstream
    let bits = reader.read_bits(4);
    assert_eq!(
        bits.err().unwrap(),
        "Not enough bits left to read in stream"
    );

    let bits = reader.read_bits(33);
    assert_eq!(
        bits.err().unwrap(),
        "Cannot read more than 32 bits at a time"
    );
}

#[test]
fn test_extract_header_success() {
    let data: Vec<u8> = vec![120, 156];

    let res = extract_header(&data).unwrap();

    assert_eq!(res.cm, 8);
    assert_eq!(res.cinfo, 7);
    assert_eq!(res.fcheck, 28);
    assert_eq!(res.fdict, 0);
    assert_eq!(res.flevel, 2);
}

#[test]
fn test_extract_header_fail_to_short() {
    let data: Vec<u8> = vec![156];
    let res = extract_header(&data);

    assert!(res.is_err());
    assert_eq!(res.err().unwrap(), "Invalid ZLIB header");
}
#[test]
fn test_deflate() {
    let data: Vec<u8> = [
        120, 156, 173, 154, 107, 108, 28, 87, 21, 199, 215, 161, 136, 72, 228, 75, 161, 60, 138, 4,
        186, 80, 165, 222, 72, 190, 238, 190, 252, 2, 2, 246, 238, 122, 109, 175, 189, 177, 181,
        107, 59, 77, 112, 72, 175, 119, 175, 119, 174, 61, 51, 119, 185, 51, 99, 123, 13, 125, 80,
        53, 45, 15, 209, 34, 62, 20, 69, 106, 40, 77, 65, 8, 218, 70, 249, 80, 250, 41, 17, 40,
        109, 1, 169, 109, 212, 22, 33, 62, 32, 90, 94, 66, 2, 145, 82, 30, 31, 80, 133, 4, 231,
        220, 245, 58, 206, 218, 215, 81, 118, 118, 101, 237, 181, 119, 102, 231, 252, 230, 220,
        123, 206, 253, 159, 51, 126, 79, 36, 18, 41, 168, 180, 45, 221, 202, 55, 122, 246, 71, 50,
        240, 103, 215, 44, 188, 13, 15, 115, 94, 174, 45, 47, 140, 102, 11, 11, 241, 161, 161, 65,
        66, 73, 142, 185, 62, 219, 16, 12, 126, 77, 43, 225, 11, 207, 34, 35, 174, 111, 113, 199,
        35, 209, 66, 138, 29, 90, 136, 197, 225, 80, 150, 173, 114, 248, 188, 202, 109, 252, 74,
        224, 174, 144, 66, 224, 137, 114, 175, 147, 98, 151, 79, 94, 136, 52, 95, 251, 54, 199,
        145, 46, 48, 8, 227, 29, 56, 78, 181, 107, 56, 1, 135, 102, 242, 240, 54, 206, 106, 181,
        58, 48, 212, 61, 52, 120, 246, 142, 159, 25, 13, 126, 22, 199, 249, 118, 13, 38, 225, 208,
        20, 171, 6, 46, 158, 83, 170, 9, 219, 230, 138, 228, 148, 116, 72, 81, 72, 52, 253, 242,
        247, 87, 141, 166, 47, 162, 147, 173, 118, 77, 167, 224, 208, 172, 98, 2, 45, 206, 48, 181,
        226, 193, 223, 69, 38, 60, 78, 142, 201, 64, 129, 7, 220, 138, 71, 38, 92, 50, 107, 193,
        60, 8, 133, 48, 95, 62, 251, 132, 17, 230, 93, 56, 142, 183, 11, 211, 135, 183, 95, 95, 20,
        56, 217, 71, 173, 58, 26, 251, 228, 7, 95, 52, 26, 59, 140, 227, 106, 187, 198, 250, 113,
        150, 185, 172, 217, 156, 204, 185, 21, 174, 170, 74, 6, 110, 133, 228, 252, 94, 82, 178,
        152, 146, 46, 57, 10, 19, 33, 152, 131, 46, 209, 171, 142, 76, 120, 100, 38, 112, 106, 194,
        173, 34, 218, 195, 231, 94, 49, 162, 77, 118, 133, 89, 249, 3, 112, 232, 136, 168, 90, 190,
        35, 87, 185, 66, 251, 211, 56, 23, 50, 240, 56, 26, 62, 112, 243, 89, 163, 225, 155, 112,
        252, 124, 187, 134, 241, 172, 180, 205, 202, 43, 36, 35, 93, 151, 151, 125, 1, 110, 160,
        100, 76, 64, 20, 22, 56, 41, 90, 117, 223, 114, 16, 225, 238, 195, 127, 53, 34, 212, 67,
        221, 251, 16, 28, 26, 101, 158, 79, 250, 6, 124, 139, 148, 124, 92, 18, 204, 15, 84, 133,
        233, 245, 240, 137, 83, 63, 55, 26, 214, 227, 93, 109, 26, 142, 199, 244, 141, 122, 28,
        130, 143, 228, 193, 5, 158, 190, 245, 41, 152, 1, 112, 134, 227, 64, 32, 56, 220, 245, 117,
        38, 248, 195, 189, 111, 26, 33, 44, 28, 231, 218, 133, 192, 156, 151, 225, 248, 51, 195,
        93, 225, 249, 154, 33, 39, 92, 102, 219, 250, 246, 255, 236, 93, 39, 233, 181, 235, 247,
        56, 38, 189, 60, 171, 87, 56, 199, 184, 176, 97, 6, 96, 193, 103, 21, 135, 245, 143, 134,
        63, 243, 161, 111, 27, 13, 255, 19, 51, 208, 137, 118, 13, 99, 242, 195, 236, 50, 233, 202,
        53, 155, 87, 170, 104, 127, 196, 131, 152, 244, 33, 15, 224, 129, 108, 99, 230, 237, 63,
        221, 111, 36, 120, 13, 9, 166, 219, 37, 192, 28, 152, 102, 206, 162, 148, 91, 191, 84, 133,
        142, 180, 219, 127, 188, 191, 203, 100, 243, 41, 28, 87, 218, 181, 169, 83, 157, 197, 33,
        227, 99, 194, 73, 215, 49, 223, 148, 124, 200, 44, 118, 29, 87, 29, 247, 245, 173, 167, 57,
        243, 201, 184, 240, 73, 55, 215, 97, 247, 143, 219, 30, 51, 250, 224, 29, 244, 65, 185, 93,
        30, 204, 134, 37, 190, 78, 167, 113, 148, 174, 248, 66, 128, 243, 48, 1, 20, 50, 128, 68,
        68, 38, 124, 114, 148, 121, 184, 41, 232, 185, 24, 254, 186, 145, 227, 141, 48, 115, 81,
        164, 169, 254, 254, 228, 0, 141, 39, 99, 131, 3, 131, 241, 190, 228, 64, 239, 114, 141, 87,
        225, 173, 250, 82, 102, 107, 42, 244, 171, 99, 54, 250, 82, 91, 54, 78, 47, 183, 216, 56,
        220, 166, 141, 50, 38, 110, 188, 226, 91, 15, 237, 187, 246, 138, 193, 13, 95, 49, 17, 139,
        197, 182, 107, 162, 41, 238, 145, 34, 38, 98, 24, 179, 162, 42, 124, 102, 115, 220, 35,
        162, 227, 188, 142, 19, 116, 8, 246, 78, 230, 119, 123, 48, 119, 176, 122, 74, 184, 171,
        225, 156, 189, 254, 221, 75, 198, 57, 251, 84, 123, 169, 99, 59, 89, 66, 239, 146, 106,
        133, 144, 34, 103, 101, 139, 228, 164, 130, 45, 3, 13, 95, 57, 243, 148, 209, 240, 7, 112,
        228, 161, 12, 39, 181, 98, 113, 43, 117, 146, 81, 108, 109, 73, 170, 138, 86, 15, 112, 58,
        68, 19, 153, 118, 201, 8, 4, 22, 211, 202, 229, 194, 87, 255, 110, 36, 121, 160, 189, 112,
        222, 78, 146, 218, 125, 247, 156, 232, 118, 200, 24, 124, 192, 200, 24, 132, 53, 76, 16,
        228, 152, 69, 157, 210, 46, 79, 157, 55, 242, 124, 180, 171, 173, 164, 186, 157, 7, 115,
        76, 1, 166, 223, 39, 197, 192, 179, 28, 169, 48, 164, 181, 125, 238, 251, 92, 123, 228,
        204, 137, 157, 4, 195, 219, 9, 78, 134, 34, 232, 215, 18, 158, 28, 145, 10, 246, 242, 28,
        43, 67, 150, 7, 77, 197, 26, 159, 206, 200, 53, 134, 12, 63, 149, 175, 25, 189, 80, 194,
        113, 33, 20, 3, 138, 169, 121, 225, 75, 184, 107, 23, 7, 176, 45, 188, 50, 201, 203, 242,
        10, 111, 8, 251, 211, 7, 235, 70, 128, 231, 49, 102, 69, 40, 128, 65, 61, 13, 112, 170, 43,
        65, 106, 131, 126, 228, 62, 168, 9, 92, 24, 254, 6, 107, 126, 2, 193, 186, 41, 43, 135,
        127, 255, 140, 145, 230, 214, 240, 238, 208, 250, 170, 82, 17, 32, 231, 29, 86, 129, 32,
        165, 13, 97, 121, 181, 176, 178, 94, 250, 154, 17, 224, 119, 232, 142, 74, 24, 0, 173, 179,
        50, 124, 85, 184, 160, 107, 60, 139, 171, 205, 237, 63, 7, 82, 3, 170, 15, 144, 90, 28,
        212, 174, 143, 32, 19, 207, 254, 197, 8, 178, 129, 227, 241, 80, 32, 152, 75, 103, 148,
        172, 89, 220, 247, 136, 92, 106, 100, 74, 116, 7, 40, 111, 93, 122, 252, 205, 92, 116, 253,
        36, 18, 218, 60, 38, 204, 227, 20, 130, 194, 215, 147, 160, 53, 54, 236, 178, 144, 176,
        230, 106, 104, 126, 232, 206, 95, 24, 205, 103, 67, 39, 7, 45, 187, 166, 132, 199, 64, 225,
        242, 69, 253, 53, 40, 118, 32, 97, 66, 173, 135, 154, 23, 9, 230, 15, 222, 103, 36, 184,
        28, 9, 77, 144, 210, 75, 15, 132, 45, 196, 162, 229, 54, 228, 246, 56, 80, 112, 59, 88,
        102, 214, 199, 145, 96, 252, 197, 239, 24, 9, 240, 173, 107, 49, 20, 1, 38, 200, 108, 158,
        228, 177, 214, 159, 115, 21, 183, 57, 243, 120, 101, 235, 212, 121, 105, 147, 56, 98, 188,
        143, 188, 108, 196, 200, 135, 14, 73, 173, 189, 70, 192, 251, 30, 36, 101, 220, 185, 124,
        110, 113, 86, 105, 196, 133, 163, 103, 194, 127, 243, 215, 70, 128, 19, 161, 35, 161, 72,
        227, 137, 212, 64, 127, 10, 20, 209, 64, 95, 50, 158, 28, 76, 37, 104, 127, 42, 214, 183,
        37, 139, 238, 185, 171, 69, 22, 117, 216, 220, 16, 29, 234, 31, 76, 110, 153, 123, 235,
        209, 22, 115, 233, 48, 230, 182, 164, 216, 254, 82, 203, 101, 237, 150, 203, 110, 93, 15,
        243, 81, 35, 41, 102, 96, 125, 110, 110, 228, 176, 26, 122, 9, 174, 152, 89, 88, 163, 100,
        134, 5, 54, 137, 234, 157, 36, 129, 186, 140, 78, 187, 80, 169, 128, 41, 170, 39, 80, 199,
        16, 29, 227, 46, 87, 12, 78, 150, 16, 209, 181, 228, 131, 61, 145, 29, 213, 68, 115, 179,
        125, 18, 71, 191, 115, 64, 9, 154, 201, 147, 52, 156, 12, 202, 136, 150, 130, 42, 83, 216,
        189, 40, 173, 113, 14, 139, 172, 21, 236, 210, 217, 157, 85, 101, 19, 172, 15, 199, 90,
        231, 192, 146, 116, 74, 174, 65, 222, 229, 158, 23, 40, 78, 117, 149, 35, 150, 118, 129,
        58, 231, 252, 202, 8, 117, 162, 195, 80, 41, 90, 178, 216, 198, 6, 115, 28, 58, 99, 5, 238,
        10, 199, 237, 112, 131, 243, 149, 86, 168, 223, 254, 247, 156, 17, 234, 99, 93, 187, 200,
        251, 16, 80, 125, 116, 4, 4, 211, 244, 18, 200, 69, 143, 211, 41, 177, 186, 153, 158, 179,
        204, 173, 238, 116, 215, 149, 119, 94, 53, 146, 205, 119, 152, 172, 159, 234, 157, 163,
        192, 234, 52, 19, 40, 79, 67, 206, 75, 89, 129, 2, 186, 8, 159, 181, 144, 5, 135, 46, 238,
        189, 186, 238, 235, 28, 217, 0, 77, 219, 80, 185, 78, 177, 170, 148, 46, 109, 150, 209,
        141, 238, 221, 120, 189, 230, 74, 95, 108, 232, 214, 102, 43, 228, 179, 143, 127, 203, 8,
        249, 118, 100, 151, 14, 107, 8, 200, 65, 90, 96, 170, 14, 82, 152, 185, 130, 198, 99, 177,
        131, 173, 48, 255, 187, 101, 103, 219, 161, 9, 115, 30, 199, 245, 206, 193, 12, 209, 35,
        216, 94, 3, 227, 105, 169, 92, 50, 166, 36, 100, 46, 143, 66, 249, 198, 85, 0, 85, 204,
        142, 133, 118, 225, 150, 211, 70, 79, 233, 30, 196, 189, 29, 131, 139, 199, 116, 126, 200,
        217, 18, 36, 82, 142, 87, 0, 3, 191, 67, 117, 83, 174, 200, 33, 129, 168, 198, 85, 90, 25,
        185, 251, 186, 145, 241, 206, 206, 102, 218, 120, 92, 51, 110, 22, 226, 68, 63, 251, 0,
        255, 233, 141, 40, 112, 90, 193, 126, 120, 116, 167, 164, 109, 130, 221, 131, 227, 114,
        231, 192, 18, 180, 8, 98, 166, 20, 184, 0, 40, 188, 93, 119, 164, 249, 131, 59, 251, 56,
        77, 156, 63, 70, 58, 154, 52, 226, 73, 216, 134, 220, 170, 205, 60, 15, 251, 23, 48, 103,
        57, 206, 237, 173, 46, 87, 43, 89, 247, 219, 63, 50, 146, 125, 184, 179, 233, 44, 158, 162,
        121, 217, 120, 194, 145, 229, 46, 200, 8, 128, 177, 229, 34, 176, 228, 216, 6, 76, 101, 11,
        217, 11, 103, 159, 52, 146, 29, 192, 241, 212, 141, 144, 245, 52, 209, 62, 167, 113, 226,
        39, 80, 75, 76, 6, 171, 140, 78, 120, 110, 183, 238, 187, 205, 10, 40, 137, 162, 250, 73,
        88, 65, 2, 8, 239, 246, 200, 104, 160, 36, 201, 216, 193, 34, 41, 136, 245, 67, 91, 136,
        163, 120, 85, 37, 93, 44, 228, 106, 201, 199, 95, 40, 25, 211, 72, 15, 124, 208, 117, 127,
        72, 210, 4, 45, 113, 37, 192, 110, 86, 0, 240, 36, 231, 53, 248, 86, 13, 42, 75, 27, 139,
        153, 232, 230, 193, 235, 97, 222, 250, 204, 211, 70, 135, 126, 4, 199, 181, 144, 152, 73,
        122, 76, 46, 75, 114, 84, 170, 21, 216, 66, 41, 212, 28, 21, 44, 179, 162, 241, 196, 73,
        50, 207, 149, 7, 223, 50, 177, 253, 155, 189, 215, 232, 194, 231, 186, 58, 224, 66, 92,
        123, 117, 221, 44, 155, 18, 75, 32, 30, 153, 95, 182, 216, 102, 187, 40, 43, 65, 194, 50,
        225, 122, 190, 2, 217, 182, 151, 11, 213, 247, 254, 101, 116, 225, 35, 56, 126, 41, 36,
        102, 31, 45, 73, 59, 208, 169, 22, 195, 22, 148, 36, 108, 245, 248, 132, 138, 68, 143, 4,
        160, 187, 175, 55, 199, 183, 93, 250, 166, 17, 240, 10, 38, 154, 187, 67, 2, 246, 211, 140,
        132, 114, 13, 31, 220, 208, 124, 224, 117, 55, 26, 9, 209, 34, 179, 107, 150, 192, 136, 65,
        71, 238, 69, 248, 234, 208, 206, 222, 96, 147, 240, 33, 28, 191, 18, 146, 112, 128, 102,
        187, 129, 79, 56, 192, 113, 59, 148, 157, 172, 190, 8, 50, 51, 7, 185, 175, 23, 210, 33,
        40, 188, 28, 135, 66, 133, 68, 167, 21, 108, 37, 46, 240, 237, 1, 251, 244, 162, 89, 82,
        245, 119, 221, 232, 30, 188, 11, 236, 32, 61, 78, 26, 173, 9, 58, 38, 125, 159, 17, 29,
        220, 51, 65, 163, 169, 26, 29, 93, 175, 65, 93, 1, 187, 203, 30, 140, 177, 243, 215, 97,
        124, 36, 36, 227, 16, 108, 40, 139, 14, 87, 85, 78, 150, 180, 23, 243, 204, 109, 182, 18,
        232, 44, 91, 209, 143, 44, 211, 245, 198, 229, 0, 151, 68, 231, 101, 121, 111, 191, 190,
        114, 241, 148, 145, 249, 55, 184, 76, 207, 132, 99, 6, 65, 147, 69, 71, 30, 151, 110, 19,
        186, 241, 188, 59, 207, 151, 150, 148, 128, 205, 102, 2, 27, 247, 141, 240, 79, 99, 34, 85,
        141, 32, 203, 230, 201, 88, 192, 92, 239, 186, 129, 246, 240, 99, 191, 52, 222, 193, 241,
        14, 36, 44, 144, 59, 248, 111, 24, 117, 16, 138, 156, 67, 45, 34, 171, 160, 21, 133, 130,
        165, 48, 87, 35, 209, 217, 53, 97, 203, 205, 34, 156, 140, 86, 132, 111, 194, 92, 255, 226,
        251, 141, 121, 245, 121, 28, 31, 14, 137, 153, 160, 19, 40, 94, 73, 70, 248, 117, 10, 137,
        20, 159, 94, 55, 92, 185, 89, 186, 128, 183, 9, 44, 99, 216, 162, 184, 11, 101, 86, 129,
        233, 238, 203, 30, 158, 125, 96, 114, 159, 17, 249, 7, 56, 62, 24, 18, 57, 73, 143, 28,
        131, 201, 207, 9, 151, 131, 4, 1, 102, 108, 213, 107, 145, 4, 240, 209, 121, 161, 27, 133,
        37, 225, 192, 218, 177, 109, 177, 23, 234, 115, 255, 185, 108, 92, 4, 186, 163, 62, 210,
        130, 218, 40, 144, 230, 32, 114, 148, 207, 43, 11, 115, 182, 47, 28, 230, 243, 171, 29,
        150, 171, 224, 11, 189, 217, 210, 201, 18, 160, 240, 155, 62, 29, 185, 230, 181, 163, 29,
        168, 101, 214, 44, 171, 219, 82, 45, 148, 252, 0, 123, 92, 176, 100, 134, 161, 208, 88,
        179, 241, 223, 26, 18, 3, 61, 177, 193, 158, 161, 212, 85, 51, 14, 171, 112, 2, 249, 209,
        226, 160, 118, 92, 234, 167, 84, 62, 179, 94, 20, 105, 127, 180, 119, 141, 47, 58, 111, 60,
        122, 243, 187, 175, 49, 249, 68, 223, 129, 200, 228, 182, 191, 255, 15, 50, 255, 123, 134,
    ]
    .to_vec();

    let expected: Vec<u8> = [
        7, 0, 0, 0, 77, 114, 66, 108, 111, 110, 100, 141, 44, 8, 0, 67, 0, 0, 0, 1, 84, 0, 0, 0,
        64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 49, 57, 57, 56, 32, 45, 32, 70, 97,
        110, 116, 97, 122, 105, 97, 32, 45, 32, 66, 114, 105, 116, 105, 115, 104, 32, 65, 110, 116,
        104, 101, 109, 115, 32, 40, 77, 52, 97, 41, 92, 48, 49, 32, 45, 32, 68, 97, 118, 101, 32,
        65, 110, 103, 101, 108, 32, 45, 32, 70, 117, 110, 107, 32, 77, 117, 115, 105, 99, 46, 109,
        52, 97, 206, 95, 188, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 65, 1, 0, 0, 1, 0,
        0, 0, 47, 1, 0, 0, 1, 76, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 49,
        57, 57, 56, 32, 45, 32, 70, 97, 110, 116, 97, 122, 105, 97, 32, 45, 32, 66, 114, 105, 116,
        105, 115, 104, 32, 65, 110, 116, 104, 101, 109, 115, 32, 40, 77, 52, 97, 41, 92, 48, 50,
        32, 45, 32, 80, 74, 32, 45, 32, 72, 97, 112, 112, 121, 32, 68, 97, 121, 115, 46, 109, 52,
        97, 162, 47, 198, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 65, 1, 0, 0, 1, 0, 0,
        0, 63, 1, 0, 0, 1, 86, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 49, 57,
        57, 56, 32, 45, 32, 70, 97, 110, 116, 97, 122, 105, 97, 32, 45, 32, 66, 114, 105, 116, 105,
        115, 104, 32, 65, 110, 116, 104, 101, 109, 115, 32, 40, 77, 52, 97, 41, 92, 48, 51, 32, 45,
        32, 76, 97, 103, 117, 110, 97, 32, 45, 32, 83, 112, 105, 108, 108, 101, 114, 32, 70, 114,
        111, 109, 32, 82, 105, 111, 46, 109, 52, 97, 204, 165, 118, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2,
        0, 0, 0, 0, 0, 0, 0, 65, 1, 0, 0, 1, 0, 0, 0, 189, 0, 0, 0, 1, 104, 0, 0, 0, 64, 64, 101,
        101, 99, 112, 106, 92, 69, 68, 77, 92, 49, 57, 57, 56, 32, 45, 32, 70, 97, 110, 116, 97,
        122, 105, 97, 32, 45, 32, 66, 114, 105, 116, 105, 115, 104, 32, 65, 110, 116, 104, 101,
        109, 115, 32, 40, 77, 52, 97, 41, 92, 48, 52, 32, 45, 32, 84, 114, 97, 105, 108, 101, 114,
        32, 80, 97, 114, 107, 115, 32, 45, 32, 82, 97, 105, 115, 101, 32, 89, 111, 117, 114, 32,
        72, 97, 110, 100, 115, 32, 73, 110, 32, 84, 104, 101, 32, 65, 105, 114, 46, 109, 52, 97,
        129, 162, 161, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 65, 1, 0, 0, 1, 0, 0, 0,
        3, 1, 0, 0, 1, 72, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 49, 57, 57,
        56, 32, 45, 32, 70, 97, 110, 116, 97, 122, 105, 97, 32, 45, 32, 66, 114, 105, 116, 105,
        115, 104, 32, 65, 110, 116, 104, 101, 109, 115, 32, 40, 77, 52, 97, 41, 92, 48, 53, 32, 45,
        32, 83, 121, 98, 105, 108, 32, 45, 32, 87, 104, 121, 46, 109, 52, 97, 58, 22, 197, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 65, 1, 0, 0, 1, 0, 0, 0, 61, 1, 0, 0, 1, 118, 0,
        0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 49, 57, 57, 56, 32, 45, 32, 70,
        97, 110, 116, 97, 122, 105, 97, 32, 45, 32, 66, 114, 105, 116, 105, 115, 104, 32, 65, 110,
        116, 104, 101, 109, 115, 32, 40, 77, 52, 97, 41, 92, 48, 54, 32, 45, 32, 80, 101, 111, 112,
        108, 101, 32, 85, 110, 100, 101, 114, 103, 114, 111, 117, 110, 100, 32, 70, 116, 46, 32,
        83, 104, 97, 114, 111, 110, 32, 87, 105, 108, 108, 105, 97, 109, 115, 32, 45, 32, 77, 117,
        115, 105, 99, 32, 73, 115, 32, 80, 117, 109, 112, 105, 110, 103, 46, 109, 52, 97, 142, 174,
        205, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 65, 1, 0, 0, 1, 0, 0, 0, 75, 1, 0,
        0, 1, 84, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 49, 57, 57, 56, 32,
        45, 32, 70, 97, 110, 116, 97, 122, 105, 97, 32, 45, 32, 66, 114, 105, 116, 105, 115, 104,
        32, 65, 110, 116, 104, 101, 109, 115, 32, 40, 77, 52, 97, 41, 92, 48, 55, 32, 45, 32, 78,
        105, 103, 104, 116, 109, 111, 118, 101, 114, 115, 32, 45, 32, 79, 117, 114, 32, 72, 111,
        117, 115, 101, 46, 109, 52, 97, 12, 17, 162, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0,
        0, 0, 65, 1, 0, 0, 1, 0, 0, 0, 4, 1, 0, 0, 1, 94, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106,
        92, 69, 68, 77, 92, 49, 57, 57, 56, 32, 45, 32, 70, 97, 110, 116, 97, 122, 105, 97, 32, 45,
        32, 66, 114, 105, 116, 105, 115, 104, 32, 65, 110, 116, 104, 101, 109, 115, 32, 40, 77, 52,
        97, 41, 92, 48, 56, 32, 45, 32, 66, 108, 97, 99, 107, 32, 67, 111, 110, 110, 101, 99, 116,
        105, 111, 110, 32, 45, 32, 71, 105, 118, 101, 32, 77, 101, 32, 82, 104, 121, 116, 104, 109,
        46, 109, 52, 97, 125, 61, 234, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 65, 1, 0,
        0, 1, 0, 0, 0, 121, 1, 0, 0, 1, 84, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68,
        77, 92, 49, 57, 57, 56, 32, 45, 32, 70, 97, 110, 116, 97, 122, 105, 97, 32, 45, 32, 66,
        114, 105, 116, 105, 115, 104, 32, 65, 110, 116, 104, 101, 109, 115, 32, 40, 77, 52, 97, 41,
        92, 48, 57, 32, 45, 32, 69, 97, 115, 116, 32, 53, 55, 116, 104, 32, 83, 116, 32, 45, 32,
        83, 97, 116, 117, 114, 100, 97, 121, 46, 109, 52, 97, 34, 133, 199, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 2, 0, 0, 0, 0, 0, 0, 0, 65, 1, 0, 0, 1, 0, 0, 0, 65, 1, 0, 0, 1, 96, 0, 0, 0, 64, 64,
        101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 49, 57, 57, 56, 32, 45, 32, 70, 97, 110, 116,
        97, 122, 105, 97, 32, 45, 32, 66, 114, 105, 116, 105, 115, 104, 32, 65, 110, 116, 104, 101,
        109, 115, 32, 40, 77, 52, 97, 41, 92, 49, 48, 32, 45, 32, 71, 105, 115, 101, 108, 108, 101,
        32, 74, 97, 99, 107, 115, 111, 110, 32, 45, 32, 76, 111, 118, 101, 32, 67, 111, 109, 109,
        97, 110, 100, 109, 101, 110, 116, 115, 46, 109, 52, 97, 226, 127, 223, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 65, 1, 0, 0, 1, 0, 0, 0, 104, 1, 0, 0, 1, 85, 0, 0, 0, 64,
        64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 49, 57, 57, 56, 32, 45, 32, 70, 97, 110,
        116, 97, 122, 105, 97, 32, 45, 32, 66, 114, 105, 116, 105, 115, 104, 32, 65, 110, 116, 104,
        101, 109, 115, 32, 40, 77, 52, 97, 41, 92, 49, 49, 32, 45, 32, 67, 101, 32, 67, 101, 32,
        80, 101, 110, 105, 115, 116, 111, 110, 32, 45, 32, 70, 105, 110, 97, 108, 108, 121, 46,
        109, 52, 97, 230, 115, 188, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 65, 1, 0, 0,
        1, 0, 0, 0, 47, 1, 0, 0, 1, 84, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77,
        92, 49, 57, 57, 56, 32, 45, 32, 70, 97, 110, 116, 97, 122, 105, 97, 32, 45, 32, 66, 114,
        105, 116, 105, 115, 104, 32, 65, 110, 116, 104, 101, 109, 115, 32, 40, 77, 52, 97, 41, 92,
        49, 50, 32, 45, 32, 74, 97, 121, 100, 101, 101, 32, 45, 32, 80, 108, 97, 115, 116, 105, 99,
        32, 68, 114, 101, 97, 109, 115, 46, 109, 52, 97, 62, 23, 151, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2,
        0, 0, 0, 0, 0, 0, 0, 65, 1, 0, 0, 1, 0, 0, 0, 242, 0, 0, 0, 1, 93, 0, 0, 0, 64, 64, 101,
        101, 99, 112, 106, 92, 69, 68, 77, 92, 49, 57, 57, 56, 32, 45, 32, 70, 97, 110, 116, 97,
        122, 105, 97, 32, 45, 32, 66, 114, 105, 116, 105, 115, 104, 32, 65, 110, 116, 104, 101,
        109, 115, 32, 40, 77, 52, 97, 41, 92, 49, 51, 32, 45, 32, 84, 104, 101, 32, 75, 110, 111,
        119, 108, 101, 100, 103, 101, 32, 45, 32, 65, 115, 32, 85, 110, 116, 105, 108, 32, 84, 104,
        101, 32, 68, 97, 121, 46, 109, 52, 97, 108, 228, 130, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0,
        0, 0, 0, 0, 0, 65, 1, 0, 0, 1, 0, 0, 0, 209, 0, 0, 0, 1, 79, 0, 0, 0, 64, 64, 101, 101, 99,
        112, 106, 92, 69, 68, 77, 92, 49, 57, 57, 56, 32, 45, 32, 70, 97, 110, 116, 97, 122, 105,
        97, 32, 45, 32, 66, 114, 105, 116, 105, 115, 104, 32, 65, 110, 116, 104, 101, 109, 115, 32,
        40, 77, 52, 97, 41, 92, 49, 52, 32, 45, 32, 66, 97, 109, 98, 111, 111, 32, 45, 32, 66, 97,
        109, 98, 111, 111, 103, 105, 101, 46, 109, 52, 97, 38, 181, 8, 1, 0, 0, 0, 0, 0, 0, 0, 0,
        2, 0, 0, 0, 0, 0, 0, 0, 65, 1, 0, 0, 1, 0, 0, 0, 171, 1, 0, 0, 1, 107, 0, 0, 0, 64, 64,
        101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 49, 57, 57, 56, 32, 45, 32, 70, 97, 110, 116,
        97, 122, 105, 97, 32, 45, 32, 66, 114, 105, 116, 105, 115, 104, 32, 65, 110, 116, 104, 101,
        109, 115, 32, 40, 77, 52, 97, 41, 92, 49, 53, 32, 45, 32, 83, 104, 101, 110, 97, 32, 70,
        116, 46, 32, 66, 121, 114, 111, 110, 32, 83, 116, 105, 110, 103, 108, 121, 32, 45, 32, 76,
        101, 116, 32, 84, 104, 101, 32, 66, 101, 97, 116, 32, 72, 105, 116, 32, 39, 101, 109, 46,
        109, 52, 97, 241, 35, 156, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 65, 1, 0, 0,
        1, 0, 0, 0, 250, 0, 0, 0, 1, 99, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77,
        92, 49, 57, 57, 56, 32, 45, 32, 70, 97, 110, 116, 97, 122, 105, 97, 32, 45, 32, 66, 114,
        105, 116, 105, 115, 104, 32, 65, 110, 116, 104, 101, 109, 115, 32, 40, 77, 52, 97, 41, 92,
        49, 54, 32, 45, 32, 83, 101, 120, 45, 79, 32, 45, 32, 83, 111, 110, 105, 113, 117, 101, 32,
        45, 32, 73, 32, 84, 104, 111, 117, 103, 104, 116, 32, 73, 116, 32, 87, 97, 115, 32, 89,
        111, 117, 46, 109, 52, 97, 108, 64, 139, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0,
        65, 1, 0, 0, 1, 0, 0, 0, 222, 0, 0, 0, 1, 79, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92,
        69, 68, 77, 92, 49, 57, 57, 56, 32, 45, 32, 70, 97, 110, 116, 97, 122, 105, 97, 32, 45, 32,
        66, 114, 105, 116, 105, 115, 104, 32, 65, 110, 116, 104, 101, 109, 115, 32, 40, 77, 52, 97,
        41, 92, 82, 45, 52, 54, 54, 51, 55, 45, 49, 51, 48, 56, 55, 56, 49, 53, 51, 55, 46, 106,
        112, 101, 103, 46, 106, 112, 103, 203, 67, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 79,
        0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 49, 57, 57, 56, 32, 45, 32,
        70, 97, 110, 116, 97, 122, 105, 97, 32, 45, 32, 66, 114, 105, 116, 105, 115, 104, 32, 65,
        110, 116, 104, 101, 109, 115, 32, 40, 77, 52, 97, 41, 92, 82, 45, 52, 54, 54, 51, 55, 45,
        49, 51, 48, 56, 55, 56, 49, 53, 53, 52, 46, 106, 112, 101, 103, 46, 106, 112, 103, 153,
        106, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 61, 0, 0, 0, 64, 64, 101, 101, 99, 112,
        106, 92, 69, 68, 77, 92, 49, 57, 57, 56, 32, 45, 32, 70, 97, 110, 116, 97, 122, 105, 97,
        32, 45, 32, 66, 114, 105, 116, 105, 115, 104, 32, 65, 110, 116, 104, 101, 109, 115, 32, 40,
        77, 52, 97, 41, 92, 99, 111, 118, 101, 114, 46, 106, 112, 103, 238, 135, 2, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 1, 117, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77,
        92, 49, 57, 57, 56, 32, 45, 32, 70, 97, 110, 116, 97, 122, 105, 97, 32, 45, 32, 66, 114,
        105, 116, 105, 115, 104, 32, 65, 110, 116, 104, 101, 109, 115, 32, 50, 48, 48, 48, 32, 40,
        77, 52, 97, 41, 92, 48, 49, 32, 45, 32, 76, 101, 115, 32, 82, 121, 116, 104, 109, 101, 115,
        32, 68, 105, 103, 105, 116, 97, 108, 101, 115, 32, 45, 32, 40, 72, 101, 121, 32, 89, 111,
        117, 41, 32, 87, 104, 97, 116, 39, 115, 32, 84, 104, 97, 116, 32, 83, 111, 117, 110, 100,
        46, 109, 52, 97, 210, 160, 194, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 65, 1,
        0, 0, 1, 0, 0, 0, 59, 1, 0, 0, 1, 84, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68,
        77, 92, 49, 57, 57, 56, 32, 45, 32, 70, 97, 110, 116, 97, 122, 105, 97, 32, 45, 32, 66,
        114, 105, 116, 105, 115, 104, 32, 65, 110, 116, 104, 101, 109, 115, 32, 50, 48, 48, 48, 32,
        40, 77, 52, 97, 41, 92, 48, 50, 32, 45, 32, 77, 117, 114, 107, 32, 32, 82, 101, 97, 99,
        104, 32, 70, 111, 114, 32, 77, 101, 46, 109, 52, 97, 237, 157, 171, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 2, 0, 0, 0, 0, 0, 0, 0, 65, 1, 0, 0, 1, 0, 0, 0, 21, 1, 0, 0, 1, 101, 0, 0, 0, 64, 64,
        101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 49, 57, 57, 56, 32, 45, 32, 70, 97, 110, 116,
        97, 122, 105, 97, 32, 45, 32, 66, 114, 105, 116, 105, 115, 104, 32, 65, 110, 116, 104, 101,
        109, 115, 32, 50, 48, 48, 48, 32, 40, 77, 52, 97, 41, 92, 48, 51, 32, 45, 32, 82, 97, 110,
        100, 121, 32, 67, 114, 97, 119, 102, 111, 114, 100, 32, 45, 32, 87, 104, 105, 115, 104,
        105, 110, 103, 32, 79, 110, 32, 65, 32, 83, 116, 97, 114, 46, 109, 52, 97, 188, 137, 239,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 65, 1, 0, 0, 1, 0, 0, 0, 132, 1, 0, 0,
        1, 107, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 49, 57, 57, 56, 32,
        45, 32, 70, 97, 110, 116, 97, 122, 105, 97, 32, 45, 32, 66, 114, 105, 116, 105, 115, 104,
        32, 65, 110, 116, 104, 101, 109, 115, 32, 50, 48, 48, 48, 32, 40, 77, 52, 97, 41, 92, 48,
        52, 32, 45, 32, 66, 108, 97, 99, 107, 32, 67, 111, 110, 110, 101, 99, 116, 105, 111, 110,
        32, 45, 32, 73, 39, 109, 32, 71, 111, 110, 110, 97, 32, 71, 101, 116, 32, 89, 111, 117, 32,
        66, 97, 98, 121, 46, 109, 52, 97, 206, 76, 176, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0,
        0, 0, 0, 65, 1, 0, 0, 1, 0, 0, 0, 29, 1, 0, 0, 1, 93, 0, 0, 0, 64, 64, 101, 101, 99, 112,
        106, 92, 69, 68, 77, 92, 49, 57, 57, 56, 32, 45, 32, 70, 97, 110, 116, 97, 122, 105, 97,
        32, 45, 32, 66, 114, 105, 116, 105, 115, 104, 32, 65, 110, 116, 104, 101, 109, 115, 32, 50,
        48, 48, 48, 32, 40, 77, 52, 97, 41, 92, 48, 53, 32, 45, 32, 77, 111, 117, 110, 116, 32, 82,
        117, 115, 104, 109, 111, 114, 101, 32, 45, 32, 89, 111, 117, 32, 66, 101, 116, 116, 101,
        114, 46, 109, 52, 97, 157, 93, 176, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 64,
        1, 0, 0, 1, 0, 0, 0, 29, 1, 0, 0, 1, 95, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69,
        68, 77, 92, 49, 57, 57, 56, 32, 45, 32, 70, 97, 110, 116, 97, 122, 105, 97, 32, 45, 32, 66,
        114, 105, 116, 105, 115, 104, 32, 65, 110, 116, 104, 101, 109, 115, 32, 50, 48, 48, 48, 32,
        40, 77, 52, 97, 41, 92, 48, 54, 32, 45, 32, 68, 97, 32, 78, 111, 114, 116, 104, 32, 70, 97,
        99, 101, 32, 75, 105, 108, 108, 97, 32, 45, 32, 68, 97, 32, 80, 111, 119, 97, 46, 109, 52,
        97, 192, 111, 209, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 65, 1, 0, 0, 1, 0, 0,
        0, 83, 1, 0, 0, 1, 92, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 49, 57,
        57, 56, 32, 45, 32, 70, 97, 110, 116, 97, 122, 105, 97, 32, 45, 32, 66, 114, 105, 116, 105,
        115, 104, 32, 65, 110, 116, 104, 101, 109, 115, 32, 50, 48, 48, 48, 32, 40, 77, 52, 97, 41,
        92, 48, 55, 32, 45, 32, 86, 105, 116, 111, 32, 66, 101, 110, 105, 116, 111, 32, 45, 32, 68,
        105, 115, 99, 32, 74, 111, 99, 107, 101, 121, 115, 46, 109, 52, 97, 153, 37, 121, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 65, 1, 0, 0, 1, 0, 0, 0, 195, 0, 0, 0, 1, 105, 0,
        0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 49, 57, 57, 56, 32, 45, 32, 70,
        97, 110, 116, 97, 122, 105, 97, 32, 45, 32, 66, 114, 105, 116, 105, 115, 104, 32, 65, 110,
        116, 104, 101, 109, 115, 32, 50, 48, 48, 48, 32, 40, 77, 52, 97, 41, 92, 48, 56, 32, 45,
        32, 77, 111, 110, 116, 97, 110, 111, 32, 84, 114, 117, 109, 112, 101, 116, 109, 97, 110,
        32, 45, 32, 73, 116, 122, 97, 32, 84, 114, 117, 109, 112, 101, 116, 32, 84, 104, 105, 110,
        103, 46, 109, 52, 97, 64, 225, 173, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 65,
        1, 0, 0, 1, 0, 0, 0, 25, 1, 0, 0, 1, 92, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69,
        68, 77, 92, 49, 57, 57, 56, 32, 45, 32, 70, 97, 110, 116, 97, 122, 105, 97, 32, 45, 32, 66,
        114, 105, 116, 105, 115, 104, 32, 65, 110, 116, 104, 101, 109, 115, 32, 50, 48, 48, 48, 32,
        40, 77, 52, 97, 41, 92, 48, 57, 32, 45, 32, 69, 100, 100, 105, 101, 32, 65, 109, 97, 100,
        111, 114, 32, 45, 32, 72, 111, 117, 115, 101, 32, 77, 117, 115, 105, 99, 46, 109, 52, 97,
        104, 203, 138, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 65, 1, 0, 0, 1, 0, 0, 0,
        224, 0, 0, 0, 1, 100, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 49, 57,
        57, 56, 32, 45, 32, 70, 97, 110, 116, 97, 122, 105, 97, 32, 45, 32, 66, 114, 105, 116, 105,
        115, 104, 32, 65, 110, 116, 104, 101, 109, 115, 32, 50, 48, 48, 48, 32, 40, 77, 52, 97, 41,
        92, 49, 48, 32, 45, 32, 67, 101, 118, 105, 110, 32, 70, 105, 115, 104, 101, 114, 32, 45,
        32, 84, 104, 101, 32, 70, 114, 101, 97, 107, 115, 32, 67, 111, 109, 101, 32, 79, 117, 116,
        46, 109, 52, 97, 73, 180, 233, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 65, 1, 0,
        0, 1, 0, 0, 0, 122, 1, 0, 0, 1, 90, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68,
        77, 92, 49, 57, 57, 56, 32, 45, 32, 70, 97, 110, 116, 97, 122, 105, 97, 32, 45, 32, 66,
        114, 105, 116, 105, 115, 104, 32, 65, 110, 116, 104, 101, 109, 115, 32, 50, 48, 48, 48, 32,
        40, 77, 52, 97, 41, 92, 49, 49, 32, 45, 32, 80, 114, 111, 112, 104, 101, 116, 115, 32, 111,
        102, 32, 83, 111, 117, 110, 100, 32, 45, 32, 72, 105, 103, 104, 46, 109, 52, 97, 58, 236,
        118, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 65, 1, 0, 0, 1, 0, 0, 0, 191, 0, 0,
        0, 1, 90, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 49, 57, 57, 56, 32,
        45, 32, 70, 97, 110, 116, 97, 122, 105, 97, 32, 45, 32, 66, 114, 105, 116, 105, 115, 104,
        32, 65, 110, 116, 104, 101, 109, 115, 32, 50, 48, 48, 48, 32, 40, 77, 52, 97, 41, 92, 49,
        50, 32, 45, 32, 90, 45, 70, 97, 99, 116, 111, 114, 32, 45, 32, 71, 105, 118, 101, 32, 73,
        116, 32, 79, 110, 32, 85, 112, 46, 109, 52, 97, 57, 88, 200, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2,
        0, 0, 0, 0, 0, 0, 0, 65, 1, 0, 0, 1, 0, 0, 0, 68, 1, 0, 0, 1, 93, 0, 0, 0, 64, 64, 101,
        101, 99, 112, 106, 92, 69, 68, 77, 92, 49, 57, 57, 56, 32, 45, 32, 70, 97, 110, 116, 97,
        122, 105, 97, 32, 45, 32, 66, 114, 105, 116, 105, 115, 104, 32, 65, 110, 116, 104, 101,
        109, 115, 32, 50, 48, 48, 48, 32, 40, 77, 52, 97, 41, 92, 49, 51, 32, 45, 32, 76, 105, 115,
        97, 32, 76, 111, 101, 98, 32, 45, 32, 70, 97, 108, 108, 105, 110, 103, 32, 73, 110, 32, 76,
        111, 118, 101, 46, 109, 52, 97, 86, 37, 128, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0,
        0, 0, 65, 1, 0, 0, 1, 0, 0, 0, 206, 0, 0, 0, 1, 93, 0, 0, 0, 64, 64, 101, 101, 99, 112,
        106, 92, 69, 68, 77, 92, 49, 57, 57, 56, 32, 45, 32, 70, 97, 110, 116, 97, 122, 105, 97,
        32, 45, 32, 66, 114, 105, 116, 105, 115, 104, 32, 65, 110, 116, 104, 101, 109, 115, 32, 50,
        48, 48, 48, 32, 40, 77, 52, 97, 41, 92, 49, 52, 32, 45, 32, 72, 111, 108, 108, 121, 32, 74,
        111, 104, 110, 115, 111, 110, 32, 45, 32, 72, 97, 108, 108, 101, 108, 117, 106, 97, 104,
        33, 46, 109, 52, 97, 72, 197, 158, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 65,
        1, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 1, 98, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69,
        68, 77, 92, 49, 57, 57, 56, 32, 45, 32, 70, 97, 110, 116, 97, 122, 105, 97, 32, 45, 32, 66,
        114, 105, 116, 105, 115, 104, 32, 65, 110, 116, 104, 101, 109, 115, 32, 50, 48, 48, 48, 32,
        40, 77, 52, 97, 41, 92, 49, 53, 32, 45, 32, 68, 74, 32, 74, 74, 32, 45, 32, 85, 110, 114,
        101, 108, 101, 97, 115, 101, 100, 32, 65, 110, 116, 104, 101, 109, 115, 32, 86, 111, 108,
        32, 49, 46, 109, 52, 97, 18, 32, 204, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0,
        65, 1, 0, 0, 1, 0, 0, 0, 74, 1, 0, 0, 1, 92, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92,
        69, 68, 77, 92, 49, 57, 57, 56, 32, 45, 32, 70, 97, 110, 116, 97, 122, 105, 97, 32, 45, 32,
        66, 114, 105, 116, 105, 115, 104, 32, 65, 110, 116, 104, 101, 109, 115, 32, 50, 48, 48, 48,
        32, 40, 77, 52, 97, 41, 92, 49, 54, 32, 45, 32, 65, 108, 108, 105, 115, 116, 101, 114, 32,
        87, 104, 105, 116, 101, 104, 101, 97, 100, 32, 45, 32, 84, 104, 101, 109, 101, 46, 109, 52,
        97, 116, 223, 215, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 65, 1, 0, 0, 1, 0, 0,
        0, 93, 1, 0, 0, 1, 90, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 49, 57,
        57, 56, 32, 45, 32, 70, 97, 110, 116, 97, 122, 105, 97, 32, 45, 32, 66, 114, 105, 116, 105,
        115, 104, 32, 65, 110, 116, 104, 101, 109, 115, 32, 50, 48, 48, 48, 32, 40, 77, 52, 97, 41,
        92, 82, 45, 49, 50, 52, 55, 54, 52, 45, 49, 51, 55, 53, 51, 49, 51, 56, 52, 50, 45, 54, 52,
        48, 53, 46, 106, 112, 101, 103, 46, 106, 112, 103, 126, 96, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 1, 90, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 49, 57, 57,
        56, 32, 45, 32, 70, 97, 110, 116, 97, 122, 105, 97, 32, 45, 32, 66, 114, 105, 116, 105,
        115, 104, 32, 65, 110, 116, 104, 101, 109, 115, 32, 50, 48, 48, 48, 32, 40, 77, 52, 97, 41,
        92, 82, 45, 49, 50, 52, 55, 54, 52, 45, 49, 51, 55, 53, 51, 49, 51, 56, 52, 57, 45, 57, 54,
        56, 51, 46, 106, 112, 101, 103, 46, 106, 112, 103, 238, 150, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 1, 66, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 49, 57, 57,
        56, 32, 45, 32, 70, 97, 110, 116, 97, 122, 105, 97, 32, 45, 32, 66, 114, 105, 116, 105,
        115, 104, 32, 65, 110, 116, 104, 101, 109, 115, 32, 50, 48, 48, 48, 32, 40, 77, 52, 97, 41,
        92, 99, 111, 118, 101, 114, 46, 106, 112, 103, 8, 83, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 1, 108, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 70, 97, 110,
        116, 97, 122, 105, 97, 32, 84, 104, 101, 32, 72, 111, 117, 115, 101, 32, 67, 111, 108, 108,
        101, 99, 116, 105, 111, 110, 32, 86, 111, 108, 46, 32, 53, 32, 45, 32, 84, 97, 108, 108,
        32, 80, 97, 117, 108, 32, 40, 68, 105, 115, 99, 32, 50, 41, 92, 48, 49, 45, 79, 110, 101,
        32, 68, 111, 118, 101, 45, 87, 104, 105, 116, 101, 32, 76, 111, 118, 101, 45, 71, 101, 110,
        101, 114, 97, 108, 32, 80, 111, 112, 46, 109, 112, 51, 134, 44, 0, 1, 0, 0, 0, 0, 0, 0, 0,
        0, 2, 0, 0, 0, 0, 0, 0, 0, 64, 1, 0, 0, 1, 0, 0, 0, 163, 1, 0, 0, 1, 116, 0, 0, 0, 64, 64,
        101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 70, 97, 110, 116, 97, 122, 105, 97, 32, 84,
        104, 101, 32, 72, 111, 117, 115, 101, 32, 67, 111, 108, 108, 101, 99, 116, 105, 111, 110,
        32, 86, 111, 108, 46, 32, 53, 32, 45, 32, 84, 97, 108, 108, 32, 80, 97, 117, 108, 32, 40,
        68, 105, 115, 99, 32, 50, 41, 92, 48, 50, 45, 67, 74, 32, 66, 111, 108, 108, 97, 110, 100,
        45, 83, 117, 103, 97, 114, 32, 73, 115, 32, 83, 119, 101, 101, 116, 101, 114, 45, 71, 101,
        110, 101, 114, 97, 108, 32, 80, 111, 112, 46, 109, 112, 51, 194, 162, 188, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 64, 1, 0, 0, 1, 0, 0, 0, 53, 1, 0, 0, 1, 112, 0, 0, 0,
        64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 70, 97, 110, 116, 97, 122, 105, 97, 32,
        84, 104, 101, 32, 72, 111, 117, 115, 101, 32, 67, 111, 108, 108, 101, 99, 116, 105, 111,
        110, 32, 86, 111, 108, 46, 32, 53, 32, 45, 32, 84, 97, 108, 108, 32, 80, 97, 117, 108, 32,
        40, 68, 105, 115, 99, 32, 50, 41, 92, 48, 51, 45, 76, 111, 119, 32, 80, 114, 101, 115, 115,
        117, 114, 101, 45, 84, 104, 101, 32, 66, 105, 102, 116, 101, 114, 45, 71, 101, 110, 101,
        114, 97, 108, 32, 80, 111, 112, 46, 109, 112, 51, 174, 109, 213, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        2, 0, 0, 0, 0, 0, 0, 0, 64, 1, 0, 0, 1, 0, 0, 0, 93, 1, 0, 0, 1, 112, 0, 0, 0, 64, 64, 101,
        101, 99, 112, 106, 92, 69, 68, 77, 92, 70, 97, 110, 116, 97, 122, 105, 97, 32, 84, 104,
        101, 32, 72, 111, 117, 115, 101, 32, 67, 111, 108, 108, 101, 99, 116, 105, 111, 110, 32,
        86, 111, 108, 46, 32, 53, 32, 45, 32, 84, 97, 108, 108, 32, 80, 97, 117, 108, 32, 40, 68,
        105, 115, 99, 32, 50, 41, 92, 48, 52, 45, 83, 104, 97, 122, 122, 97, 109, 109, 45, 80, 104,
        117, 110, 107, 101, 101, 32, 77, 117, 122, 101, 101, 107, 45, 71, 101, 110, 101, 114, 97,
        108, 32, 80, 111, 112, 46, 109, 112, 51, 221, 252, 174, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0,
        0, 0, 0, 0, 0, 64, 1, 0, 0, 1, 0, 0, 0, 30, 1, 0, 0, 1, 117, 0, 0, 0, 64, 64, 101, 101, 99,
        112, 106, 92, 69, 68, 77, 92, 70, 97, 110, 116, 97, 122, 105, 97, 32, 84, 104, 101, 32, 72,
        111, 117, 115, 101, 32, 67, 111, 108, 108, 101, 99, 116, 105, 111, 110, 32, 86, 111, 108,
        46, 32, 53, 32, 45, 32, 84, 97, 108, 108, 32, 80, 97, 117, 108, 32, 40, 68, 105, 115, 99,
        32, 50, 41, 92, 48, 53, 45, 65, 99, 101, 32, 79, 102, 32, 66, 97, 115, 101, 45, 76, 105,
        118, 105, 110, 103, 32, 73, 110, 32, 68, 97, 110, 103, 101, 114, 45, 71, 101, 110, 101,
        114, 97, 108, 32, 80, 111, 112, 46, 109, 112, 51, 237, 250, 208, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        2, 0, 0, 0, 0, 0, 0, 0, 64, 1, 0, 0, 1, 0, 0, 0, 86, 1, 0, 0, 1, 117, 0, 0, 0, 64, 64, 101,
        101, 99, 112, 106, 92, 69, 68, 77, 92, 70, 97, 110, 116, 97, 122, 105, 97, 32, 84, 104,
        101, 32, 72, 111, 117, 115, 101, 32, 67, 111, 108, 108, 101, 99, 116, 105, 111, 110, 32,
        86, 111, 108, 46, 32, 53, 32, 45, 32, 84, 97, 108, 108, 32, 80, 97, 117, 108, 32, 40, 68,
        105, 115, 99, 32, 50, 41, 92, 48, 54, 45, 76, 105, 115, 97, 32, 77, 97, 121, 45, 67, 117,
        114, 115, 101, 32, 79, 102, 32, 86, 111, 111, 100, 111, 111, 32, 82, 97, 121, 45, 71, 101,
        110, 101, 114, 97, 108, 32, 80, 111, 112, 46, 109, 112, 51, 117, 41, 189, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 64, 1, 0, 0, 1, 0, 0, 0, 53, 1, 0, 0, 1, 128, 0, 0, 0, 64,
        64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 70, 97, 110, 116, 97, 122, 105, 97, 32, 84,
        104, 101, 32, 72, 111, 117, 115, 101, 32, 67, 111, 108, 108, 101, 99, 116, 105, 111, 110,
        32, 86, 111, 108, 46, 32, 53, 32, 45, 32, 84, 97, 108, 108, 32, 80, 97, 117, 108, 32, 40,
        68, 105, 115, 99, 32, 50, 41, 92, 48, 55, 45, 66, 108, 117, 101, 32, 76, 97, 103, 111, 111,
        110, 45, 76, 101, 116, 32, 84, 104, 101, 32, 77, 117, 115, 105, 99, 32, 72, 121, 112, 110,
        111, 116, 105, 122, 101, 32, 89, 111, 117, 45, 71, 101, 110, 101, 114, 97, 108, 32, 80,
        111, 112, 46, 109, 112, 51, 180, 159, 146, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0,
        0, 64, 1, 0, 0, 1, 0, 0, 0, 240, 0, 0, 0, 1, 104, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106,
        92, 69, 68, 77, 92, 70, 97, 110, 116, 97, 122, 105, 97, 32, 84, 104, 101, 32, 72, 111, 117,
        115, 101, 32, 67, 111, 108, 108, 101, 99, 116, 105, 111, 110, 32, 86, 111, 108, 46, 32, 53,
        32, 45, 32, 84, 97, 108, 108, 32, 80, 97, 117, 108, 32, 40, 68, 105, 115, 99, 32, 50, 41,
        92, 48, 56, 45, 77, 97, 114, 121, 32, 75, 105, 97, 110, 105, 45, 49, 48, 48, 37, 45, 71,
        101, 110, 101, 114, 97, 108, 32, 80, 111, 112, 46, 109, 112, 51, 255, 20, 8, 1, 0, 0, 0, 0,
        0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 64, 1, 0, 0, 1, 0, 0, 0, 176, 1, 0, 0, 1, 120, 0, 0, 0,
        64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 70, 97, 110, 116, 97, 122, 105, 97, 32,
        84, 104, 101, 32, 72, 111, 117, 115, 101, 32, 67, 111, 108, 108, 101, 99, 116, 105, 111,
        110, 32, 86, 111, 108, 46, 32, 53, 32, 45, 32, 84, 97, 108, 108, 32, 80, 97, 117, 108, 32,
        40, 68, 105, 115, 99, 32, 50, 41, 92, 48, 57, 45, 78, 97, 116, 117, 114, 97, 108, 32, 66,
        111, 114, 110, 32, 71, 114, 111, 111, 118, 101, 115, 45, 70, 111, 114, 101, 114, 117, 110,
        110, 101, 114, 45, 71, 101, 110, 101, 114, 97, 108, 32, 80, 111, 112, 46, 109, 112, 51,
        188, 20, 153, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 64, 1, 0, 0, 1, 0, 0, 0,
        250, 0, 0, 0, 1, 127, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 70, 97,
        110, 116, 97, 122, 105, 97, 32, 84, 104, 101, 32, 72, 111, 117, 115, 101, 32, 67, 111, 108,
        108, 101, 99, 116, 105, 111, 110, 32, 86, 111, 108, 46, 32, 53, 32, 45, 32, 84, 97, 108,
        108, 32, 80, 97, 117, 108, 32, 40, 68, 105, 115, 99, 32, 50, 41, 92, 49, 48, 45, 84, 104,
        101, 32, 70, 108, 111, 111, 114, 32, 70, 101, 100, 101, 114, 97, 116, 105, 111, 110, 45,
        76, 111, 118, 101, 32, 82, 101, 115, 117, 114, 114, 101, 99, 116, 105, 111, 110, 45, 71,
        101, 110, 101, 114, 97, 108, 32, 80, 111, 112, 46, 109, 112, 51, 101, 110, 210, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 64, 1, 0, 0, 1, 0, 0, 0, 88, 1, 0, 0, 1, 116, 0, 0,
        0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 70, 97, 110, 116, 97, 122, 105, 97,
        32, 84, 104, 101, 32, 72, 111, 117, 115, 101, 32, 67, 111, 108, 108, 101, 99, 116, 105,
        111, 110, 32, 86, 111, 108, 46, 32, 53, 32, 45, 32, 84, 97, 108, 108, 32, 80, 97, 117, 108,
        32, 40, 68, 105, 115, 99, 32, 50, 41, 92, 49, 49, 45, 84, 104, 101, 32, 68, 105, 103, 105,
        116, 97, 108, 32, 66, 108, 111, 110, 100, 101, 115, 45, 65, 110, 116, 104, 101, 117, 109,
        45, 71, 101, 110, 101, 114, 97, 108, 32, 80, 111, 112, 46, 109, 112, 51, 169, 87, 233, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 64, 1, 0, 0, 1, 0, 0, 0, 126, 1, 0, 0, 1,
        106, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 70, 97, 110, 116, 97,
        122, 105, 97, 32, 84, 104, 101, 32, 72, 111, 117, 115, 101, 32, 67, 111, 108, 108, 101, 99,
        116, 105, 111, 110, 32, 86, 111, 108, 46, 32, 53, 32, 45, 32, 84, 97, 108, 108, 32, 80, 97,
        117, 108, 32, 40, 68, 105, 115, 99, 32, 50, 41, 92, 49, 50, 45, 82, 101, 100, 32, 83, 117,
        110, 45, 84, 104, 105, 115, 32, 76, 111, 118, 101, 45, 71, 101, 110, 101, 114, 97, 108, 32,
        80, 111, 112, 46, 109, 112, 51, 86, 37, 139, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0,
        0, 0, 64, 1, 0, 0, 1, 0, 0, 0, 227, 0, 0, 0, 1, 117, 0, 0, 0, 64, 64, 101, 101, 99, 112,
        106, 92, 69, 68, 77, 92, 70, 97, 110, 116, 97, 122, 105, 97, 32, 84, 104, 101, 32, 72, 111,
        117, 115, 101, 32, 67, 111, 108, 108, 101, 99, 116, 105, 111, 110, 32, 86, 111, 108, 46,
        32, 53, 32, 45, 32, 84, 97, 108, 108, 32, 80, 97, 117, 108, 32, 40, 68, 105, 115, 99, 32,
        50, 41, 92, 49, 51, 45, 83, 117, 110, 103, 108, 97, 115, 115, 101, 115, 32, 82, 111, 110,
        45, 70, 101, 101, 108, 32, 84, 104, 101, 32, 66, 101, 97, 116, 45, 71, 101, 110, 101, 114,
        97, 108, 32, 80, 111, 112, 46, 109, 112, 51, 39, 240, 170, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0,
        0, 0, 0, 0, 0, 0, 64, 1, 0, 0, 1, 0, 0, 0, 24, 1, 0, 0, 1, 117, 0, 0, 0, 64, 64, 101, 101,
        99, 112, 106, 92, 69, 68, 77, 92, 70, 97, 110, 116, 97, 122, 105, 97, 32, 84, 104, 101, 32,
        72, 111, 117, 115, 101, 32, 67, 111, 108, 108, 101, 99, 116, 105, 111, 110, 32, 86, 111,
        108, 46, 32, 53, 32, 45, 32, 84, 97, 108, 108, 32, 80, 97, 117, 108, 32, 40, 68, 105, 115,
        99, 32, 50, 41, 92, 49, 52, 45, 74, 111, 110, 32, 84, 104, 101, 32, 68, 101, 110, 116, 105,
        115, 116, 45, 71, 108, 111, 98, 97, 108, 32, 70, 97, 122, 101, 115, 45, 71, 101, 110, 101,
        114, 97, 108, 32, 80, 111, 112, 46, 109, 112, 51, 196, 162, 163, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        2, 0, 0, 0, 0, 0, 0, 0, 64, 1, 0, 0, 1, 0, 0, 0, 12, 1, 0, 0, 1, 133, 0, 0, 0, 64, 64, 101,
        101, 99, 112, 106, 92, 69, 68, 77, 92, 70, 97, 110, 116, 97, 122, 105, 97, 32, 84, 104,
        101, 32, 72, 111, 117, 115, 101, 32, 67, 111, 108, 108, 101, 99, 116, 105, 111, 110, 44,
        32, 86, 111, 108, 46, 32, 53, 32, 91, 68, 105, 115, 99, 32, 49, 93, 92, 48, 49, 45, 75,
        117, 118, 97, 45, 73, 115, 110, 39, 116, 32, 73, 116, 32, 84, 105, 109, 101, 32, 40, 68,
        97, 118, 101, 32, 77, 111, 114, 97, 108, 101, 39, 115, 32, 69, 117, 114, 111, 32, 67, 108,
        117, 98, 32, 77, 105, 120, 41, 45, 71, 101, 110, 101, 114, 97, 108, 32, 69, 108, 101, 99,
        116, 114, 111, 110, 105, 99, 46, 109, 112, 51, 159, 196, 83, 1, 0, 0, 0, 0, 0, 0, 0, 0, 2,
        0, 0, 0, 0, 0, 0, 0, 64, 1, 0, 0, 1, 0, 0, 0, 44, 2, 0, 0, 1, 130, 0, 0, 0, 64, 64, 101,
        101, 99, 112, 106, 92, 69, 68, 77, 92, 70, 97, 110, 116, 97, 122, 105, 97, 32, 84, 104,
        101, 32, 72, 111, 117, 115, 101, 32, 67, 111, 108, 108, 101, 99, 116, 105, 111, 110, 44,
        32, 86, 111, 108, 46, 32, 53, 32, 91, 68, 105, 115, 99, 32, 49, 93, 92, 48, 50, 45, 83,
        101, 114, 105, 97, 108, 32, 68, 105, 118, 97, 45, 75, 101, 101, 112, 32, 72, 111, 112, 101,
        32, 65, 108, 105, 118, 101, 32, 40, 83, 101, 114, 105, 97, 108, 32, 67, 108, 117, 98, 32,
        77, 105, 120, 41, 45, 71, 101, 110, 101, 114, 97, 108, 32, 69, 108, 101, 99, 116, 114, 111,
        110, 105, 99, 46, 109, 112, 51, 25, 173, 172, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0,
        0, 0, 64, 1, 0, 0, 1, 0, 0, 0, 26, 1, 0, 0, 1, 119, 0, 0, 0, 64, 64, 101, 101, 99, 112,
        106, 92, 69, 68, 77, 92, 70, 97, 110, 116, 97, 122, 105, 97, 32, 84, 104, 101, 32, 72, 111,
        117, 115, 101, 32, 67, 111, 108, 108, 101, 99, 116, 105, 111, 110, 44, 32, 86, 111, 108,
        46, 32, 53, 32, 91, 68, 105, 115, 99, 32, 49, 93, 92, 48, 51, 45, 89, 111, 106, 111, 32,
        87, 111, 114, 107, 105, 110, 103, 45, 72, 111, 108, 100, 32, 79, 110, 32, 40, 49, 50, 95,
        32, 86, 101, 114, 115, 105, 111, 110, 41, 45, 71, 101, 110, 101, 114, 97, 108, 32, 69, 108,
        101, 99, 116, 114, 111, 110, 105, 99, 46, 109, 112, 51, 244, 97, 11, 1, 0, 0, 0, 0, 0, 0,
        0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 64, 1, 0, 0, 1, 0, 0, 0, 182, 1, 0, 0, 1, 130, 0, 0, 0, 64,
        64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 70, 97, 110, 116, 97, 122, 105, 97, 32, 84,
        104, 101, 32, 72, 111, 117, 115, 101, 32, 67, 111, 108, 108, 101, 99, 116, 105, 111, 110,
        44, 32, 86, 111, 108, 46, 32, 53, 32, 91, 68, 105, 115, 99, 32, 49, 93, 92, 48, 52, 45, 74,
        111, 121, 32, 70, 111, 114, 32, 76, 105, 102, 101, 45, 87, 97, 116, 99, 104, 97, 32, 71,
        111, 110, 110, 97, 32, 68, 111, 32, 40, 77, 97, 105, 110, 115, 116, 114, 101, 101, 116, 32,
        77, 105, 120, 41, 45, 71, 101, 110, 101, 114, 97, 108, 32, 69, 108, 101, 99, 116, 114, 111,
        110, 105, 99, 46, 109, 112, 51, 114, 164, 243, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0,
        0, 0, 64, 1, 0, 0, 1, 0, 0, 0, 143, 1, 0, 0, 1, 124, 0, 0, 0, 64, 64, 101, 101, 99, 112,
        106, 92, 69, 68, 77, 92, 70, 97, 110, 116, 97, 122, 105, 97, 32, 84, 104, 101, 32, 72, 111,
        117, 115, 101, 32, 67, 111, 108, 108, 101, 99, 116, 105, 111, 110, 44, 32, 86, 111, 108,
        46, 32, 53, 32, 91, 68, 105, 115, 99, 32, 49, 93, 92, 48, 53, 45, 83, 111, 108, 117, 116,
        105, 111, 110, 45, 70, 101, 101, 108, 115, 32, 83, 111, 32, 82, 105, 103, 104, 116, 32, 40,
        78, 117, 115, 104, 32, 67, 108, 117, 98, 32, 77, 105, 120, 41, 45, 71, 101, 110, 101, 114,
        97, 108, 32, 69, 108, 101, 99, 116, 114, 111, 110, 105, 99, 46, 109, 112, 51, 35, 194, 144,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 64, 1, 0, 0, 1, 0, 0, 0, 237, 0, 0, 0,
        1, 125, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 70, 97, 110, 116, 97,
        122, 105, 97, 32, 84, 104, 101, 32, 72, 111, 117, 115, 101, 32, 67, 111, 108, 108, 101, 99,
        116, 105, 111, 110, 44, 32, 86, 111, 108, 46, 32, 53, 32, 91, 68, 105, 115, 99, 32, 49, 93,
        92, 48, 54, 45, 67, 111, 111, 108, 32, 74, 97, 99, 107, 45, 74, 117, 115, 39, 32, 67, 111,
        109, 101, 32, 40, 82, 97, 108, 112, 104, 105, 101, 39, 115, 32, 77, 97, 105, 110, 32, 77,
        105, 120, 41, 45, 71, 101, 110, 101, 114, 97, 108, 32, 69, 108, 101, 99, 116, 114, 111,
        110, 105, 99, 46, 109, 112, 51, 208, 57, 239, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0,
        0, 0, 64, 1, 0, 0, 1, 0, 0, 0, 135, 1, 0, 0, 1, 136, 0, 0, 0, 64, 64, 101, 101, 99, 112,
        106, 92, 69, 68, 77, 92, 70, 97, 110, 116, 97, 122, 105, 97, 32, 84, 104, 101, 32, 72, 111,
        117, 115, 101, 32, 67, 111, 108, 108, 101, 99, 116, 105, 111, 110, 44, 32, 86, 111, 108,
        46, 32, 53, 32, 91, 68, 105, 115, 99, 32, 49, 93, 92, 48, 55, 45, 68, 39, 32, 74, 97, 105,
        109, 105, 110, 32, 38, 32, 68, 74, 97, 121, 98, 101, 101, 32, 70, 101, 97, 116, 46, 32, 82,
        111, 115, 101, 45, 70, 101, 118, 101, 114, 32, 40, 79, 114, 105, 103, 105, 110, 97, 108,
        32, 77, 105, 120, 41, 45, 71, 101, 110, 101, 114, 97, 108, 32, 69, 108, 101, 99, 116, 114,
        111, 110, 105, 99, 46, 109, 112, 51, 172, 98, 189, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0,
        0, 0, 0, 0, 64, 1, 0, 0, 1, 0, 0, 0, 54, 1, 0, 0, 1, 127, 0, 0, 0, 64, 64, 101, 101, 99,
        112, 106, 92, 69, 68, 77, 92, 70, 97, 110, 116, 97, 122, 105, 97, 32, 84, 104, 101, 32, 72,
        111, 117, 115, 101, 32, 67, 111, 108, 108, 101, 99, 116, 105, 111, 110, 44, 32, 86, 111,
        108, 46, 32, 53, 32, 91, 68, 105, 115, 99, 32, 49, 93, 92, 48, 56, 45, 90, 32, 70, 97, 99,
        116, 111, 114, 45, 71, 111, 116, 116, 97, 32, 75, 101, 101, 112, 32, 80, 117, 115, 104,
        105, 110, 103, 32, 40, 69, 120, 112, 97, 110, 100, 101, 100, 32, 77, 105, 120, 41, 45, 71,
        101, 110, 101, 114, 97, 108, 32, 69, 108, 101, 99, 116, 114, 111, 110, 105, 99, 46, 109,
        112, 51, 48, 176, 189, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 64, 1, 0, 0, 1,
        0, 0, 0, 54, 1, 0, 0, 1, 143, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92,
        70, 97, 110, 116, 97, 122, 105, 97, 32, 84, 104, 101, 32, 72, 111, 117, 115, 101, 32, 67,
        111, 108, 108, 101, 99, 116, 105, 111, 110, 44, 32, 86, 111, 108, 46, 32, 53, 32, 91, 68,
        105, 115, 99, 32, 49, 93, 92, 48, 57, 45, 83, 117, 98, 109, 101, 114, 103, 101, 32, 102,
        101, 97, 116, 46, 32, 74, 97, 110, 32, 74, 111, 104, 110, 115, 111, 110, 45, 84, 97, 107,
        101, 32, 77, 101, 32, 66, 121, 32, 84, 104, 101, 32, 72, 97, 110, 100, 32, 40, 86, 111, 99,
        97, 108, 32, 77, 105, 120, 41, 45, 71, 101, 110, 101, 114, 97, 108, 32, 69, 108, 101, 99,
        116, 114, 111, 110, 105, 99, 46, 109, 112, 51, 205, 189, 133, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2,
        0, 0, 0, 0, 0, 0, 0, 64, 1, 0, 0, 1, 0, 0, 0, 219, 0, 0, 0, 1, 157, 0, 0, 0, 64, 64, 101,
        101, 99, 112, 106, 92, 69, 68, 77, 92, 70, 97, 110, 116, 97, 122, 105, 97, 32, 84, 104,
        101, 32, 72, 111, 117, 115, 101, 32, 67, 111, 108, 108, 101, 99, 116, 105, 111, 110, 44,
        32, 86, 111, 108, 46, 32, 53, 32, 91, 68, 105, 115, 99, 32, 49, 93, 92, 49, 48, 45, 68,
        101, 101, 112, 32, 90, 111, 110, 101, 32, 102, 101, 97, 116, 46, 32, 83, 121, 98, 105, 108,
        32, 74, 101, 102, 102, 114, 105, 101, 115, 45, 73, 116, 39, 115, 32, 71, 111, 110, 110, 97,
        32, 66, 101, 32, 65, 108, 114, 105, 103, 104, 116, 32, 40, 68, 74, 32, 71, 117, 97, 110,
        115, 32, 67, 108, 117, 98, 32, 77, 105, 120, 41, 45, 71, 101, 110, 101, 114, 97, 108, 32,
        69, 108, 101, 99, 116, 114, 111, 110, 105, 99, 46, 109, 112, 51, 142, 156, 211, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 64, 1, 0, 0, 1, 0, 0, 0, 90, 1, 0, 0, 1, 130, 0, 0,
        0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 70, 97, 110, 116, 97, 122, 105, 97,
        32, 84, 104, 101, 32, 72, 111, 117, 115, 101, 32, 67, 111, 108, 108, 101, 99, 116, 105,
        111, 110, 44, 32, 86, 111, 108, 46, 32, 53, 32, 91, 68, 105, 115, 99, 32, 49, 93, 92, 49,
        49, 45, 70, 117, 110, 107, 121, 32, 71, 114, 101, 101, 110, 32, 68, 111, 103, 115, 45, 70,
        105, 114, 101, 100, 32, 85, 112, 32, 40, 84, 119, 105, 108, 111, 32, 65, 110, 116, 104,
        101, 109, 32, 69, 100, 105, 116, 41, 45, 71, 101, 110, 101, 114, 97, 108, 32, 69, 108, 101,
        99, 116, 114, 111, 110, 105, 99, 46, 109, 112, 51, 120, 123, 19, 1, 0, 0, 0, 0, 0, 0, 0, 0,
        2, 0, 0, 0, 0, 0, 0, 0, 64, 1, 0, 0, 1, 0, 0, 0, 195, 1, 0, 0, 1, 142, 0, 0, 0, 64, 64,
        101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 70, 97, 110, 116, 97, 122, 105, 97, 32, 84,
        104, 101, 32, 72, 111, 117, 115, 101, 32, 67, 111, 108, 108, 101, 99, 116, 105, 111, 110,
        44, 32, 86, 111, 108, 46, 32, 53, 32, 91, 68, 105, 115, 99, 32, 49, 93, 92, 49, 50, 45, 73,
        110, 110, 101, 114, 32, 67, 105, 116, 121, 45, 68, 111, 32, 77, 101, 32, 82, 105, 103, 104,
        116, 32, 40, 76, 105, 115, 97, 32, 77, 97, 114, 105, 101, 32, 69, 120, 112, 101, 114, 105,
        101, 110, 99, 101, 32, 77, 97, 115, 116, 101, 114, 32, 77, 105, 120, 41, 45, 71, 101, 110,
        101, 114, 97, 108, 32, 69, 108, 101, 99, 116, 114, 111, 110, 105, 99, 46, 109, 112, 51,
        132, 75, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 64, 1, 0, 0, 1, 0, 0, 0,
        167, 1, 0, 0, 1, 134, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 70, 97,
        110, 116, 97, 122, 105, 97, 32, 84, 104, 101, 32, 72, 111, 117, 115, 101, 32, 67, 111, 108,
        108, 101, 99, 116, 105, 111, 110, 44, 32, 86, 111, 108, 46, 32, 53, 32, 91, 68, 105, 115,
        99, 32, 49, 93, 92, 49, 51, 45, 78, 89, 39, 115, 32, 70, 105, 110, 101, 115, 116, 45, 68,
        111, 32, 89, 111, 117, 32, 70, 101, 101, 108, 32, 77, 101, 32, 40, 86, 105, 99, 116, 111,
        114, 32, 83, 105, 109, 111, 110, 101, 108, 108, 105, 32, 77, 105, 120, 41, 45, 71, 101,
        110, 101, 114, 97, 108, 32, 69, 108, 101, 99, 116, 114, 111, 110, 105, 99, 46, 109, 112,
        51, 182, 248, 206, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 64, 1, 0, 0, 1, 0, 0,
        0, 83, 1, 0, 0, 1, 65, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 77,
        117, 115, 105, 99, 32, 85, 110, 115, 111, 114, 116, 101, 100, 92, 85, 108, 116, 105, 109,
        97, 116, 101, 32, 70, 97, 110, 116, 97, 122, 105, 97, 32, 67, 111, 108, 108, 101, 99, 116,
        105, 111, 110, 92, 46, 68, 83, 95, 83, 116, 111, 114, 101, 4, 60, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 1, 93, 0, 0, 0, 64, 64, 101, 101, 99, 112, 106, 92, 69, 68, 77, 92, 80,
        97, 117, 108, 32, 84, 97, 121, 108, 111, 114, 92, 83, 116, 117, 32, 65, 108, 108, 101, 110,
        32, 64, 32, 66, 111, 119, 108, 101, 114, 115, 32, 50, 55, 44, 48, 56, 44, 57, 52, 32, 70,
        97, 110, 116, 97, 122, 105, 97, 32, 109, 97, 100, 101, 32, 105, 110, 32, 104, 101, 97, 118,
        101, 110, 45, 116, 52, 114, 74, 67, 120, 82, 105, 66, 116, 69, 46, 119, 101, 98, 109, 222,
        150, 17, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 161, 53, 12, 0, 75, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0,
    ]
    .to_vec();
    let deflated = deflate(data).unwrap();
    assert_eq!(deflated, expected)
}
