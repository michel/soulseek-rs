//https://www.rfc-editor.org/rfc/rfc1950

struct BitReader {
    mem: Vec<u8>,
    pos: usize,
    b: u8,
    numbits: i32,
}

impl BitReader {
    fn new(mem: Vec<u8>) -> Self {
        Self {
            mem,
            pos: 0,
            b: 0,
            numbits: 0,
        }
    }

    fn read_byte(&mut self) -> Result<u8, String> {
        self.numbits = 0; // discard unread bits
        if self.pos >= self.mem.len() {
            return Err("End of data".to_string());
        }
        let b = self.mem[self.pos];
        self.pos += 1;
        Ok(b)
    }

    fn read_bit(&mut self) -> Result<u8, String> {
        if self.numbits <= 0 {
            self.b = self.read_byte()?;
            self.numbits = 8;
        }
        self.numbits -= 1;
        // shift bit out of byte
        let bit = self.b & 1;
        self.b >>= 1;
        Ok(bit)
    }

    fn read_bits(&mut self, n: usize) -> Result<u32, String> {
        let mut o = 0u32;
        for i in 0..n {
            o |= (self.read_bit()? as u32) << i;
        }
        Ok(o)
    }

    fn read_bytes(&mut self, n: usize) -> Result<u32, String> {
        // read bytes as an integer in little-endian
        let mut o = 0u32;
        for i in 0..n {
            o |= (self.read_byte()? as u32) << (8 * i);
        }
        Ok(o)
    }
}

pub fn deflate(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut r = BitReader::new(input.to_vec());
    let cmf = r.read_byte()?;
    let cm = cmf & 15; // Compression method
    if cm != 8 {
        // only CM=8 is supported
        return Err("invalid CM".to_string());
    }
    let cinfo = (cmf >> 4) & 15; // Compression info
    if cinfo > 7 {
        return Err("invalid CINFO".to_string());
    }
    let flg = r.read_byte()?;
    if ((cmf as u32) * 256 + (flg as u32)) % 31 != 0 {
        return Err("CMF+FLG checksum failed".to_string());
    }
    let fdict = (flg >> 5) & 1; // preset dictionary?
    if fdict != 0 {
        return Err("preset dictionary not supported".to_string());
    }
    let out = inflate(&mut r)?; // decompress DEFLATE data
    let _adler32 = r.read_bytes(4)?; // Adler-32 checksum (for this exercise, we ignore it)
    Ok(out)
}

fn inflate(r: &mut BitReader) -> Result<Vec<u8>, String> {
    let mut bfinal = 0;
    let mut out = Vec::new();
    while bfinal == 0 {
        bfinal = r.read_bit()?;
        let btype = r.read_bits(2)?;
        match btype {
            0 => inflate_block_no_compression(r, &mut out)?,
            1 => inflate_block_fixed(r, &mut out)?,
            2 => inflate_block_dynamic(r, &mut out)?,
            _ => return Err("invalid BTYPE".to_string()),
        }
    }
    Ok(out)
}

fn inflate_block_no_compression(
    r: &mut BitReader,
    o: &mut Vec<u8>,
) -> Result<(), String> {
    let len = r.read_bytes(2)?;
    let _nlen = r.read_bytes(2)?;
    for _ in 0..len {
        o.push(r.read_byte()?);
    }
    Ok(())
}

#[derive(Clone)]
struct Node {
    symbol: Option<u32>,
    left: Option<Box<Node>>,
    right: Option<Box<Node>>,
}

impl Node {
    fn new() -> Self {
        Self {
            symbol: None,
            left: None,
            right: None,
        }
    }
}

struct HuffmanTree {
    root: Node,
}

impl HuffmanTree {
    fn new() -> Self {
        Self { root: Node::new() }
    }

    fn insert(&mut self, codeword: u32, n: usize, symbol: u32) {
        // Insert an entry into the tree mapping `codeword` of len `n` to `symbol`
        let mut node = &mut self.root;
        for i in (0..n).rev() {
            let b = (codeword >> i) & 1;
            if b != 0 {
                if node.right.is_none() {
                    node.right = Some(Box::new(Node::new()));
                }
                node = node.right.as_mut().unwrap();
            } else {
                if node.left.is_none() {
                    node.left = Some(Box::new(Node::new()));
                }
                node = node.left.as_mut().unwrap();
            }
        }
        node.symbol = Some(symbol);
    }
}

