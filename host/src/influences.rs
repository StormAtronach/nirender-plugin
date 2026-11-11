//! The bone influences of the shapes that a packed vertex cannot hold. The packed vertex of a
//! skinned shape holds only 4 bones whose weights sum to 1. A shape that the game skins in software
//! (MWSE's flag on the geometry) has every influence with its raw weight, as
//! `NiSkinInstance::Deform` 0x6FA000 sums them. Another shape with a vertex of more than 4 has
//! every influence with the weights that sum to 1 (the plugin's `every_influence`).
//!
//! One storage buffer holds the lists of all such geometries, because the vertex shader reads
//! it from one binding. A vertex of such a geometry holds its first entry and its count in the
//! floats of its weights, and [`Influences::put`] moves the first entry by the range of its
//! geometry. The buffer grows when no range has room. The offsets stay valid, and the frame
//! group binds the new buffer. A copy of the bytes stays here for that write, so the growth
//! needs no encoder (`queue.write_buffer` lands before the commands of the frame).

use crate::objects::Create;
use crate::pool;

/// The bytes of an entry of the list: the bone (u32) and its weight (f32).
pub(crate) const ENTRY_BYTES: u64 = 8;
/// The room of the buffer at first: it grows when the game shows such geometries.
const FIRST_BYTES: u64 = 1 << 12;
/// The vertex format of a geometry with a list: this bit on the format of a skinned vertex. It
/// is above the bits of a real format here: the plugin writes no sizes of coordinate sets.
pub(crate) const FVF_INFLUENCES: u32 = 0x4000_0000;
/// The bytes of a skinned vertex before its first weight: the position.
const WEIGHTS_AT: usize = 12;

/// A range of the list, to give back: the offset and the size in bytes.
pub(crate) type ListRange = (u64, u64);

/// The bones of an instance that a list needs: its largest bone and one. The largest can be the
/// matrix with only the place of the shape, which comes with the bones of each instance.
pub(crate) fn bones_needed(list: &[u8]) -> u32 {
    let entry = ENTRY_BYTES as usize;
    (0..list.len() / entry)
        .map(|i| {
            u32::from_le_bytes([list[i * entry], list[i * entry + 1], list[i * entry + 2], list[i * entry + 3]])
                .saturating_add(1)
        })
        .max()
        .unwrap_or(0)
}

/// The first entry of each vertex moved by `first`, for vertices of `stride` bytes.
pub(crate) fn moved_vertices(vertices: &[u8], stride: usize, first: u32) -> Vec<u8> {
    let mut out = vertices.to_vec();
    for vertex in out.chunks_exact_mut(stride) {
        let at: [u8; 4] = vertex[WEIGHTS_AT..WEIGHTS_AT + 4].try_into().expect("four bytes");
        let moved = f32::from_le_bytes(at) + first as f32;
        vertex[WEIGHTS_AT..WEIGHTS_AT + 4].copy_from_slice(&moved.to_le_bytes());
    }
    out
}

/// The room of the list and a copy of its bytes.
struct ListSpace {
    bytes: Vec<u8>,
    free: Vec<(u64, u64)>,
    used: u64,
    grown: u32,
}

impl ListSpace {
    fn new(size: u64) -> ListSpace {
        ListSpace {
            bytes: vec![0; size as usize],
            free: vec![(0, size)],
            used: 0,
            grown: 0,
        }
    }

    /// Takes a range for the entries and copies them there: the range, its first entry as the
    /// vertices name it, and true when the room grew. The old bytes keep their offsets.
    fn put(&mut self, entries: &[u8]) -> (ListRange, u32, bool) {
        let size = (entries.len() as u64).max(ENTRY_BYTES);
        let mut grew = false;
        let at = match pool::carve(&mut self.free, size, ENTRY_BYTES) {
            Some(at) => at,
            None => {
                let old = self.bytes.len() as u64;
                let new = (old + size).next_power_of_two();
                self.bytes.resize(new as usize, 0);
                pool::merge(&mut self.free, old, new - old);
                self.grown += 1;
                grew = true;
                pool::carve(&mut self.free, size, ENTRY_BYTES).expect("the new room has the range")
            }
        };
        self.bytes[at as usize..at as usize + entries.len()].copy_from_slice(entries);
        self.used += size;
        ((at, size), (at / ENTRY_BYTES) as u32, grew)
    }

    fn give(&mut self, range: ListRange) {
        pool::merge(&mut self.free, range.0, range.1);
        self.used -= range.1;
    }
}

pub(crate) struct Influences {
    pub(crate) buffer: wgpu::Buffer,
    space: ListSpace,
}

