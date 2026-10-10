//! `NiDX8SystemDesc`, `NiDX8AdapterDesc` and `NiDX8DeviceDesc`: what the adapters of the
//! machine can do, asked from the `IDirect3D8` object when the renderer is made.
//!
//! The constructors only read from `IDirect3D8`, so both runs of a comparison make the same
//! calls. Their heap use goes through `operator new` and `operator delete`.

use core::mem::transmute;

use crate::gen::layouts::{
    NiDX8AdapterDesc as AD, NiDX8AdapterDesc_ModeDesc as MD, NiDX8DeviceDesc as DD, NiDX8DeviceDesc_ScreenFormatInfo as SFI,
    NiDX8SystemDesc as SD,
};
use crate::mem::{rd, wr};

// NiTListBase
const LIST_SIZE: usize = 0x18;
const LIST_COUNT: usize = 0x00;
const LIST_HEAD: usize = 0x04;
const LIST_TAIL: usize = 0x08;
const LIST_FREE: usize = 0x0C;
const LIST_BLOCKS: usize = 0x10;
const LIST_BLOCK_ITEMS: usize = 0x14;
// TListItem
const ITEM_SIZE: usize = 0x0C;
const ITEM_DATA: usize = 0x00;
const ITEM_NEXT: usize = 0x04;
const ITEM_PREVIOUS: usize = 0x08;
// NiTListAllocator::Data
const BLOCK_SIZE: usize = 0x08;
const BLOCK_NEXT: usize = 0x00;
const BLOCK_ITEMS: usize = 0x04;
// THashMap
const MAP_VTBL: usize = 0x00;
const MAP_COUNT: usize = 0x04;
const MAP_BUCKET_COUNT: usize = 0x08;
const MAP_BUCKETS: usize = 0x0C;
// THashMap::Node
const NODE_SIZE: usize = 0x0C;
const NODE_KEY: usize = 0x00;
const NODE_VALUE: usize = 0x04;
const NODE_NEXT: usize = 0x08;
// The virtual functions of THashMap
const MAP_HASH_KEY: usize = 0x04;
const MAP_CMP_KEY: usize = 0x08;
const MAP_MAKE_PAIR: usize = 0x0C;
const MAP_DTOR_PAIR: usize = 0x10;
// TArray of pointers
const ARRAY_VTBL: usize = 0x00;
const ARRAY_STORAGE: usize = 0x04;
const ARRAY_STORAGE_COUNT: usize = 0x08;
const ARRAY_END: usize = 0x0C;
const ARRAY_FILLED: usize = 0x10;
const ARRAY_GROW_BY: usize = 0x14;
// D3DDISPLAYMODE, as an array of four dwords
const MODE_WIDTH: usize = 0;
const MODE_HEIGHT: usize = 1;
const MODE_REFRESH: usize = 2;
const MODE_FORMAT: usize = 3;

/// The vtable of the mode array of an adapter (`vtbl_sg_TArray_of_ModeDesc`).
const MODE_ARRAY_VTBL: usize = 0x74F648;
/// The vtable of the adapter array (`vtbl_sg_TArray_of_NiDX8AdapterDesc`).
const ADAPTER_ARRAY_VTBL: usize = 0x74F64C;
/// The depth formats a screen format is tried with.
const DEPTH_FORMAT_COUNT: usize = 7;
const HASH_BUCKETS: u32 = 37;
/// `D3DCAPS2_CANRENDERWINDOWED`
const CAPS2_CAN_RENDER_WINDOWED: u32 = 0x80000;

// ---- engine functions and data at fixed addresses

#[cfg(not(test))]
mod eng {
    use core::mem::transmute;

    type This0 = unsafe extern "thiscall" fn(usize) -> u32;
    type This1 = unsafe extern "thiscall" fn(usize, u32) -> u32;
    type This2 = unsafe extern "thiscall" fn(usize, u32, u32) -> u32;

    /// 0x727692 `operator new`
    pub unsafe fn operator_new(size: usize) -> usize {
        transmute::<usize, unsafe extern "C" fn(usize) -> usize>(0x727692)(size)
    }
    /// 0x727530 `operator delete`
    pub unsafe fn delete(block: usize) {
        transmute::<usize, unsafe extern "C" fn(usize)>(0x727530)(block)
    }
    /// 0x6BBEF0 `NiTListAllocator::Data::ctor`: allocates `count` items and chains them.
    pub unsafe fn list_block_ctor(block: usize, count: u32) -> usize {
        transmute::<usize, This1>(0x6BBEF0)(block, count) as usize
    }
    /// 0x6B2D00 `sub_6B2D00`: frees the item blocks of a list and clears it.
    pub unsafe fn list_clear(list: usize) {
        transmute::<usize, This0>(0x6B2D00)(list);
    }
    /// 0x6B2D40 `sub_6B2D40`: puts a value at the head of a list.
    pub unsafe fn list_add_head(list: usize, value: u32) {
        transmute::<usize, This1>(0x6B2D40)(list, value);
    }
    /// 0x6B2DA0 `NiTListBase::advanceIterator`: returns the value at the iterator and moves it on.
    pub unsafe fn list_advance_iterator(list: usize, iterator: usize) -> u32 {
        transmute::<usize, This1>(0x6B2DA0)(list, iterator as u32)
    }
    /// 0x6CEE40 `TArray_of_pointer::SetSize`
    pub unsafe fn array_set_size(array: usize, size: u32) {
        transmute::<usize, This1>(0x6CEE40)(array, size);
    }
    /// 0x6CB260 `TArray_of_pointer::SetAt`
    pub unsafe fn array_set_at(array: usize, index: u32, element: usize) {
        transmute::<usize, This2>(0x6CB260)(array, index, element as u32);
    }
    /// 0x6BC8A0 `NiDX8TextureManager::clearHashMap`: frees every node of a hash map.
    pub unsafe fn hash_map_clear(map: usize) {
        transmute::<usize, This0>(0x6BC8A0)(map);
    }
    /// 0x74F634 `vtbl_hashmap_74F634`: the vtable of the depth-stencil map of a screen format.
    pub fn hash_map_vtbl() -> usize {
        0x74F634
    }
    /// 0x7C3520 `data_NiDX8DeviceDesc::ScreenFormatInfo_depthFormats`
    pub fn depth_formats() -> usize {
        0x7C3520
    }
}

/// Stand-ins for the engine functions, so the tests need no game. They do what the game
/// code does and keep a log of the heap calls.
#[cfg(test)]
mod eng {
    use super::*;
    use std::cell::RefCell;
    use std::sync::OnceLock;

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Heap {
        New(usize, usize),
        Delete(usize),
    }

    thread_local! {
        pub static HEAP_LOG: RefCell<Vec<Heap>> = const { RefCell::new(Vec::new()) };
    }