fn decode_symbol(r: &mut BitReader, t: &HuffmanTree) -> Result<u32, String> {
    let mut node = &t.root;
    while node.left.is_some() || node.right.is_some() {
        let b = r.read_bit()?;
        node = if b != 0 {
            node.right.as_ref().unwrap()
        } else {
            node.left.as_ref().unwrap()
        };
    }
    node.symbol.ok_or("No symbol found".to_string())
}

const LENGTH_EXTRA_BITS: [usize; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5,
    5, 5, 5, 0,
];
const LENGTH_BASE: [u32; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59,
    67, 83, 99, 115, 131, 163, 195, 227, 258,
];
const DISTANCE_EXTRA_BITS: [usize; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10,
    11, 11, 12, 12, 13, 13,
];
const DISTANCE_BASE: [u32; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513,
    769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];

fn inflate_block_data(
    r: &mut BitReader,
    literal_length_tree: &HuffmanTree,
    distance_tree: &HuffmanTree,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    loop {
        let sym = decode_symbol(r, literal_length_tree)?;
        if sym <= 255 {
            // Literal byte
            out.push(sym as u8);
        } else if sym == 256 {
            // End of block
            return Ok(());
        } else {
            // <length, backward distance> pair
            let sym_idx = (sym - 257) as usize;
            if sym_idx >= LENGTH_EXTRA_BITS.len() {
                return Err("Invalid length symbol".to_string());
            }
            let length =
                r.read_bits(LENGTH_EXTRA_BITS[sym_idx])? + LENGTH_BASE[sym_idx];
            let dist_sym = decode_symbol(r, distance_tree)?;
            if dist_sym as usize >= DISTANCE_EXTRA_BITS.len() {
                return Err("Invalid distance symbol".to_string());
            }
            let dist = r.read_bits(DISTANCE_EXTRA_BITS[dist_sym as usize])?
                + DISTANCE_BASE[dist_sym as usize];
            if dist as usize > out.len() {
                return Err("Distance too large".to_string());
            }
            for _ in 0..length {
                let idx = out.len() - dist as usize;
                let byte = out[idx];
                out.push(byte);
            }
        }
    }
}

fn bl_list_to_tree(bl: &[usize], alphabet: &[u32]) -> HuffmanTree {
    let max_bits = *bl.iter().max().unwrap_or(&0);
    let mut bl_count = vec![0; max_bits + 1];
    for &bitlen in bl {
        if bitlen != 0 {
            bl_count[bitlen] += 1;
        }
    }

    let mut next_code = vec![0; max_bits + 1];
    for bits in 2..=max_bits {
        next_code[bits] = (next_code[bits - 1] + bl_count[bits - 1]) << 1;
    }

    let mut t = HuffmanTree::new();
    for (i, &bitlen) in bl.iter().enumerate() {
        if bitlen != 0 && i < alphabet.len() {
            t.insert(next_code[bitlen], bitlen, alphabet[i]);
            next_code[bitlen] += 1;
        }
    }
    t
}

const CODE_LENGTH_CODES_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

