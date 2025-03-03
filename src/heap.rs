use alloc::{boxed::Box, vec::Vec};
use core::{
    alloc::{AllocError, Allocator, GlobalAlloc, Layout},
    arch::global_asm,
    cmp::max,
    fmt::Debug,
    ptr::{self, slice_from_raw_parts_mut, NonNull},
    range::Range,
    sync::atomic::{AtomicPtr, AtomicU16, AtomicUsize, Ordering::SeqCst},
};

use crate::{data::AtomicBitVec, println, sync::Mutex};

extern "C" {
    pub fn get_heap_base() -> *mut u8;
}

global_asm!(include_str!("heap.S"));

const RAM_BASE: *mut u8 = 0x40000000 as *mut u8;
const RAM_LENGTH: usize = 1024 * 1024 * 1024 * 4;
const RAM_END: *mut u8 = RAM_BASE.wrapping_add(RAM_LENGTH);
const RAM_RANGE: Range<*mut u8> = Range {
    start: RAM_BASE,
    end: RAM_END,
};
pub const PAGE_SIZE: usize = 4096;

struct BumpAllocator {
    offset: AtomicUsize,
}

impl BumpAllocator {
    fn get_heap_top(&self) -> *const u8 {
        unsafe { get_heap_base().add(self.offset.load(SeqCst)) }
    }
}

unsafe impl Allocator for &BumpAllocator {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        unsafe {
            let heap_base = get_heap_base();
            match self.offset.fetch_update(SeqCst, SeqCst, |offset| {
                let heap_top = heap_base.add(offset);
                let aligned: *mut u8 = heap_top.add(heap_top.align_offset(layout.align()));
                if RAM_END.offset_from(aligned) > layout.size() as isize {
                    let new_offset = aligned.offset_from(heap_base) as usize + layout.size();
                    Some(new_offset)
                } else {
                    None
                }
            }) {
                Ok(prev) => Ok(NonNull::new(slice_from_raw_parts_mut(
                    heap_base.add(prev),
                    layout.size(),
                ))
                .expect("Allocated null pointer!")),
                Err(_) => Err(AllocError {}),
            }
        }
    }

    unsafe fn deallocate(&self, _ptr: NonNull<u8>, _layout: Layout) {
        panic!("Deallocated during heap initialization!");
    }
}

#[repr(align(4096))]
struct PageLink {
    prev: AtomicPtr<PageLink>,
    next: AtomicPtr<PageLink>,
}

impl PageLink {
    unsafe fn allocate(&mut self) -> Option<*mut PageLink> {
        let self_addr = self as *mut PageLink;
        let prev = self.prev.load(SeqCst);
        let next: *mut PageLink = self.next.load(SeqCst);
        if next == self_addr {
            assert!(prev == next);
            return None;
        }
        (*prev).next.store(next, SeqCst);
        (*next).prev.store(prev, SeqCst);
        Some(next)
    }

    unsafe fn deallocate(&mut self, other: &AtomicPtr<PageLink>) {
        let self_addr = self as *mut PageLink;
        match other.load(SeqCst) {
            null_other if null_other.is_null() => {
                self.prev.store(self_addr, SeqCst);
                self.next.store(self_addr, SeqCst);
                other.store(self_addr, SeqCst);
            }
            other => {
                let next = (*other).next.swap(self_addr, SeqCst);
                self.prev.store((other as usize) as *mut PageLink, SeqCst);
                self.next.store(next, SeqCst);
                (*next).prev.store(self_addr, SeqCst);
            }
        }
    }
}

struct PageFreeList {
    available: AtomicBitVec<&'static BumpAllocator>,
    pages: AtomicPtr<PageLink>,
    grain: usize,
}

impl PageFreeList {
    fn new(num_pages: usize, grain: usize) -> PageFreeList {
        PageFreeList {
            available: AtomicBitVec::new_in(num_pages >> grain, &BUMP_ALLOCATOR),
            pages: AtomicPtr::default(),
            grain: grain,
        }
    }

    fn get_index(&self, page: *const PageLink) -> usize {
        unsafe { page.offset_from(RAM_BASE as *const PageLink) as usize >> self.grain }
    }

    fn get_page(&self, index: usize) -> *mut PageLink {
        unsafe { (RAM_BASE as *mut PageLink).offset((index << self.grain) as isize) }
    }

    fn allocate_page(&self) -> Option<*mut PageLink> {
        match self.pages.load(SeqCst) {
            free_page if free_page.is_null() => None,
            free_page => {
                self.allocate_target_page(free_page);
                let new_page_ptr = self.pages.load(SeqCst);
                assert!(
                    new_page_ptr.is_null()
                        || self
                            .available
                            .get(self.get_index(new_page_ptr))
                            .is_some_and(|v| v),
                    "Invalid page pointer state after page allocation!"
                );
                Some(free_page)
            }
        }
    }

