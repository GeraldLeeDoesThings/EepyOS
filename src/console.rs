use core::{mem::replace, str::Split};

use alloc::{alloc::Global, boxed::Box};

use crate::{
    heap::{get_bump_addr, PageAllocator, PAGE_ALLOCATOR, PAGE_SIZE, SLAB_ALLOCATOR},
    println,
    sync::Mutex,
};

/// A possible page or slab allocation.
#[allow(dead_code, reason = "Fields are just to keep allocations alive")]
enum MaybeAlloc {
    /// No allocation, or an allocation that has be deallocated already.
    None,
    /// An active page allocation.
    PageAlloc(Box<[u8], &'static Mutex<PageAllocator>>),
    /// An active slab allocation.
    SlabAlloc(Box<[u8], Global>),
}

/// A fixed length buffer to store allocations made with the console for
/// testing.
static ALLOC_BUFFER: Mutex<[MaybeAlloc; ALLOC_BUFFER_MAX_LENGTH]> =
    Mutex::new([const { MaybeAlloc::None }; ALLOC_BUFFER_MAX_LENGTH]);
/// The current length of the `ALLOC_BUFFER`.
/// This value is only for convenience for inferring where to allocate and
/// deallocate. It is unimportant for safety or correctness.
static mut ALLOC_BUFFER_LENGTH: usize = 0;
/// The maximum length of the `ALLOC_BUFFER`.
const ALLOC_BUFFER_MAX_LENGTH: usize = 32;

#[allow(
    clippy::unnecessary_wraps,
    reason = "Returns a result to be compatible with other console functions"
)]
/// Prints the top of the bump allocator address.
fn exec_bumpa() -> Result<(), &'static str> {
    println!("Bump Addr: {:p}", get_bump_addr());
    Ok(())
}

/// Debug prints a slab allocator.
/// The next argument in `args` is be the grain of the allocator.
fn exec_pagea(args: &mut Split<char>) -> Result<(), &'static str> {
    let grain: usize = args
        .next()
        .ok_or("Missing first argument for 'grain'")?
        .parse()
        .map_err(|_| "Argument for 'grain' is not a valid usize")?;
    PAGE_ALLOCATOR
        .lock_blocking()
        .dump_at_grain(grain)
        .map_err(|()| "Error while dumping page allocator memory")
}

/// Debug prints a slab allocator.
/// The next argument in `args` is the block size of the allocator.
fn exec_slaba(args: &mut Split<char>) -> Result<(), &'static str> {
    let block_size: u16 = args
        .next()
        .ok_or("Missing first argument for 'block size'")?
        .parse()
        .map_err(|_| "Argument for 'block size' is not a valid usize")?;
    SLAB_ALLOCATOR
        .lock_blocking()
        .dump_slot(block_size)
        .map_err(|()| "Error while dumping slab allocator memory")
}

/// Allocates with a slab allocator.
/// The first argument in `args` is the size of the allocation in bytes.
/// The second argument in `args` is optionally an index into `ALLOC_BUFFER` to
/// store the allocation. If not provided, it is inferred as
/// `ALLOC_BUFFER_LENGTH`.
fn exec_alloc(args: &mut Split<char>) -> Result<(), &'static str> {
    let mut allocator = ALLOC_BUFFER.lock_mut().unwrap();
    let block_size: u16 = args
        .next()
        .ok_or("Missing first argument for 'block size'")?
        .parse()
        .map_err(|_| "Argument for 'block size' is not a valid usize")?;
    // SAFETY: Single threaded access to mutable static.
    let index: usize = unsafe {
        args.next().map_or_else(
            || Ok(ALLOC_BUFFER_LENGTH),
            |index_str| {
                index_str
                    .parse()
                    .map_err(|_| "Argument for 'index' is not a valid usize")
            },
        )?
    };
    allocator.get_mut(index).map_or(Ok(()), |val| match val {
        MaybeAlloc::None => {
            // SAFETY: Box creation is just to cause an allocation. It is never read or
            // written to.
            let _ = unsafe {
                replace(
                    val,
                    MaybeAlloc::SlabAlloc(Box::new_uninit_slice(block_size as usize).assume_init()),
                )
            };
            // SAFETY: Single threaded access.
            if index >= unsafe { ALLOC_BUFFER_LENGTH } {
                // SAFETY: Single threaded access.
                unsafe {
                    ALLOC_BUFFER_LENGTH = index + 1;
                }
            }
            Ok(())
        }
        _ => Err("Failed to allocate with global allocator"),
    })
}

