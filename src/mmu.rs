use core::{
    arch::global_asm,
    error::Error,
    fmt::{Debug, Display},
    marker::PhantomPinned,
    mem::ManuallyDrop,
    ops::{Deref, DerefMut},
    pin::Pin,
    ptr::read_volatile,
    sync::atomic::{AtomicUsize, Ordering::SeqCst},
};

use alloc::boxed::Box;
use paste::paste;

use crate::{consts::MAX_LOCK_ACQUIRE_CYCLES, println};

macro_rules! impl_bit_access {
    ($name:ident, $shift:literal) => {
        paste! {
            fn [<is_ $name>](&self) -> bool {
                (self.data.load(SeqCst) >> $shift) & 1 > 0
            }

            unsafe fn [<set_ $name>](&mut self, $name: bool) {
                let bitmask = usize::from($name) << $shift;
                self.data.update(SeqCst, SeqCst, |data| (data & !bitmask) | bitmask);
            }
        }
    };
}

macro_rules! impl_physical_page_number_access {
    ($name:ident, $bits:literal, $shift:literal) => {
        paste! {
            impl_physical_page_number_access!(
                [<get_physical_page_number_ $name>],
                [<set_physical_page_number_ $name>],
                $bits,
                $shift
            );
        }
    };
    ($bits:literal, $shift:literal) => {
        impl_physical_page_number_access!(
            get_physical_page_number,
            set_physical_page_number,
            $bits,
            $shift
        );
    };
    ($getter:ident, $setter:ident, $bits:literal, $shift:literal) => {
        fn $getter(&self) -> usize {
            (self.data.load(SeqCst) & ($bits << $shift)) >> $shift
        }

        unsafe fn $setter(&mut self, physical_page_number: usize) -> Result<(), ()> {
            if physical_page_number > $bits {
                return Err(());
            }

            self.data.update(SeqCst, SeqCst, |data| {
                (data & !($bits << $shift)) ^ (physical_page_number << $shift)
            });
            Ok(())
        }
    };
}

extern "C" {
    #[allow(improper_ctypes)]
    fn activate_page_table(table: *const Sv39PageTable, address_space: usize) -> usize;
    fn emit_mmu_fence_asm();
}

fn emit_mmu_fence() {
    unsafe {
        emit_mmu_fence_asm();
    }
}

global_asm!(include_str!("mmu.S"));

#[allow(unused)]
#[derive(Clone, Copy, Debug)]
pub enum PagePermissions {
    ReadOnly = 0b001,
    ReadWrite = 0b011,
    ExecuteOnly = 0b100,
    ReadExecute = 0b101,
    ReadWriteExecute = 0b111,
}

impl PagePermissions {
    const fn read_allowed(self) -> bool {
        self as u8 & 0b001 > 0
    }

    const fn write_allowed(self) -> bool {
        self as u8 & 0b010 > 0
    }

    const fn execute_allowed(self) -> bool {
        self as u8 & 0b100 > 0
    }
}

#[derive(Debug)]
#[repr(transparent)]
struct Sv39PageTableEntry {
    data: AtomicUsize,
}

#[allow(unused)]
impl Sv39PageTableEntry {
    impl_bit_access!(valid, 0);
    impl_bit_access!(readable, 1);
    impl_bit_access!(writable, 2);
    impl_bit_access!(executable, 3);
    impl_bit_access!(user_mode_accessible, 4);
    impl_bit_access!(global, 5);
    impl_bit_access!(napot, 63);

    fn accessed(&self) -> bool {
        unsafe { (read_volatile(&self.data).load(SeqCst) >> 6) & 1 > 0 }
    }

    fn clear_accessed(&mut self) {
        unsafe {
            read_volatile(&self.data).fetch_and(!(1 << 6), SeqCst);
        }
    }

    fn set_accessed(&mut self) {
        unsafe {
            self.data.fetch_or((1 << 6), SeqCst);
        }
    }