    fn allocate_target_page(&self, page: *mut PageLink) {
        let index = self.get_index(page);
        self.allocate_page_exact(index, page)
    }

    fn _allocate_target_page_from_index(&self, index: usize) {
        let page = self.get_page(index);
        self.allocate_page_exact(index, page)
    }

    fn allocate_page_exact(&self, index: usize, page: *mut PageLink) {
        assert!(
            self.available
                .get(index)
                .expect("Allocating page out of bounds!"),
            "HOW"
        );
        self.available.set(index, false);
        match unsafe { (*page).allocate() } {
            Some(new_addr) => {
                self.pages.store(new_addr, SeqCst);
                assert!(
                    self.available
                        .get(self.get_index(new_addr))
                        .is_some_and(|v| v),
                    "New address is invalid after allocating page!"
                );
            }
            None => self.pages.store(ptr::null_mut(), SeqCst),
        }
    }

    fn deallocate_page(&self, page: *mut PageLink) -> Option<*mut PageLink> {
        self.deallocate_page_exact(self.get_index(page), page)
    }

    fn deallocate_page_from_index(&self, index: usize) -> Option<*mut PageLink> {
        self.deallocate_page_exact(index, self.get_page(index))
    }

    #[inline(always)]
    fn deallocate_page_exact(&self, index: usize, page: *mut PageLink) -> Option<*mut PageLink> {
        let buddy_index = index ^ 1;
        let lower_index = index & (!1);
        if self.available.get(buddy_index).unwrap_or(false) {
            self.allocate_target_page(self.get_page(buddy_index));
            Some(self.get_page(lower_index))
        } else {
            self.available.set(index, true);
            unsafe {
                assert!(
                    RAM_RANGE.contains(&(page as *mut u8)),
                    "Bad page deallocation at {:#01x}",
                    page as usize
                );
                (*page).deallocate(&self.pages);
            }
            None
        }
    }
}

impl Debug for PageFreeList {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Page Free List: ")?;
        writeln!(f, "{:?}", self.available)?;
        Ok(())
    }
}

pub struct PageAllocator {
    grained_lists: Vec<PageFreeList, &'static BumpAllocator>,
}

static BUMP_ALLOCATOR: BumpAllocator = BumpAllocator {
    offset: AtomicUsize::new(0),
};

pub fn get_bump_addr() -> *const u8 {
    unsafe { RAM_BASE.add(BUMP_ALLOCATOR.offset.load(SeqCst)) as *const u8 }
}

pub static PAGE_ALLOCATOR: Mutex<PageAllocator> = Mutex::new(PageAllocator {
    grained_lists: Vec::new_in(&BUMP_ALLOCATOR),
});

enum PageAllocationError {
    OutOfMemory,
}

#[derive(Debug)]
enum PageDeallocationError {
    OutOfBounds,
}

impl PageAllocator {
    fn init(&mut self) {
        let num_pages = RAM_LENGTH / PAGE_SIZE;
        let depth = num_pages.checked_ilog2().expect("System has zero pages!");
        self.grained_lists
            .try_reserve_exact(1 + depth as usize)
            .expect("Failed to allocate memory for Page Allocator");
        (0..=depth).for_each(|grain| {
            self.grained_lists
                .push_within_capacity(PageFreeList::new(num_pages, grain as usize))
                .unwrap();
        });

        let bytes_allocated = BUMP_ALLOCATOR.get_heap_top() as usize;
        let pages_allocated = bytes_allocated.div_ceil(PAGE_SIZE);
        (pages_allocated + 1..num_pages).for_each(|page_index| {
            self.deallocate_page_from_index(page_index, 0)
                .expect("Failed to free pages while initializing page allocator!");
        });
    }

    fn split_block(&self, target_grain: usize) -> Option<*mut PageLink> {
        self.grained_lists[target_grain + 1..]
            .iter()
            .enumerate()
            .find_map(|(grain_offset, free_list)| {
                free_list
                    .allocate_page()
                    .map(|page| (target_grain + 1 + grain_offset, page))
            })
            .map(|(mut grain, block)| {
                assert!(grain < self.grained_lists.len());
                while grain > target_grain {
                    grain -= 1;
                    let free_list = self.grained_lists.get(grain).unwrap();
                    free_list.deallocate_page_from_index(free_list.get_index(block) + 1);
                }
                block
            })
    }

    fn allocate_pages(&self, num_pages: usize) -> Result<*mut PageLink, PageAllocationError> {
        let mut grain = num_pages.ilog2() as usize;
        grain = grain + (num_pages > (1 << grain)) as usize;
        match self.grained_lists.get(grain) {
            Some(free_list) => match free_list.allocate_page() {
                Some(block) => Ok(block),
                None => match self.split_block(grain) {
                    Some(block) => Ok(block),
                    None => Err(PageAllocationError::OutOfMemory),
                },
            },
            None => Err(PageAllocationError::OutOfMemory),
        }
    }

