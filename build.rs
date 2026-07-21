use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::Path;

fn main() {
    println!("cargo::rerun-if-changed=assets/lucide.ttf");

    let font_data = fs::read("assets/lucide.ttf").expect("failed to read lucide.ttf");
    let icons = extract_icon_map(&font_data);

    let out_dir = env::var("OUT_DIR").unwrap();
    let dest = Path::new(&out_dir).join("icon_map.rs");

    let mut code = String::new();
    code.push_str(&format!(
        "static ICON_MAP: [(&str, char); {}] = [\n",
        icons.len()
    ));
    for (name, codepoint) in &icons {
        code.push_str(&format!("    (\"{name}\", '\\u{{{codepoint:04X}}}'),\n"));
    }
    code.push_str("];\n");

    fs::write(dest, code).expect("failed to write icon_map.rs");
}

fn extract_icon_map(data: &[u8]) -> BTreeMap<String, u32> {
    let num_tables = u16::from_be_bytes([data[4], data[5]]) as usize;

    let mut tables: std::collections::HashMap<[u8; 4], (usize, usize)> =
        std::collections::HashMap::new();
    for i in 0..num_tables {
        let offset = 12 + i * 16;
        let tag: [u8; 4] = data[offset..offset + 4].try_into().unwrap();
        let tbl_offset =
            u32::from_be_bytes(data[offset + 8..offset + 12].try_into().unwrap()) as usize;
        let length =
            u32::from_be_bytes(data[offset + 12..offset + 16].try_into().unwrap()) as usize;
        tables.insert(tag, (tbl_offset, length));
    }

    let glyph_names = parse_post_table(data, &tables);
    let gid_to_codepoint = parse_cmap_table(data, &tables);

    let mut result = BTreeMap::new();
    for (gid, name) in &glyph_names {
        if let Some(&cp) = gid_to_codepoint.get(gid) {
            if cp >= 0xE000 && !name.is_empty() {
                result.insert(name.clone(), cp);
            }
        }
    }
    result
}

fn parse_post_table(
    data: &[u8],
    tables: &std::collections::HashMap<[u8; 4], (usize, usize)>,
) -> std::collections::HashMap<u16, String> {
    let mut glyph_names = std::collections::HashMap::new();
    let (off, length) = match tables.get(b"post") {
        Some(v) => *v,
        None => return glyph_names,
    };

    let version = u32::from_be_bytes(data[off..off + 4].try_into().unwrap());
    if version != 0x00020000 {
        return glyph_names;
    }

    let num_glyphs = u16::from_be_bytes(data[off + 32..off + 34].try_into().unwrap()) as usize;
    let mut indices = Vec::with_capacity(num_glyphs);
    for j in 0..num_glyphs {
        let idx = u16::from_be_bytes(
            data[off + 34 + j * 2..off + 34 + j * 2 + 2]
                .try_into()
                .unwrap(),
        );
        indices.push(idx);
    }

    let str_start = off + 34 + num_glyphs * 2;
    let mut names = Vec::new();
    let mut pos = str_start;
    while pos < off + length {
        let name_len = data[pos] as usize;
        let name = String::from_utf8_lossy(&data[pos + 1..pos + 1 + name_len]).into_owned();
        names.push(name);
        pos += 1 + name_len;
    }

    for (gid, &idx) in indices.iter().enumerate() {
        if idx >= 258 {
            let name_idx = (idx - 258) as usize;
            if name_idx < names.len() {
                glyph_names.insert(gid as u16, names[name_idx].clone());
            }
        }
    }

    glyph_names
}

fn parse_cmap_table(
    data: &[u8],
    tables: &std::collections::HashMap<[u8; 4], (usize, usize)>,
) -> std::collections::HashMap<u16, u32> {
    let mut gid_to_cp = std::collections::HashMap::new();
    let (off, _) = match tables.get(b"cmap") {
        Some(v) => *v,
        None => return gid_to_cp,
    };

    let num_subtables =
        u16::from_be_bytes(data[off + 2..off + 4].try_into().unwrap()) as usize;

    for i in 0..num_subtables {
        let so = off + 4 + i * 8;
        let sub_offset =
            u32::from_be_bytes(data[so + 4..so + 8].try_into().unwrap()) as usize;
        let subtable_off = off + sub_offset;
        let fmt = u16::from_be_bytes(data[subtable_off..subtable_off + 2].try_into().unwrap());

        if fmt == 12 {
            let num_groups = u32::from_be_bytes(
                data[subtable_off + 12..subtable_off + 16].try_into().unwrap(),
            ) as usize;
            for g in 0..num_groups {
                let go = subtable_off + 16 + g * 12;
                let start_char =
                    u32::from_be_bytes(data[go..go + 4].try_into().unwrap());
                let end_char =
                    u32::from_be_bytes(data[go + 4..go + 8].try_into().unwrap());
                let start_glyph =
                    u32::from_be_bytes(data[go + 8..go + 12].try_into().unwrap());
                for c in start_char..=end_char {
                    let gid = start_glyph + (c - start_char);
                    gid_to_cp.insert(gid as u16, c);
                }
            }
            break;
        }
    }

    gid_to_cp
}