    pub unsafe fn operator_new(size: usize) -> usize {
        let layout = std::alloc::Layout::from_size_align(size.max(1), 4).unwrap();
        let block = std::alloc::alloc(layout) as usize;
        core::ptr::write_bytes(block as *mut u8, 0xCD, size);
        HEAP_LOG.with(|log| log.borrow_mut().push(Heap::New(block, size)));
        block
    }
    /// The block stays allocated, so a test can still look at it.
    pub unsafe fn delete(block: usize) {
        HEAP_LOG.with(|log| log.borrow_mut().push(Heap::Delete(block)));
    }
    pub unsafe fn list_block_ctor(block: usize, count: u32) -> usize {
        wr::<usize>(block, BLOCK_NEXT, 0);
        let bytes = count as usize * ITEM_SIZE;
        wr(block, BLOCK_ITEMS, operator_new(bytes));
        let items: usize = rd(block, BLOCK_ITEMS);
        for i in 0..count as usize - 1 {
            wr(items, i * ITEM_SIZE + ITEM_NEXT, items + (i + 1) * ITEM_SIZE);
        }
        wr::<usize>(items, bytes - ITEM_SIZE + ITEM_NEXT, 0);
        block
    }
    pub unsafe fn list_clear(list: usize) {
        let mut block: usize = rd(list, LIST_BLOCKS);
        while block != 0 {
            let current = block;
            block = rd(current, BLOCK_NEXT);
            delete(rd(current, BLOCK_ITEMS));
            delete(current);
        }
        for offset in [LIST_COUNT, LIST_HEAD, LIST_TAIL, LIST_FREE, LIST_BLOCKS] {
            wr::<u32>(list, offset, 0);
        }
    }
    pub unsafe fn list_add_head(list: usize, value: u32) {
        if rd::<usize>(list, LIST_FREE) == 0 {
            let block = list_block_ctor(operator_new(BLOCK_SIZE), rd(list, LIST_BLOCK_ITEMS));
            wr::<usize>(block, BLOCK_NEXT, rd(list, LIST_BLOCKS));
            wr(list, LIST_BLOCKS, block);
            wr::<usize>(list, LIST_FREE, rd(block, BLOCK_ITEMS));
        }
        let item: usize = rd(list, LIST_FREE);
        wr::<usize>(list, LIST_FREE, rd(item, ITEM_NEXT));
        wr(item, ITEM_DATA, value);
        wr::<usize>(item, ITEM_PREVIOUS, 0);
        let head: usize = rd(list, LIST_HEAD);
        wr(item, ITEM_NEXT, head);
        if head != 0 {
            wr(head, ITEM_PREVIOUS, item);
        } else {
            wr(list, LIST_TAIL, item);
        }
        wr(list, LIST_HEAD, item);
        wr::<u32>(list, LIST_COUNT, rd::<u32>(list, LIST_COUNT) + 1);
    }
    pub unsafe fn list_advance_iterator(_list: usize, iterator: usize) -> u32 {
        let item: usize = rd(iterator, 0);
        if item == 0 {
            return 0;
        }
        wr::<usize>(iterator, 0, rd(item, ITEM_NEXT));
        rd(item, ITEM_DATA)
    }
    pub unsafe fn array_set_size(array: usize, size: u32) {
        if size == rd::<u32>(array, ARRAY_STORAGE_COUNT) {
            return;
        }
        assert!(size >= rd::<u32>(array, ARRAY_END), "the tests only grow an array");
        let old: usize = rd(array, ARRAY_STORAGE);
        wr(array, ARRAY_STORAGE_COUNT, size);
        let new = operator_new(size as usize * 4);
        wr(array, ARRAY_STORAGE, new);
        for i in 0..rd::<u32>(array, ARRAY_END) as usize {
            wr::<u32>(new, i * 4, rd(old, i * 4));
        }
        delete(old);
    }
    pub unsafe fn array_set_at(array: usize, index: u32, element: usize) {
        if index >= rd::<u32>(array, ARRAY_STORAGE_COUNT) {
            return;
        }
        let storage: usize = rd(array, ARRAY_STORAGE);
        let value: usize = rd(element, 0);
        let old: usize = if index < rd::<u32>(array, ARRAY_END) { rd(storage, index as usize * 4) } else { 0 };
        if index >= rd::<u32>(array, ARRAY_END) {
            wr(array, ARRAY_END, index + 1);
        }
        let filled: u32 = rd(array, ARRAY_FILLED);
        if value != 0 && old == 0 {
            wr(array, ARRAY_FILLED, filled + 1);
        } else if value == 0 && old != 0 {
            wr(array, ARRAY_FILLED, filled - 1);
        }
        wr(storage, index as usize * 4, value);
    }
    pub unsafe fn hash_map_clear(map: usize) {
        for i in 0..rd::<u32>(map, MAP_BUCKET_COUNT) as usize {
            let buckets: usize = rd(map, MAP_BUCKETS);
            loop {
                let node: usize = rd(buckets, i * 4);
                if node == 0 {
                    break;
                }
                wr::<usize>(buckets, i * 4, rd(node, NODE_NEXT));
                delete(node);
            }
        }
        wr::<u32>(map, MAP_COUNT, 0);
    }

    // The virtual functions of the hash map: 0x51A440, 0x482120, 0x482140, 0x4F1C50.
    unsafe extern "thiscall" fn hash_key(map: usize, key: u32) -> u32 {
        key % rd::<u32>(map, MAP_BUCKET_COUNT)
    }
    unsafe extern "thiscall" fn cmp_key(_map: usize, a: u32, b: u32) -> u32 {
        (a == b) as u32
    }
    unsafe extern "thiscall" fn make_pair(_map: usize, node: usize, key: u32, value: u32) -> u32 {
        wr(node, NODE_KEY, key);
        wr(node, NODE_VALUE, value);
        0
    }
    unsafe extern "thiscall" fn dtor_pair(_map: usize, _node: usize) -> u32 {
        0
    }
    pub fn hash_map_vtbl() -> usize {
        static VTBL: OnceLock<[usize; 5]> = OnceLock::new();
        VTBL.get_or_init(|| [0, hash_key as *const () as usize, cmp_key as *const () as usize, make_pair as *const () as usize, dtor_pair as *const () as usize]).as_ptr() as usize
    }
    pub fn depth_formats() -> usize {
        static FORMATS: [u32; DEPTH_FORMAT_COUNT] = [70, 71, 73, 75, 80, 77, 79];
        FORMATS.as_ptr() as usize
    }
}

// ---- virtual functions of the hash map, called through the vtable of the map

unsafe fn map_virtual(map: usize, offset: usize) -> usize {
    rd(rd::<usize>(map, MAP_VTBL), offset)
}

unsafe fn map_hash_key(map: usize, key: u32) -> u32 {
    transmute::<usize, unsafe extern "thiscall" fn(usize, u32) -> u32>(map_virtual(map, MAP_HASH_KEY))(map, key)
}

/// The result is in AL.
unsafe fn map_cmp_key(map: usize, a: u32, b: u32) -> u8 {
    transmute::<usize, unsafe extern "thiscall" fn(usize, u32, u32) -> u32>(map_virtual(map, MAP_CMP_KEY))(map, a, b) as u8
}

unsafe fn map_make_pair(map: usize, node: usize, key: u32, value: usize) {
    let f: unsafe extern "thiscall" fn(usize, usize, u32, usize) -> u32 = transmute(map_virtual(map, MAP_MAKE_PAIR));
    f(map, node, key, value);
}

unsafe fn map_dtor_pair(map: usize, node: usize) {
    transmute::<usize, unsafe extern "thiscall" fn(usize, usize) -> u32>(map_virtual(map, MAP_DTOR_PAIR))(map, node);
}

// ---- IDirect3D8, called through the vtable of the object

unsafe fn d3d_virtual(d3d: usize, offset: usize) -> usize {
    rd(rd::<usize>(d3d, 0), offset)
}

/// `IDirect3D8::GetAdapterCount`
unsafe fn d3d_get_adapter_count(d3d: usize) -> u32 {
    transmute::<usize, unsafe extern "system" fn(usize) -> u32>(d3d_virtual(d3d, 0x10))(d3d)
}

/// `IDirect3D8::GetAdapterIdentifier`
unsafe fn d3d_get_adapter_identifier(d3d: usize, adapter: u32, flags: u32, identifier: usize) -> u32 {
    transmute::<usize, unsafe extern "system" fn(usize, u32, u32, usize) -> u32>(d3d_virtual(d3d, 0x14))(d3d, adapter, flags, identifier)
}

/// `IDirect3D8::GetAdapterModeCount`
unsafe fn d3d_get_adapter_mode_count(d3d: usize, adapter: u32) -> u32 {
    transmute::<usize, unsafe extern "system" fn(usize, u32) -> u32>(d3d_virtual(d3d, 0x18))(d3d, adapter)
}

/// `IDirect3D8::EnumAdapterModes`
unsafe fn d3d_enum_adapter_modes(d3d: usize, adapter: u32, mode: u32, out: usize) -> u32 {
    transmute::<usize, unsafe extern "system" fn(usize, u32, u32, usize) -> u32>(d3d_virtual(d3d, 0x1C))(d3d, adapter, mode, out)
}

/// `IDirect3D8::GetAdapterDisplayMode`
unsafe fn d3d_get_adapter_display_mode(d3d: usize, adapter: u32, out: usize) -> u32 {
    transmute::<usize, unsafe extern "system" fn(usize, u32, usize) -> u32>(d3d_virtual(d3d, 0x20))(d3d, adapter, out)
}

/// `IDirect3D8::CheckDeviceType`
unsafe fn d3d_check_device_type(d3d: usize, adapter: u32, device_type: u32, display: u32, back_buffer: u32, windowed: u32) -> u32 {
    let f: unsafe extern "system" fn(usize, u32, u32, u32, u32, u32) -> u32 = transmute(d3d_virtual(d3d, 0x24));
    f(d3d, adapter, device_type, display, back_buffer, windowed)
}

/// `IDirect3D8::CheckDeviceFormat`
unsafe fn d3d_check_device_format(d3d: usize, adapter: u32, device_type: u32, adapter_format: u32, usage: u32, resource: u32, format: u32) -> u32 {
    let f: unsafe extern "system" fn(usize, u32, u32, u32, u32, u32, u32) -> u32 = transmute(d3d_virtual(d3d, 0x28));
    f(d3d, adapter, device_type, adapter_format, usage, resource, format)
}

/// `IDirect3D8::CheckDepthStencilMatch`
unsafe fn d3d_check_depth_stencil_match(d3d: usize, adapter: u32, device_type: u32, adapter_format: u32, target: u32, depth: u32) -> u32 {
    let f: unsafe extern "system" fn(usize, u32, u32, u32, u32, u32) -> u32 = transmute(d3d_virtual(d3d, 0x30));
    f(d3d, adapter, device_type, adapter_format, target, depth)
}

/// `IDirect3D8::GetDeviceCaps`
unsafe fn d3d_get_device_caps(d3d: usize, adapter: u32, device_type: u32, caps: usize) -> u32 {
    transmute::<usize, unsafe extern "system" fn(usize, u32, u32, usize) -> u32>(d3d_virtual(d3d, 0x34))(d3d, adapter, device_type, caps)
}

// ---- list code that is inline in the engine functions

unsafe fn list_init(list: usize) {
    wr::<u32>(list, LIST_COUNT, 0);
    wr::<u32>(list, LIST_HEAD, 0);
    wr::<u32>(list, LIST_TAIL, 0);
    wr::<u32>(list, LIST_FREE, 0);
    wr::<u32>(list, LIST_BLOCKS, 0);
    wr::<u32>(list, LIST_BLOCK_ITEMS, 2);
}