    fn dirty(&self) -> bool {
        unsafe { (read_volatile(&self.data).load(SeqCst) >> 7) & 1 > 0 }
    }

    fn clear_dirty(&mut self) {
        unsafe {
            read_volatile(&self.data).fetch_and(!(1 << 7), SeqCst);
        }
    }

    fn set_dirty(&mut self) {
        unsafe {
            self.data.fetch_or((1 << 7), SeqCst);
        }
    }

    #[allow(clippy::cast_possible_truncation)]
    fn get_reserved(&self) -> u8 {
        ((self.data.load(SeqCst) & (0b11 << 8)) >> 8) as u8
    }

    #[allow(clippy::needless_pass_by_ref_mut)]
    fn set_reserved(&mut self, reserved: u8) -> Result<(), ()> {
        if reserved > 0b11 {
            return Err(());
        }

        self.data.update(SeqCst, SeqCst, |data| {
            (data & !(0b11 << 8)) ^ (usize::from(reserved) << 8)
        });
        Ok(())
    }

    unsafe fn set_reserved_atomic(&self, reserved: u8) -> Result<(), ()> {
        if reserved > 0b11 {
            return Err(());
        }

        self.data.update(SeqCst, SeqCst, |data| {
            (data & !(0b11 << 8)) ^ (usize::from(reserved) << 8)
        });
        Ok(())
    }

    fn atomic_swap_reserved(&self, current: u8, new: u8) -> Result<usize, usize> {
        let current_cleared = self.data.load(SeqCst) & !(0b11 << 8);
        let current_full = current_cleared | (usize::from(current) << 8);
        let new_full = current_cleared | (usize::from(new) << 8);
        self.data
            .compare_exchange(current_full, new_full, SeqCst, SeqCst)
    }

    fn is_pointer(&self) -> bool {
        !(self.is_readable() || self.is_writable() || self.is_executable())
    }

    fn as_pointer_mut(&self) -> Option<Sv39PageTableMutRef> {
        Sv39PageTableMutRef::new((self.get_physical_page_number() << 12) as *mut Sv39PageTable)
    }

    fn as_pointer(&self) -> Option<Sv39PageTableRef> {
        Sv39PageTableRef::new((self.get_physical_page_number() << 12) as *mut Sv39PageTable)
    }

    fn as_pointer_blocking(&self) -> Sv39PageTableRef {
        let mut attempts = 0;
        let mut attempt = self.as_pointer();
        while attempt.is_none() && attempts < MAX_LOCK_ACQUIRE_CYCLES {
            attempts += 1;
            attempt = self.as_pointer();
        }
        attempt.expect("Failed to obtain reference to subtable fast enough.")
    }

    fn as_pointer_mut_blocking(&self) -> Sv39PageTableMutRef {
        let mut attempts = 0;
        let mut attempt = self.as_pointer_mut();
        while attempt.is_none() && attempts < MAX_LOCK_ACQUIRE_CYCLES {
            attempts += 1;
            attempt = self.as_pointer_mut();
        }
        attempt.expect("Failed to obtain mutable reference to subtable fast enough.")
    }

    impl_physical_page_number_access!(0xFFF_FFFF_FFFF, 10);
    impl_physical_page_number_access!(level_0, 0x1FF, 10);
    impl_physical_page_number_access!(level_1, 0x1FF, 19);
    impl_physical_page_number_access!(level_2, 0x3FF_FFFF, 28);

    fn get_physical_page_number_for_level(&self, level: u8) -> Option<usize> {
        match level {
            2 => Some(self.get_physical_page_number_level_2()),
            1 => Some(self.get_physical_page_number_level_1()),
            0 => Some(self.get_physical_page_number_level_0()),
            _ => None,
        }
    }

    fn set_physical_page_number_for_level(
        &mut self,
        level: u8,
        physical_page_number: usize,
    ) -> Result<(), ()> {
        match level {
            2 => unsafe { self.set_physical_page_number_level_2(physical_page_number) },
            1 => unsafe { self.set_physical_page_number_level_1(physical_page_number) },
            0 => unsafe { self.set_physical_page_number_level_0(physical_page_number) },
            _ => Err(()),
        }
    }

