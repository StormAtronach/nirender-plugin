//! Pools of GPU buffers for the geometry of the game: the vertices of all meshes in a few
//! large buffers, the indexes in others, so that a draw binds no vertex or index buffer of
//! its own (the draw names its base vertex and first index). A
//! range goes back to its pool when the geometry record that took it is replaced, one frame
//! later, when the GPU is done with the draws that used it.

use std::rc::Rc;

/// A range of a buffer of a pool, taken for one geometry record.
pub struct Range {
    pub buffer: Rc<wgpu::Buffer>,
    pub offset: u64,
    pub size: u64,
}

struct Chunk {
    buffer: Rc<wgpu::Buffer>,
    /// The free ranges (offset, size), in the order of the offsets, none touching another.
    free: Vec<(u64, u64)>,
}

pub struct Pool {
    label: &'static str,
    usage: wgpu::BufferUsages,
    chunk_size: u64,
    chunks: Vec<Chunk>,
    /// The bytes in use.
    pub used: u64,
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// The alignment of a range: a multiple of the stride of its vertices (so that the base
/// vertex is a whole number) and of 4 (a write of the queue needs it).
fn alignment(align: u64) -> u64 {
    let align = align.max(1);
    align * 4 / gcd(align, 4)
}

/// Takes `size` bytes at a multiple of `align` out of the free ranges (first fit): the offset,
/// or `None` when no range has room.
fn carve(free: &mut Vec<(u64, u64)>, size: u64, align: u64) -> Option<u64> {
    let slot = free.iter().position(|&(offset, room)| offset.next_multiple_of(align) + size <= offset + room)?;
    let (offset, room) = free.remove(slot);
    let at = offset.next_multiple_of(align);
    // What is left before and after the range stays free.
    let mut back = Vec::with_capacity(2);
    if at > offset {
        back.push((offset, at - offset));
    }
    if at + size < offset + room {
        back.push((at + size, offset + room - at - size));
    }
    for (i, range) in back.into_iter().enumerate() {
        free.insert(slot + i, range);
    }
    Some(at)
}

/// Gives a range back to the free ranges, joined with the neighbours that touch it.
fn merge(free: &mut Vec<(u64, u64)>, offset: u64, size: u64) {
    let at = free.partition_point(|&(start, _)| start < offset);
    free.insert(at, (offset, size));
    if at + 1 < free.len() && free[at].0 + free[at].1 == free[at + 1].0 {
        let (_, next) = free.remove(at + 1);
        free[at].1 += next;
    }
    if at > 0 && free[at - 1].0 + free[at - 1].1 == free[at].0 {
        let (_, this) = free.remove(at);
        free[at - 1].1 += this;
    }
}

impl Pool {
    pub fn new(label: &'static str, usage: wgpu::BufferUsages, chunk_size: u64) -> Pool {
        Pool { label, usage: usage | wgpu::BufferUsages::COPY_DST, chunk_size, chunks: Vec::new(), used: 0 }
    }

    pub fn chunks(&self) -> usize {
        self.chunks.len()
    }

    pub fn capacity(&self) -> u64 {
        self.chunks.iter().map(|chunk| chunk.buffer.size()).sum()
    }

    /// Takes a range for `bytes` at an offset that is a multiple of `align` and of 4, writes
    /// the bytes there, and returns the range. A new chunk when no chunk has room.
    pub fn take(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, bytes: &[u8], align: u64) -> Range {
        let align = alignment(align);
        let size = (bytes.len() as u64).next_multiple_of(4).max(4);
        let found = self.chunks.iter_mut().enumerate().find_map(|(index, chunk)| carve(&mut chunk.free, size, align).map(|at| (index, at)));
        let (index, at) = match found {
            Some(found) => found,
            None => {
                let chunk_size = self.chunk_size.max(size + align);
                let buffer = device.create_buffer(&wgpu::BufferDescriptor { label: Some(self.label), size: chunk_size, usage: self.usage, mapped_at_creation: false });
                let mut free = vec![(0, chunk_size)];
                let at = carve(&mut free, size, align).expect("a new chunk has room for its first range");
                self.chunks.push(Chunk { buffer: Rc::new(buffer), free });
                (self.chunks.len() - 1, at)
            }
        };
        let buffer = &self.chunks[index].buffer;
        if bytes.len() as u64 == size {
            queue.write_buffer(buffer, at, bytes);
        } else {
            let mut padded = bytes.to_vec();
            padded.resize(size as usize, 0);
            queue.write_buffer(buffer, at, &padded);
        }
        self.used += size;
        Range { buffer: buffer.clone(), offset: at, size }
    }

    /// Gives a range back. A range of a buffer that is not a chunk of this pool is ignored.
    pub fn give(&mut self, range: &Range) {
        let Some(chunk) = self.chunks.iter_mut().find(|chunk| Rc::ptr_eq(&chunk.buffer, &range.buffer)) else { return };
        merge(&mut chunk.free, range.offset, range.size);
        self.used -= range.size;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_alignment_is_a_multiple_of_the_stride_and_of_four() {
        assert_eq!(alignment(36), 36);
        assert_eq!(alignment(28), 28);
        assert_eq!(alignment(30), 60);
        assert_eq!(alignment(1), 4);
        assert_eq!(alignment(2), 4);
        assert_eq!(alignment(0), 4);
    }

    #[test]
    fn ranges_come_out_aligned_and_apart() {
        let mut free = vec![(0, 1000)];
        let a = carve(&mut free, 100, 36).unwrap();
        let b = carve(&mut free, 100, 36).unwrap();
        let c = carve(&mut free, 8, 4).unwrap();
        assert_eq!(a % 36, 0);
        assert_eq!(b % 36, 0);
        assert_eq!(c % 4, 0);
        let taken = [(a, 100), (b, 100), (c, 8)];
        for (i, &(x, n)) in taken.iter().enumerate() {
            for &(y, m) in &taken[i + 1..] {
                assert!(x + n <= y || y + m <= x, "{taken:?}");
            }
        }
        assert_eq!(free.iter().map(|r| r.1).sum::<u64>() + 208, 1000);
    }

    #[test]
    fn a_range_given_back_joins_its_neighbours() {
        let mut free = vec![(0, 1000)];
        let a = carve(&mut free, 100, 4).unwrap();
        let b = carve(&mut free, 100, 4).unwrap();
        let c = carve(&mut free, 100, 4).unwrap();
        merge(&mut free, a, 100);
        merge(&mut free, c, 100);
        assert_eq!(free.len(), 2, "{free:?}");
        merge(&mut free, b, 100);
        assert_eq!(free, vec![(0, 1000)]);
    }

    #[test]
    fn no_room_gives_none() {
        let mut free = vec![(0, 100)];
        assert_eq!(carve(&mut free, 101, 4), None);
        assert_eq!(carve(&mut free, 100, 4), Some(0));
        assert!(free.is_empty());
        assert_eq!(carve(&mut free, 4, 4), None);
    }
}