/// A new block of items for a list with none free, made with `NiTListAllocator::Data::ctor`.
unsafe fn list_grow(list: usize) {
    let mut block = eng::operator_new(BLOCK_SIZE);
    block = if block != 0 { eng::list_block_ctor(block, rd(list, LIST_BLOCK_ITEMS)) } else { 0 };
    wr::<usize>(block, BLOCK_NEXT, rd(list, LIST_BLOCKS));
    wr(list, LIST_BLOCKS, block);
    wr::<usize>(list, LIST_FREE, rd(block, BLOCK_ITEMS));
}

/// Takes a free item and puts it at the tail.
unsafe fn list_link_tail(list: usize, value: u32) {
    let item: usize = rd(list, LIST_FREE);
    wr::<usize>(list, LIST_FREE, rd(item, ITEM_NEXT));
    wr(item, ITEM_DATA, value);
    wr::<u32>(item, ITEM_NEXT, 0);
    wr::<usize>(item, ITEM_PREVIOUS, rd(list, LIST_TAIL));
    let tail: usize = rd(list, LIST_TAIL);
    if tail != 0 {
        wr(tail, ITEM_NEXT, item);
    } else {
        wr(list, LIST_HEAD, item);
    }
    wr(list, LIST_TAIL, item);
    wr::<u32>(list, LIST_COUNT, rd::<u32>(list, LIST_COUNT).wrapping_add(1));
}

/// Frees the item blocks of a list.
unsafe fn list_free_blocks(list: usize) {
    let mut block: usize = rd(list, LIST_BLOCKS);
    while block != 0 {
        let current = block;
        block = rd(current, BLOCK_NEXT);
        eng::delete(rd(current, BLOCK_ITEMS));
        eng::delete(current);
    }
}

unsafe fn list_zero(list: usize) {
    wr::<u32>(list, LIST_COUNT, 0);
    wr::<u32>(list, LIST_HEAD, 0);
    wr::<u32>(list, LIST_TAIL, 0);
    wr::<u32>(list, LIST_FREE, 0);
    wr::<u32>(list, LIST_BLOCKS, 0);
}

fn abs_difference(a: u32, b: u32) -> i32 {
    (a.wrapping_sub(b) as i32).wrapping_abs()
}

// ---- NiDX8DeviceDesc::ScreenFormatInfo

/// 0x6B1600 `NiDX8DeviceDesc::ScreenFormatInfo::ctor`. For each render target format the
/// device has with this screen format, the list of the depth formats that go with it.
pub unsafe fn screen_format_info_ctor(this: usize, d3d: usize, adapter: u32, device_type: u32, format: u32, fullscreen: u8, windowed: u8) -> usize {
    crate::shadow::touch(&screen_format_info_ctor_port::STATE);
    let map = this + SFI::validDepthStencils;
    wr(map, MAP_VTBL, eng::hash_map_vtbl());
    wr(map, MAP_BUCKET_COUNT, HASH_BUCKETS);
    wr::<u32>(map, MAP_COUNT, 0);
    let buckets = eng::operator_new(0x94);
    let bytes = rd::<u32>(map, MAP_BUCKET_COUNT) << 2;
    wr(map, MAP_BUCKETS, buckets);
    core::ptr::write_bytes(buckets as *mut u8, 0, bytes as usize);
    wr(this, SFI::format, format);
    wr(this, SFI::bFullscreen, fullscreen);
    wr(this, SFI::bWindowed, windowed);

    // Index 0 does nothing; 1 to 11 are D3DFMT_R8G8B8 (20) to D3DFMT_X4R4G4B4 (30).
    for index in 1u32..12 {
        let target = index + 19;
        if d3d_check_device_format(d3d, adapter, device_type, format, 1, 1, target) != 0 {
            continue;
        }
        let mut list = 0;
        let block = eng::operator_new(LIST_SIZE);
        if block != 0 {
            list_init(block);
            list = block;
        }
        for i in 0..DEPTH_FORMAT_COUNT {
            let depth: u32 = rd(eng::depth_formats(), i * 4);
            if d3d_check_device_format(d3d, adapter, device_type, format, 2, 1, depth) != 0 {
                continue;
            }
            if d3d_check_depth_stencil_match(d3d, adapter, device_type, format, target, depth) != 0 {
                continue;
            }
            if rd::<usize>(list, LIST_FREE) == 0 {
                list_grow(list);
            }
            list_link_tail(list, depth);
        }
        if rd::<u32>(list, LIST_COUNT) != 0 {
            let hash = map_hash_key(map, target) as usize;
            let mut node: usize = rd(rd::<usize>(map, MAP_BUCKETS), hash * 4);
            while node != 0 {
                if map_cmp_key(map, target, rd(node, NODE_KEY)) != 0 {
                    break;
                }
                node = rd(node, NODE_NEXT);
            }
            if node != 0 {
                map_dtor_pair(map, node);
                map_make_pair(map, node, target, list);
            } else {
                let node = eng::operator_new(NODE_SIZE);
                map_make_pair(map, node, target, list);
                wr::<usize>(node, NODE_NEXT, rd(rd::<usize>(map, MAP_BUCKETS), hash * 4));
                wr(rd::<usize>(map, MAP_BUCKETS), hash * 4, node);
                wr::<u32>(map, MAP_COUNT, rd::<u32>(map, MAP_COUNT).wrapping_add(1));
            }
        } else {
            list_free_blocks(list);
            list_zero(list);
            eng::delete(list);
        }
    }
    this
}

/// The depth and stencil bits of a depth format, as the search counts them.
fn depth_stencil_bits(format: u32) -> (u32, u32) {
    match format {
        70 | 80 => (16, 0), // D3DFMT_D16_LOCKABLE, D3DFMT_D16
        71 => (32, 0),      // D3DFMT_D32
        73 => (15, 1),      // D3DFMT_D15S1
        75 => (24, 8),      // D3DFMT_D24S8
        77 => (24, 0),      // D3DFMT_D24X8
        79 => (24, 4),      // D3DFMT_D24X4S4
        _ => (0, 0),
    }
}

/// 0x6B1940 `NiDX8DeviceDesc::ScreenFormatInfo::findClosestDepthStencilFormat`. Returns the
/// depth format for the render target `format`, or 0.
pub unsafe fn screen_format_info_find_closest_depth_stencil_format(this: usize, format: u32, depth_bits: u32, stencil_bits: u32) -> u32 {
    crate::shadow::touch(&screen_format_info_find_closest_depth_stencil_format_port::STATE);
    let map = this + SFI::validDepthStencils;
    let hash = map_hash_key(map, format) as usize;
    let mut node: usize = rd(rd::<usize>(map, MAP_BUCKETS), hash * 4);
    if node == 0 {
        return 0;
    }
    while map_cmp_key(map, format, rd(node, NODE_KEY)) == 0 {
        node = rd(node, NODE_NEXT);
        if node == 0 {
            return 0;
        }
    }
    let list: usize = rd(node, NODE_VALUE);
    let mut best_format = 0u32;
    let mut best_depth = 0u32;
    let mut best_stencil = 0u32;
    let mut item: usize = rd(list, LIST_HEAD);
    while item != 0 {
        let candidate: u32 = rd(item, ITEM_DATA);
        item = rd(item, ITEM_NEXT);
        let (depth, stencil) = depth_stencil_bits(candidate);
        if abs_difference(best_depth, depth_bits) < abs_difference(depth, depth_bits) {
            continue;
        }
        if abs_difference(best_stencil, stencil_bits) < abs_difference(stencil, stencil_bits) {
            continue;
        }
        best_depth = depth;
        best_stencil = stencil;
        best_format = candidate;
    }
    best_format
}

// ---- NiDX8DeviceDesc