    unsafe fn apply_permissions(&mut self, permissions: PagePermissions) {
        self.set_readable(permissions.read_allowed());
        self.set_writable(permissions.write_allowed());
        self.set_executable(permissions.execute_allowed());
    }

    fn set_to_direct_mapping(&mut self, physical_address: usize, permissions: PagePermissions) {
        unsafe {
            self.drop_pointer_ref_if_pointer();
            self.set_valid(false);
            self.set_physical_page_number((physical_address & 0xFFF_FFFF_FFFF) >> 12);
            self.apply_permissions(permissions);
            self.set_valid(true);
            emit_mmu_fence();
        }
    }

    fn set_to_pointer(&mut self, table: &Sv39PageTableMutRef) {
        unsafe {
            self.drop_pointer_ref_if_pointer();
            self.set_valid(false);
            let table_address =
                core::ptr::from_ref::<Sv39PageTable>(Pin::get_ref(table.as_ref())) as usize;
            self.set_physical_page_number((table_address & 0xFFF_FFFF_FFFF) >> 12);
            self.set_readable(false);
            self.set_writable(false);
            self.set_executable(false);
            let mut table_ref = table.as_ref().acquire_reference_lock();
            unsafe { table_ref.set_parent_reference_alive() };
            self.set_valid(true);
            emit_mmu_fence();
        }
    }

    fn drop_pointer_ref_if_pointer(&mut self) {
        if self.is_pointer() && self.is_valid() {
            let subtable_ref = self.as_pointer_blocking();
            unsafe {
                subtable_ref
                    .acquire_reference_lock()
                    .clear_parent_reference_alive();
            }
            unsafe {
                self.set_valid(false);
            }
            emit_mmu_fence();
        }
    }

    const fn new() -> Self {
        Self {
            data: AtomicUsize::new(0),
        }
    }
}

impl Drop for Sv39PageTableEntry {
    fn drop(&mut self) {
        self.drop_pointer_ref_if_pointer();
    }
}

#[allow(unused)]
#[derive(Debug)]
pub struct TaggedSv39PageTableEntry {
    table: Option<Sv39PageTableRef>,
    index: usize,
}

#[allow(unused)]
#[derive(Debug)]
pub enum VirtualAddressTranslationError {
    UpperBitsMalformed,
    InvalidEntry(TaggedSv39PageTableEntry),
    LevelZeroPointer(TaggedSv39PageTableEntry),
}

impl Display for VirtualAddressTranslationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UpperBitsMalformed => write!(f, "Upper bits of virtual address must match most significant bit used for translation."),
            _ => unimplemented!()
        }
    }
}

impl Error for VirtualAddressTranslationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }

    fn description(&self) -> &'static str {
        "description() is deprecated; use Display"
    }

    fn cause(&self) -> Option<&dyn Error> {
        self.source()
    }

    fn provide<'a>(&'a self, _request: &mut core::error::Request<'a>) {}
}

#[allow(unused)]
pub enum VirtualAddressSetMappingError {
    ImpossibleLevel(u8),
    MappingIsActivePointer(TaggedSv39PageTableEntry),
    AddressAlreadyInUse(TaggedSv39PageTableEntry),
}

#[derive(Debug)]
#[repr(transparent)]
struct Sv39PageTableMutRef {
    table: ManuallyDrop<Pin<&'static mut Sv39PageTable>>,
}

impl Sv39PageTableMutRef {
    fn new(table: *mut Sv39PageTable) -> Option<Self> {
        let pinned_table =
            unsafe { Pin::new_unchecked(table.as_mut().expect("Created ref to null page table!")) };
        let mut ref_lock = pinned_table.as_ref().acquire_reference_lock();

        if !ref_lock.claim_mut_reference() {
            return None;
        }

        ref_lock.increment_reference_count();

        Some(Self {
            table: ManuallyDrop::new(pinned_table),
        })
    }

