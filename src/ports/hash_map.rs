//! The `THashMap` of the engine, as the game code uses it inline in the renderer.
//!
//! A map is a vtable, a count, a bucket count and an array of bucket heads. A node is a key, a
//! value and the next node of the bucket. The map calls four virtual functions for the keys
//! and the nodes. Each function here makes the same reads, writes and virtual calls as the
//! inline game code, in the same order [I: transcribed from the inline game code; there is
//! no single function to compare with].
//!
//! A function that allocates or frees a node has two forms. The plain form calls
//! `operator new` (0x727692) or `operator delete` (0x727530) of the game. The `_with` form
//! takes the allocator as an argument, for a port whose unit tests have their own allocator.


use crate::engine::{self, vcall1, vcall2, vcall3};
use crate::gen::layouts::{THashMap as HM, THashMap_Node as HN};
use crate::mem::{rd, wr};

/// The vtable slot of `hashKey`. It gives the bucket index of a key.
pub const SLOT_HASH_KEY: usize = crate::gen::layouts::InterfaceTHashMap::hashKey;
/// The vtable slot of `cmpKey`. It returns a `bool` in AL: 1 when the two keys are equal.
pub const SLOT_CMP_KEY: usize = crate::gen::layouts::InterfaceTHashMap::cmpKey;
/// The vtable slot of `makeKeyValuePair`. It writes the key and the value into a node.
pub const SLOT_MAKE_KEY_VALUE_PAIR: usize = crate::gen::layouts::InterfaceTHashMap::makeKeyValuePair;
/// The vtable slot of `dtorKeyValuePair`. It destroys the key and the value of a node.
pub const SLOT_DTOR_KEY_VALUE_PAIR: usize = crate::gen::layouts::InterfaceTHashMap::dtorKeyValuePair;

// ---- the virtual functions

/// `THashMap::hashKey` through the vtable, slot 0x04. Returns the bucket index of `key`.
///
/// # Safety
///
/// `map` points at a live engine `THashMap`: the function calls its virtual table.
#[inline(always)]
pub unsafe fn hash_key(map: usize, key: u32) -> u32 {
    vcall1(map, SLOT_HASH_KEY, key)
}

/// `THashMap::cmpKey` through the vtable, slot 0x08. The result of the game function is in
/// AL, thus only the low byte counts.
///
/// # Safety
///
/// `map` points at a live engine `THashMap`: the function calls its virtual table.
#[inline(always)]
pub unsafe fn cmp_key(map: usize, a: u32, b: u32) -> bool {
    vcall2(map, SLOT_CMP_KEY, a, b) as u8 != 0
}

/// `THashMap::makeKeyValuePair` through the vtable, slot 0x0C.
///
/// # Safety
///
/// `map` points at a live engine `THashMap`, and `node` at a node of `HN::SIZE` bytes that the
/// map's `makeKeyValuePair` fills.
#[inline(always)]
pub unsafe fn make_key_value_pair(map: usize, node: usize, key: u32, value: usize) {
    vcall3(map, SLOT_MAKE_KEY_VALUE_PAIR, node as u32, key, value as u32);
}

/// `THashMap::dtorKeyValuePair` through the vtable, slot 0x10.
///
/// # Safety
///
/// `map` points at a live engine `THashMap`, and `node` at one of its nodes whose pair nothing uses
/// after the call.
#[inline(always)]
pub unsafe fn dtor_key_value_pair(map: usize, node: usize) {
    vcall1(map, SLOT_DTOR_KEY_VALUE_PAIR, node as u32);
}

// ---- iteration

/// The first node of bucket `bucket`, or 0. It reads `arrayBuckets` of the map on each call.
///
/// # Safety
///
/// `map` points at a live engine `THashMap`, and `bucket` is below its bucket count, which nothing
/// here checks.
#[inline(always)]
pub unsafe fn bucket_head(map: usize, bucket: usize) -> usize {
    rd(rd::<usize>(map, HM::arrayBuckets), bucket * 4)
}