/// 0x6B1AB0 `NiDX8DeviceDesc::ctor`
pub unsafe fn device_desc_ctor(this: usize, d3d: usize, adapter: u32, device_type: u32, current_display_format: u32) -> usize {
    crate::shadow::touch(&device_desc_ctor_port::STATE);
    let list = this + DD::screenFormats;
    list_init(list);
    let caps = this + DD::caps;
    wr(this, DD::deviceType, device_type);
    if d3d_get_device_caps(d3d, adapter, device_type, caps) != 0 {
        wr::<u32>(this, DD::caps_DeviceType, 0);
    } else {
        let mode_count = d3d_get_adapter_mode_count(d3d, adapter);
        let mut mode = [0u32; 4];
        let mut index = 0u32;
        while index < mode_count {
            let mut fullscreen = 0u8;
            let mut windowed = 0u8;
            if d3d_enum_adapter_modes(d3d, adapter, index, mode.as_mut_ptr() as usize) == 0 {
                d3d_check_device_type(d3d, adapter, rd(this, DD::deviceType), mode[MODE_FORMAT], mode[MODE_FORMAT], 0);
                fullscreen = 1;
            }
            if d3d_check_device_type(d3d, adapter, rd(this, DD::deviceType), current_display_format, mode[MODE_FORMAT], 1) == 0
                && rd::<u32>(this, DD::caps_Caps2) & CAPS2_CAN_RENDER_WINDOWED != 0
            {
                windowed = 1;
            }
            if (fullscreen != 0 || windowed != 0) && !device_desc_has_format(this, mode[MODE_FORMAT]) {
                let block = eng::operator_new(SFI::SIZE);
                let info = if block != 0 {
                    screen_format_info_ctor(block, d3d, adapter, rd(this, DD::deviceType), mode[MODE_FORMAT], fullscreen, windowed)
                } else {
                    0
                };
                if rd::<usize>(list, LIST_FREE) == 0 {
                    // The constructor of the item block is inline here.
                    let mut block = eng::operator_new(BLOCK_SIZE);
                    if block != 0 {
                        let count: u32 = rd(list, LIST_BLOCK_ITEMS);
                        wr::<u32>(block, BLOCK_NEXT, 0);
                        let bytes = count.wrapping_mul(3) << 2;
                        let items = eng::operator_new(bytes as usize);
                        wr(block, BLOCK_ITEMS, items);
                        let mut left = count.wrapping_sub(1);
                        let mut offset = 0usize;
                        while left != 0 {
                            let items: usize = rd(block, BLOCK_ITEMS);
                            wr(items, offset + ITEM_NEXT, items + offset + ITEM_SIZE);
                            offset += ITEM_SIZE;
                            left -= 1;
                        }
                        wr::<u32>(rd::<usize>(block, BLOCK_ITEMS), bytes as usize - 8, 0);
                    } else {
                        block = 0;
                    }
                    wr::<usize>(block, BLOCK_NEXT, rd(list, LIST_BLOCKS));
                    wr(list, LIST_BLOCKS, block);
                    wr::<usize>(list, LIST_FREE, rd(block, BLOCK_ITEMS));
                }
                list_link_tail(list, info as u32);
            }
            index += 1;
        }
    }

    let mut render_windowed = 0u8;
    let mut item: usize = rd(list, LIST_HEAD);
    while item != 0 {
        let info: usize = rd(item, ITEM_DATA);
        item = rd(item, ITEM_NEXT);
        if rd::<u32>(info, SFI::format) == current_display_format {
            if rd::<u8>(info, SFI::bWindowed) != 0 {
                render_windowed = 1;
            }
            break;
        }
    }
    wr(this, DD::renderWindowed, render_windowed);
    this
}

/// The search for a screen format that is inline in the constructor.
unsafe fn device_desc_has_format(this: usize, format: u32) -> bool {
    let mut item: usize = rd(this, DD::screenFormats_head);
    while item != 0 {
        let info: usize = rd(item, ITEM_DATA);
        item = rd(item, ITEM_NEXT);
        if info != 0 && rd::<u32>(info, SFI::format) == format {
            return true;
        }
    }
    false
}

/// 0x6B1D60 `NiDX8DeviceDesc::dtor`
pub unsafe fn device_desc_dtor(this: usize) {
    crate::shadow::touch(&device_desc_dtor_port::STATE);
    let mut item: usize = rd(this, DD::screenFormats_head);
    while item != 0 {
        let info: usize = rd(item, ITEM_DATA);
        item = rd(item, ITEM_NEXT);
        if info == 0 {
            continue;
        }
        let map = info + SFI::validDepthStencils;
        let bucket_count: u32 = rd(map, MAP_BUCKET_COUNT);
        // The first node of the map.
        let mut node = 0usize;
        if bucket_count != 0 {
            let buckets: usize = rd(map, MAP_BUCKETS);
            let mut index = 0u32;
            loop {
                if rd::<usize>(buckets, index as usize * 4) != 0 {
                    node = rd(buckets, index as usize * 4);
                    break;
                }
                index += 1;
                if index >= bucket_count {
                    break;
                }
            }
        }
        while node != 0 {
            let chained: usize = rd(node, NODE_NEXT);
            let depth_list: usize = rd(node, NODE_VALUE);
            let next;
            if chained != 0 {
                next = chained;
            } else {
                // The first node of the buckets after this one.
                let mut index = map_hash_key(map, rd(node, NODE_KEY)).wrapping_add(1);
                let count: u32 = rd(map, MAP_BUCKET_COUNT);
                let mut found = 0usize;
                if index < count {
                    let buckets: usize = rd(map, MAP_BUCKETS);
                    loop {
                        found = rd(buckets, index as usize * 4);
                        if found != 0 {
                            break;
                        }
                        index += 1;
                        if index >= count {
                            break;
                        }
                    }
                }
                next = found;
            }
            if depth_list != 0 {
                eng::list_clear(depth_list);
                eng::delete(depth_list);
            }
            node = next;
        }
        wr(map, MAP_VTBL, eng::hash_map_vtbl());
        eng::hash_map_clear(map);
        eng::delete(rd(map, MAP_BUCKETS));
        eng::delete(info);
    }
    let list = this + DD::screenFormats;
    list_free_blocks(list);
    list_zero(list);
}

/// The index of a screen format in the renderer's own list: 1 for D3DFMT_R8G8B8 (20) to
/// 11 for D3DFMT_X4R4G4B4 (30), 0 for the rest.
fn screen_format_enum(format: u32) -> u32 {
    if (20..=30).contains(&format) {
        format - 19
    } else {
        0
    }
}

/// 0x6B1EC0 `NiDX8DeviceDesc::getScreenFormatEnum`
pub unsafe fn device_desc_get_screen_format_enum(this: usize, is_windowed: u8, bit_depth: u32) -> u32 {
    crate::shadow::touch(&device_desc_get_screen_format_enum_port::STATE);
    let mut item: usize = rd(this, DD::screenFormats_head);
    if is_windowed != 0 {
        while item != 0 {
            let info: usize = rd(item, ITEM_DATA);
            item = rd(item, ITEM_NEXT);
            if info != 0 && rd::<u8>(info, SFI::bWindowed) != 0 {
                return screen_format_enum(rd(info, SFI::format));
            }
        }
        return 0;
    }
    while item != 0 {
        let info: usize = rd(item, ITEM_DATA);
        item = rd(item, ITEM_NEXT);
        if info == 0 || rd::<u8>(info, SFI::bFullscreen) == 0 {
            continue;
        }
        // The engine function has the code of GetBPPFromFormat inline here.
        if get_bpp_from_format(rd(info, SFI::format)) == bit_depth {
            return screen_format_enum(rd(info, SFI::format));
        }
    }
    0
}

/// 0x6B21D0 `NiDX8DeviceDesc::findClosestDepthStencilForScreenFormat`
pub unsafe fn device_desc_find_closest_depth_stencil_for_screen_format(
    this: usize,
    screen_format: u32,
    target_format: u32,
    depth_bits: u32,
    stencil_bits: u32,
) -> u32 {
    crate::shadow::touch(&device_desc_find_closest_depth_stencil_for_screen_format_port::STATE);
    let mut item: usize = rd(this, DD::screenFormats_head);
    while item != 0 {
        let info: usize = rd(item, ITEM_DATA);
        item = rd(item, ITEM_NEXT);
        if info == 0 || rd::<u32>(info, SFI::format) != screen_format {
            continue;
        }
        let mut depth = depth_bits;
        let mut stencil = stencil_bits;
        if stencil != 0 {
            if depth > 0x10 {
                depth = 0x18;
                if stencil != 1 {
                    stencil = 8;
                }
            } else if stencil == 1 {
                depth = 0xF;
            } else {
                stencil = 8;
                depth = 0x18;
            }
        }
        return screen_format_info_find_closest_depth_stencil_format(info, target_format, depth, stencil);
    }
    0
}

// ---- NiDX8AdapterDesc