    #[allow(unused)]
    fn new_blocking(table: *mut Sv39PageTable) -> Self {
        let mut attempts = 0;
        while attempts < MAX_LOCK_ACQUIRE_CYCLES {
            if let Some(table_ref) = Self::new(table) {
                return table_ref;
            }
            attempts += 1;
        }
        panic!("Failed to obtain a mutable table reference in time.");
    }
}

impl Drop for Sv39PageTableMutRef {
    fn drop(&mut self) {
        unsafe {
            let mut ref_table = self.table.as_ref().acquire_reference_lock();
            ref_table.decrement_reference_count();
            ref_table.release_mut_reference();
        }
    }
}

impl Deref for Sv39PageTableMutRef {
    type Target = Pin<&'static mut Sv39PageTable>;

    fn deref(&self) -> &Self::Target {
        &self.table
    }
}

impl DerefMut for Sv39PageTableMutRef {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.table
    }
}

#[allow(unused)]
#[derive(Debug)]
#[repr(transparent)]
struct Sv39PageTableRef {
    table: ManuallyDrop<Pin<&'static Sv39PageTable>>,
}

impl Sv39PageTableRef {
    fn new(table: *const Sv39PageTable) -> Option<Self> {
        let pinned_table =
            unsafe { Pin::new_unchecked(table.as_ref().expect("Created ref to null page table!")) };
        let mut ref_lock = pinned_table.acquire_reference_lock();
        if ref_lock.has_mut_reference() {
            return None;
        }
        ref_lock.increment_reference_count();
        Some(Self {
            table: ManuallyDrop::new(pinned_table),
        })
    }

    #[allow(unused)]
    fn new_blocking(table: *const Sv39PageTable) -> Self {
        let mut attempts = 0;
        while attempts < MAX_LOCK_ACQUIRE_CYCLES {
            if let Some(table_ref) = Self::new(table) {
                return table_ref;
            }
            attempts += 1;
        }
        panic!("Failed to obtain a table reference in time.");
    }
}

impl Drop for Sv39PageTableRef {
    fn drop(&mut self) {
        unsafe {
            self.table
                .as_ref()
                .acquire_reference_lock()
                .decrement_reference_count();
        }
    }
}

impl Deref for Sv39PageTableRef {
    type Target = Pin<&'static Sv39PageTable>;

    fn deref(&self) -> &Self::Target {
        &self.table
    }
}

struct Sv39PageTableReferenceCounterHandle {
    table: *mut Sv39PageTable,
}

impl Sv39PageTableReferenceCounterHandle {
    const UNSET: u8 = 0;
    const MUT_REF_HELD: u8 = 1;
    const PARENT_REF_ALIVE: u8 = 1;

    unsafe fn new(table: *mut Sv39PageTable) -> Self {
        let mut handle = Self { table };
        handle.increment_reference_count();
        handle
    }

    fn reference_count(&self) -> u8 {
        let table: &Sv39PageTable = unsafe { self.table.as_ref().unwrap() };
        table.entries[Sv39PageTable::REFERENCE_COUNT_INDEX_0].get_reserved()
            | (table.entries[Sv39PageTable::REFERENCE_COUNT_INDEX_1].get_reserved() << 2)
            | (table.entries[Sv39PageTable::REFERENCE_COUNT_INDEX_2].get_reserved() << 4)
            | (table.entries[Sv39PageTable::REFERENCE_COUNT_INDEX_3].get_reserved() << 6)
    }

    fn has_mut_reference(&self) -> bool {
        let table: &Sv39PageTable = unsafe { self.table.as_ref().unwrap() };
        table.entries[Sv39PageTable::MUT_REFERENCE_COUNT_INDEX].get_reserved() > 0
    }

