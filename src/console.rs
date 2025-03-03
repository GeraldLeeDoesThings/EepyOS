use core::str::Split;

use alloc::{alloc::Global, boxed::Box};

use crate::{
    heap::{get_bump_addr, PageAllocator, PAGE_ALLOCATOR, PAGE_SIZE, SLAB_ALLOCATOR},
    println,
    sync::Mutex,
};

#[allow(unused)]
enum MaybeAlloc {
    None,
    PageAlloc(Box<[u8], &'static Mutex<PageAllocator>>),
    SlabAlloc(Box<[u8], Global>),
}

static ALLOC_BUFFER: Mutex<[MaybeAlloc; ALLOC_BUFFER_MAX_LENGTH]> =
    Mutex::new([const { MaybeAlloc::None }; ALLOC_BUFFER_MAX_LENGTH]);
static mut ALLOC_BUFFER_LENGTH: usize = 0;
const ALLOC_BUFFER_MAX_LENGTH: usize = 32;

fn exec_bumpa() -> Result<(), &'static str> {
    println!("Bump Addr: {:p}", get_bump_addr());
    Ok(())
}

fn exec_pagea(args: &mut Split<char>) -> Result<(), &'static str> {
    let grain: usize = args
        .next()
        .ok_or("Missing first argument for 'grain'")?
        .parse()
        .map_err(|_| "Argument for 'grain' is not a valid usize")?;
    PAGE_ALLOCATOR
        .lock_blocking()
        .dump_at_grain(grain)
        .map_err(|_| "Error while dumping page allocator memory")
}

fn exec_slaba(args: &mut Split<char>) -> Result<(), &'static str> {
    let block_size: u16 = args
        .next()
        .ok_or("Missing first argument for 'block size'")?
        .parse()
        .map_err(|_| "Argument for 'block size' is not a valid usize")?;
    SLAB_ALLOCATOR
        .lock_blocking()
        .dump_slot(block_size)
        .map_err(|_| "Error while dumping slab allocator memory")
}

fn exec_alloc(args: &mut Split<char>) -> Result<(), &'static str> {
    let mut allocator = ALLOC_BUFFER.lock_mut().unwrap();
    unsafe {
        let block_size: u16 = args
            .next()
            .ok_or("Missing first argument for 'block size'")?
            .parse()
            .map_err(|_| "Argument for 'block size' is not a valid usize")?;
        let index: usize = args.next().map_or_else(
            || Ok(ALLOC_BUFFER_LENGTH),
            |index_str| {
                index_str
                    .parse()
                    .map_err(|_| "Argument for 'index' is not a valid usize")
            },
        )?;
        if let Some(val) = allocator.get(index) {
            match val {
                MaybeAlloc::None => {
                    *allocator.get_mut(index).unwrap() = MaybeAlloc::SlabAlloc(
                        Box::new_uninit_slice(block_size as usize).assume_init(),
                    );
                    if index >= ALLOC_BUFFER_LENGTH {
                        ALLOC_BUFFER_LENGTH = index + 1;
                    }
                    Ok(())
                }
                _ => Err("Failed to allocate with global allocator"),
            }
        } else {
            Ok(())
        }
    }
}

fn exec_palloc(args: &mut Split<char>) -> Result<(), &'static str> {
    let mut allocator = ALLOC_BUFFER.lock_mut().unwrap();
    unsafe {
        let num_pages: u16 = args
            .next()
            .ok_or("Missing first argument for 'number of pages'")?
            .parse()
            .map_err(|_| "Argument for 'number of pages' is not a valid usize")?;
        let index: usize = args.next().map_or_else(
            || Ok(ALLOC_BUFFER_LENGTH),
            |index_str| {
                index_str
                    .parse()
                    .map_err(|_| "Argument for 'index' is not a valid usize")
            },
        )?;
        if let Some(val) = allocator.get(index) {
            match val {
                MaybeAlloc::None => {
                    *allocator.get_mut(index).unwrap() = MaybeAlloc::PageAlloc(
                        Box::new_uninit_slice_in(
                            (num_pages - 1) as usize * PAGE_SIZE + 1,
                            &PAGE_ALLOCATOR,
                        )
                        .assume_init(),
                    );
                    if index >= ALLOC_BUFFER_LENGTH {
                        ALLOC_BUFFER_LENGTH = index + 1;
                    }
                    Ok(())
                }
                _ => Err("Slot at index is already allocated"),
            }
        } else {
            Ok(())
        }
    }
}

fn exec_dealloc(args: &mut Split<char>) -> Result<(), &'static str> {
    let mut allocator = ALLOC_BUFFER.lock_mut().unwrap();
    unsafe {
        let index: usize = args.next().map_or_else(
            || Ok(ALLOC_BUFFER_LENGTH),
            |index_str| {
                index_str
                    .parse()
                    .map_err(|_| "Argument for 'index' is not a valid usize")
            },
        )?;
        if let Some(val) = allocator.get_mut(index) {
            match val {
                MaybeAlloc::None => Err("Slot at index is already deallocated"),
                _ => {
                    *val = MaybeAlloc::None;
                    if index == ALLOC_BUFFER_LENGTH - 1 {
                        ALLOC_BUFFER_LENGTH -= 1;
                    }
                    Ok(())
                }
            }
        } else {
            Err("Index is out of bounds")
        }
    }
}

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