/// 0x6B2260 `NiDX8AdapterDesc::ctor`
pub unsafe fn adapter_desc_ctor(this: usize, d3d: usize, adapter_index: u32) -> usize {
    crate::shadow::touch(&adapter_desc_ctor_port::STATE);
    let modes = this + AD::modeList;
    wr(modes, ARRAY_VTBL, MODE_ARRAY_VTBL);
    wr::<u32>(modes, ARRAY_STORAGE_COUNT, 1);
    wr::<u32>(modes, ARRAY_GROW_BY, 1);
    wr::<u32>(modes, ARRAY_END, 0);
    wr::<u32>(modes, ARRAY_FILLED, 0);
    wr(modes, ARRAY_STORAGE, eng::operator_new(4));
    wr(this, AD::index, adapter_index);
    d3d_get_adapter_identifier(d3d, adapter_index, 2, this + AD::d3dAdapterIdentifier);
    let mut current = [0u32; 4];
    d3d_get_adapter_display_mode(d3d, rd(this, AD::index), current.as_mut_ptr() as usize);
    let index: u32 = rd(this, AD::index);
    wr::<u32>(this, AD::modeListCount, 0);
    let mode_count = d3d_get_adapter_mode_count(d3d, index);

    let mut mode = [0u32; 4];
    let mut mode_index = 0u32;
    while mode_index < mode_count {
        d3d_enum_adapter_modes(d3d, rd(this, AD::index), mode_index, mode.as_mut_ptr() as usize);
        let height = mode[MODE_HEIGHT];
        let width = mode[MODE_WIDTH];
        let format = mode[MODE_FORMAT];
        let mut found = 0usize;
        let mut refresh = 0u32;
        if rd::<u32>(this, AD::modeListCount) != 0 {
            refresh = mode[MODE_REFRESH];
            let mut i = 0u32;
            loop {
                let desc: usize = rd(rd::<usize>(modes, ARRAY_STORAGE), i as usize * 4);
                if desc != 0 && rd::<u32>(desc, MD::d3dFormat) == format && rd::<u32>(desc, MD::width) == width && rd::<u32>(desc, MD::height) == height {
                    if refresh != 0x7FFF_FFFF && refresh != 0 {
                        let rates = desc + MD::listRefreshRates;
                        let mut iterator: usize = rd(desc, MD::listRefreshRates_head);
                        refresh = eng::list_advance_iterator(rates, &mut iterator as *mut usize as usize);
                        // The best difference starts at 0, so no later rate replaces the first.
                        let mut best = 0u32;
                        while iterator != 0 {
                            let rate = eng::list_advance_iterator(rates, &mut iterator as *mut usize as usize);
                            let difference = abs_difference(rate, refresh) as u32;
                            if difference < best {
                                best = difference;
                                refresh = rate;
                            }
                        }
                    }
                    found = desc;
                    mode[MODE_REFRESH] = refresh;
                }
                i += 1;
                if i >= rd::<u32>(this, AD::modeListCount) {
                    break;
                }
            }
        }
        if found != 0 {
            if refresh != 0 {
                // A new item at the head of the refresh rates.
                let rates = found + MD::listRefreshRates;
                if rd::<usize>(rates, LIST_FREE) == 0 {
                    list_grow(rates);
                }
                let item: usize = rd(rates, LIST_FREE);
                wr::<usize>(rates, LIST_FREE, rd(item, ITEM_NEXT));
                wr(item, ITEM_DATA, refresh);
                wr::<u32>(item, ITEM_PREVIOUS, 0);
                wr::<usize>(item, ITEM_NEXT, rd(rates, LIST_HEAD));
                let head: usize = rd(rates, LIST_HEAD);
                if head != 0 {
                    wr(head, ITEM_PREVIOUS, item);
                } else {
                    wr(rates, LIST_TAIL, item);
                }
                wr(rates, LIST_HEAD, item);
                wr::<u32>(rates, LIST_COUNT, rd::<u32>(rates, LIST_COUNT).wrapping_add(1));
            }
        } else {
            let mut desc = eng::operator_new(MD::SIZE);
            if desc != 0 {
                list_init(desc + MD::listRefreshRates);
                wr(desc, MD::width, mode[MODE_WIDTH]);
                wr(desc, MD::height, mode[MODE_HEIGHT]);
                eng::list_add_head(desc + MD::listRefreshRates, mode[MODE_REFRESH]);
                wr(desc, MD::bpp, get_bpp_from_format(mode[MODE_FORMAT]));
                wr(desc, MD::d3dFormat, mode[MODE_FORMAT]);
            } else {
                desc = 0;
            }
            let element: usize = desc;
            let count: u32 = rd(this, AD::modeListCount);
            if count >= rd::<u32>(modes, ARRAY_STORAGE_COUNT) {
                eng::array_set_size(modes, rd::<u32>(modes, ARRAY_GROW_BY).wrapping_add(count));
            }
            eng::array_set_at(modes, count, &element as *const usize as usize);
            wr::<u32>(this, AD::modeListCount, rd::<u32>(this, AD::modeListCount).wrapping_add(1));
        }
        mode_index += 1;
    }

    let block = eng::operator_new(DD::SIZE);
    let hal = if block != 0 { device_desc_ctor(block, d3d, adapter_index, 1, current[MODE_FORMAT]) } else { 0 };
    wr(this, AD::HALDeviceDesc, hal);
    let block = eng::operator_new(DD::SIZE);
    if block != 0 {
        wr(this, AD::REFDeviceDesc, device_desc_ctor(block, d3d, adapter_index, 2, current[MODE_FORMAT]));
    } else {
        wr::<u32>(this, AD::REFDeviceDesc, 0);
    }
    this
}

/// 0x6B25D0 `NiDX8AdapterDesc::findClosestMode`. Returns the `ModeDesc` or 0. A refresh rate
/// that is not 0 and not 0x7FFFFFFF is replaced with one the mode has.
pub unsafe fn adapter_desc_find_closest_mode(this: usize, format: u32, width: u32, height: u32, inout_refresh_rate: usize) -> usize {
    crate::shadow::touch(&adapter_desc_find_closest_mode_port::STATE);
    let mut result = 0usize;
    if rd::<u32>(this, AD::modeListCount) == 0 {
        return result;
    }
    let mut i = 0u32;
    loop {
        let desc: usize = rd(rd::<usize>(this, AD::modeList_storage), i as usize * 4);
        if desc != 0 && rd::<u32>(desc, MD::d3dFormat) == format && rd::<u32>(desc, MD::width) == width && rd::<u32>(desc, MD::height) == height {
            let mut refresh: u32 = rd(inout_refresh_rate, 0);
            if refresh != 0x7FFF_FFFF && refresh != 0 {
                let mut item: usize = rd(desc, MD::listRefreshRates_head);
                if item == 0 {
                    refresh = 0;
                } else {
                    refresh = rd(item, ITEM_DATA);
                    item = rd(item, ITEM_NEXT);
                }
                // The best difference starts at 0, so no later rate replaces the first.
                let mut best = 0u32;
                while item != 0 {
                    let rate: u32 = rd(item, ITEM_DATA);
                    item = rd(item, ITEM_NEXT);
                    let difference = abs_difference(rate, refresh) as u32;
                    if difference < best {
                        best = difference;
                        refresh = rate;
                    }
                }
            }
            wr(inout_refresh_rate, 0, refresh);
            result = desc;
        }
        i += 1;
        if i >= rd::<u32>(this, AD::modeListCount) {
            break;
        }
    }
    result
}

// ---- NiDX8SystemDesc

/// 0x6B26A0 `NiDX8SystemDesc::ctor`
pub unsafe fn system_desc_ctor(this: usize, d3d: usize) -> usize {
    crate::shadow::touch(&system_desc_ctor_port::STATE);
    let adapters = this + SD::adapterList;
    wr(adapters, ARRAY_VTBL, ADAPTER_ARRAY_VTBL);
    wr::<u32>(adapters, ARRAY_STORAGE_COUNT, 1);
    wr::<u32>(adapters, ARRAY_GROW_BY, 1);
    wr::<u32>(adapters, ARRAY_END, 0);
    wr::<u32>(adapters, ARRAY_FILLED, 0);
    wr(adapters, ARRAY_STORAGE, eng::operator_new(4));
    wr::<u32>(this, SD::d3dAdapterCount, 0);
    let count = d3d_get_adapter_count(d3d);
    wr(this, SD::d3dAdapterCount, count);

    // The SetSize of the array is inline here.
    if count != rd::<u32>(adapters, ARRAY_STORAGE_COUNT) {
        if count < rd::<u32>(adapters, ARRAY_END) {
            let mut i = count;
            loop {
                let slot = rd::<usize>(adapters, ARRAY_STORAGE) + i as usize * 4;
                if rd::<u32>(slot, 0) != 0 {
                    wr::<u32>(slot, 0, 0);
                    wr::<u32>(adapters, ARRAY_FILLED, rd::<u32>(adapters, ARRAY_FILLED).wrapping_sub(1));
                }
                i += 1;
                if i >= rd::<u32>(adapters, ARRAY_END) {
                    break;
                }
            }
            wr(adapters, ARRAY_END, count);
        }
        let old: usize = rd(adapters, ARRAY_STORAGE);
        wr(adapters, ARRAY_STORAGE_COUNT, count);
        let storage = eng::operator_new((count << 2) as usize);
        let end: u32 = rd(adapters, ARRAY_END);
        wr(adapters, ARRAY_STORAGE, storage);
        if end != 0 {
            let mut i = 0u32;
            loop {
                wr::<u32>(rd::<usize>(adapters, ARRAY_STORAGE), i as usize * 4, rd(old, i as usize * 4));
                i += 1;
                if i >= rd::<u32>(adapters, ARRAY_END) {
                    break;
                }
            }
        }
        eng::delete(old);
    }

    let mut index = 0u32;
    while index < rd::<u32>(this, SD::d3dAdapterCount) {
        let block = eng::operator_new(AD::SIZE);
        let adapter = if block != 0 { adapter_desc_ctor(block, d3d, index) } else { 0 };
        // The SetAt of the array is inline here.
        if index < rd::<u32>(adapters, ARRAY_STORAGE_COUNT) {
            let filled: u32 = rd(adapters, ARRAY_FILLED);
            if index >= rd::<u32>(adapters, ARRAY_END) {
                wr(adapters, ARRAY_END, index + 1);
                if adapter != 0 {
                    wr(adapters, ARRAY_FILLED, filled.wrapping_add(1));
                }
            } else if adapter != 0 {
                if rd::<u32>(rd::<usize>(adapters, ARRAY_STORAGE), index as usize * 4) == 0 {
                    wr(adapters, ARRAY_FILLED, filled.wrapping_add(1));
                }
            } else if rd::<u32>(rd::<usize>(adapters, ARRAY_STORAGE), index as usize * 4) != 0 {
                wr(adapters, ARRAY_FILLED, filled.wrapping_sub(1));
            }
            wr(rd::<usize>(adapters, ARRAY_STORAGE), index as usize * 4, adapter);
        }
        index += 1;
    }
    this
}