    fn claim_mut_reference(&mut self) -> bool {
        let table: &Sv39PageTable = unsafe { self.table.as_ref().unwrap() };

        // Reference count will be at least 1 because of this handle
        if self.reference_count() > 1 {
            return false;
        }

        table.entries[Sv39PageTable::MUT_REFERENCE_COUNT_INDEX]
            .atomic_swap_reserved(0, Self::MUT_REF_HELD)
            .is_ok()
    }

    unsafe fn release_mut_reference(&mut self) {
        let table: &Sv39PageTable = unsafe { self.table.as_ref().unwrap() };
        table.entries[Sv39PageTable::MUT_REFERENCE_COUNT_INDEX]
            .atomic_swap_reserved(Self::MUT_REF_HELD, Self::UNSET)
            .expect("Failed to release mut reference.");
    }

    fn increment_reference_count(&mut self) {
        let table: &mut Sv39PageTable = unsafe { self.table.as_mut().unwrap() };
        for i in Sv39PageTable::REFERENCE_COUNTS {
            match unsafe {
                table.entries[i].set_reserved_atomic(table.entries[i].get_reserved() + 1)
            } {
                Ok(()) => return,
                Err(()) => unsafe {
                    table.entries[i].set_reserved_atomic(0).expect(
                        "Failed to zero bits out in page table reference count when carrying.",
                    );
                },
            }
        }
        panic!("Reference count for page table overflowed.");
    }

    unsafe fn decrement_reference_count(&mut self) {
        let table: &mut Sv39PageTable = unsafe { self.table.as_mut().unwrap() };
        for i in Sv39PageTable::REFERENCE_COUNTS {
            let current = table.entries[i].get_reserved();
            if current == 0 {
                unsafe {
                    table.entries[i].set_reserved_atomic(0b11).expect(
                        "Failed to fill bits in page table reference count when borrowing.",
                    );
                };
            } else {
                unsafe {
                    table.entries[i]
                        .set_reserved_atomic(current - 1)
                        .expect("Failed to decrement bits in page table reference count.");
                };
                return;
            }
        }
        panic!("Reference count for page table underflowed.");
    }

    unsafe fn set_parent_reference_alive(&mut self) {
        let table: &mut Sv39PageTable = unsafe { self.table.as_mut().unwrap() };
        table.entries[Sv39PageTable::PARENT_REFERENCE_INDEX]
            .atomic_swap_reserved(Self::UNSET, Self::PARENT_REF_ALIVE)
            .expect("Failed to set parent reference as alive.");
    }

    unsafe fn clear_parent_reference_alive(&mut self) {
        let table: &mut Sv39PageTable = unsafe { self.table.as_mut().unwrap() };
        table.entries[Sv39PageTable::PARENT_REFERENCE_INDEX]
            .atomic_swap_reserved(Self::PARENT_REF_ALIVE, Self::UNSET)
            .expect("Failed to unset parent reference as alive.");
    }

    fn has_living_parent_reference(&self) -> bool {
        let table: &Sv39PageTable = unsafe { self.table.as_ref().unwrap() };
        table.entries[Sv39PageTable::PARENT_REFERENCE_INDEX].get_reserved()
            == Self::PARENT_REF_ALIVE
    }
}

impl Drop for Sv39PageTableReferenceCounterHandle {
    fn drop(&mut self) {
        let table: Pin<&mut Sv39PageTable> =
            unsafe { Pin::new_unchecked(self.table.as_mut().unwrap()) };

        unsafe {
            self.decrement_reference_count();
        }

        if self.reference_count() == 0
            && !self.has_mut_reference()
            && !self.has_living_parent_reference()
        {
            drop(unsafe { Box::from_raw(self.table) });
        } else {
            unsafe {
                table.as_ref().release_reference_lock();
            }
        }
    }
}

#[derive(Debug)]
#[repr(align(4096))]
pub struct Sv39PageTable {
    entries: [Sv39PageTableEntry; 512],
    _pin: PhantomPinned,
}