fn decode_trees(
    r: &mut BitReader,
) -> Result<(HuffmanTree, HuffmanTree), String> {
    // The number of literal/length codes
    let hlit = r.read_bits(5)? + 257;

    // The number of distance codes
    let hdist = r.read_bits(5)? + 1;

    // The number of code length codes
    let hclen = r.read_bits(4)? + 4;

    // Read code lengths for the code length alphabet
    let mut code_length_tree_bl = vec![0; 19];
    for i in 0..hclen as usize {
        code_length_tree_bl[CODE_LENGTH_CODES_ORDER[i]] =
            r.read_bits(3)? as usize;
    }

    // Construct code length tree
    let code_length_alphabet: Vec<u32> = (0..19).collect();
    let code_length_tree =
        bl_list_to_tree(&code_length_tree_bl, &code_length_alphabet);

    // Read literal/length + distance code length list
    let mut bl = Vec::new();
    while bl.len() < (hlit + hdist) as usize {
        let sym = decode_symbol(r, &code_length_tree)?;
        if sym <= 15 {
            // literal value
            bl.push(sym as usize);
        } else if sym == 16 {
            // copy the previous code length 3..6 times.
            // the next 2 bits indicate repeat length ( 0 = 3, ..., 3 = 6 )
            if bl.is_empty() {
                return Err("No previous code length".to_string());
            }
            let prev_code_length = bl[bl.len() - 1];
            let repeat_length = r.read_bits(2)? + 3;
            for _ in 0..repeat_length {
                bl.push(prev_code_length);
            }
        } else if sym == 17 {
            // repeat code length 0 for 3..10 times. (3 bits of length)
            let repeat_length = r.read_bits(3)? + 3;
            bl.resize(bl.len() + repeat_length as usize, 0);
        } else if sym == 18 {
            // repeat code length 0 for 11..138 times. (7 bits of length)
            let repeat_length = r.read_bits(7)? + 11;
            bl.resize(bl.len() + repeat_length as usize, 0);
        } else {
            return Err("Invalid symbol".to_string());
        }
    }

    // Construct trees
    let literal_length_alphabet: Vec<u32> = (0..286).collect();
    let literal_length_tree =
        bl_list_to_tree(&bl[..hlit as usize], &literal_length_alphabet);

    let distance_alphabet: Vec<u32> = (0..30).collect();
    let distance_tree =
        bl_list_to_tree(&bl[hlit as usize..], &distance_alphabet);

    Ok((literal_length_tree, distance_tree))
}

fn inflate_block_dynamic(
    r: &mut BitReader,
    o: &mut Vec<u8>,
) -> Result<(), String> {
    let (literal_length_tree, distance_tree) = decode_trees(r)?;
    inflate_block_data(r, &literal_length_tree, &distance_tree, o)
}

