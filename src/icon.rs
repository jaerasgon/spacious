//! The app icon: a quadtree. A square split in four, one quarter split again, and one of
//! those split once more, drawn as blocks with gaps on a dark macOS-style tile.

/// Leaf cells in unit coordinates (x0, y0, x1, y1) and their colour.
fn leaves() -> Vec<([f32; 4], [u8; 3])> {
    const BLUE: [u8; 3] = [0x2D, 0x8C, 0xEB];
    const G1: [u8; 3] = [0x5A, 0x5A, 0x62];
    const G2: [u8; 3] = [0x46, 0x46, 0x4E];
    const G3: [u8; 3] = [0x3A, 0x3A, 0x42];
    vec![
        // Top-left quarter: one big block.
        ([0.0, 0.0, 0.5, 0.5], BLUE),
        // Bottom-left and bottom-right quarters.
        ([0.0, 0.5, 0.5, 1.0], G1),
        ([0.5, 0.5, 1.0, 1.0], G2),
        // Top-right quarter split in four.
        ([0.5, 0.0, 0.75, 0.25], G2),
        ([0.75, 0.0, 1.0, 0.25], G3),
        ([0.75, 0.25, 1.0, 0.5], G2),
        // ... and its bottom-left split once more.
        ([0.5, 0.25, 0.625, 0.375], G1),
        ([0.625, 0.25, 0.75, 0.375], G3),
        ([0.5, 0.375, 0.625, 0.5], G3),
        ([0.625, 0.375, 0.75, 0.5], G1),
    ]
}

fn in_rrect(x: f32, y: f32, r: [f32; 4], rad: f32) -> bool {
    if x < r[0] || y < r[1] || x > r[2] || y > r[3] {
        return false;
    }
    let cx = x.clamp(r[0] + rad, r[2] - rad);
    let cy = y.clamp(r[1] + rad, r[3] - rad);
    let (dx, dy) = (x - cx, y - cy);
    dx * dx + dy * dy <= rad * rad
}

/// Square RGBA image of the icon, `size` pixels a side.
pub fn rgba(size: u32) -> Vec<u8> {
    let s = size as f32;
    // macOS icon grid: the tile is 824/1024 of the canvas, corner radius about 22.5%.
    let tile_inset = s * 100.0 / 1024.0;
    let tile = [tile_inset, tile_inset, s - tile_inset, s - tile_inset];
    let tile_r = (tile[2] - tile[0]) * 0.225;
    let grid_inset = s * 250.0 / 1024.0;
    let g0 = grid_inset;
    let gs = s - 2.0 * grid_inset;
    let gap = (gs * 0.035).max(1.0);
    let cell_r = (gs * 0.02).max(0.5);
    let cells: Vec<([f32; 4], [u8; 3])> = leaves()
        .into_iter()
        .map(|(u, c)| {
            (
                [g0 + u[0] * gs + gap / 2.0, g0 + u[1] * gs + gap / 2.0, g0 + u[2] * gs - gap / 2.0, g0 + u[3] * gs - gap / 2.0],
                c,
            )
        })
        .collect();

    const SS: u32 = 4; // supersampling per axis
    let mut out = vec![0u8; (size * size * 4) as usize];
    for py in 0..size {
        for px in 0..size {
            let (mut r, mut g, mut b, mut a) = (0f32, 0f32, 0f32, 0f32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = px as f32 + (sx as f32 + 0.5) / SS as f32;
                    let y = py as f32 + (sy as f32 + 0.5) / SS as f32;
                    if !in_rrect(x, y, tile, tile_r) {
                        continue;
                    }
                    let mut col = [34.0, 34.0, 34.0]; // flat #222222 tile
                    for (cr, c) in &cells {
                        if in_rrect(x, y, *cr, cell_r) {
                            col = [c[0] as f32, c[1] as f32, c[2] as f32];
                            break;
                        }
                    }
                    r += col[0];
                    g += col[1];
                    b += col[2];
                    a += 1.0;
                }
            }
            let i = ((py * size + px) * 4) as usize;
            if a > 0.0 {
                out[i] = (r / a).round() as u8;
                out[i + 1] = (g / a).round() as u8;
                out[i + 2] = (b / a).round() as u8;
                out[i + 3] = (a / (SS * SS) as f32 * 255.0).round() as u8;
            }
        }
    }
    out
}

/// Minimal PNG encoder (stored deflate blocks), enough for writing the icon set.
pub fn png(size: u32, rgba: &[u8]) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut c = 0xFFFF_FFFFu32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = out.len();
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let crc = crc32(&out[start..]);
        out.extend_from_slice(&crc.to_be_bytes());
    }
    let mut raw = Vec::with_capacity((size * (size * 4 + 1)) as usize);
    for row in rgba.chunks((size * 4) as usize) {
        raw.push(0); // filter: none
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (k, blk) in blocks.iter().enumerate() {
        z.push((k + 1 == blocks.len()) as u8);
        let len = blk.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(blk);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Write the PNGs `iconutil` needs into `dir` (an .iconset folder).
pub fn write_iconset(dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for base in [16u32, 32, 128, 256, 512] {
        for scale in [1u32, 2] {
            let px = base * scale;
            let name = if scale == 1 { format!("icon_{base}x{base}.png") } else { format!("icon_{base}x{base}@2x.png") };
            std::fs::write(dir.join(name), png(px, &rgba(px)))?;
        }
    }
    Ok(())
}
