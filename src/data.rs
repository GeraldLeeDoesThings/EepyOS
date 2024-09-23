use alloc::{alloc::Global, boxed::Box, vec::Vec};
use core::{
    alloc::Allocator,
    sync::atomic::{AtomicUsize, Ordering::Relaxed},
    usize,
};

pub struct AtomicBitVec<A: Allocator = Global> {
    inner: Box<[AtomicUsize], A>,
    _length: usize,
}

impl<A: Allocator> AtomicBitVec<A> {
    pub fn new_in(size: usize, allocator: A) -> AtomicBitVec<A> {
        let num_elems = size.div_ceil(usize::BITS as usize);
        let mut inner = Vec::with_capacity_in(num_elems, allocator);
        inner.extend((0..num_elems).map(|_| AtomicUsize::new(0)));
        AtomicBitVec {
            inner: inner.into_boxed_slice(),
            _length: size,
        }
    }

    pub fn get(&self, index: usize) -> Option<bool> {
        let inner_index = index / usize::BITS as usize;
        let inner_offset = index % usize::BITS as usize;
        let packed = self.inner.get(inner_index)?.load(Relaxed);
        Some(packed & (1 << inner_offset) > 0)
    }

    pub fn set(&self, index: usize, val: bool) -> Option<bool> {
        let inner_index = index / usize::BITS as usize;
        let inner_offset = index % usize::BITS as usize;
        if val {
            self.inner
                .get(inner_index)?
                .fetch_or(1 << inner_offset, Relaxed);
        } else {
            self.inner
                .get(inner_index)?
                .fetch_and(!(1 << inner_offset), Relaxed);
        }
        Some(val)
    }

    pub fn _find_false(&self) -> Option<usize> {
        for (index, val) in self.inner.iter().enumerate() {
            let packed = val.load(Relaxed);
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
                packed.fetch_or(op, Relaxed);
            } else {
                packed.fetch_and(op, Relaxed);
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
                    )
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

    pub fn _len(&self) -> usize {
        self._length
    }
}