/// The first node of the map: the head of the first bucket that has a node. The result is 0
/// for a map without nodes. It makes no virtual call.
///
/// # Safety
///
/// `map` points at a live engine `THashMap` with its array of `bucketCount` buckets.
#[inline(always)]
pub unsafe fn first(map: usize) -> usize {
    let count: u32 = rd(map, HM::bucketCount);
    if count != 0 {
        let buckets: usize = rd(map, HM::arrayBuckets);
        let mut index = 0u32;
        loop {
            let node: usize = rd(buckets, index as usize * 4);
            if node != 0 {
                return node;
            }
            index += 1;
            if index >= count {
                break;
            }
        }
    }
    0
}

/// The head of the first bucket with a node after the bucket of `key`, or 0. It calls
/// `hashKey` one time, also when the bucket of `key` is the last bucket.
///
/// # Safety
///
/// `map` points at a live engine `THashMap` with its virtual table and its array of `bucketCount`
/// buckets.
#[inline(always)]
pub unsafe fn next_bucket(map: usize, key: u32) -> usize {
    let mut index = hash_key(map, key).wrapping_add(1);
    let count: u32 = rd(map, HM::bucketCount);
    if index < count {
        let buckets: usize = rd(map, HM::arrayBuckets);
        loop {
            let node: usize = rd(buckets, index as usize * 4);
            if node != 0 {
                return node;
            }
            index += 1;
            if index >= count {
                break;
            }
        }
    }
    0
}

/// The node after `node` in the order of iteration, or 0. It is the next node of the bucket.
/// At the end of the bucket it is `next_bucket` for the key of `node`, which calls `hashKey`.
/// Call it at the place where the game code makes that step, before or after the work on the
/// node.
///
/// # Safety
///
/// `map` points at a live engine `THashMap` with its virtual table and its buckets, and `node` at a
/// live node of it.
#[inline(always)]
pub unsafe fn next(map: usize, node: usize) -> usize {
    let next: usize = rd(node, HN::nextNode);
    if next != 0 {
        next
    } else {
        next_bucket(map, rd(node, HN::key))
    }
}

// ---- lookup

/// The bucket index of `key` and the node of `key`. The node is 0 when the map has no such
/// key. It calls `hashKey` one time, then `cmpKey(key, key of the node)` for each node of the
/// bucket until one is equal.
///
/// # Safety
///
/// `map` points at a live engine `THashMap` with its virtual table and its buckets, whose nodes are
/// live.
#[inline(always)]
pub unsafe fn find_in_bucket(map: usize, key: u32) -> (usize, usize) {
    let bucket = hash_key(map, key) as usize;
    let mut node = bucket_head(map, bucket);
    while node != 0 {
        if cmp_key(map, key, rd(node, HN::key)) {
            break;
        }
        node = rd(node, HN::nextNode);
    }
    (bucket, node)
}

/// The node of `key`, or 0. It makes the calls of `find_in_bucket`.
///
/// # Safety
///
/// `map` points at a live engine `THashMap` with its virtual table and its buckets, whose nodes are
/// live.
#[inline(always)]
pub unsafe fn find(map: usize, key: u32) -> usize {
    find_in_bucket(map, key).1
}

/// The value that the map has for `key`, or 0 when the map has no such key. It makes the
/// calls of `find_in_bucket`.
///
/// # Safety
///
/// `map` points at a live engine `THashMap` with its virtual table and its buckets, whose nodes are
/// live.
#[inline(always)]
pub unsafe fn find_value(map: usize, key: u32) -> usize {
    let node = find(map, key);
    if node != 0 {
        rd(node, HN::data)
    } else {
        0
    }
}

// ---- insert

/// `THashMap::SetAt` as the game code has it inline, with the allocator as an argument.
///
/// With a node for `key`: `dtorKeyValuePair` for the node, then `makeKeyValuePair`. Without
/// one: `operator_new(0xC)`, `makeKeyValuePair` for the new node, the old head of the bucket
/// goes to `nextNode` of the new node, the new node becomes the head, the count goes up by 1.
///
/// # Safety
///
/// `map` points at a live engine `THashMap` with its virtual table and its buckets, whose nodes are
/// live; `operator_new` gives a block of `HN::SIZE` bytes, not 0: the function does not test it.
#[inline(always)]
pub unsafe fn set_with(map: usize, key: u32, value: usize, operator_new: impl FnOnce(usize) -> usize) {
    let (bucket, node) = find_in_bucket(map, key);
    if node != 0 {
        dtor_key_value_pair(map, node);
        make_key_value_pair(map, node, key, value);
    } else {
        let node = operator_new(HN::SIZE);
        make_key_value_pair(map, node, key, value);
        wr::<usize>(node, HN::nextNode, bucket_head(map, bucket));
        wr(rd::<usize>(map, HM::arrayBuckets), bucket * 4, node);
        wr(map, HM::count, rd::<u32>(map, HM::count).wrapping_add(1));
    }
}