    fn deallocate_page(
        &self,
        page: *mut PageLink,
        grain: usize,
    ) -> Result<(), PageDeallocationError> {
        match self.grained_lists.get(grain) {
            Some(free_list) => match free_list.deallocate_page(page) {
                Some(coalesced_block) => self.deallocate_page(coalesced_block, grain + 1),
                None => Ok(()),
            },
            None => Err(PageDeallocationError::OutOfBounds),
        }
    }

    fn deallocate_page_from_index(
        &self,
        index: usize,
        grain: usize,
    ) -> Result<(), PageDeallocationError> {
        match self.grained_lists.get(grain) {
            Some(free_list) => match free_list.deallocate_page_from_index(index) {
                Some(coalesced_block) => self.deallocate_page(coalesced_block, grain + 1),
                None => Ok(()),
            },
            None => Err(PageDeallocationError::OutOfBounds),
        }
    }

    #[inline(always)]
    fn get_num_pages(layout: Layout) -> usize {
        layout.size().max(layout.align()).div_ceil(PAGE_SIZE)
    }

    pub fn dump_at_grain(&self, grain: usize) -> Result<(), ()> {
        println!("{:?}", self.grained_lists.get(grain).ok_or(())?);
        Ok(())
    }
}

unsafe impl Allocator for Mutex<PageAllocator> {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        let num_pages = PageAllocator::get_num_pages(layout);
        match self.lock_blocking().allocate_pages(num_pages) {
            Ok(block) => Ok(NonNull::new(slice_from_raw_parts_mut(
                block as *mut u8,
                num_pages * PAGE_SIZE,
            ))
            .expect("Allocated null pointer")),
            Err(_) => Err(AllocError),
        }
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        let num_pages = PageAllocator::get_num_pages(layout);
        let mut grain = num_pages.ilog2() as usize;
        grain = grain + (num_pages > (1 << grain)) as usize;
        self.lock_blocking()
            .deallocate_page(ptr.as_ptr() as *mut PageLink, grain)
            .expect("Deallocating page failed!");
    }

    unsafe fn grow(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        let old_pages = PageAllocator::get_num_pages(old_layout);
        let new_pages = PageAllocator::get_num_pages(new_layout);
        if old_pages == new_pages {
            return Ok(
                NonNull::new(slice_from_raw_parts_mut(ptr.as_ptr(), new_pages))
                    .expect("Grew memory from a null pointer!"),
            );
        }

        // TODO: Can try much harder to grow the existing block
        let new_block = self.allocate(new_layout)?;
        ptr::copy_nonoverlapping(ptr.as_ptr(), new_block.as_mut_ptr(), old_layout.size());
        self.deallocate(ptr, old_layout);
        Ok(new_block)
    }
}

#[derive(Debug)]
struct FreeLink {
    prev: AtomicU16,
    next: AtomicU16,
}

#[derive(Debug)]
struct SlabHeader {
    page_memory: Box<[FreeLink; PAGE_SIZE / size_of::<FreeLink>()], &'static Mutex<PageAllocator>>,
    slot_size: u16,
    in_use: u16,
    offset: Option<u16>,
}

pub struct SlabAllocator {
    headers: Vec<SlabHeader, &'static Mutex<PageAllocator>>,
}

impl SlabAllocator {
    fn get_slot_size(layout: Layout) -> u16 {
        max(layout.size(), layout.align()).div_ceil(size_of::<FreeLink>()) as u16
    }

    pub fn dump_slot(&self, slot_size: u16) -> Result<(), ()> {
        for header in self.headers.iter() {
            println!("{:?}", header);
        }
        let key = self
            .headers
            .binary_search_by_key(&slot_size, |header| header.slot_size)
            .map_err(|_| ())?;
        let _header = self.headers.get(key).unwrap();
        todo!()
    }
}

impl SlabHeader {
    fn new(layout: Layout) -> SlabHeader {
        let slot_size = SlabAllocator::get_slot_size(layout);
        assert!(slot_size > 0);
        let page_memory: Box<
            [FreeLink; PAGE_SIZE / size_of::<FreeLink>()],
            &'static Mutex<PageAllocator>,
        > = unsafe { Box::new_uninit_in(&PAGE_ALLOCATOR).assume_init() };
        let last_index =
            page_memory
                .iter()
                .step_by(slot_size as usize)
                .fold(0, |current, flink| {
                    let next = current + slot_size;
                    flink
                        .prev
                        .store(u16::wrapping_sub(current, slot_size), SeqCst);
                    flink.next.store(next, SeqCst);
                    next
                })
                - slot_size; // Fold returns next, so go "back" one
        page_memory[0].prev.store(last_index, SeqCst);
        page_memory[last_index as usize].next.store(0, SeqCst);
        SlabHeader {
            page_memory: page_memory,
            slot_size: slot_size,
            in_use: 0,
            offset: Some(0),
        }
    }