#[allow(unused)]
impl Sv39PageTable {
    const LEVEL_INDEX: usize = 0;
    const REFERENCE_COUNT_INDEX_0: usize = 1;
    const REFERENCE_COUNT_INDEX_1: usize = 2;
    const REFERENCE_COUNT_INDEX_2: usize = 3;
    const REFERENCE_COUNT_INDEX_3: usize = 4;
    const REFERENCE_COUNTS: [usize; 4] = [
        Self::REFERENCE_COUNT_INDEX_0,
        Self::REFERENCE_COUNT_INDEX_1,
        Self::REFERENCE_COUNT_INDEX_2,
        Self::REFERENCE_COUNT_INDEX_3,
    ];
    const MUT_REFERENCE_COUNT_INDEX: usize = 5;
    const REFERENCE_LOCK_INDEX: usize = 6;
    const PARENT_REFERENCE_INDEX: usize = 7;

    pub fn new() -> Pin<Box<Self>> {
        assert_eq!(usize::BITS, 64);
        let mut new_table: Pin<Box<Self>> =
            unsafe { Box::into_pin(Box::new_zeroed().assume_init()) };
        Pin::as_mut(&mut new_table)
            .set_level(2)
            .expect("Failed to set Sv39 page table level!");
        unsafe {
            new_table
                .as_ref()
                .acquire_reference_lock()
                .set_parent_reference_alive();
        }
        new_table
    }

    unsafe fn new_subtable(self: Pin<&Self>) -> Sv39PageTableMutRef {
        let mut new_table: Pin<&mut Self> =
            unsafe { Pin::new_unchecked(Box::leak(Box::new_zeroed().assume_init())) };
        Pin::as_mut(&mut new_table)
            .set_level(self.as_ref().level() - 1)
            .expect("Failed to set Sv39 page table level!");
        Sv39PageTableMutRef::new(new_table.get_unchecked_mut())
            .expect("New table somehow already has references")
    }

    pub fn flat_map(self: Pin<&mut Self>) {
        let level = self.as_ref().level();
        let mut inner_self = unsafe { self.get_unchecked_mut() };
        for (index, entry) in inner_self.entries.iter_mut().enumerate() {
            unsafe {
                entry
                    .set_physical_page_number_for_level(level, index)
                    .expect("Sv39 page table has too many entries!");
                entry.apply_permissions(PagePermissions::ReadWriteExecute);
                entry.set_accessed();
                entry.set_dirty();
                entry.set_valid(true);
            }
        }
        emit_mmu_fence();
    }

    fn level(self: Pin<&Self>) -> u8 {
        self.entries[Self::LEVEL_INDEX].get_reserved()
    }

    #[must_use]
    fn acquire_reference_lock(self: Pin<&Self>) -> Sv39PageTableReferenceCounterHandle {
        let mut attempts: usize = 0;
        while attempts < MAX_LOCK_ACQUIRE_CYCLES {
            if self.entries[Self::REFERENCE_LOCK_INDEX]
                .atomic_swap_reserved(0, 1)
                .is_ok()
            {
                return unsafe {
                    Sv39PageTableReferenceCounterHandle::new(unsafe {
                        core::ptr::from_ref::<Self>(Pin::into_inner_unchecked(self)).cast_mut()
                    })
                };
            }
            attempts += 1;
        }
        panic!("Failed to acquire page table reference lock in time.");
    }

    unsafe fn release_reference_lock(self: Pin<&Self>) {
        self.entries[Self::REFERENCE_LOCK_INDEX]
            .atomic_swap_reserved(1, 0)
            .expect("Failed to release page table lock.");
    }

    fn set_level(self: Pin<&mut Self>, level: u8) -> Result<(), ()> {
        if level > 2 {
            return Err(());
        }
        unsafe { Pin::get_unchecked_mut(self).entries[0].set_reserved(level) }
    }

    pub fn activate(self: Pin<&mut Self>) {
        unsafe {
            println!(
                "Satp: {:#02x}",
                activate_page_table(Pin::get_ref(self.as_ref()), 0)
            );
        }
        emit_mmu_fence();
    }