/// 0x6B2800 `NiDX8SystemDesc::dtor`
pub unsafe fn system_desc_dtor(this: usize) {
    crate::shadow::touch(&system_desc_dtor_port::STATE);
    let adapters = this + SD::adapterList;
    let mut index = 0u32;
    while index < rd::<u32>(adapters, ARRAY_END) {
        let adapter: usize = rd(rd::<usize>(adapters, ARRAY_STORAGE), index as usize * 4);
        if adapter != 0 {
            let mut i = 0u32;
            while i < rd::<u32>(adapter, AD::modeList_endIndex) {
                let desc: usize = rd(rd::<usize>(adapter, AD::modeList_storage), i as usize * 4);
                if desc != 0 {
                    eng::list_clear(desc + MD::listRefreshRates);
                    eng::delete(desc);
                }
                i += 1;
            }
            let hal: usize = rd(adapter, AD::HALDeviceDesc);
            if hal != 0 {
                device_desc_dtor(hal);
                eng::delete(hal);
            }
            let reference: usize = rd(adapter, AD::REFDeviceDesc);
            if reference != 0 {
                device_desc_dtor(reference);
                eng::delete(reference);
            }
            let storage: usize = rd(adapter, AD::modeList_storage);
            wr(adapter, AD::modeList_vtbl, MODE_ARRAY_VTBL);
            eng::delete(storage);
            eng::delete(adapter);
        }
        index += 1;
    }
    let storage: usize = rd(adapters, ARRAY_STORAGE);
    wr(adapters, ARRAY_VTBL, ADAPTER_ARRAY_VTBL);
    eng::delete(storage);
}

/// 0x6B2B80 `NiDX8SystemDesc_static::GetBPPFromFormat`. Bits per pixel of a `D3DFORMAT`:
/// 0 for a format without a fixed size, 0xFFFFFFFF for one the function does not know.
pub unsafe fn get_bpp_from_format(format: u32) -> u32 {
    crate::shadow::touch(&get_bpp_from_format_port::STATE);
    const UNKNOWN: u32 = 0xFFFF_FFFF;
    match format {
        0 => 0,
        20 => 24,
        21 | 22 | 62..=65 | 71 => 32,
        23..=26 | 29 | 30 | 40 | 51 | 60 | 61 | 70 | 73 => 16,
        27 | 28 | 41 | 50 | 52 => 8,
        75 | 77 | 79 | 102 => 32,
        80 | 101 => 16,
        100 => 0,
        // DXT1 to DXT5, YUY2, UYVY
        0x3154_5844 | 0x3254_5844 | 0x3354_5844 | 0x3454_5844 | 0x3554_5844 | 0x3259_5559 | 0x5956_5955 => 0,
        _ => UNKNOWN,
    }
}

// ---- regions

/// The destructor frees the screen formats, their maps and their depth lists, and it clears
/// them first. A call with screen formats needs a region for each of these blocks, so that
/// the engine run finds them as they were: more than the 8 there are. Such a call is not
/// compared. A description without screen formats (the device has no caps) is compared.
unsafe fn device_desc_dtor_regions(set: &mut crate::shadow::Regions, this: usize) {
    set.add(this, DD::SIZE);
    if rd::<usize>(this, DD::screenFormats_head) != 0 {
        return set.skip();
    }
    set.allocates();
}

/// As for the device description: the destructor clears the refresh rate list of every mode
/// and the screen formats of both devices before it frees them. A call with an adapter is
/// not compared; an empty adapter list is.
unsafe fn system_desc_dtor_regions(set: &mut crate::shadow::Regions, this: usize) {
    set.add(this, SD::SIZE);
    let storage: usize = rd(this, SD::adapterList_storage);
    for i in 0..rd::<u32>(this, SD::adapterList_endIndex) as usize {
        if rd::<usize>(storage, i * 4) != 0 {
            return set.skip();
        }
    }
    set.allocates();
}

unsafe fn ctor_regions(set: &mut crate::shadow::Regions, this: usize, size: usize) {
    // The object is the caller's; everything else the constructor writes is new.
    set.add(this, size);
    set.allocates();
}

port_thiscall!(screen_format_info_ctor_port, 0x6B1600, "NiDX8DeviceDesc::ScreenFormatInfo::ctor",
    fn(this, d3d: usize, adapter: u32, device_type: u32, format: u32, fullscreen: u8, windowed: u8) -> usize,
    compare_eax = true,
    regions = |set| ctor_regions(set, this, SFI::SIZE),
    rust = screen_format_info_ctor);

port_thiscall!(screen_format_info_find_closest_depth_stencil_format_port, 0x6B1940,
    "NiDX8DeviceDesc::ScreenFormatInfo::findClosestDepthStencilFormat",
    fn(this, format: u32, depth_bits: u32, stencil_bits: u32) -> u32,
    compare_eax = true,
    regions = |set| {},
    rust = screen_format_info_find_closest_depth_stencil_format);

port_thiscall!(device_desc_ctor_port, 0x6B1AB0, "NiDX8DeviceDesc::ctor",
    fn(this, d3d: usize, adapter: u32, device_type: u32, current_display_format: u32) -> usize,
    compare_eax = true,
    regions = |set| ctor_regions(set, this, DD::SIZE),
    rust = device_desc_ctor);

port_thiscall!(device_desc_dtor_port, 0x6B1D60, "NiDX8DeviceDesc::dtor",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| device_desc_dtor_regions(set, this),
    rust = device_desc_dtor);

port_thiscall!(device_desc_get_screen_format_enum_port, 0x6B1EC0, "NiDX8DeviceDesc::getScreenFormatEnum",
    fn(this, is_windowed: u8, bit_depth: u32) -> u32,
    compare_eax = true,
    regions = |set| {},
    rust = device_desc_get_screen_format_enum);

port_thiscall!(device_desc_find_closest_depth_stencil_for_screen_format_port, 0x6B21D0,
    "NiDX8DeviceDesc::findClosestDepthStencilForScreenFormat",
    fn(this, screen_format: u32, target_format: u32, depth_bits: u32, stencil_bits: u32) -> u32,
    compare_eax = true,
    regions = |set| {},
    rust = device_desc_find_closest_depth_stencil_for_screen_format);

port_thiscall!(adapter_desc_ctor_port, 0x6B2260, "NiDX8AdapterDesc::ctor",
    fn(this, d3d: usize, adapter_index: u32) -> usize,
    compare_eax = true,
    regions = |set| ctor_regions(set, this, AD::SIZE),
    rust = adapter_desc_ctor);

port_thiscall!(adapter_desc_find_closest_mode_port, 0x6B25D0, "NiDX8AdapterDesc::findClosestMode",
    fn(this, format: u32, width: u32, height: u32, inout_refresh_rate: usize) -> usize,
    compare_eax = true,
    regions = |set| set.add(inout_refresh_rate, 4),
    rust = adapter_desc_find_closest_mode);

port_thiscall!(system_desc_ctor_port, 0x6B26A0, "NiDX8SystemDesc::ctor",
    fn(this, d3d: usize) -> usize,
    compare_eax = true,
    regions = |set| ctor_regions(set, this, SD::SIZE),
    rust = system_desc_ctor);

port_thiscall!(system_desc_dtor_port, 0x6B2800, "NiDX8SystemDesc::dtor",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| system_desc_dtor_regions(set, this),
    rust = system_desc_dtor);

port_cdecl!(get_bpp_from_format_port, 0x6B2B80, "NiDX8SystemDesc_static::GetBPPFromFormat",
    fn(format: u32) -> u32,
    compare_eax = true,
    regions = |set| {},
    rust = get_bpp_from_format);

pub static PORTS: &[&crate::shadow::PortState] = &[
    &screen_format_info_ctor_port::STATE,
    &screen_format_info_find_closest_depth_stencil_format_port::STATE,
    &device_desc_ctor_port::STATE,
    &device_desc_dtor_port::STATE,
    &device_desc_get_screen_format_enum_port::STATE,
    &device_desc_find_closest_depth_stencil_for_screen_format_port::STATE,
    &adapter_desc_ctor_port::STATE,
    &adapter_desc_find_closest_mode_port::STATE,
    &system_desc_ctor_port::STATE,
    &system_desc_dtor_port::STATE,
    &get_bpp_from_format_port::STATE,
];

#[cfg(test)]
mod tests {
    use super::eng::{Heap, HEAP_LOG};
    use super::*;
    use std::cell::RefCell;

    /// An `IDirect3D8` that answers from a table and keeps a log of the calls.
    #[repr(C)]
    struct FakeD3d {
        vtbl: *const usize,
        table: Box<[usize; 16]>,
        adapters: u32,
        /// width, height, refresh rate, format
        modes: Vec<[u32; 4]>,
        current: [u32; 4],
        /// Formats the device can render to.
        targets: Vec<u32>,
        /// Depth formats the device has.
        depths: Vec<u32>,
        /// (target, depth) pairs that do not go together.
        no_match: Vec<(u32, u32)>,
        caps2: u32,
        calls: RefCell<Vec<String>>,
    }