/// `THashMap::SetAt` as the game code has it inline. It gets a new node from `operator new`
/// (0x727692). See `set_with` for the order of the calls and the writes.
///
/// # Safety
///
/// `map` points at a live engine `THashMap` with its virtual table and its buckets, whose nodes are
/// live; the engine's `operator new` does not fail, since the function does not test the new node.
#[inline(always)]
pub unsafe fn set(map: usize, key: u32, value: usize) {
    set_with(map, key, value, |size| engine::operator_new(size));
}

// ---- remove

/// Takes the node of `key` out of its bucket. Returns the node, or 0 when the map has no such
/// key. It writes the head of the bucket or `nextNode` of the node before. It does not
/// destroy the node and it does not change the count.
///
/// # Safety
///
/// `map` points at a live engine `THashMap` with its virtual table and its buckets, whose nodes are
/// live. The node it returns is out of the map, and the caller frees it.
#[inline(always)]
pub unsafe fn unlink(map: usize, key: u32) -> usize {
    let bucket = hash_key(map, key) as usize;
    let mut node = bucket_head(map, bucket);
    if node == 0 {
        return 0;
    }
    if cmp_key(map, key, rd(node, HN::key)) {
        let next: usize = rd(node, HN::nextNode);
        wr(rd::<usize>(map, HM::arrayBuckets), bucket * 4, next);
        return node;
    }
    loop {
        let previous = node;
        node = rd(node, HN::nextNode);
        if node == 0 {
            return 0;
        }
        if cmp_key(map, key, rd(node, HN::key)) {
            wr(previous, HN::nextNode, rd::<usize>(node, HN::nextNode));
            return node;
        }
    }
}

/// `THashMap::RemoveAt` as the game code has it inline, with the allocator as an argument. It
/// does `unlink`. With a node: `dtorKeyValuePair`, `operator_delete(node)`, the count goes
/// down by 1. Returns true when the map had the key.
///
/// # Safety
///
/// `map` points at a live engine `THashMap` with its virtual table and its buckets, whose nodes are
/// live, and `operator_delete` frees a node of the allocation that the map's nodes come from.
#[inline(always)]
pub unsafe fn remove_with(map: usize, key: u32, operator_delete: impl FnOnce(usize)) -> bool {
    let node = unlink(map, key);
    if node == 0 {
        return false;
    }
    dtor_key_value_pair(map, node);
    operator_delete(node);
    wr(map, HM::count, rd::<u32>(map, HM::count).wrapping_sub(1));
    true
}

/// `THashMap::RemoveAt` as the game code has it inline. It frees the node with
/// `operator delete` (0x727530). See `remove_with`.
///
/// # Safety
///
/// `map` points at a live engine `THashMap` with its virtual table and its buckets, whose nodes
/// come from the engine's `operator new` (0x727692).
#[inline(always)]
pub unsafe fn remove(map: usize, key: u32) -> bool {
    remove_with(map, key, |node| engine::operator_delete(node))
}

