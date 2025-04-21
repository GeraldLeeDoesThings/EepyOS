use alloc::{boxed::Box, vec::Vec};
use core::{
    alloc::{AllocError, Allocator, GlobalAlloc, Layout},
    arch::global_asm,
    cmp::max,
    fmt::Debug,
    ptr::{self, slice_from_raw_parts_mut, NonNull},
    range::Range,
    sync::atomic::{AtomicPtr, AtomicU16, Ordering::SeqCst},
};

use crate::{data::AtomicBitVec, pointer::offset_between, println, sync::Mutex};

extern "C" {
    pub fn get_heap_base() -> *mut u8;
}

global_asm!(include_str!("heap.S"));

/// The lowest physical address used for RAM.
const RAM_BASE: *mut u8 = 0x4000_0000 as *mut u8;
/// The number of bytes available in RAM.
const RAM_LENGTH: usize = 1024 * 1024 * 1024 * 4;
/// One byte ***past*** the last byte of RAM.
const RAM_END: *mut u8 = RAM_BASE.wrapping_add(RAM_LENGTH);
/// All RAM addresses.
const RAM_RANGE: Range<*mut u8> = Range {
    start: RAM_BASE,
    end: RAM_END,
};
/// Size of a page in bytes.
pub const PAGE_SIZE: usize = 4096;

/// An allocator for use by the page allocator to permanently allocate
/// heap memory.
struct BumpAllocator {
    /// Offset in bytes from `RAM_BASE` representing heap RAM that has been
    /// permanently claimed by the kernel.
    offset: Mutex<usize>,
}

impl BumpAllocator {
    /// Returns the next byte that will be allocated by the `BUMP_ALLOCATOR`.
    ///
    /// # Safety
    ///
    /// The caller must not dereference the returned pointer, since the memory
    /// it points to may be allocated for use.
    ///
    /// If the bump allocator will not be used again, then the returned pointer
    /// may be used by other allocators to calculate the size of remaining heap
    /// memory.
    unsafe fn get_heap_top(&self) -> *const u8 {
        // SAFETY: Constant read of heap base.
        let heap_base = unsafe { get_heap_base() };
        // SAFETY: Allocated object is all of RAM. Assert ensures resulting pointer is
        // valid.
        let heap_top = unsafe { heap_base.add(*self.offset.lock_blocking()) };
        assert!(
            RAM_RANGE.contains(&heap_top),
            "Bump allocator has allocated all the RAM."
        );
        heap_top
    }
}

/// SAFETY: Checks with `RAM_END` ensure memory returned is always valid.
///
/// Memory is never deallocated, so the validity of returned memory certainly
/// lives sufficiently long.
///
/// `BumpAllocator` is not cloneable or copyable, so vacuously, all copies
/// and clones are valid.
unsafe impl Allocator for &BumpAllocator {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        // SAFETY: `get_heap_base` is used only to ensure valid memory is returned, and
        // is further checked by `RAM_END`.
        let heap_base = unsafe { get_heap_base() };
        let mut offset = self.offset.lock_blocking_mut();
        // SAFETY: `offset` is within RAM range since it is checked before being written
        // to. `offset` also fits within an isize since RAM is not that
        // large.
        let mut heap_top = unsafe { heap_base.add(*offset) };
        let aligned = heap_top.wrapping_add(heap_top.align_offset(layout.align()));
        heap_top = aligned.wrapping_add(layout.size());

        *offset = offset_between(heap_top, heap_base)
            .expect("Memory overflowed isize")
            .try_into()
            .expect("Heap top is below heap bottom!");

        if RAM_RANGE.contains(&heap_top) {
            Ok(
                NonNull::new(slice_from_raw_parts_mut(aligned, layout.size()))
                    .expect("Allocated null pointer!"),
            )
        } else {
            Err(AllocError {})
        }
    }

    unsafe fn deallocate(&self, _ptr: NonNull<u8>, _layout: Layout) {
        panic!("Deallocated during heap initialization!");
    }
}

/// An possibly, unallocated page, used to store pointers to other unallocaged
/// pages, forming a linked list of free pages.
#[repr(align(4096))]
struct PageLink {
    /// The previous free page, or this page if no other free pages exist.
    prev: AtomicPtr<PageLink>,
    /// The next free page, or this page if no other free pages exist.
    next: AtomicPtr<PageLink>,
}