    impl FakeD3d {
        fn new() -> Box<FakeD3d> {
            let mut table = Box::new([0usize; 16]);
            table[0x10 / 4] = get_adapter_count as *const () as usize;
            table[0x14 / 4] = get_adapter_identifier as *const () as usize;
            table[0x18 / 4] = get_adapter_mode_count as *const () as usize;
            table[0x1C / 4] = enum_adapter_modes as *const () as usize;
            table[0x20 / 4] = get_adapter_display_mode as *const () as usize;
            table[0x24 / 4] = check_device_type as *const () as usize;
            table[0x28 / 4] = check_device_format as *const () as usize;
            table[0x30 / 4] = check_depth_stencil_match as *const () as usize;
            table[0x34 / 4] = get_device_caps as *const () as usize;
            let mut d3d = Box::new(FakeD3d {
                vtbl: core::ptr::null(),
                table,
                adapters: 1,
                modes: Vec::new(),
                current: [1024, 768, 60, 22],
                targets: vec![22, 23],
                depths: vec![80, 75, 77],
                no_match: Vec::new(),
                caps2: CAPS2_CAN_RENDER_WINDOWED,
                calls: RefCell::new(Vec::new()),
            });
            d3d.vtbl = d3d.table.as_ptr();
            d3d
        }
        fn addr(&self) -> usize {
            self as *const FakeD3d as usize
        }
        fn log(&self, text: String) {
            self.calls.borrow_mut().push(text);
        }
        fn count(&self, prefix: &str) -> usize {
            self.calls.borrow().iter().filter(|c| c.starts_with(prefix)).count()
        }
    }

    unsafe fn fake<'a>(this: usize) -> &'a FakeD3d {
        &*(this as *const FakeD3d)
    }

    unsafe extern "system" fn get_adapter_count(this: usize) -> u32 {
        fake(this).log("GetAdapterCount".into());
        fake(this).adapters
    }
    unsafe extern "system" fn get_adapter_identifier(this: usize, adapter: u32, flags: u32, out: usize) -> u32 {
        fake(this).log(format!("GetAdapterIdentifier {adapter} {flags}"));
        core::ptr::write_bytes(out as *mut u8, 0, 0x42C);
        wr::<u32>(out, 0x408, 0x1234 + adapter); // VendorId
        0
    }
    unsafe extern "system" fn get_adapter_mode_count(this: usize, adapter: u32) -> u32 {
        fake(this).log(format!("GetAdapterModeCount {adapter}"));
        fake(this).modes.len() as u32
    }
    unsafe extern "system" fn enum_adapter_modes(this: usize, adapter: u32, mode: u32, out: usize) -> u32 {
        fake(this).log(format!("EnumAdapterModes {adapter} {mode}"));
        wr(out, 0, fake(this).modes[mode as usize]);
        0
    }
    unsafe extern "system" fn get_adapter_display_mode(this: usize, adapter: u32, out: usize) -> u32 {
        fake(this).log(format!("GetAdapterDisplayMode {adapter}"));
        wr(out, 0, fake(this).current);
        0
    }
    unsafe extern "system" fn check_device_type(this: usize, adapter: u32, device_type: u32, display: u32, back_buffer: u32, windowed: u32) -> u32 {
        fake(this).log(format!("CheckDeviceType {adapter} {device_type} {display} {back_buffer} {windowed}"));
        0
    }
    unsafe extern "system" fn check_device_format(this: usize, adapter: u32, device_type: u32, adapter_format: u32, usage: u32, resource: u32, format: u32) -> u32 {
        let d3d = fake(this);
        d3d.log(format!("CheckDeviceFormat {adapter} {device_type} {adapter_format} {usage} {resource} {format}"));
        let known = if usage == 1 { d3d.targets.contains(&format) } else { d3d.depths.contains(&format) };
        if known {
            0
        } else {
            0x8876_086A
        }
    }
    unsafe extern "system" fn check_depth_stencil_match(this: usize, adapter: u32, device_type: u32, adapter_format: u32, target: u32, depth: u32) -> u32 {
        let d3d = fake(this);
        d3d.log(format!("CheckDepthStencilMatch {adapter} {device_type} {adapter_format} {target} {depth}"));
        if d3d.no_match.contains(&(target, depth)) {
            0x8876_086A
        } else {
            0
        }
    }
    /// The reference device (type 2) has no caps.
    unsafe extern "system" fn get_device_caps(this: usize, adapter: u32, device_type: u32, caps: usize) -> u32 {
        fake(this).log(format!("GetDeviceCaps {adapter} {device_type}"));
        if device_type == 2 {
            return 0x8876_086A;
        }
        core::ptr::write_bytes(caps as *mut u8, 0, 0xD4);
        wr(caps, 0x0, device_type);
        wr(caps, 0xC, fake(this).caps2);
        0
    }

    fn heap_log() -> Vec<Heap> {
        HEAP_LOG.with(|log| log.borrow().clone())
    }
    fn heap_reset() {
        HEAP_LOG.with(|log| log.borrow_mut().clear());
    }
    /// The blocks that were allocated and not freed.
    fn live_blocks() -> Vec<usize> {
        let mut live = Vec::new();
        for call in heap_log() {
            match call {
                Heap::New(block, _) => live.push(block),
                Heap::Delete(block) => {
                    let at = live.iter().position(|&b| b == block).expect("a block that was not allocated is freed");
                    live.remove(at);
                }
            }
        }
        live
    }
    unsafe fn list_values(list: usize) -> Vec<u32> {
        let mut values = Vec::new();
        let mut item: usize = rd(list, LIST_HEAD);
        while item != 0 {
            values.push(rd(item, ITEM_DATA));
            item = rd(item, ITEM_NEXT);
        }
        assert_eq!(values.len(), rd::<u32>(list, LIST_COUNT) as usize);
        values
    }

    #[test]
    fn bpp_of_the_formats() {
        unsafe {
            assert_eq!(get_bpp_from_format(0), 0);
            assert_eq!(get_bpp_from_format(20), 24);
            for format in [21, 22, 62, 63, 64, 65, 71, 75, 77, 79, 102] {
                assert_eq!(get_bpp_from_format(format), 32, "{format}");
            }
            for format in [23, 24, 25, 26, 29, 30, 40, 51, 60, 61, 70, 73, 80, 101] {
                assert_eq!(get_bpp_from_format(format), 16, "{format}");
            }
            for format in [27, 28, 41, 50, 52] {
                assert_eq!(get_bpp_from_format(format), 8, "{format}");
            }
            for format in [0x31545844, 0x32545844, 0x33545844, 0x34545844, 0x35545844, 0x32595559, 0x59565955, 100] {
                assert_eq!(get_bpp_from_format(format), 0, "{format:#x}");
            }
            for format in [1, 19, 31, 42, 53, 66, 72, 74, 76, 78, 81, 99, 103, 0x31545845, 0x36545844, 0x8000_0000, 0xFFFF_FFFF] {
                assert_eq!(get_bpp_from_format(format), 0xFFFF_FFFF, "{format:#x}");
            }
        }
    }

    #[test]
    fn screen_format_info_keeps_the_depth_formats_of_each_target() {
        heap_reset();
        let mut d3d = FakeD3d::new();
        d3d.targets = vec![22, 23, 25];
        d3d.no_match = vec![(23, 80), (25, 80), (25, 75), (25, 77)];
        let mut info = [0u8; SFI::SIZE];
        let this = info.as_mut_ptr() as usize;
        unsafe {
            assert_eq!(screen_format_info_ctor(this, d3d.addr(), 0, 1, 22, 1, 0), this);
            assert_eq!(rd::<u32>(this, SFI::format), 22);
            assert_eq!(rd::<u8>(this, SFI::bFullscreen), 1);
            assert_eq!(rd::<u8>(this, SFI::bWindowed), 0);
            // Target 25 has no depth format: its list is freed again and it gets no node.
            assert_eq!(rd::<u32>(this, SFI::validDepthStencils_count), 2);
            let buckets: usize = rd(this, SFI::validDepthStencils_arrayBuckets);
            let node: usize = rd(buckets, 22 * 4);
            assert_eq!(rd::<u32>(node, NODE_KEY), 22);
            // In the order of the engine's table of depth formats: 70 71 73 75 80 77 79.
            assert_eq!(list_values(rd(node, NODE_VALUE)), [75, 80, 77]);
            let node: usize = rd(buckets, 23 * 4);
            assert_eq!(list_values(rd(node, NODE_VALUE)), [75, 77]);
            assert_eq!(rd::<usize>(buckets, 25 * 4), 0);

            // 11 target formats, then 7 depth formats for each of the 3 the device has.
            assert_eq!(d3d.count("CheckDeviceFormat 0 1 22 1 1 "), 11);
            assert_eq!(d3d.count("CheckDeviceFormat 0 1 22 2 1 "), 21);
            assert_eq!(d3d.count("CheckDepthStencilMatch"), 9);
            assert_eq!(d3d.calls.borrow()[0], "CheckDeviceFormat 0 1 22 1 1 20");
            assert_eq!(d3d.calls.borrow()[3], "CheckDeviceFormat 0 1 22 2 1 70");

            // The heap calls of target 22: buckets, list, item block (2 items), node, and a
            // second item block for the third depth format.
            let sizes: Vec<usize> = heap_log().iter().filter_map(|c| if let Heap::New(_, size) = c { Some(*size) } else { None }).collect();
            assert_eq!(&sizes[..8], [0x94, 0x18, 8, 24, 8, 24, 0xC, 0x18]);
            // Target 25: only its list, freed with no item block.
            assert_eq!(sizes.len(), 12);
            assert_eq!(heap_log().iter().filter(|c| matches!(c, Heap::Delete(_))).count(), 1);

            assert_eq!(screen_format_info_find_closest_depth_stencil_format(this, 22, 24, 8), 75);
            assert_eq!(screen_format_info_find_closest_depth_stencil_format(this, 22, 16, 0), 80);
            assert_eq!(screen_format_info_find_closest_depth_stencil_format(this, 22, 24, 0), 77);
            // Target 23 has no 16 bit depth format.
            assert_eq!(screen_format_info_find_closest_depth_stencil_format(this, 23, 16, 0), 77);
            assert_eq!(screen_format_info_find_closest_depth_stencil_format(this, 23, 24, 8), 75);
            assert_eq!(screen_format_info_find_closest_depth_stencil_format(this, 25, 24, 8), 0);
            assert_eq!(screen_format_info_find_closest_depth_stencil_format(this, 22 + 37, 24, 8), 0);
        }
    }