    fn allocate(&mut self) -> Option<*mut u8> {
        Some(self.allocate_at(self.offset?) as *mut u8)
    }

    fn allocate_at(&mut self, index: u16) -> *mut FreeLink {
        let val = self
            .page_memory
            .get_mut(index as usize)
            .expect("Invalid offset when allocating in slab!");
        let val_ptr = val as *mut FreeLink;
        let prev_index = val.prev.load(SeqCst);
        let next_index = val.next.load(SeqCst);
        if prev_index == next_index {
            self.offset.take();
        } else {
            let prev = self
                .page_memory
                .get_mut(prev_index as usize)
                .expect("Invalid prev offset found when allocating!");
            prev.next.store(next_index, SeqCst);
            let next = self
                .page_memory
                .get_mut(next_index as usize)
                .expect("Invalid next offset found when allocating!");
            next.prev.store(prev_index, SeqCst);
            self.offset = Some(next_index);
        }
        self.in_use += 1;
        val_ptr
    }

    fn deallocate_at(&mut self, index: u16) {
        assert!(
            (index % self.slot_size) == 0,
            "Deallocation index is not divisible by slot size!"
        );
        match self.offset {
            Some(prev_index) => {
                let prev = self
                    .page_memory
                    .get_mut(prev_index as usize)
                    .expect("Slab header stored invalid offset!");
                let next_index = prev.next.swap(index, SeqCst);
                let next = self
                    .page_memory
                    .get_mut(next_index as usize)
                    .expect("Slab header stored invalid offset!");
                next.prev.store(index, SeqCst);
                let val = self
                    .page_memory
                    .get_mut(index as usize)
                    .expect("Invalid offset when deallocating in slab!");
                val.prev.store(prev_index, SeqCst);
                val.next.store(next_index, SeqCst);
            }
            None => {
                let val = self
                    .page_memory
                    .get_mut(index as usize)
                    .expect("Invalid offset when deallocating in slab!");
                val.prev.store(index, SeqCst);
                val.next.store(index, SeqCst);
                self.offset = Some(index);
            }
        }
        self.in_use -= 1;
    }

    fn deallocate(&mut self, memory: *mut u8) {
        let link_ptr = memory as *mut FreeLink;
        assert!(self.owns(memory), "Deallocated invalid memory!");
        let link_offset = unsafe { link_ptr.offset_from(self.page_memory.as_ptr()) };
        assert!(link_offset >= 0, "Deallocation index is out of bounds!");
        self.deallocate_at(link_offset as u16);
    }

    fn owns(&self, ptr: *mut u8) -> bool {
        self.page_memory
            .as_ptr_range()
            .contains(&(ptr as *const FreeLink))
    }
}

unsafe impl GlobalAlloc for Mutex<SlabAllocator> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut allocator = self.lock_blocking_mut();
        let block_size = SlabAllocator::get_slot_size(layout);
        if block_size == 0 {
            return ptr::null_mut();
        }
        match allocator
            .headers
            .binary_search_by_key(&block_size, |header| header.slot_size)
        {
            Ok(index) => allocator
                .headers
                .get_mut(index)
                .expect("Binary search returned invalid index!")
                .allocate()
                .unwrap_or(ptr::null_mut()),
            Err(index) => {
                allocator.headers.insert(index, SlabHeader::new(layout));
                allocator
                    .headers
                    .get_mut(index)
                    .expect("Insertion into slab headers failed!")
                    .allocate()
                    .expect("Allocation in fresh slab header failed!")
            }
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let mut allocator = self.lock_blocking_mut();
        let block_size = SlabAllocator::get_slot_size(layout);
        match allocator
            .headers
            .binary_search_by_key(&block_size, |header| header.slot_size)
        {
            Ok(index) => allocator
                .headers
                .get_mut(index)
                .expect("Binary search returned invalid index!")
                .deallocate(ptr),
            Err(_) => panic!("Invalid slab deallocation!"),
        }
    }
}

#[global_allocator]
pub static SLAB_ALLOCATOR: Mutex<SlabAllocator> = Mutex::new(SlabAllocator {
    headers: Vec::new_in(&PAGE_ALLOCATOR),
});

pub fn init_allocators() {
    PAGE_ALLOCATOR
        .lock_mut()
        .expect("Page allocator is not available for allocation!")
        .init();
    println!(
        "Page Allocator initialized. Heap top: {:p}",
        BUMP_ALLOCATOR.get_heap_top()
    );
}