fn inflate_block_fixed(
    r: &mut BitReader,
    o: &mut Vec<u8>,
) -> Result<(), String> {
    let mut bl = Vec::new();
    bl.extend(vec![8; 144]); // 0-143: 8 bits
    bl.extend(vec![9; 112]); // 144-255: 9 bits
    bl.extend(vec![7; 24]); // 256-279: 7 bits
    bl.extend(vec![8; 8]); // 280-287: 8 bits

    let literal_length_alphabet: Vec<u32> = (0..286).collect();
    let literal_length_tree = bl_list_to_tree(&bl, &literal_length_alphabet);

    let bl_dist = vec![5; 30];
    let distance_alphabet: Vec<u32> = (0..30).collect();
    let distance_tree = bl_list_to_tree(&bl_dist, &distance_alphabet);

    inflate_block_data(r, &literal_length_tree, &distance_tree, o)
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_bitreader_read_bits() {
        let data = vec![0b11010010, 0b10110101];
        let mut reader = BitReader::new(data);

        assert_eq!(reader.read_bits(3).unwrap(), 0b010); // First 3 bits: 010
        assert_eq!(reader.read_bits(5).unwrap(), 0b11010); // Next 5 bits: 11010
    }

    #[test]
    fn test_bitreader_read_bytes() {
        let data = vec![0x12, 0x34, 0x56, 0x78];
        let mut reader = BitReader::new(data);

        assert_eq!(reader.read_bytes(2).unwrap(), 0x3412); // Little-endian: 0x3412
        assert_eq!(reader.read_bytes(2).unwrap(), 0x7856); // Little-endian: 0x7856
    }

    #[test]
    fn test_extract_header_success() {
        let _data = [120, 156]; // Valid zlib header
        let result = deflate(&[120, 156, 3, 0, 0, 0, 0, 1]); // Minimal valid zlib stream
        assert!(result.is_ok());
    }

    #[test]
    fn test_extract_header_fail_to_short() {
        let data = vec![120]; // Too short
        let mut reader = BitReader::new(data);
        let result = reader.read_byte();
        assert!(result.is_ok());
        let result = reader.read_byte();
        assert!(result.is_err());
    }

    #[test]
    fn test_deflate() {
        // Test data from the original test - this should work with our new implementation
        let data = vec![
            120, 156, 203, 72, 205, 201, 201, 87, 8, 207, 47, 202, 73, 1, 0,
            24, 11, 4, 93,
        ];
        let result = deflate(&data);
        assert!(result.is_ok());
        let decompressed = result.unwrap();
        assert_eq!(decompressed, b"hello World");
    }

    #[test]
    fn test_deflate2() {
        let data = vec![
            120, 156, 99, 103, 96, 96, 72, 201, 79, 201, 76, 79, 204, 203, 213,
            158, 98, 194, 4, 228, 50, 250, 3, 9, 7, 135, 162, 156, 148, 194,
            188, 152, 228, 252, 220, 130, 156, 212, 146, 212, 24, 231, 196,
            188, 228, 204, 252, 188, 212, 226, 152, 144, 162, 210, 226, 226,
            212, 28, 93, 67, 75, 115, 75, 93, 119, 160, 144, 130, 91, 126, 145,
            66, 72, 70, 170, 66, 120, 106, 106, 118, 106, 94, 138, 174, 161,
            89, 82, 102, 137, 174, 137, 137, 142, 161, 119, 70, 149, 94, 90,
            78, 98, 114, 203, 175, 243, 32, 163, 193, 128, 25, 100, 7, 16, 23,
            0, 9, 22, 32, 237, 178, 134, 129, 129, 21, 72, 11, 128, 196, 243,
            176, 217, 29, 156, 153, 151, 158, 147, 90, 12, 54, 95, 193, 216,
            84, 193, 200, 192, 200, 36, 198, 45, 181, 168, 40, 53, 57, 91, 193,
            37, 177, 60, 79, 71, 193, 55, 177, 44, 181, 40, 19, 200, 13, 78,
            76, 42, 74, 85, 80, 83, 240, 75, 45, 7, 10, 38, 103, 100, 2, 221,
            167, 139, 238, 66, 5, 13, 144, 17, 154, 96, 167, 173, 228, 215, 98,
            68, 119, 218, 74, 6, 76, 167, 49, 60, 153, 202, 200, 160, 199, 128,
            0, 0, 161, 99, 76, 142,
        ];
        let expect = vec![
            7, 0, 0, 0, 100, 111, 100, 105, 103, 97, 110, 109, 43, 148, 52, 2,
            0, 0, 0, 1, 79, 0, 0, 0, 64, 64, 114, 108, 100, 113, 110, 92, 99,
            111, 109, 112, 108, 101, 116, 101, 92, 67, 97, 110, 99, 105, 111,
            110, 101, 115, 92, 84, 114, 117, 115, 115, 101, 108, 45, 49, 57,
            55, 57, 45, 71, 111, 110, 101, 32, 70, 111, 114, 32, 84, 104, 101,
            32, 87, 101, 101, 107, 101, 110, 100, 45, 49, 54, 98, 105, 116, 45,
            52, 52, 44, 49, 75, 104, 122, 46, 102, 108, 97, 99, 132, 250, 207,
            2, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, 1, 0, 0, 0, 112, 1, 0, 0, 4,
            0, 0, 0, 68, 172, 0, 0, 5, 0, 0, 0, 16, 0, 0, 0, 1, 110, 0, 0, 0,
            64, 64, 114, 108, 100, 113, 110, 92, 99, 111, 109, 112, 108, 101,
            116, 101, 92, 83, 105, 110, 103, 108, 101, 115, 32, 87, 101, 101,
            107, 32, 51, 53, 32, 50, 48, 50, 52, 92, 70, 101, 114, 114, 101,
            99, 107, 32, 68, 97, 119, 110, 44, 32, 77, 97, 118, 101, 114, 105,
            99, 107, 32, 83, 97, 98, 114, 101, 32, 38, 32, 78, 101, 119, 32,
            77, 97, 99, 104, 105, 110, 101, 32, 45, 32, 70, 111, 114, 32, 84,
            104, 101, 32, 87, 101, 101, 107, 101, 110, 100, 32, 40, 50, 48, 50,
            52, 41, 46, 102, 108, 97, 99, 169, 15, 42, 1, 0, 0, 0, 0, 0, 0, 0,
            0, 3, 0, 0, 0, 1, 0, 0, 0, 169, 0, 0, 0, 4, 0, 0, 0, 68, 172, 0, 0,
            5, 0, 0, 0, 16, 0, 0, 0, 0, 228, 149, 1, 0, 46, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0,
        ]
        .to_vec();
        let result = deflate(&data);

        assert!(result.is_ok());
        let decompressed = result.unwrap();

        assert_eq!(decompressed, expect);
    }
}
