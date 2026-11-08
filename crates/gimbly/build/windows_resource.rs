//! Builds the Windows icon resource as a COFF object so no external resource
//! compiler (`windres`, `rc`) is needed. The linker merges the `.rsrc$01` and
//! `.rsrc$02` sections into the executable's `.rsrc` section.

use object::write::{Object, Relocation};
use object::{
    Architecture, BinaryFormat, Endianness, RelocationEncoding, RelocationFlags, RelocationKind,
    SectionKind,
};
use std::error::Error;

const RT_ICON: u32 = 3;
const RT_GROUP_ICON: u32 = 14;
const SUBDIRECTORY: u32 = 0x8000_0000;
const DIRECTORY_SIZE: usize = 16;
const ENTRY_SIZE: usize = 8;
const DATA_ENTRY_SIZE: usize = 16;

pub struct Icon {
    /// Width and height in pixels; icons are square.
    pub size: u32,
    pub png: Vec<u8>,
}

/// A COFF object holding `icons` as RT_ICON resources plus one RT_GROUP_ICON
/// (id 1) that lists them all.
pub fn icon_object(arch: &str, icons: &[Icon]) -> Result<Vec<u8>, Box<dyn Error>> {
    let architecture = match arch {
        "x86_64" => Architecture::X86_64,
        "x86" => Architecture::I386,
        "aarch64" => Architecture::Aarch64,
        other => return Err(format!("icon resources are not supported on {other}").into()),
    };

    // Raw resource data: every icon image, then the group directory.
    let mut data = Vec::new();
    let mut blobs = Vec::new();
    for icon in icons {
        blobs.push(append_aligned(&mut data, &icon.png));
    }
    blobs.push(append_aligned(&mut data, &group_directory(icons, &blobs)));

    // Directory tree: root -> type -> id -> language -> data entry.
    // Offsets are relative to the start of `.rsrc$01`.
    let count = icons.len();
    let leaves = count + 1;
    let icon_dir = DIRECTORY_SIZE + ENTRY_SIZE * 2;
    let group_dir = icon_dir + DIRECTORY_SIZE + ENTRY_SIZE * count;
    let language_dirs = group_dir + DIRECTORY_SIZE + ENTRY_SIZE;
    let language_dir = |leaf: usize| language_dirs + (DIRECTORY_SIZE + ENTRY_SIZE) * leaf;
    let data_entries = language_dir(leaves);
    let data_entry = |leaf: usize| data_entries + DATA_ENTRY_SIZE * leaf;

    let mut tree = Vec::new();
    directory(&mut tree, 2);
    entry(&mut tree, RT_ICON, SUBDIRECTORY | icon_dir as u32);
    entry(&mut tree, RT_GROUP_ICON, SUBDIRECTORY | group_dir as u32);
    directory(&mut tree, count);
    for leaf in 0..count {
        entry(
            &mut tree,
            leaf as u32 + 1,
            SUBDIRECTORY | language_dir(leaf) as u32,
        );
    }
    directory(&mut tree, 1);
    entry(&mut tree, 1, SUBDIRECTORY | language_dir(count) as u32);
    for leaf in 0..leaves {
        directory(&mut tree, 1);
        // Language 0 is neutral.
        entry(&mut tree, 0, data_entry(leaf) as u32);
    }
    for &(_, size) in &blobs {
        // The RVA is filled in by a relocation below.
        tree.extend_from_slice(&[0; 4]);
        tree.extend_from_slice(&size.to_le_bytes());
        tree.extend_from_slice(&[0; 8]);
    }
    debug_assert_eq!(tree.len(), data_entry(leaves));

    let mut object = Object::new(BinaryFormat::Coff, architecture, Endianness::Little);
    let tree_section =
        object.add_section(Vec::new(), b".rsrc$01".to_vec(), SectionKind::ReadOnlyData);
    let data_section =
        object.add_section(Vec::new(), b".rsrc$02".to_vec(), SectionKind::ReadOnlyData);
    object.append_section_data(tree_section, &tree, 4);
    object.append_section_data(data_section, &data, 4);
    let data_symbol = object.section_symbol(data_section);
    for (leaf, &(offset, _)) in blobs.iter().enumerate() {
        object
            .add_relocation(
                tree_section,
                Relocation {
                    offset: data_entry(leaf) as u64,
                    symbol: data_symbol,
                    // Stored as the implicit addend: the blob's offset in `.rsrc$02`.
                    addend: i64::from(offset),
                    flags: RelocationFlags::Generic {
                        kind: RelocationKind::ImageOffset,
                        encoding: RelocationEncoding::Generic,
                        size: 32,
                    },
                },
            )
            .map_err(|e| e.to_string())?;
    }
    Ok(object.write().map_err(|e| e.to_string())?)
}

/// Appends `bytes` at a 4-byte boundary; returns their offset and length.
fn append_aligned(data: &mut Vec<u8>, bytes: &[u8]) -> (u32, u32) {
    data.resize(data.len().next_multiple_of(4), 0);
    let offset = data.len() as u32;
    data.extend_from_slice(bytes);
    (offset, bytes.len() as u32)
}

/// The RT_GROUP_ICON payload (GRPICONDIR). Icon `i` has resource id `i + 1`.
fn group_directory(icons: &[Icon], blobs: &[(u32, u32)]) -> Vec<u8> {
    let mut group = Vec::new();
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&(icons.len() as u16).to_le_bytes());
    for (i, icon) in icons.iter().enumerate() {
        // A zero byte means 256.
        let side = if icon.size >= 256 { 0 } else { icon.size as u8 };
        group.extend_from_slice(&[side, side, 0, 0]);
        group.extend_from_slice(&1u16.to_le_bytes());
        group.extend_from_slice(&32u16.to_le_bytes());
        group.extend_from_slice(&blobs[i].1.to_le_bytes());
        group.extend_from_slice(&(i as u16 + 1).to_le_bytes());
    }
    group
}

/// A resource directory header announcing `ids` entries identified by number.
fn directory(out: &mut Vec<u8>, ids: usize) {
    out.extend_from_slice(&[0; 12]);
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(ids as u16).to_le_bytes());
}

fn entry(out: &mut Vec<u8>, id: u32, offset: u32) {
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&offset.to_le_bytes());
}