/// Removes every node, with the allocator as an argument. For each bucket, while it has a
/// head: the next node becomes the head, then `dtorKeyValuePair`, then `operator_delete` for
/// the old head. It does not write the count: the caller does that. It does nothing for a map
/// with a bucket count of 0.
///
/// # Safety
///
/// `map` points at a live engine `THashMap` with its virtual table and its buckets, and
/// `operator_delete` frees a node of the allocation that its nodes come from; nothing uses the
/// nodes after the call, and the caller writes the count.
#[inline(always)]
pub unsafe fn drain_with(map: usize, mut operator_delete: impl FnMut(usize)) {
    if rd::<u32>(map, HM::bucketCount) == 0 {
        return;
    }
    let mut bucket = 0u32;
    loop {
        let mut buckets: usize = rd(map, HM::arrayBuckets);
        let mut node: usize = rd(buckets, bucket as usize * 4);
        while node != 0 {
            wr::<usize>(buckets, bucket as usize * 4, rd(node, HN::nextNode));
            dtor_key_value_pair(map, node);
            operator_delete(node);
            buckets = rd(map, HM::arrayBuckets);
            node = rd(buckets, bucket as usize * 4);
        }
        bucket += 1;
        if bucket >= rd::<u32>(map, HM::bucketCount) {
            break;
        }
    }
}

/// Removes every node and frees each with `operator delete` (0x727530). The caller writes the
/// count. See `drain_with`.
///
/// # Safety
///
/// `map` points at a live engine `THashMap` with its virtual table and its buckets, whose nodes
/// come from the engine's `operator new`; nothing uses the nodes after the call, and the caller
/// writes the count.
#[inline(always)]
pub unsafe fn drain(map: usize) {
    drain_with(map, |node| engine::operator_delete(node));
}

/// `drain_with`, then the count becomes 0. The write of the count is the last write. It is
/// also done for a map with a bucket count of 0.
///
/// # Safety
///
/// `map` points at a live engine `THashMap` with its virtual table and its buckets, and
/// `operator_delete` frees a node of the allocation that its nodes come from; nothing uses the
/// nodes after the call.
#[inline(always)]
pub unsafe fn drain_and_zero_count_with(map: usize, operator_delete: impl FnMut(usize)) {
    drain_with(map, operator_delete);
    wr::<u32>(map, HM::count, 0);
}