    #[test]
    fn system_desc_builds_and_frees_the_whole_tree() {
        heap_reset();
        let mut d3d = FakeD3d::new();
        d3d.adapters = 2;
        d3d.modes = vec![[640, 480, 60, 23], [640, 480, 75, 23], [800, 600, 0, 23], [800, 600, 85, 23], [640, 480, 60, 22], [640, 480, 72, 23]];
        let mut system = [0u8; SD::SIZE];
        let this = system.as_mut_ptr() as usize;
        unsafe {
            assert_eq!(system_desc_ctor(this, d3d.addr()), this);
            assert_eq!(rd::<u32>(this, SD::d3dAdapterCount), 2);
            assert_eq!(rd::<u32>(this, SD::adapterList_vtbl), ADAPTER_ARRAY_VTBL as u32);
            assert_eq!(rd::<u32>(this, SD::adapterList_storageCount), 2);
            assert_eq!(rd::<u32>(this, SD::adapterList_endIndex), 2);
            assert_eq!(rd::<u32>(this, SD::adapterList_filledCount), 2);
            // The array of one adapter is replaced with one of two before the adapters are made.
            let log = heap_log();
            assert!(matches!(log[0], Heap::New(_, 4)));
            assert!(matches!(log[1], Heap::New(_, 8)));
            assert!(matches!((log[0], log[2]), (Heap::New(a, _), Heap::Delete(b)) if a == b));
            assert!(matches!(log[3], Heap::New(_, size) if size == AD::SIZE));

            let storage: usize = rd(this, SD::adapterList_storage);
            let adapter: usize = rd(storage, 4);
            assert_eq!(rd::<u32>(adapter, AD::index), 1);
            assert_eq!(rd::<u32>(adapter, AD::d3dAdapterIdentifier + 0x408), 0x1235);
            assert_eq!(d3d.count("GetAdapterIdentifier 1 2"), 1);

            // Three modes; a mode that comes again adds the first refresh rate of the list
            // again, not its own (the search starts with a best difference of 0).
            assert_eq!(rd::<u32>(adapter, AD::modeListCount), 3);
            assert_eq!(rd::<u32>(adapter, AD::modeList_endIndex), 3);
            assert_eq!(rd::<u32>(adapter, AD::modeList_filledCount), 3);
            let modes: usize = rd(adapter, AD::modeList_storage);
            let mode: usize = rd(modes, 0);
            assert_eq!(rd::<[u32; 4]>(mode, 0), [640, 480, 16, 23]);
            assert_eq!(list_values(mode + MD::listRefreshRates), [60, 60, 60]);
            let mode: usize = rd(modes, 4);
            assert_eq!(rd::<[u32; 4]>(mode, 0), [800, 600, 16, 23]);
            assert_eq!(list_values(mode + MD::listRefreshRates), [0]);
            let mode: usize = rd(modes, 8);
            assert_eq!(rd::<[u32; 4]>(mode, 0), [640, 480, 32, 22]);
            assert_eq!(list_values(mode + MD::listRefreshRates), [60]);

            let mut refresh = 75u32;
            let found = adapter_desc_find_closest_mode(adapter, 23, 640, 480, &mut refresh as *mut u32 as usize);
            assert_eq!((found, refresh), (rd::<usize>(modes, 0), 60));
            refresh = 0x7FFF_FFFF;
            assert_eq!(adapter_desc_find_closest_mode(adapter, 22, 640, 480, &mut refresh as *mut u32 as usize), rd::<usize>(modes, 8));
            assert_eq!(refresh, 0x7FFF_FFFF);
            refresh = 85;
            assert_eq!(adapter_desc_find_closest_mode(adapter, 22, 800, 600, &mut refresh as *mut u32 as usize), 0);
            assert_eq!(refresh, 85);

            // The HAL device: one screen format for each display format of the modes.
            let hal: usize = rd(adapter, AD::HALDeviceDesc);
            assert_eq!(rd::<u32>(hal, DD::deviceType), 1);
            assert_eq!(rd::<u32>(hal, DD::caps_Caps2), CAPS2_CAN_RENDER_WINDOWED);
            let formats = list_values(hal + DD::screenFormats);
            assert_eq!(formats.len(), 2);
            assert_eq!(rd::<u32>(formats[0] as usize, SFI::format), 23);
            assert_eq!(rd::<u32>(formats[1] as usize, SFI::format), 22);
            assert_eq!(rd::<u8>(formats[1] as usize, SFI::bFullscreen), 1);
            assert_eq!(rd::<u8>(formats[1] as usize, SFI::bWindowed), 1);
            // The current display format is 22 and it can be windowed.
            assert_eq!(rd::<u8>(hal, DD::renderWindowed), 1);
            assert_eq!(d3d.count("CheckDeviceType 1 1 23 23 0"), 5);
            assert_eq!(d3d.count("CheckDeviceType 1 1 22 23 1"), 5);

            assert_eq!(device_desc_get_screen_format_enum(hal, 1, 0), 4);
            assert_eq!(device_desc_get_screen_format_enum(hal, 0, 32), 3);
            assert_eq!(device_desc_get_screen_format_enum(hal, 0, 16), 4);
            assert_eq!(device_desc_get_screen_format_enum(hal, 0, 24), 0);
            assert_eq!(device_desc_find_closest_depth_stencil_for_screen_format(hal, 22, 22, 32, 0), 77);
            assert_eq!(device_desc_find_closest_depth_stencil_for_screen_format(hal, 22, 22, 16, 0), 80);
            // With stencil the request becomes 24/8, or 15/1 for one bit with 16 bit depth.
            assert_eq!(device_desc_find_closest_depth_stencil_for_screen_format(hal, 22, 23, 16, 4), 75);
            assert_eq!(device_desc_find_closest_depth_stencil_for_screen_format(hal, 22, 23, 32, 1), 77);
            assert_eq!(device_desc_find_closest_depth_stencil_for_screen_format(hal, 22, 23, 16, 1), 80);
            assert_eq!(device_desc_find_closest_depth_stencil_for_screen_format(hal, 21, 22, 32, 0), 0);

            // The reference device has no caps: no screen formats.
            let reference: usize = rd(adapter, AD::REFDeviceDesc);
            assert_eq!(rd::<u32>(reference, DD::deviceType), 2);
            assert_eq!(rd::<u32>(reference, DD::caps_DeviceType), 0);
            assert_eq!(rd::<u32>(reference, DD::screenFormats_count), 0);
            assert_eq!(rd::<u8>(reference, DD::renderWindowed), 0);
            assert_eq!(device_desc_get_screen_format_enum(reference, 1, 0), 0);
            assert_eq!(d3d.count("GetDeviceCaps 1 2"), 1);

            assert!(!live_blocks().is_empty());
            system_desc_dtor(this);
            assert_eq!(live_blocks(), Vec::<usize>::new());
            assert_eq!(rd::<u32>(hal, DD::screenFormats_head), 0);
            assert_eq!(rd::<u32>(hal, DD::screenFormats_allocator_allocator), 0);
        }
    }

    #[test]
    fn a_device_without_windowed_caps_keeps_only_fullscreen_formats() {
        heap_reset();
        let mut d3d = FakeD3d::new();
        d3d.caps2 = 0;
        d3d.modes = vec![[640, 480, 60, 22]];
        let mut device = [0u8; DD::SIZE];
        let this = device.as_mut_ptr() as usize;
        unsafe {
            assert_eq!(device_desc_ctor(this, d3d.addr(), 0, 1, 22), this);
            let formats = list_values(this + DD::screenFormats);
            assert_eq!(formats.len(), 1);
            assert_eq!(rd::<u8>(formats[0] as usize, SFI::bFullscreen), 1);
            assert_eq!(rd::<u8>(formats[0] as usize, SFI::bWindowed), 0);
            assert_eq!(rd::<u8>(this, DD::renderWindowed), 0);
            assert_eq!(rd::<u32>(this, DD::screenFormats_allocator_count), 2);
            assert_eq!(device_desc_get_screen_format_enum(this, 1, 0), 0);
            assert_eq!(device_desc_get_screen_format_enum(this, 0, 32), 3);
            assert_eq!(&d3d.calls.borrow()[..5], [
                "GetDeviceCaps 0 1",
                "GetAdapterModeCount 0",
                "EnumAdapterModes 0 0",
                "CheckDeviceType 0 1 22 22 0",
                "CheckDeviceType 0 1 22 22 1",
            ]);
            device_desc_dtor(this);
            assert_eq!(live_blocks(), Vec::<usize>::new());
            assert_eq!(rd::<u32>(this, DD::screenFormats_count), 0);
        }
    }
}