impl PageLink {
    /// Allocates this page, ensuring that the pages pointed to by next
    /// and prev now point to eachother. If this is the last free page,
    /// then `next` and `prev` are not updated. Returns a pointer to
    /// a free page, or `None` if none exist.
    ///
    /// # Safety
    ///
    /// `PageLink` alone cannot determine if this page is allocated or not.
    /// Therefore, the caller must ensure that this `PageLink` is represented
    /// with unallocated memory before calling this function.
    unsafe fn allocate(&mut self) -> Option<*mut Self> {
        let self_addr = self as *mut Self;
        let prev = self.prev.load(SeqCst);
        let next: *mut Self = self.next.load(SeqCst);
        if next == self_addr {
            assert!(prev == next);
            return None;
        }
        // SAFETY: self is currently free, so `prev` must be point to a free page.
        unsafe {
            (*prev).next.store(next, SeqCst);
        }
        // SAFETY: self is currently free, so `next` must be point to a free page.
        unsafe {
            (*next).prev.store(prev, SeqCst);
        }
        Some(next)
    }

    /// Deallocates this page, ensuring that free page pointed to by `other`
    /// now links to this page, fixing all other links in the process. `other`
    /// must be null when this page is the only free page.
    ///
    /// # Safety
    ///
    /// - `PageLink` alone cannot determine if this page is allocated or not.
    ///   Therefore, the caller must ensure that this `PageLink` is represented
    ///   with freshly deallocated memory before calling this function.
    ///
    /// - `other` must point to another free page if one exists, and be null
    ///   otherwise.
    unsafe fn deallocate(&mut self, other: &AtomicPtr<Self>) {
        let self_addr = self as *mut Self;
        match other.load(SeqCst) {
            null_other if null_other.is_null() => {
                self.prev.store(self_addr, SeqCst);
                self.next.store(self_addr, SeqCst);
                other.store(self_addr, SeqCst);
            }
            other => {
                // SAFETY: Other must point to a free page.
                let next = unsafe { (*other).next.swap(self_addr, SeqCst) };
                self.prev.store((other as usize) as *mut Self, SeqCst);
                self.next.store(next, SeqCst);
                // SAFETY: `next` was just updated to a valid free page.
                unsafe { (*next).prev.store(self_addr, SeqCst) };
            }
        }
    }
}

/// A free list of pages, represented as a linked list. Additionally,
/// ensures that pages are double allocated or double freed.
struct PageFreeList {
    /// Bit vec representing if a page is free.
    available: AtomicBitVec<&'static BumpAllocator>,
    /// Pointer to a free page, or null otherwise.
    pages: AtomicPtr<PageLink>,
    /// Multiplier for page size, where the multiplier is `2^grain`.
    grain: usize,
}

impl PageFreeList {
    /// Creates a page free list with `num_pages` pages, with a size multiplier
    /// of `2^grain`. The resulting page list stores `num_pages * 2^grain`
    /// total normal sized pages. Memory used for the underlying bit vec is
    /// allocated with the `BUMP_ALLOCATOR`, and therefore is never
    /// recovered.
    fn new(num_pages: usize, grain: usize) -> Self {
        Self {
            available: AtomicBitVec::new_in(num_pages >> grain, &BUMP_ALLOCATOR),
            pages: AtomicPtr::default(),
            grain,
        }
    }