    fn make_tagged_entry(self: Pin<&Self>, index: usize) -> TaggedSv39PageTableEntry {
        TaggedSv39PageTableEntry {
            table: Sv39PageTableRef::new(Pin::get_ref(self)),
            index,
        }
    }

    pub fn set_map(
        mut self: Pin<&mut Self>,
        virtual_address: usize,
        physical_address: usize,
        level: u8,
        permissions: PagePermissions,
    ) -> Result<(), VirtualAddressSetMappingError> {
        let current_level = self.as_ref().level();
        if level > current_level {
            return Err(VirtualAddressSetMappingError::ImpossibleLevel(level));
        }
        let offset = 12 + 9 * current_level;
        let index = (virtual_address & (0x1FF << offset)) >> offset;
        let mut page_table_entry = &mut unsafe { self.as_mut().get_unchecked_mut() }.entries[index];

        if level == current_level {
            if !page_table_entry.is_valid() {
                page_table_entry.set_to_direct_mapping(physical_address, permissions);
                return Ok(());
            }
            if page_table_entry.is_pointer() {
                return Err(VirtualAddressSetMappingError::MappingIsActivePointer(
                    self.as_ref().make_tagged_entry(index),
                ));
            }
            return Err(VirtualAddressSetMappingError::AddressAlreadyInUse(
                self.as_ref().make_tagged_entry(index),
            ));
        }
        if !page_table_entry.is_valid() {
            let mut subtable = unsafe { self.as_ref().new_subtable() };
            let subtable_map_result =
                subtable
                    .as_mut()
                    .set_map(virtual_address, physical_address, level, permissions);
            if subtable_map_result.is_ok() {
                unsafe { self.as_mut().get_unchecked_mut() }.entries[index]
                    .set_to_pointer(&subtable);
            }
            return subtable_map_result;
        } else if page_table_entry.is_pointer() {
            let mut subtable = page_table_entry.as_pointer_mut_blocking();
            return subtable.as_mut().set_map(
                virtual_address,
                physical_address,
                level,
                permissions,
            );
        }
        Err(VirtualAddressSetMappingError::AddressAlreadyInUse(
            self.as_ref().make_tagged_entry(index),
        ))
    }

    pub fn map(
        self: Pin<&Self>,
        virtual_address: usize,
    ) -> Result<usize, VirtualAddressTranslationError> {
        let high_bit = (virtual_address & (1 << 38)) >> 38;
        if (39..64)
            .map(|i| (virtual_address & (1 << i)) >> i)
            .any(|bit| bit != high_bit)
        {
            return Err(VirtualAddressTranslationError::UpperBitsMalformed);
        }

        let offset = 12 + 9 * self.level();
        let index = (virtual_address & (0x1FF << offset)) >> offset;
        assert!(index <= 0x1FF);

        let page_table_entry = &self.entries[index];

        if !page_table_entry.is_valid() {
            return Err(VirtualAddressTranslationError::InvalidEntry(
                TaggedSv39PageTableEntry {
                    table: Sv39PageTableRef::new(Pin::get_ref(self)),
                    index,
                },
            ));
        }

        if page_table_entry.is_pointer() {
            if self.level() == 0 {
                return Err(VirtualAddressTranslationError::LevelZeroPointer(
                    TaggedSv39PageTableEntry {
                        table: Sv39PageTableRef::new(Pin::get_ref(self)),
                        index,
                    },
                ));
            }
            let pointee = page_table_entry.as_pointer_blocking();
            return pointee.as_ref().map(virtual_address);
        }

        let mut physical_address = virtual_address & 0xFFF;

        for lower_level in 0..self.level() {
            // Copy bits more bits if this is a superpage
            physical_address |= virtual_address & (0x1FF << (12 + 9 * lower_level));
        }

        for level in self.level()..=2 {
            physical_address |= page_table_entry
                .get_physical_page_number_for_level(level)
                .expect("Failed to fetch physical page number")
                << (12 + 9 * level);
        }

        Ok(physical_address)
    }
}
