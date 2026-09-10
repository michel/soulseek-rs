//https://www.rfc-editor.org/rfc/rfc1950

struct BitReader {
    mem: Vec<u8>,
    pos: usize,
    b: u8,
    numbits: i32,
}

impl BitReader {
    const fn new(mem: Vec<u8>) -> Self {
        Self {
            mem,
            pos: 0,
            b: 0,
            numbits: 0,
        }
    }

    fn read_byte(&mut self) -> std::result::Result<u8, String> {
        self.numbits = 0; // discard unread bits
        if self.pos >= self.mem.len() {
            return Err("End of data".to_string());
        }
        let b = self.mem[self.pos];
        self.pos += 1;
        Ok(b)
    }

    fn read_bit(&mut self) -> std::result::Result<u8, String> {
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

    fn read_bits(&mut self, n: usize) -> std::result::Result<u32, String> {
        let mut o = 0u32;
        for i in 0..n {
            o |= u32::from(self.read_bit()?) << i;
        }
        Ok(o)
    }

    fn read_bytes(&mut self, n: usize) -> std::result::Result<u32, String> {
        // read bytes as an integer in little-endian
        let mut o = 0u32;
        for i in 0..n {
            o |= u32::from(self.read_byte()?) << (8 * i);
        }
        Ok(o)
    }
}

use crate::error::{Result, SoulseekRs};

/// The most a peer payload may inflate to unless the caller says otherwise.
///
/// Nicotine+ passes this ceiling to its decompressor for a search or folder
/// reply, and a larger one only for a browse listing.
pub const MAX_INFLATED: usize = 128 * 1024 * 1024;

/// The ceiling for a whole browse listing, which a large sharer legitimately
/// fills. The same number Nicotine+ allows.
pub const MAX_INFLATED_LISTING: usize = 2 * 1024 * 1024 * 1024;

/// What a payload that outgrows its ceiling is refused with.
const TOO_BIG: &str = "compressed payload expands past its limit";

/// Inflate a zlib stream, checking its adler32 trailer.
///
/// Bounded by [`MAX_INFLATED`]; use [`inflate_limited`] for a payload with a
/// different ceiling.
pub fn inflate(input: &[u8]) -> Result<Vec<u8>> {
    inflate_limited(input, MAX_INFLATED)
}

/// Inflate a zlib stream, refusing to grow the output past `max_out`.
///
/// The ceiling is the point: deflate reaches roughly 1000:1, so a few
/// kilobytes from a peer we have never met can otherwise expand into
/// gigabytes and take the process with it.
pub fn inflate_limited(input: &[u8], max_out: usize) -> Result<Vec<u8>> {
    let mut r = BitReader::new(input.to_vec());
    let cmf = r.read_byte()?;
    let cm = cmf & 15; // Compression method
    if cm != 8 {
        // only CM=8 is supported
        return Err(SoulseekRs::CompressionError("invalid CM".to_string()));
    }
    let cinfo = (cmf >> 4) & 15; // Compression info
    if cinfo > 7 {
        return Err(SoulseekRs::CompressionError("invalid CINFO".to_string()));
    }
    let flg = r.read_byte()?;
    if !(u32::from(cmf) * 256 + u32::from(flg)).is_multiple_of(31) {
        return Err(SoulseekRs::CompressionError(
            "CMF+FLG checksum failed".to_string(),
        ));
    }
    let fdict = (flg >> 5) & 1; // preset dictionary?
    if fdict != 0 {
        return Err(SoulseekRs::CompressionError(
            "preset dictionary not supported".to_string(),
        ));
    }
    let out = inflate_blocks(&mut r, max_out)
        .map_err(SoulseekRs::CompressionError)?;
    // `read_bytes` assembles little-endian; the zlib trailer is big-endian.
    if r.read_bytes(4)?.swap_bytes() != adler32(&out) {
        return Err(SoulseekRs::CompressionError(
            "adler32 checksum failed".to_string(),
        ));
    }
    Ok(out)
}

/// RFC 1950 adler32 checksum.
fn adler32(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in data {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn inflate_blocks(
    r: &mut BitReader,
    max_out: usize,
) -> std::result::Result<Vec<u8>, String> {
    let mut bfinal = 0;
    let mut out = Vec::new();
    while bfinal == 0 {
        bfinal = r.read_bit()?;
        let btype = r.read_bits(2)?;
        match btype {
            0 => inflate_block_no_compression(r, &mut out, max_out)?,
            1 => inflate_block_fixed(r, &mut out, max_out)?,
            2 => inflate_block_dynamic(r, &mut out, max_out)?,
            _ => return Err("invalid BTYPE".to_string()),
        }
        if out.len() > max_out {
            return Err(TOO_BIG.to_string());
        }
    }
    Ok(out)
}

fn inflate_block_no_compression(
    r: &mut BitReader,
    o: &mut Vec<u8>,
    max_out: usize,
) -> std::result::Result<(), String> {
    let len = r.read_bytes(2)?;
    let _nlen = r.read_bytes(2)?;
    if o.len().saturating_add(len as usize) > max_out {
        return Err(TOO_BIG.to_string());
    }
    for _ in 0..len {
        o.push(r.read_byte()?);
    }
    Ok(())
}

#[derive(Clone)]
struct Node {
    symbol: Option<u32>,
    left: Option<Box<Self>>,
    right: Option<Box<Self>>,
}

impl Node {
    const fn new() -> Self {
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
    const fn new() -> Self {
        Self { root: Node::new() }
    }

    fn insert(&mut self, codeword: u32, n: usize, symbol: u32) {
        // Insert an entry into the tree mapping `codeword` of len `n` to `symbol`
        let mut node = &mut self.root;
        for i in (0..n).rev() {
            let b = (codeword >> i) & 1;
            let child = if b != 0 {
                &mut node.right
            } else {
                &mut node.left
            };
            node = child.get_or_insert_with(|| Box::new(Node::new()));
        }
        node.symbol = Some(symbol);
    }
}

fn decode_symbol(
    r: &mut BitReader,
    t: &HuffmanTree,
) -> std::result::Result<u32, String> {
    let mut node = &t.root;
    while node.left.is_some() || node.right.is_some() {
        let b = r.read_bit()?;
        let next = if b != 0 { &node.right } else { &node.left };
        // A degenerate/incomplete Huffman code (attacker-controlled) can point
        // at a missing child; error out instead of unwrap-panicking.
        node = next
            .as_ref()
            .ok_or_else(|| "Invalid Huffman code".to_string())?;
    }
    node.symbol.ok_or_else(|| "No symbol found".to_string())
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
    max_out: usize,
) -> std::result::Result<(), String> {
    loop {
        // Checked per symbol, not per block: a single block can emit output
        // without bound, which is exactly how a bomb is built.
        if out.len() > max_out {
            return Err(TOO_BIG.to_string());
        }
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
) -> std::result::Result<(HuffmanTree, HuffmanTree), String> {
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
    max_out: usize,
) -> std::result::Result<(), String> {
    let (literal_length_tree, distance_tree) = decode_trees(r)?;
    inflate_block_data(r, &literal_length_tree, &distance_tree, o, max_out)
}

fn inflate_block_fixed(
    r: &mut BitReader,
    o: &mut Vec<u8>,
    max_out: usize,
) -> std::result::Result<(), String> {
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

    inflate_block_data(r, &literal_length_tree, &distance_tree, o, max_out)
}

struct BitWriter {
    out: Vec<u8>,
    bit: u32,
    held: u32,
}

impl BitWriter {
    fn new() -> Self {
        Self {
            out: vec![0x78, 0x01],
            bit: 0,
            held: 0,
        }
    }

    /// Extra bits, least significant first.
    fn bits(&mut self, value: u32, count: u32) {
        self.held |= value << self.bit;
        self.bit += count;
        while self.bit >= 8 {
            self.out.push((self.held & 0xFF) as u8);
            self.held >>= 8;
            self.bit -= 8;
        }
    }

    /// A Huffman code, most significant bit first.
    fn code(&mut self, value: u32, count: u32) {
        for i in (0..count).rev() {
            self.bits((value >> i) & 1, 1);
        }
    }

    fn finish(mut self, trailer: u32) -> Vec<u8> {
        if self.bit > 0 {
            self.out.push((self.held & 0xFF) as u8);
        }
        self.out.extend_from_slice(&trailer.to_be_bytes());
        self.out
    }
}

const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = 258;
const WINDOW: usize = 32_768;
const MATCH_TRIES: usize = 128;
/// A match this long is good enough to stop walking the chain.
const NICE_MATCH: usize = 64;

/// Hash table size for `len` bytes: a search reply of a few hundred bytes
/// must not zero 256 KiB of table, a listing of megabytes wants all of it.
fn hash_bits(len: usize) -> u32 {
    len.max(2).next_power_of_two().trailing_zeros().clamp(8, 15)
}

/// The fixed Huffman code for a literal or length symbol (RFC 1951 3.2.6).
fn symbol(w: &mut BitWriter, symbol: usize) {
    let symbol = symbol as u32;
    match symbol {
        0..=143 => w.code(0x30 + symbol, 8),
        144..=255 => w.code(0x190 + symbol - 144, 9),
        256..=279 => w.code(symbol - 256, 7),
        _ => w.code(0xC0 + symbol - 280, 8),
    }
}

fn emit_match(w: &mut BitWriter, length: usize, distance: usize) {
    let l = LENGTH_BASE
        .iter()
        .rposition(|&base| base as usize <= length)
        .unwrap_or(0);
    symbol(w, 257 + l);
    w.bits(length as u32 - LENGTH_BASE[l], LENGTH_EXTRA_BITS[l] as u32);
    let d = DISTANCE_BASE
        .iter()
        .rposition(|&base| base as usize <= distance)
        .unwrap_or(0);
    w.code(d as u32, 5);
    w.bits(
        distance as u32 - DISTANCE_BASE[d],
        DISTANCE_EXTRA_BITS[d] as u32,
    );
}

fn hash(data: &[u8], at: usize, bits: u32) -> usize {
    let three = u32::from(data[at]) << 16
        | u32::from(data[at + 1]) << 8
        | u32::from(data[at + 2]);
    (three.wrapping_mul(2_654_435_761) >> (32 - bits)) as usize
}

/// Chain the three bytes at `at` into the match finder.
fn remember(head: &mut [usize], prev: &mut [usize], data: &[u8], at: usize) {
    if at + MIN_MATCH > data.len() {
        return;
    }
    let h = hash(data, at, head.len().trailing_zeros());
    prev[at] = head[h];
    head[h] = at;
}

/// Deflate `data` into a zlib stream.
///
/// One fixed-Huffman block over an LZ77 hash-chain match finder, which is
/// what shrinks a listing's repeated path prefixes. ponytail: fixed codes, not
/// dynamic; listings compress 5-10x either way and this is a tenth of the code.
#[must_use]
pub fn deflate(data: &[u8]) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.bits(1, 1); // final block
    w.bits(1, 2); // fixed Huffman codes
    let bits = hash_bits(data.len());
    let mut head = vec![usize::MAX; 1 << bits];
    let mut prev = vec![usize::MAX; data.len()];
    let mut i = 0;
    while i < data.len() {
        let mut best = (0, 0);
        if i + MIN_MATCH <= data.len() {
            let h = hash(data, i, bits);
            let mut candidate = head[h];
            let mut tries = MATCH_TRIES;
            let max = MAX_MATCH.min(data.len() - i);
            while candidate != usize::MAX
                && tries > 0
                && i - candidate <= WINDOW
            {
                let len = (0..max)
                    .take_while(|&k| data[candidate + k] == data[i + k])
                    .count();
                if len > best.0 {
                    best = (len, i - candidate);
                    if len == max || len >= NICE_MATCH {
                        break;
                    }
                }
                candidate = prev[candidate];
                tries -= 1;
            }
        }
        remember(&mut head, &mut prev, data, i);
        let (length, distance) = best;
        if length < MIN_MATCH {
            symbol(&mut w, usize::from(data[i]));
            i += 1;
            continue;
        }
        emit_match(&mut w, length, distance);
        for at in i + 1..i + length {
            remember(&mut head, &mut prev, data, at);
        }
        i += length;
    }
    symbol(&mut w, 256); // end of block
    w.finish(adler32(data))
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn a_payload_that_expands_past_its_ceiling_is_refused() {
        // Deflate reaches ~1000:1, so a few kilobytes from an unknown peer can
        // become gigabytes. The ceiling is what stops that being our problem.
        let bomb = deflate(&vec![0u8; 1024 * 1024]);
        assert!(
            bomb.len() < 8 * 1024,
            "the test needs a payload that expands hugely, got {} bytes",
            bomb.len()
        );

        let error = inflate_limited(&bomb, 64 * 1024)
            .expect_err("a payload past the ceiling must be refused");
        assert!(
            format!("{error}").contains("expands past its limit"),
            "unexpected error: {error}"
        );

        // The same bytes are fine when they fit.
        assert_eq!(
            inflate_limited(&bomb, 2 * 1024 * 1024).unwrap().len(),
            1024 * 1024
        );
    }

    #[test]
    fn a_corrupt_adler32_trailer_is_rejected() {
        let mut stream = deflate(b"the bytes a peer sent");
        let last = stream.len() - 1;
        stream[last] ^= 0xFF;
        assert!(inflate(&stream).is_err(), "a bad checksum must not pass");
    }

    #[test]
    fn a_reference_stream_inflates() {
        let stream = [
            120, 156, 203, 72, 205, 201, 201, 87, 8, 207, 47, 202, 73, 1, 0,
            25, 107, 4, 61,
        ];
        assert_eq!(inflate(&stream).unwrap(), b"hello World");
    }

    fn listing() -> Vec<u8> {
        (0..3000)
            .flat_map(|n| {
                format!("Artist - Album - {n:04} - Track Title.flac")
                    .into_bytes()
            })
            .collect()
    }

    #[test]
    fn every_shape_round_trips() {
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut noise = |n: usize| -> Vec<u8> {
            (0..n)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    (seed >> 24) as u8
                })
                .collect()
        };
        let cases = [
            Vec::new(),
            vec![0],
            vec![0x41; 258],
            vec![0x41; 70_000],
            vec![0x5A; 100_000],
            b"the same words the same words the same words".to_vec(),
            listing(),
            noise(1),
            noise(4096),
            noise(70_000),
        ];
        for data in cases {
            assert_eq!(
                inflate(&deflate(&data)).unwrap(),
                data,
                "{}",
                data.len()
            );
        }
    }

    #[test]
    fn a_repetitive_listing_shrinks() {
        let data = listing();
        assert!(deflate(&data).len() * 4 < data.len());
    }

    #[test]
    fn decode_symbol_on_degenerate_tree_errors_instead_of_panicking() {
        // A malformed Huffman table from an untrusted peer can produce a node
        // with only one child. Inserting codeword "0" gives the root a left
        // child but no right child; decoding a '1' bit must error rather than
        // unwrap-panic.
        let mut tree = HuffmanTree::new();
        tree.insert(0, 1, 42);
        let mut reader = BitReader::new(vec![0b0000_0001]); // first bit = 1
        assert!(decode_symbol(&mut reader, &tree).is_err());
    }

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
    fn test_extract_header_fail_to_short() {
        let data = vec![120]; // Too short
        let mut reader = BitReader::new(data);
        let result = reader.read_byte();
        assert!(result.is_ok());
        let result = reader.read_byte();
        assert!(result.is_err());
    }

    #[test]
    fn a_dynamic_huffman_stream_from_a_real_peer_inflates() {
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
        let result = inflate(&data);

        assert!(result.is_ok());
        let decompressed = result.unwrap();

        assert_eq!(decompressed, expect);
    }
}