fn make_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.counted_buffer(&wgpu::BufferDescriptor {
        label: Some("bone influences"),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl Influences {
    pub(crate) fn new(device: &wgpu::Device) -> Influences {
        Influences {
            buffer: make_buffer(device, FIRST_BYTES),
            space: ListSpace::new(FIRST_BYTES),
        }
    }

    /// Takes a range for the entries, and writes them. Returns the range, and its first entry
    /// as the vertices name it. The entries of a list come in whole numbers of [`ENTRY_BYTES`].
    pub(crate) fn put(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, entries: &[u8]) -> (ListRange, u32) {
        let (range, first, grew) = self.space.put(entries);
        if grew {
            self.buffer = make_buffer(device, self.space.bytes.len() as u64);
            queue.write_buffer(&self.buffer, 0, &self.space.bytes);
        } else if !entries.is_empty() {
            queue.write_buffer(&self.buffer, range.0, entries);
        }
        (range, first)
    }

    /// Gives a range back, after the frame that may still use it.
    pub(crate) fn give(&mut self, range: ListRange) {
        self.space.give(range);
    }

    /// For the report: the bytes in use, the room of the buffer, and how many times it grew.
    pub(crate) fn report(&self) -> (u64, u64, u32) {
        (self.space.used, self.space.bytes.len() as u64, self.space.grown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertex(first: f32, count: f32) -> Vec<u8> {
        let mut bytes = vec![0u8; 28];
        bytes[12..16].copy_from_slice(&first.to_le_bytes());
        bytes[16..20].copy_from_slice(&count.to_le_bytes());
        bytes
    }

    fn entries(count: usize, fill: u8) -> Vec<u8> {
        vec![fill; count * ENTRY_BYTES as usize]
    }

    #[test]
    fn a_vertex_names_its_entries_after_the_move() {
        let vertices = [vertex(0.0, 6.0), vertex(6.0, 2.0)].concat();
        let moved = moved_vertices(&vertices, 28, 1000);
        let float = |at: usize| f32::from_le_bytes(moved[at..at + 4].try_into().unwrap());
        assert_eq!((float(12), float(16)), (1000.0, 6.0));
        assert_eq!((float(28 + 12), float(28 + 16)), (1006.0, 2.0));
    }

    #[test]
    fn the_largest_first_entry_is_exact_in_a_float() {
        // 2^24 entries is 128 MiB, the default limit of a storage binding: each whole number
        // up to there is exact in the float of the vertex.
        let last = (1u32 << 24) - 1;
        assert_eq!(last as f32 as u32, last);
    }

    /// A list that does not fit grows the room, and the lists before it keep their offset and
    /// their bytes: the vertices that name them are not written again.
    #[test]
    fn the_room_grows_and_the_old_lists_stay() {
        let mut space = ListSpace::new(64);
        let (a, a_first, a_grew) = space.put(&entries(4, 0xAA));
        assert_eq!((a, a_first, a_grew), ((0, 32), 0, false));
        let (b, b_first, b_grew) = space.put(&entries(100, 0xBB));
        assert!(b_grew);
        assert_eq!(space.grown, 1);
        assert_eq!(b.0 % ENTRY_BYTES, 0);
        assert_eq!(b_first as u64, b.0 / ENTRY_BYTES);
        assert!(space.bytes.len() as u64 >= 32 + 800);
        assert!(space.bytes[..32].iter().all(|&byte| byte == 0xAA));
        assert!(space.bytes[b.0 as usize..(b.0 + b.1) as usize]
            .iter()
            .all(|&byte| byte == 0xBB));
        assert_eq!(space.used, 832);
    }

    /// A range given back serves the next list, with no growth.
    #[test]
    fn a_range_given_back_is_used_again() {
        let mut space = ListSpace::new(64);
        let (a, _, _) = space.put(&entries(8, 1));
        space.give(a);
        assert_eq!(space.used, 0);
        let (b, _, grew) = space.put(&entries(8, 2));
        assert!(!grew);
        assert_eq!(b, a);
        assert_eq!(space.grown, 0);
    }

    #[test]
    fn a_list_needs_its_largest_bone_and_one() {
        assert_eq!(bones_needed(&[]), 0);
        let mut list = Vec::new();
        for (bone, weight) in [(2u32, 0.25f32), (6, 0.5), (0, 0.25)] {
            list.extend_from_slice(&bone.to_le_bytes());
            list.extend_from_slice(&weight.to_le_bytes());
        }
        assert_eq!(bones_needed(&list), 7);
    }
}
