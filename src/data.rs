use alloc::{alloc::Global, boxed::Box, vec::Vec};
use core::{
    alloc::Allocator,
    fmt::{Debug, Write},
    sync::atomic::{
        AtomicUsize,
        Ordering::{AcqRel, Acquire, Relaxed},
    },
};

pub struct AtomicBitVec<A: Allocator = Global> {
    inner: Box<[AtomicUsize], A>,
    length: usize,
}

impl<A: Allocator> AtomicBitVec<A> {
    pub fn new_in(size: usize, allocator: A) -> Self {
        let num_elems = size.div_ceil(usize::BITS as usize);
        let mut inner = Vec::with_capacity_in(num_elems, allocator);
        (0..num_elems).for_each(|_| {
            inner
                .push_within_capacity(AtomicUsize::new(0))
                .expect("Bit vec inner does not have enough capacity");
        });
        let raw_inner = inner.into_boxed_slice();
        Self {
            inner: raw_inner,
            length: size,
        }
    }

    pub fn get(&self, index: usize) -> Option<bool> {
        if index >= self.length {
            return None;
        }
        let inner_index = index / usize::BITS as usize;
        let inner_offset = index % usize::BITS as usize;
        let packed = self.inner.get(inner_index)?.load(Acquire);
        Some(packed & (1 << inner_offset) > 0)
    }

    pub fn set(&self, index: usize, val: bool) -> Option<bool> {
        if index >= self.length {
            return None;
        }
        let inner_index = index / usize::BITS as usize;
        let inner_offset = index % usize::BITS as usize;
        if val {
            self.inner
                .get(inner_index)?
                .fetch_or(1 << inner_offset, AcqRel);
        } else {
            self.inner
                .get(inner_index)?
                .fetch_and(!(1 << inner_offset), AcqRel);
        }
        Some(val)
    }

    pub fn _find_false(&self) -> Option<usize> {
        for (index, val) in self.inner.iter().enumerate() {
            let packed = val.load(Acquire);
            if packed < usize::MAX {
                return Some(index * 8 + usize::BITS as usize - 1 - packed.leading_ones() as usize);
            }
        }
        None
    }

    pub fn _bulk_write(&self, lo_index: usize, hi_index: usize, val: bool) -> Option<usize> {
        fn generate_op(lo: usize, hi: usize, val: bool) -> usize {
            assert!(lo <= hi);
            if val {
                (lo..=hi).fold(0, |acc, offset| acc | (1 << offset))
            } else {
                (lo..=hi).fold(usize::MAX, |acc, offset| acc ^ (1 << offset))
            }
        }

        fn apply_op(packed: &AtomicUsize, op: usize, val: bool) {
            if val {
                packed.fetch_or(op, AcqRel);
            } else {
                packed.fetch_and(op, AcqRel);
            }
        }

        assert!(lo_index <= hi_index);
        let lo_inner_index = lo_index / usize::BITS as usize;
        let lo_inner_offset = lo_index % usize::BITS as usize;
        let hi_inner_index = hi_index / usize::BITS as usize;
        let hi_inner_offset = hi_index % usize::BITS as usize;
        match hi_inner_index - lo_inner_index {
            0 => {
                apply_op(
                    self.inner.get(lo_inner_index)?,
                    generate_op(lo_inner_offset, hi_inner_offset, val),
                    val,
                );
                Some(hi_inner_offset - lo_inner_offset + 1)
            }
            1 => {
                apply_op(
                    self.inner.get(lo_inner_index)?,
                    generate_op(lo_inner_offset, usize::BITS as usize - 1, val),
                    val,
                );
                apply_op(
                    self.inner.get(hi_inner_index)?,
                    generate_op(0, hi_inner_offset, val),
                    val,
                );
                Some(usize::BITS as usize - lo_inner_offset + hi_inner_offset + 1)
            }
            indices => {
                apply_op(
                    self.inner.get(lo_inner_index)?,
                    generate_op(lo_inner_offset, usize::BITS as usize - 1, val),
                    val,
                );
                apply_op(
                    self.inner.get(hi_inner_index)?,
                    generate_op(0, hi_inner_offset, val),
                    val,
                );
                let op = if val { usize::MAX } else { 0 };
                ((lo_inner_index + 1)..hi_inner_index).for_each(|index| {
                    apply_op(
                        self.inner
                            .get(index)
                            .expect("Ran out of values during mass write!"),
                        op,
                        val,
                    );
                });
                Some(
                    usize::BITS as usize - lo_inner_offset
                        + hi_inner_offset
                        + 1
                        + usize::BITS as usize * (indices - 2),
                )
            }
        }
    }

    pub const fn _len(&self) -> usize {
        self.length
    }
}

impl<A: Allocator> Debug for AtomicBitVec<A> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_char('[')?;
        for packed in &self.inner {
            write!(f, "{:b}", packed.load(Relaxed))?;
        }
        f.write_char(']')?;
        Ok(())
    }
}