/// Removes every node with `operator delete` (0x727530), then the count becomes 0. This is
/// the body of `NiDX8TextureManager::clearHashMap` (0x6BC8A0) where the game code has it
/// inline.
///
/// # Safety
///
/// `map` points at a live engine `THashMap` with its virtual table and its buckets, whose nodes
/// come from the engine's `operator new`; nothing uses the nodes after the call.
#[inline(always)]
pub unsafe fn drain_and_zero_count(map: usize) {
    drain_and_zero_count_with(map, |node| engine::operator_delete(node));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// One call that the map made, in the order of the calls.
    #[derive(Debug, PartialEq, Clone, Copy)]
    enum Call {
        Hash(u32),
        /// The two keys, in the order of the arguments.
        Cmp(u32, u32),
        /// The new key and the new value.
        Make(u32, usize),
        /// The key that the node had.
        Dtor(u32),
        New(usize),
        /// The key that the node had.
        Delete(u32),
    }

    thread_local! {
        static CALLS: RefCell<Vec<Call>> = const { RefCell::new(Vec::new()) };
    }

    fn note(call: Call) {
        CALLS.with(|calls| calls.borrow_mut().push(call));
    }

    fn calls() -> Vec<Call> {
        CALLS.with(|calls| core::mem::take(&mut *calls.borrow_mut()))
    }

    unsafe extern "thiscall" fn fake_hash(map: usize, key: u32) -> u32 {
        note(Call::Hash(key));
        key % rd::<u32>(map, HM::bucketCount)
    }

    unsafe extern "thiscall" fn fake_cmp(_map: usize, a: u32, b: u32) -> u32 {
        note(Call::Cmp(a, b));
        // Only AL is the result. The other bytes are what the register had.
        0x1234_5600 | (a == b) as u32
    }

    unsafe extern "thiscall" fn fake_make(_map: usize, node: usize, key: u32, value: usize) -> u32 {
        note(Call::Make(key, value));
        wr(node, HN::key, key);
        wr(node, HN::data, value);
        0
    }

    unsafe extern "thiscall" fn fake_dtor(_map: usize, node: usize) -> u32 {
        note(Call::Dtor(rd(node, HN::key)));
        0
    }

    /// A map in test memory with 4 buckets. The memory of the nodes stays allocated, thus a
    /// test can read a node after the map freed it.
    struct Map {
        vtable: Box<[usize; 5]>,
        buckets: Box<[usize; 4]>,
        map: Box<[usize; 4]>,
    }

    impl Map {
        fn new() -> Map {
            let vtable = Box::new([
                0,
                fake_hash as *const () as usize,
                fake_cmp as *const () as usize,
                fake_make as *const () as usize,
                fake_dtor as *const () as usize,
            ]);
            let mut buckets = Box::new([0usize; 4]);
            // vtable, count, bucket count, buckets
            let map = Box::new([vtable.as_ptr() as usize, 0, 4, buckets.as_mut_ptr() as usize]);
            assert_eq!((HM::vtbl, HM::count, HM::bucketCount, HM::arrayBuckets), (0, 4, 8, 12));
            Map { vtable, buckets, map }
        }

        fn at(&mut self) -> usize {
            self.map.as_mut_ptr() as usize
        }

        fn count(&self) -> usize {
            self.map[1]
        }

        /// The keys of a bucket, from the head.
        unsafe fn keys(&self, bucket: usize) -> Vec<u32> {
            let mut keys = Vec::new();
            let mut node = self.buckets[bucket];
            while node != 0 {
                keys.push(rd(node, HN::key));
                node = rd(node, HN::nextNode);
            }
            keys
        }

        unsafe fn insert(&mut self, key: u32, value: usize) {
            let at = self.at();
            set_with(at, key, value, test_new);
        }
    }

    fn test_new(size: usize) -> usize {
        note(Call::New(size));
        Box::leak(Box::new([0xEEEE_EEEEu32; 3])).as_mut_ptr() as usize
    }

    unsafe fn test_delete(node: usize) {
        note(Call::Delete(rd(node, HN::key)));
    }

    #[test]
    fn set_puts_a_new_node_at_the_head_of_its_bucket() {
        let mut map = Map::new();
        let at = map.at();
        unsafe {
            map.insert(1, 100);
            assert_eq!(calls(), [Call::Hash(1), Call::New(HN::SIZE), Call::Make(1, 100)]);
            assert_eq!(rd::<usize>(map.buckets[1], HN::nextNode), 0);
            map.insert(5, 500);
            // The bucket has the node of key 1, thus one compare: the new key, then the key of the node.
            assert_eq!(calls(), [Call::Hash(5), Call::Cmp(5, 1), Call::New(HN::SIZE), Call::Make(5, 500)]);
            map.insert(2, 200);
            calls();
            assert_eq!(map.keys(1), [5, 1]);
            assert_eq!(map.keys(2), [2]);
            assert_eq!(map.count(), 3);
            assert_eq!(find_value(at, 1), 100);
            assert_eq!(calls(), [Call::Hash(1), Call::Cmp(1, 5), Call::Cmp(1, 1)]);
            assert_eq!(find_value(at, 5), 500);
            assert_eq!(find_value(at, 9), 0);
            assert_eq!(find(at, 3), 0);
            assert_eq!(find_in_bucket(at, 2), (2, map.buckets[2]));
            assert_eq!(find_in_bucket(at, 7), (3, 0));
            assert_eq!(bucket_head(at, 1), find(at, 5));
        }
    }

    #[test]
    fn set_for_a_key_of_the_map_destroys_the_pair_then_makes_it_again() {
        let mut map = Map::new();
        let at = map.at();
        unsafe {
            map.insert(1, 100);
            map.insert(5, 500);
            let node = find(at, 1);
            calls();
            map.insert(1, 111);
            assert_eq!(calls(), [Call::Hash(1), Call::Cmp(1, 5), Call::Cmp(1, 1), Call::Dtor(1), Call::Make(1, 111)]);
            assert_eq!(find(at, 1), node);
            assert_eq!(rd::<usize>(node, HN::data), 111);
            assert_eq!(map.keys(1), [5, 1]);
            assert_eq!(map.count(), 2);
        }
    }

    #[test]
    fn the_walk_goes_bucket_by_bucket() {
        let mut map = Map::new();
        let at = map.at();
        unsafe {
            assert_eq!(first(at), 0);
            map.insert(1, 0);
            map.insert(5, 0);
            map.insert(3, 0);
            calls();
            let (node5, node1, node3) = (find(at, 5), find(at, 1), find(at, 3));
            calls();
            assert_eq!(first(at), node5);
            // `first` makes no virtual call.
            assert_eq!(calls(), []);
            assert_eq!(next(at, node5), node1);
            assert_eq!(calls(), []);
            assert_eq!(next(at, node1), node3);
            assert_eq!(calls(), [Call::Hash(1)]);
            assert_eq!(next(at, node3), 0);
            // The bucket of key 3 is the last bucket. The hash call is made also then.
            assert_eq!(calls(), [Call::Hash(3)]);
            assert_eq!(next_bucket(at, 5), node3);
            assert_eq!(next_bucket(at, 0), node5);
            // A bucket count of 0: no read of the bucket array.
            wr::<u32>(at, HM::bucketCount, 0);
            wr::<usize>(at, HM::arrayBuckets, 0);
            assert_eq!(first(at), 0);
        }
    }

    #[test]
    fn unlink_takes_the_head_or_a_later_node() {
        let mut map = Map::new();
        let at = map.at();
        unsafe {
            // Bucket 1 has 9, then 5, then 1.
            map.insert(1, 100);
            map.insert(5, 500);
            map.insert(9, 900);
            let (node5, node9) = (find(at, 5), find(at, 9));
            calls();
            assert_eq!(unlink(at, 5), node5);
            assert_eq!(calls(), [Call::Hash(5), Call::Cmp(5, 9), Call::Cmp(5, 5)]);
            assert_eq!(map.keys(1), [9, 1]);
            assert_eq!(unlink(at, 9), node9);
            assert_eq!(map.keys(1), [1]);
            // `unlink` does not change the count.
            assert_eq!(map.count(), 3);
            calls();
            // A key that is not in a bucket with nodes, and an empty bucket.
            assert_eq!(unlink(at, 13), 0);
            assert_eq!(calls(), [Call::Hash(13), Call::Cmp(13, 1)]);
            assert_eq!(unlink(at, 2), 0);
            assert_eq!(calls(), [Call::Hash(2)]);
        }
    }

    #[test]
    fn remove_destroys_the_pair_before_it_frees_the_node() {
        let mut map = Map::new();
        let at = map.at();
        unsafe {
            map.insert(1, 100);
            map.insert(5, 500);
            calls();
            assert!(remove_with(at, 1, |node| test_delete(node)));
            assert_eq!(calls(), [Call::Hash(1), Call::Cmp(1, 5), Call::Cmp(1, 1), Call::Dtor(1), Call::Delete(1)]);
            assert_eq!(map.keys(1), [5]);
            assert_eq!(map.count(), 1);
            assert!(!remove_with(at, 1, |node| test_delete(node)));
            assert_eq!(calls(), [Call::Hash(1), Call::Cmp(1, 5)]);
            assert_eq!(map.count(), 1);
        }
    }

    #[test]
    fn drain_frees_each_node_from_the_head_and_leaves_the_count() {
        let mut map = Map::new();
        let at = map.at();
        unsafe {
            map.insert(1, 0);
            map.insert(5, 0);
            map.insert(3, 0);
            calls();
            drain_with(at, |node| test_delete(node));
            assert_eq!(
                calls(),
                [Call::Dtor(5), Call::Delete(5), Call::Dtor(1), Call::Delete(1), Call::Dtor(3), Call::Delete(3)]
            );
            assert_eq!(*map.buckets, [0; 4]);
            assert_eq!(map.count(), 3);

            map.insert(2, 0);
            calls();
            drain_and_zero_count_with(at, |node| test_delete(node));
            assert_eq!(calls(), [Call::Dtor(2), Call::Delete(2)]);
            assert_eq!(map.count(), 0);

            // A bucket count of 0: no read of the bucket array, and the count is written.
            wr::<u32>(at, HM::count, 7);
            wr::<u32>(at, HM::bucketCount, 0);
            wr::<usize>(at, HM::arrayBuckets, 0);
            drain_with(at, |node| test_delete(node));
            assert_eq!(map.count(), 7);
            drain_and_zero_count_with(at, |node| test_delete(node));
            assert_eq!(map.count(), 0);
            assert_eq!(calls(), []);
        }
        let _ = &map.vtable;
    }
}