    /// Determines the index of `page` if all of RAM were treated like an array
    /// of pages. The raw index is right shifted by the grain of this list,
    /// so it is as if the pages are in groups of `2^grain`.
    ///
    /// # Panics
    ///
    /// Panics if:
    ///
    /// - `page` is has an offset from `RAM_BASE` that does not fit in an isize.
    ///
    /// - `page` is not aligned to groups of `2^grain` pages.
    ///
    /// - The offset for `page` is below `RAM_BASE`.
    #[allow(
        clippy::cast_ptr_alignment,
        reason = "RAM_BASE is page aligned and constant"
    )]
    fn get_index(&self, page: *const PageLink) -> usize {
        if !page
            .wrapping_byte_sub(RAM_BASE as usize)
            .is_aligned_to(align_of::<PageLink>() * (1 << self.grain))
        {
            println!(
                "page = {:p}, alignment = {:x}({})",
                page.wrapping_byte_sub(RAM_BASE as usize),
                align_of::<PageLink>() * (1 << self.grain),
                self.grain,
            );
        }
        assert!(page
            .wrapping_byte_sub(RAM_BASE as usize)
            .is_aligned_to(align_of::<PageLink>() * (1 << self.grain)));
        usize::try_from(offset_between(page, RAM_BASE as *const PageLink).unwrap())
            .expect("Tried to get index for pointer outside of RAM.")
            >> self.grain
    }

    /// Constructes a pointer to a page corresponding to `index`, if RAM were
    /// treated like an array of pages. The raw index is right shifted by
    /// the grain of this list, so it is as if the pages are in groups of
    /// `2^grain`.
    ///
    /// # Panics
    ///
    /// Panics if:
    ///
    /// - `index` does not fit in an isize after being right shifted.
    ///
    /// - The resulting pointer does not point to valid RAM.
    #[allow(
        clippy::cast_ptr_alignment,
        reason = "RAM_BASE is page aligned and constant"
    )]
    fn get_page(&self, index: usize) -> *mut PageLink {
        // SAFETY: Offset fits in an isize, and page is checked to ensure it points to
        // valid memory.
        let page = unsafe {
            RAM_BASE
                .cast::<PageLink>()
                .offset(isize::try_from(index << self.grain).expect("Index is out of isize range."))
        };
        assert!(RAM_RANGE.contains(&page.cast()));
        page
    }

    /// Attempts to allocate a page, returning a pointer to the freshly
    /// allocated page if successful.
    fn allocate_page(&self) -> Option<*mut PageLink> {
        match self.pages.load(SeqCst) {
            free_page if free_page.is_null() => None,
            free_page => {
                // SAFETY: `free_page` is aligned and free by invariant.
                unsafe {
                    self.allocate_target_page(free_page);
                }
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

    /// Allocates the page pointed to by `page`.
    ///
    /// # Safety
    ///
    /// `page` must not already be allocated.
    ///
    /// # Panics
    ///
    /// Panics if `page` is misaligned, or does not point to valid RAM.
    unsafe fn allocate_target_page(&self, page: *mut PageLink) {
        let index = self.get_index(page);
        // SAFETY: `index` is well formed since it is the result of [get_index], which
        // also checks `page`
        unsafe {
            self.allocate_page_exact(index, page);
        }
    }

    /// Allocated the page indexed by `index`. See [`Self::get_page`] for
    /// details.
    ///
    /// # Panics
    ///
    /// - If `index` indexes a page that is out of bounds
    ///
    /// - If the page indexed by `index` is already allocated
    ///
    /// - If the page being allocated has `next` and `prev` pointers to
    ///   allocated pages.
    fn _allocate_target_page_from_index(&self, index: usize) {
        let page = self.get_page(index);
        // SAFETY: `page` is well formed since it is retreived with [get_page], which
        // also checks `index`
        unsafe {
            self.allocate_page_exact(index, page);
        }
    }

    /// Allocates the page that is **BOTH** pointed to by `page`, and indexed by
    /// `index`.
    ///
    /// # Safety
    ///
    /// - `page` must be the result of calling [`Self::get_page`] on `index`.
    ///
    /// - `index` must be the result of calling [`Self::get_index`] on `page`.
    ///
    /// # Panics
    ///
    /// Panics if:
    ///
    /// - `page` is already allocated
    ///
    /// - `page` has links to pages which are already allocated.
    unsafe fn allocate_page_exact(&self, index: usize, page: *mut PageLink) {
        assert!(
            self.available
                .get(index)
                .expect("Allocating page out of bounds!"),
            "Page is already allocated!"
        );
        self.available.set(index, false);
        // SAFETY: `page` is well formed by unsafe requirements.
        let page_ref = unsafe { page.as_mut().expect("Page pointer is null!") };
        // SAFETY: `page` is checked for availability bit vec.
        match unsafe { page_ref.allocate() } {
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

    /// Deallocates the page pointed to by `page`.
    ///
    /// # Panics
    ///
    /// Panics if `page` is misaligned, not currently allocated,
    /// or points outside valid RAM.
    fn deallocate_page(&self, page: *mut PageLink) -> Option<*mut PageLink> {
        // SAFETY: `index` is well formed since it is the result of [get_index], which
        // also checks `page`
        unsafe { self.deallocate_page_exact(self.get_index(page), page) }
    }

    /// Deallocates the page indexed by `index`, obtained from
    /// [`Self::get_page`]
    ///
    /// # Panics
    ///
    /// Panics if `index` indexes a page out of bounds of RAM,
    /// or if the resulting page is not currently allocated.
    fn deallocate_page_from_index(&self, index: usize) -> Option<*mut PageLink> {
        // SAFETY: `page` is well formed since it is retreived with [`Self::get_page`],
        // which also checks `index`
        unsafe { self.deallocate_page_exact(index, self.get_page(index)) }
    }

    /// Deallocates the page that is **BOTH** pointed to by `page`, and indexed
    /// by `index`.
    ///
    /// # Safety
    ///
    /// - `page` must be the result of calling [`Self::get_page`] on `index`.
    ///
    /// - `index` must be the result of calling [`Self::get_index`] on `page`.
    ///
    /// # Panics
    ///
    /// Panics if:
    ///
    /// - `page` is already free.
    ///
    /// - `page` points to memory outside valid RAM.
    unsafe fn deallocate_page_exact(
        &self,
        index: usize,
        page: *mut PageLink,
    ) -> Option<*mut PageLink> {
        let buddy_index = index ^ 1;
        let lower_index = index & (!1);
        if self.available.get(buddy_index).unwrap_or(false) {
            // SAFETY: `buddy_index` is guarded by `available` bit vec, so the corresponding
            // page must be free and in valid RAM.
            unsafe {
                self.allocate_target_page(self.get_page(buddy_index));
            }
            Some(self.get_page(lower_index))
        } else {
            assert!(
                RAM_RANGE.contains(&page.cast()),
                "Bad page deallocation at {:#01x}",
                page as usize
            );

            self.available.set(index, true);
            // SAFETY: `page` is well formed by unsafe requirements.
            let page_ref = unsafe { page.as_mut().expect("Page pointer is null!") };
            // SAFETY: `page` is checked for availability bit vec.
            unsafe { page_ref.deallocate(&self.pages) };
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

/// A page allocator, using a buddy system for efficient allocation.
pub struct PageAllocator {
    /// Page free lists, indexed by their granularity. The free list at
    /// index `i` manages pages in groups of `2^i`.
    grained_lists: Vec<PageFreeList, &'static BumpAllocator>,
}

/// Global bump allocator. Intended to be used to allocate large static objects.
static BUMP_ALLOCATOR: BumpAllocator = BumpAllocator {
    offset: Mutex::new(0),
};

/// Gets the next byte which will be allocated by [`BUMP_ALLOCATOR`].
pub fn get_bump_addr() -> *const u8 {
    // SAFETY: Valid `offset` should fit in an isize on systems with reasonable
    // amounts of RAM. Resulting pointer is always valid, unless it points
    // outside of RAM, which should never happen with the bump allocator.
    unsafe {
        RAM_BASE
            .add(*BUMP_ALLOCATOR.offset.lock_blocking())
            .cast_const()
    }
}

/// The page allocator for the kernel. See [`PageAllocator`] for implementation
/// details. Relies on [`BUMP_ALLOCATOR`] for several static allocations.
pub static PAGE_ALLOCATOR: Mutex<PageAllocator> = Mutex::new(PageAllocator {
    grained_lists: Vec::new_in(&BUMP_ALLOCATOR),
});

/// An error occuring when calling [`PageAllocator::allocate_pages`] to allocate
/// pages.
enum PageAllocationError {
    /// Insufficent memory to allocate the requested number of pages.
    OutOfMemory,
}

/// An error occuring when freeing pages with a [`PageAllocator`].
#[derive(Debug)]
enum PageDeallocationError {
    /// Tried to free a page not living in valid RAM.
    OutOfBounds,
}

impl PageAllocator {
    /// Sets up this page allocator to map pages for all RAM not consumed by the
    /// [`BUMP_ALLOCATOR`] or other structures already in memory. In
    /// practice, this is all addresses above [`get_bump_addr`] inclusively.
    ///
    /// All pages consumed in whole or part by the [`BUMP_ALLOCATOR`] are marked
    /// as allocated, and will never be freed by normal use of the
    /// [`PageAllocator`].
    ///
    /// # Panics
    ///
    /// Panics if [`PageAllocator::grained_lists`] is non-empty.
    fn init(&mut self) {
        assert!(
            self.grained_lists.is_empty(),
            "Tried to initialize a non-empty page allocator!"
        );

        // SAFETY: Only used for assert.
        let prev_heap_top = unsafe { BUMP_ALLOCATOR.get_heap_top() as usize };

        let num_pages = RAM_LENGTH / PAGE_SIZE;
        let depth = num_pages.checked_ilog2().expect("System has zero pages!");
        self.grained_lists
            .try_reserve_exact(1 + depth as usize)
            .expect("Failed to allocate memory for Page Allocator");
        println!("Preparing lists for grains {} .. {}", 0, depth);
        (0..=depth).for_each(|grain| {
            self.grained_lists
                .push_within_capacity(PageFreeList::new(num_pages, grain as usize))
                .unwrap();
            // SAFETY: Only used for assert.
            let new_heap_top = unsafe { BUMP_ALLOCATOR.get_heap_top() as usize };
            // SAFETY: Debug print.
            // println!("{:p}", unsafe { BUMP_ALLOCATOR.get_heap_top() });

            // DEAR COMPILER: PLEASE UPDATE THE HEAP!!!
            assert!(new_heap_top > prev_heap_top);
        });

        // SAFETY: heap top is not dereferenced.
        let bytes_allocated = unsafe { BUMP_ALLOCATOR.get_heap_top() } as usize;
        let pages_allocated = bytes_allocated.div_ceil(PAGE_SIZE);
        (pages_allocated + 1..num_pages).for_each(|page_index| {
            self.deallocate_page_from_index(page_index, 0)
                .expect("Failed to free pages while initializing page allocator!");
        });
    }

    /// Splits up larger blocks of pages down to `target_grain`. Returns a free
    /// block in the target grain if one was successfully made by splitting
    /// blocks, or `None` if no such block could be generated.
    ///
    /// Smaller blocks are searched for first, and the first such block found is
    /// split repeatedly until the target grain is reached.
    ///
    /// Note that this function does ***not check if there is already a free
    /// block with the target grain!***
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

    /// Attempts to allocate `num_pages` pages. This function may allocate more
    /// than `num_pages`, up to the nearest power of two. The pages
    /// allocated will be contiguous in physical memory.
    fn allocate_pages(&self, num_pages: usize) -> Result<*mut PageLink, PageAllocationError> {
        let mut grain = num_pages.ilog2() as usize;
        grain = grain + usize::from(num_pages > (1 << grain));
        self.grained_lists
            .get(grain)
            .map_or(Err(PageAllocationError::OutOfMemory), |free_list| {
                free_list.allocate_page().map_or_else(
                    || {
                        self.split_block(grain)
                            .ok_or(PageAllocationError::OutOfMemory)
                    },
                    Ok,
                )
            })
    }

    /// Deallocates the page (block) pointed to by `page`, with grain `grain`.
    /// If the buddy for `page` is also free, it will be merged with `page`,
    /// freeing the page one grain up that contains this page and its buddy.
    /// This process is repeated until merging blocks is no longer possible.
    ///
    /// To deallocate with an index instead of a pointer, use
    /// [`PageAllocator::deallocate_page_from_index`] instead.
    ///
    /// # Safety
    ///
    /// The following conditions must be satisifed at the time of calling:
    ///
    /// - All pages in the block defined by `page` in grains below `grain` must
    ///   be marked as allocated.
    ///
    /// - All page blocks in grains above `grain` that contain `page` must be
    ///   marked as allocated.
    ///
    /// - References to the memory contained in `page` must no longer exist,
    ///   except for those used by this instance of the [`PageAllocator`].
    ///
    /// # Panics
    ///
    /// Panics if `page` is misaligned, or not currently allocated in the
    /// free list for `grain`.
    ///
    /// # Errors
    ///
    /// Returns an error if no [`PageFreeList`] exists for `grain`.
    fn deallocate_page(
        &self,
        page: *mut PageLink,
        grain: usize,
    ) -> Result<(), PageDeallocationError> {
        self.grained_lists
            .get(grain)
            .map_or(Err(PageDeallocationError::OutOfBounds), |free_list| {
                free_list
                    .deallocate_page(page)
                    .map_or(Ok(()), |coalesced_block| {
                        self.deallocate_page(coalesced_block, grain + 1)
                    })
            })
    }

    /// Deallocates the page (block) indexed by `index`, with grain `grain`.
    /// If the buddy for `page` is also free, it will be merged with `page`,
    /// freeing the page one grain up that contains this page and its buddy.
    /// This process is repeated until merging blocks is no longer possible.
    ///
    /// To deallocate with a pointer instead of an index, use
    /// [`PageAllocator::deallocate_page`] instead.
    ///
    /// # Safety
    ///
    /// The following conditions must be satisifed at the time of calling:
    ///
    /// - All pages in the block indexed by `index` in grains below `grain` must
    ///   be marked as allocated.
    ///
    /// - All page blocks in grains above `grain` that contain the block indexed
    ///   by `index` must be marked as allocated.
    ///
    /// - References to the memory contained in the page indexed by `index` must
    ///   no longer exist, except for those used by this instance of the
    ///   [`PageAllocator`].
    ///
    /// # Panics
    ///
    /// Panics if `index` does not index a page in RAM, or if the page it
    /// indexes is not currently allocated.
    ///
    /// # Errors
    ///
    /// Returns an error if no [`PageFreeList`] exists for `grain`.
    fn deallocate_page_from_index(
        &self,
        index: usize,
        grain: usize,
    ) -> Result<(), PageDeallocationError> {
        self.grained_lists
            .get(grain)
            .map_or(Err(PageDeallocationError::OutOfBounds), |free_list| {
                free_list
                    .deallocate_page_from_index(index)
                    .map_or(Ok(()), |coalesced_block| {
                        self.deallocate_page(coalesced_block, grain + 1)
                    })
            })
    }

    /// Returns the number of pages in RAM.
    fn get_num_pages(layout: Layout) -> usize {
        layout.size().max(layout.align()).div_ceil(PAGE_SIZE)
    }

    /// Debug prints a [`PageFreeList`] with a grain corresponding to `grain`.
    ///
    /// # Errors
    ///
    /// Errors if no [`PageFreeList`] exists for `grain`.
    pub fn dump_at_grain(&self, grain: usize) -> Result<(), ()> {
        println!("{:?}", self.grained_lists.get(grain).ok_or(())?);
        Ok(())
    }
}

// SAFETY: By the correctness of the [`PageAllocator`] implementation.
unsafe impl Allocator for Mutex<PageAllocator> {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        let num_pages = PageAllocator::get_num_pages(layout);
        self.lock_blocking()
            .allocate_pages(num_pages)
            .map_or(Err(AllocError), |block| {
                Ok(NonNull::new(slice_from_raw_parts_mut(
                    block.cast(),
                    num_pages * PAGE_SIZE,
                ))
                .expect("Allocated null pointer"))
            })
    }

    #[allow(
        clippy::cast_ptr_alignment,
        reason = "Valid by safety requirements of deallocate"
    )]
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        let num_pages = PageAllocator::get_num_pages(layout);
        let mut grain = num_pages.ilog2() as usize;
        grain = grain + usize::from(num_pages > (1 << grain));
        self.lock_blocking()
            .deallocate_page(ptr.as_ptr().cast::<PageLink>(), grain)
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

        // SAFETY: Memory lives long enough by correctness of the page allocator.
        // Similarly, alignment and non-overlapping requirements are satisifed by
        // the safety requirements of this function and page allocator correctness.
        unsafe {
            ptr::copy_nonoverlapping(ptr.as_ptr(), new_block.as_mut_ptr(), old_layout.size());
        }
        // SAFETY: ptr must be well formed due to safety requirements of this function.
        unsafe { self.deallocate(ptr, old_layout) };
        Ok(new_block)
    }
}

/// Two indexes into a parent [`SlabHeader`], pointing to other [`FreeLink`]
/// structures, representing free slots (memory) in this [`SlabHeader`].
#[derive(Debug)]
struct FreeLink {
    /// Index to another [`FreeLink`]. If this points to itself, this is the
    /// last free slot. It must differ from [`FreeLink::next`], unless it is
    /// self referential.
    prev: AtomicU16,
    /// Index to another [`FreeLink`]. If this points to itself, this is the
    /// last free slot. It must differ from [`FreeLink::prev`], unless it is
    /// self referential.
    next: AtomicU16,
}

/// A manager for a page allocated for reallocation as part of a
/// [`SlabAllocator`].
#[derive(Debug)]
struct SlabHeader {
    /// A page to be used for allocations, represented as an array of
    /// [`FreeLink`]. Only every [`SlabHeader::slot_size`]th [`FreeLink`]
    /// will be used.
    page_memory: Box<[FreeLink; PAGE_SIZE / size_of::<FreeLink>()], &'static Mutex<PageAllocator>>,
    /// Number of slots to be used for a single allocation.
    slot_size: u16,
    /// Number of allocations currently active.
    in_use: u16,
    /// Absolute index into [`SlabHeader::page_memory`] pointing to an
    /// unallocated [`FreeLink`], or `None` if the entire page has been
    /// allocated already.
    offset: Option<u16>,
}

/// A SLUB allocator, implemented by maining a sorted list of [`SlabHeader`]s,
/// keyed by their slot size.
pub struct SlabAllocator {
    /// Pages dedicated for use by this allocator, sorted by
    headers: Vec<SlabHeader, &'static Mutex<PageAllocator>>,
}

impl SlabAllocator {
    /// Derives a slot size from `layout`.
    ///
    /// # Panics
    ///
    /// Panics if the calculated slot size cannot be stored in a [`u16`].
    fn get_slot_size(layout: Layout) -> u16 {
        u16::try_from(max(layout.size(), layout.align()).div_ceil(size_of::<FreeLink>()))
            .expect("Layout size or alignment is too large for slab allocator.")
    }

    /// Debug prints the [`SlabHeader`] with `slot_size` if it exists.
    pub fn dump_slot(&self, slot_size: u16) -> Result<(), ()> {
        for header in &self.headers {
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
    /// Creates a new [`SlabHeader`] with a slot size appropriate for `layout`.
    /// Allocates a single page pre-emptively.
    fn new(layout: Layout) -> Self {
        let slot_size = SlabAllocator::get_slot_size(layout);
        assert!(slot_size > 0);
        // SAFETY: Contents are immediately initialized below.
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
        Self {
            page_memory,
            slot_size,
            in_use: 0,
            offset: Some(0),
        }
    }

    /// Attempts an allocation, returning a pointer to the start of the
    /// allocated memory, or `None` if [`Self::page_memory`] is fully
    /// allocated.
    fn allocate(&mut self) -> Option<*mut u8> {
        // SAFETY: By the correctness of [`Self::offset`].
        unsafe { Some(self.allocate_at(self.offset?).cast()) }
    }

    /// Allocates the [`FreeLink`] at `index`.
    ///
    /// # Safety
    ///
    /// This function cannot check if a [`FreeLink`] is allocated or not, since
    /// an allocated [`FreeLink`] has its memory reused.
    ///
    /// Therefore, the [`FreeLink`] at `index` must be free when this function
    /// is called. Otherwise, the memory occupied by the [`FreeLink`] may be
    /// corrupted.
    ///
    /// # Panics
    ///
    /// This function panics if `index` is out of bounds of the underlying
    /// memory of the backing [`Self::page_memory`].
    unsafe fn allocate_at(&mut self, index: u16) -> *mut FreeLink {
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
        assert!(self.owns(val_ptr.cast()));
        self.in_use += 1;
        val_ptr
    }

    /// Frees the [`FreeLink`] at `index`.
    ///
    /// # Safety
    ///
    /// This function cannot check if a [`FreeLink`] is allocated or not, since
    /// an allocated [`FreeLink`] has its memory reused.
    ///
    /// Therefore, the [`FreeLink`] at `index` must be allocated when this
    /// function is called. Otherwise, the memory occupied by the
    /// [`FreeLink`] may be corrupted.
    ///
    /// # Panics
    ///
    /// This function panics if `index` is out of bounds of the underlying
    /// memory of the backing [`Self::page_memory`], or if `index` is
    /// misaligned with [`Self::slot_size`].
    unsafe fn deallocate_at(&mut self, index: u16) {
        assert!(
            (index % self.slot_size) == 0,
            "Deallocation index is not divisible by slot size!"
        );
        assert!(
            self.page_memory.get(index as usize).is_some(),
            "Deallocation index is out of bounds!"
        );
        if let Some(prev_index) = self.offset {
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
        } else {
            let val = self
                .page_memory
                .get_mut(index as usize)
                .expect("Invalid offset when deallocating in slab!");
            val.prev.store(index, SeqCst);
            val.next.store(index, SeqCst);
            self.offset = Some(index);
        }
        self.in_use -= 1;
    }

    /// Frees the [`PageLink`] pointed to by `memory`.
    ///
    /// # Safety
    ///
    /// This function cannot check if a [`FreeLink`] is allocated or not, since
    /// an allocated [`FreeLink`] has its memory reused.
    ///
    /// `memory` must point to an allocated [`FreeLink`]. Particularly, an index
    /// derived from `memory` must result in that allocated [`FreeLink`]
    /// when taken into [`Self::page_memory`].
    ///
    /// # Panics
    ///
    /// This function panics if `memory` is misaligned or does not point to
    /// memory in [`Self::page_memory`].
    #[allow(clippy::cast_ptr_alignment, reason = "Alignment is asserted")]
    unsafe fn deallocate(&mut self, memory: *mut u8) {
        assert!(memory.is_aligned_to(align_of::<FreeLink>()));
        let link_ptr = memory.cast::<FreeLink>();
        assert!(self.owns(memory), "Deallocated invalid memory!");
        let link_offset = offset_between(link_ptr, self.page_memory.as_ptr())
            .expect("Overflow calculating deallocation index!");
        assert!(link_offset >= 0, "Deallocation index is out of bounds!");
        // SAFETY: index is correct by safety requirements of this function.
        unsafe {
            self.deallocate_at(
                u16::try_from(link_offset).expect("Link offset is impossibly large."),
            );
        }
    }

    /// Checks if `ptr` points to memory inside [`Self::page_memory`].
    #[allow(clippy::cast_ptr_alignment, reason = "Cast is never dereferenced")]
    fn owns(&self, ptr: *mut u8) -> bool {
        self.page_memory
            .as_ptr_range()
            .contains(&(ptr as *const FreeLink))
    }
}

// SAFETY: By the correctness of the [`SlabAllocator`] implementation.
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
                // TODO: Allocate another page if possible.
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

    #[allow(
        clippy::match_wild_err_arm,
        reason = "Index is not needed in error case"
    )]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let mut allocator = self.lock_blocking_mut();
        let block_size = SlabAllocator::get_slot_size(layout);
        match allocator
            .headers
            .binary_search_by_key(&block_size, |header| header.slot_size)
        {
            // SAFETY: By safety requirements of this function.
            Ok(index) => unsafe {
                allocator
                    .headers
                    .get_mut(index)
                    .expect("Binary search returned invalid index!")
                    .deallocate(ptr);
            },
            Err(_would_be_index) => panic!("Invalid slab deallocation!"),
        }
    }
}

/// The global allocator for the kernel. Implements a SLUB allocator.
#[global_allocator]
pub static SLAB_ALLOCATOR: Mutex<SlabAllocator> = Mutex::new(SlabAllocator {
    headers: Vec::new_in(&PAGE_ALLOCATOR),
});

/// Performs initialization for all the allocators needed to manage
/// the heap and pages.
pub fn init_allocators() {
    PAGE_ALLOCATOR
        .lock_mut()
        .expect("Page allocator is not available for allocation!")
        .init();
    println!(
        "Page Allocator initialized. Heap top: {:p}",
        // SAFETY: Pointer is only used to debug print.
        unsafe { BUMP_ALLOCATOR.get_heap_top() }
    );
}
