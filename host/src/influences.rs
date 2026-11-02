//! The bone influences of the shapes that the game skins in software (MWSE's flag on the
//! geometry): every influence of a vertex with its raw weight, as `NiSkinInstance::Deform`
//! 0x6FA000 sums them. The packed vertex of a skinned shape holds only 4 bones whose weights
//! sum to 1.
//!
//! One storage buffer holds the lists of all such geometries, because the vertex shader reads
//! it from one binding. A vertex of such a geometry holds its first entry and its count in the
//! floats of its weights, and [`Influences::put`] moves the first entry by the range of its
//! geometry. The buffer grows when no range has room. The offsets stay valid, and the frame
//! group binds the new buffer. A copy of the bytes stays here for that write, so the growth
//! needs no encoder (`queue.write_buffer` lands before the commands of the frame).

use crate::pool;

/// The bytes of an entry of the list: the bone (u32) and its weight (f32).
pub(crate) const ENTRY_BYTES: u64 = 8;
/// The room of the buffer at first: such a geometry is rare.
const FIRST_BYTES: u64 = 1 << 12;
/// The vertex format of a geometry with a list: this bit on the format of a skinned vertex. It
/// is above the bits of a real format here: the plugin writes no sizes of coordinate sets.
pub(crate) const FVF_INFLUENCES: u32 = 0x4000_0000;
/// The bytes of a skinned vertex before its first weight: the position.
const WEIGHTS_AT: usize = 12;

/// A range of the list, to give back: the offset and the size in bytes.
pub(crate) type ListRange = (u64, u64);

pub(crate) struct Influences {
    pub(crate) buffer: wgpu::Buffer,
    bytes: Vec<u8>,
    free: Vec<(u64, u64)>,
    /// The bytes in use.
    pub(crate) used: u64,
}

fn make_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("bone influences"),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
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

impl Influences {
    pub(crate) fn new(device: &wgpu::Device) -> Influences {
        Influences { buffer: make_buffer(device, FIRST_BYTES), bytes: vec![0; FIRST_BYTES as usize], free: vec![(0, FIRST_BYTES)], used: 0 }
    }

    /// Takes a range for the entries, and writes them. Returns the range, and its first entry
    /// as the vertices name it. The entries of a list come in whole numbers of [`ENTRY_BYTES`].
    pub(crate) fn put(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, entries: &[u8]) -> (ListRange, u32) {
        let size = (entries.len() as u64).max(ENTRY_BYTES);
        let at = match pool::carve(&mut self.free, size, ENTRY_BYTES) {
            Some(at) => at,
            None => self.grow(device, queue, size),
        };
        self.bytes[at as usize..at as usize + entries.len()].copy_from_slice(entries);
        if !entries.is_empty() {
            queue.write_buffer(&self.buffer, at, entries);
        }
        self.used += size;
        ((at, size), (at / ENTRY_BYTES) as u32)
    }

    /// A buffer with room for `size` bytes more: the old bytes keep their offsets. Returns the
    /// offset of the new range.
    fn grow(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, size: u64) -> u64 {
        let old = self.bytes.len() as u64;
        let new = (old + size).next_power_of_two();
        self.buffer = make_buffer(device, new);
        self.bytes.resize(new as usize, 0);
        queue.write_buffer(&self.buffer, 0, &self.bytes[..old as usize]);
        pool::merge(&mut self.free, old, new - old);
        pool::carve(&mut self.free, size, ENTRY_BYTES).expect("the new buffer has room")
    }

    /// Gives a range back, after the frame that may still use it.
    pub(crate) fn give(&mut self, range: ListRange) {
        pool::merge(&mut self.free, range.0, range.1);
        self.used -= range.1;
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
}