/// Allocates with a page allocator.
/// The first argument in `args` is the number of pages to allocate. At least
/// one page is always allocated. The second argument in `args` is optionally an
/// index into `ALLOC_BUFFER` to store the allocation. If not provided, it is
/// inferred as `ALLOC_BUFFER_LENGTH`.
fn exec_palloc(args: &mut Split<char>) -> Result<(), &'static str> {
    let mut allocator = ALLOC_BUFFER.lock_mut().unwrap();
    let num_pages: u16 = args
        .next()
        .ok_or("Missing first argument for 'number of pages'")?
        .parse()
        .map_err(|_| "Argument for 'number of pages' is not a valid usize")?;
    // SAFETY: Single threaded access.
    let index: usize = unsafe {
        args.next().map_or_else(
            || Ok(ALLOC_BUFFER_LENGTH),
            |index_str| {
                index_str
                    .parse()
                    .map_err(|_| "Argument for 'index' is not a valid usize")
            },
        )?
    };
    allocator.get_mut(index).map_or(Ok(()), |val| match val {
        MaybeAlloc::None => {
            // SAFETY: Box creation is just to cause an allocation. It is never read or
            // written to.
            let _ = unsafe {
                replace(
                    val,
                    MaybeAlloc::PageAlloc(
                        Box::new_uninit_slice_in(
                            (num_pages - 1) as usize * PAGE_SIZE + 1,
                            &PAGE_ALLOCATOR,
                        )
                        .assume_init(),
                    ),
                )
            };
            // SAFETY: Single threaded access.
            if index >= unsafe { ALLOC_BUFFER_LENGTH } {
                // SAFETY: Single threaded access.
                unsafe {
                    ALLOC_BUFFER_LENGTH = index + 1;
                }
            }
            Ok(())
        }
        _ => Err("Slot at index is already allocated"),
    })
}

/// Deallocates the allocation in `ALLOC_BUFFER` at an index.
/// The index is either the first argument in `args`, or `ALLOC_BUFFER_LENGTH -
/// 1` by default.
fn exec_dealloc(args: &mut Split<char>) -> Result<(), &'static str> {
    let mut allocator = ALLOC_BUFFER.lock_mut().unwrap();
    let index: usize = args.next().map_or_else(
        || {
            // SAFETY: Single threaded access.
            if unsafe { ALLOC_BUFFER_LENGTH } == 0 {
                Err("Alloc buffer is empty!")
            } else {
                // SAFETY: Single threaded access.
                unsafe { ALLOC_BUFFER_LENGTH -= 1 };
                // SAFETY: Single threaded access.
                Ok(unsafe { ALLOC_BUFFER_LENGTH })
            }
        },
        |index_str| {
            index_str
                .parse()
                .map_err(|_| "Argument for 'index' is not a valid usize")
        },
    )?;
    allocator
        .get_mut(index)
        .map_or(Err("Index is out of bounds"), |val| {
            if matches!(val, MaybeAlloc::None) {
                Err("Slot at index is already deallocated")
            } else {
                *val = MaybeAlloc::None;
                // SAFETY: Single threaded access.
                if index == unsafe { ALLOC_BUFFER_LENGTH - 1 } {
                    // SAFETY: Single threaded access.
                    unsafe {
                        ALLOC_BUFFER_LENGTH -= 1;
                    }
                }
                Ok(())
            }
        })
}

/// Executes a command `command`, with arguments `args`.
pub fn exec_command(command: &str, args: &mut Split<char>) {
    let result: Result<(), &str> = match command {
        "bumpa" => exec_bumpa(),
        "pagea" => exec_pagea(args),
        "slaba" => exec_slaba(args),
        "alloc" => exec_alloc(args),
        "palloc" => exec_palloc(args),
        "dealloc" => exec_dealloc(args),
        _ => {
            println!("Unknown command!");
            Ok(())
        }
    };
    if result.is_err() {
        println!("Error while executing command: {}", result.err().unwrap());
    }
}
