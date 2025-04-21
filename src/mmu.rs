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

/// Implements simple atomic bit accesses and updates, given a bitshift.
macro_rules! impl_bit_access {
    ($name:ident, $shift:literal) => {
        paste! {
            /// Reads some bitfield of this entry.
            fn [<is_ $name>](&self) -> bool {
                (self.data.load(SeqCst) >> $shift) & 1 > 0
            }

            /// Updates some bitfield of this entry.
            ///
            /// # Safety
            ///
            /// Either this entry must not currently be in use, or the result of this function call invalidates the entry.
            /// Ideally, the valid bit will not be set.
            ///
            unsafe fn [<set_ $name>](&mut self, $name: bool) {
                let bitmask = usize::from($name) << $shift;
                self.data.update(SeqCst, SeqCst, |data| (data & !bitmask) | bitmask);
            }
        }
    };
}

/// Implements getters and setters for physical page numbers, which
/// accept bitmasks to select the correct bits.
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
        /// Reads part of the physical page number.
        fn $getter(&self) -> usize {
            (self.data.load(SeqCst) & ($bits << $shift)) >> $shift
        }

        /// Sets part of the physical page number.
        ///
        /// # Safety
        ///
        /// This entry must not currently be in use. Ideally, the valid bit will not be
        /// set.
        ///
        /// Further, the result of setting this part of the physical page number must
        /// result in this entry pointing to another page table if this entry is marked
        /// as a valid pointer.
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
    #[allow(improper_ctypes, reason = "Pointer type is only for typechecking")]
    fn activate_page_table(table: *const Sv39PageTable, address_space: usize) -> usize;
    fn emit_mmu_fence_asm();
}

/// Emits an SFENCE.VMA instruction, which syncs mmu buffers.
fn emit_mmu_fence() {
    // SAFETY: Nothing can go wrong with this.
    unsafe {
        emit_mmu_fence_asm();
    }
}

global_asm!(include_str!("mmu.S"));

#[allow(clippy::missing_docs_in_private_items, reason = "Self descriptive")]
#[allow(unused, reason = "All variants may be used in the future")]
#[derive(Clone, Copy, Debug)]
pub enum PagePermissions {
    ReadOnly = 0b001,
    ReadWrite = 0b011,
    ExecuteOnly = 0b100,
    ReadExecute = 0b101,
    ReadWriteExecute = 0b111,
}

impl PagePermissions {
    /// Returns `true` if these permissions allow reading.
    const fn read_allowed(self) -> bool {
        self as u8 & 0b001 > 0
    }

    /// Returns `true` if these permissions allow writing.
    const fn write_allowed(self) -> bool {
        self as u8 & 0b010 > 0
    }

    /// Returns `true` if these permissions allow rexecution.
    const fn execute_allowed(self) -> bool {
        self as u8 & 0b100 > 0
    }
}

/// An entry in a 39 bit page table. Essentially a [`usize`] with a
/// ton of covenience methods.
#[derive(Debug)]
#[repr(transparent)]
struct Sv39PageTableEntry {
    /// The underlying data.
    data: AtomicUsize,
}

#[allow(unused, reason = "Other access patterns may appear later")]
impl Sv39PageTableEntry {
    impl_bit_access!(valid, 0);
    impl_bit_access!(readable, 1);
    impl_bit_access!(writable, 2);
    impl_bit_access!(executable, 3);
    impl_bit_access!(user_mode_accessible, 4);
    impl_bit_access!(global, 5);
    impl_bit_access!(napot, 63);

    /// Returns `true` if this entry has possibly been accessed since the
    /// accessed bit has been cleared.
    fn accessed(&self) -> bool {
        // SAFETY: Trivially satisified by reference to self data.
        unsafe { (read_volatile(&self.data).load(SeqCst) >> 6) & 1 > 0 }
    }

    /// Clears the accessed bit in this entry.
    fn clear_accessed(&mut self) {
        // SAFETY: Trivially satisified by reference to self data.
        unsafe {
            read_volatile(&self.data).fetch_and(!(1 << 6), SeqCst);
        }
    }

    /// Sets the accessed bit in this entry.
    #[allow(
        clippy::needless_pass_by_ref_mut,
        reason = "Mutable reference is to limit access to this function"
    )]
    fn set_accessed(&mut self) {
        self.data.fetch_or((1 << 6), SeqCst);
    }

    /// Returns `true` if this entry has possibly been written to since
    /// the dirty bit has been cleared.
    fn dirty(&self) -> bool {
        // SAFETY: Trivially satisified by reference to self data.
        unsafe { (read_volatile(&self.data).load(SeqCst) >> 7) & 1 > 0 }
    }

    /// Clears the dirty bit in this entry.
    fn clear_dirty(&mut self) {
        // SAFETY: Trivially satisified by reference to self data.
        unsafe {
            read_volatile(&self.data).fetch_and(!(1 << 7), SeqCst);
        }
    }

    /// Sets the dirty bit in this entry.
    #[allow(
        clippy::needless_pass_by_ref_mut,
        reason = "Mutable reference is to limit access to this function"
    )]
    fn set_dirty(&mut self) {
        self.data.fetch_or((1 << 7), SeqCst);
    }

    /// Returns the bits reserved for supervisor (thats us!) use.
    #[allow(clippy::cast_possible_truncation, reason = "Truncation is impossible")]
    fn get_reserved(&self) -> u8 {
        ((self.data.load(SeqCst) & (0b11 << 8)) >> 8) as u8
    }

    /// Sets the bits reserved for supervisor (thats us!) use. Despite this
    /// function accepting a [`u8`], only two bits are available.
    ///
    /// # Errors
    ///
    /// If `reserved` does not fit in two bits, `Err` is returned.
    #[allow(
        clippy::needless_pass_by_ref_mut,
        reason = "Mutable reference is to limit access to this function"
    )]
    fn set_reserved(&mut self, reserved: u8) -> Result<(), ()> {
        if reserved > 0b11 {
            return Err(());
        }

        self.data.update(SeqCst, SeqCst, |data| {
            (data & !(0b11 << 8)) ^ (usize::from(reserved) << 8)
        });
        Ok(())
    }

    /// Sets the bits reserved for supervisor (thats us!) use. Despite this
    /// function accepting a [`u8`], only two bits are available.
    ///
    /// # Safety
    ///
    /// Only a *single* core may update these reserved bits at a time. This is
    /// needed since these reserved bits are sometimes used to count
    /// references, which may briefly drop to zero and cause problems without
    /// limiting access in this way.
    ///
    /// # Errors
    ///
    /// If `reserved` does not fit in two bits, `Err` is returned.
    unsafe fn set_reserved_atomic(&self, reserved: u8) -> Result<(), ()> {
        if reserved > 0b11 {
            return Err(());
        }

        self.data.update(SeqCst, SeqCst, |data| {
            (data & !(0b11 << 8)) ^ (usize::from(reserved) << 8)
        });
        Ok(())
    }

    /// Swaps the bits reserved for supervisor (thats us!) use. Despite this
    /// function accepting a [`u8`], only two bits are available, so both
    /// `current` and `new` are truncated. On a success, `Ok` is returned
    /// containing the new value of [`Self::data`].
    ///
    /// # Errors
    ///
    /// If `current` does not match the current value of [`Self::data`], an
    /// `Err` is returned containing the actual current value.
    fn atomic_swap_reserved(&self, current: u8, new: u8) -> Result<usize, usize> {
        let current_cleared = self.data.load(SeqCst) & !(0b11 << 8);
        let current_full = current_cleared | (usize::from(current) << 8);
        let new_full = current_cleared | (usize::from(new) << 8);
        self.data
            .compare_exchange(current_full, new_full, SeqCst, SeqCst)
    }

    /// Returns `true` of this entry is a pointer to another page table.
    fn is_pointer(&self) -> bool {
        !(self.is_readable() || self.is_writable() || self.is_executable())
    }

    /// Returns a mutable reference to a page table pointed to by this entry.
    /// If this entry is not a pointer, or a mutable reference cannot be obained
    /// currently, `None` is returned instead.
    fn as_pointer_mut(&self) -> Option<Sv39PageTableMutRef> {
        if !self.is_pointer() {
            return None;
        }
        // SAFETY: By the safety requirements of setting this entry to a pointer.
        unsafe {
            Sv39PageTableMutRef::new((self.get_physical_page_number() << 12) as *mut Sv39PageTable)
        }
    }

    /// Returns a reference to a page table pointed to by this entry.
    /// If this entry is not a pointer, or a reference cannot be obained
    /// currently, `None` is returned instead.
    ///
    /// Call [`Self::as_pointer_blocking`] to retry and panic instead of
    /// returning `None`.
    fn as_pointer(&self) -> Option<Sv39PageTableRef> {
        Sv39PageTableRef::new((self.get_physical_page_number() << 12) as *mut Sv39PageTable)
    }

    /// Returns a reference to a page table pointed to by this entry.
    ///
    /// # Panics
    ///
    /// Panics if this entry is not a pointer, or it acquiring a reference takes
    /// too long. Call [`Self::as_pointer`] to obtain a reference without
    /// panicing.
    fn as_pointer_blocking(&self) -> Sv39PageTableRef {
        let mut attempts = 0;
        let mut attempt = self.as_pointer();
        while attempt.is_none() && attempts < MAX_LOCK_ACQUIRE_CYCLES {
            attempts += 1;
            attempt = self.as_pointer();
        }
        attempt.expect("Failed to obtain reference to subtable fast enough.")
    }

    /// Returns a mutable reference to a page table pointed to by this entry.
    ///
    /// # Panics
    ///
    /// Panics if this entry is not a pointer, or it acquiring a mutable
    /// reference takes too long. Call [`Self::as_pointer_mut`] to obtain a
    /// reference without panicing.
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

    /// Retrieves the physical page number corresponding with `level`, or `None`
    /// if no such page number exists.
    fn get_physical_page_number_for_level(&self, level: u8) -> Option<usize> {
        match level {
            2 => Some(self.get_physical_page_number_level_2()),
            1 => Some(self.get_physical_page_number_level_1()),
            0 => Some(self.get_physical_page_number_level_0()),
            _ => None,
        }
    }

    /// Sets the physical page number corresponding with `level`. Returns `Err`
    /// if there is no physical page number to update for `level`.
    ///
    /// # Safety
    ///
    /// This entry must not currently be in use. Ideally, the valid bit will not
    /// be set.
    ///
    /// Further, the result of setting this part of the physical page number
    /// must result in this entry pointing to another page table if this
    /// entry is marked as a valid pointer.
    unsafe fn set_physical_page_number_for_level(
        &mut self,
        level: u8,
        physical_page_number: usize,
    ) -> Result<(), ()> {
        match level {
            // SAFETY: By the safety requirements of this function.
            2 => unsafe { self.set_physical_page_number_level_2(physical_page_number) },
            // SAFETY: By the safety requirements of this function.
            1 => unsafe { self.set_physical_page_number_level_1(physical_page_number) },
            // SAFETY: By the safety requirements of this function.
            0 => unsafe { self.set_physical_page_number_level_0(physical_page_number) },
            _ => Err(()),
        }
    }

    /// Sets the permissions of the page mapped by this entry to match
    /// `permissions`.
    ///
    /// # Safety
    ///
    /// This entry must not currently be in use. Ideally, the valid bit will not
    /// be set.
    unsafe fn apply_permissions(&mut self, permissions: PagePermissions) {
        // SAFETY: By the safety requirements of this function.
        unsafe {
            self.set_readable(permissions.read_allowed());
        }
        // SAFETY: By the safety requirements of this function.
        unsafe {
            self.set_writable(permissions.write_allowed());
        }
        // SAFETY: By the safety requirements of this function.
        unsafe {
            self.set_executable(permissions.execute_allowed());
        }
    }

    /// Sets this entry to map from `physical_address`, with permissions set
    /// from `permissions`.
    fn set_to_direct_mapping(&mut self, physical_address: usize, permissions: PagePermissions) {
        self.drop_pointer_ref_if_pointer();
        // SAFETY: Entry is not in use after this function call.
        unsafe {
            self.set_valid(false);
        }
        // SAFETY: Entry is invalid.
        unsafe {
            self.set_physical_page_number((physical_address & 0xFFF_FFFF_FFFF) >> 12);
        }
        // SAFETY: Entry is invalid.
        unsafe {
            self.apply_permissions(permissions);
        }
        // SAFETY: Entry is invalid.
        unsafe {
            self.set_valid(true);
        }
        emit_mmu_fence();
    }

    /// Sets this entry as a pointer to another page table.
    fn set_to_pointer(&mut self, table: &Sv39PageTableMutRef) {
        self.drop_pointer_ref_if_pointer();
        // SAFETY: Entry is not in use after this function call.
        unsafe {
            self.set_valid(false);
        }
        let table_address =
            core::ptr::from_ref::<Sv39PageTable>(Pin::get_ref(table.as_ref())) as usize;
        // SAFETY: Entry is invalid, and reference points to a valid table.
        unsafe {
            self.set_physical_page_number((table_address & 0xFFF_FFFF_FFFF) >> 12);
        }
        /// SAFETY: Entry is invalid.
        unsafe {
            self.set_readable(false);
        }
        // SAFETY: Entry is invalid.
        unsafe {
            self.set_writable(false);
        }
        // SAFETY: Entry is invalid.
        unsafe {
            self.set_executable(false);
        }
        let mut table_ref = table.as_ref().acquire_reference_lock();
        // SAFETY: Entry is invalid.
        unsafe { table_ref.set_parent_reference_alive() };
        // SAFETY: Entry is invalid.
        unsafe {
            self.set_valid(true);
        }
        emit_mmu_fence();
    }

    /// Updates the reference count of a page table pointed to by this entry, if
    /// one exists.
    fn drop_pointer_ref_if_pointer(&mut self) {
        if self.is_pointer() && self.is_valid() {
            let subtable_ref = self.as_pointer_blocking();
            // SAFETY: Pointer is no longer in use.
            unsafe {
                self.set_valid(false);
            }
            // SAFETY: Pointer is no longer in use, and will reset this reference if it
            // points to this subtable again.
            unsafe {
                subtable_ref
                    .acquire_reference_lock()
                    .clear_parent_reference_alive();
            }
            emit_mmu_fence();
        }
    }

    /// Creates a new entry that starts fully zeroed. Notably, this new entry is
    /// invalid.
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

/// A table entry represented by a reference to a table and an index into it.
#[allow(unused, reason = "May be read from in the future")]
#[derive(Debug)]
pub struct TaggedSv39PageTableEntry {
    /// The table which `index` is taken into.
    table: Sv39PageTableRef,
    /// An index into `table` leading to a desired table entry.
    index: usize,
}

impl TaggedSv39PageTableEntry {
    /// Creates a new [`TaggedSv39PageTableEntry`] from `table` and `index`.
    fn new(table: Option<Sv39PageTableRef>, index: usize) -> Option<Self> {
        assert!(index <= Sv39PageTable::NUM_ENTRIES);
        Some(Self {
            table: table?,
            index,
        })
    }
}

/// An error that occurs when translating a virtual address.
#[allow(unused, reason = "May be used in the future")]
#[derive(Debug)]
pub enum VirtualAddressTranslationError {
    /// Virtual addresses are 64 bits wide, but only 39 bits are used. Due to
    /// this, the upper 25 bits must match the most significant bit of the
    /// 39 lower bits.
    UpperBitsMalformed,
    /// The virtual address leads to a [`Sv39PageTableEntry`] whose valid bit is
    /// unset.
    InvalidEntry(Option<TaggedSv39PageTableEntry>),
    /// The virtual address leads to a pointer in a level zero table, which
    /// cannot possibly be valid, as level zero tables map the least
    /// significant bits of the page table number that the [`Sv39PageTable`]
    /// format supports.
    LevelZeroPointer(Option<TaggedSv39PageTableEntry>),
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

/// An error that occurs when setting an address translation.
#[allow(unused, reason = "May be used in the future")]
pub enum VirtualAddressSetMappingError {
    /// The table level the translation was set to live in is impossible.
    ImpossibleLevel(u8),
    /// The mapping this translation would occupy is currently a pointer to
    /// a subtable that is alive (valid).
    MappingIsActivePointer(Option<TaggedSv39PageTableEntry>),
    /// The mapping this translation would occupy is currently in use.
    AddressAlreadyInUse(Option<TaggedSv39PageTableEntry>),
}

/// A mutable reference to a [`Sv39PageTable`]. Reference abstractions are
/// needed since reference counting is done manually.
///
/// See [`Sv39PageTableRef`] for a non-mutable reference.
#[derive(Debug)]
#[repr(transparent)]
struct Sv39PageTableMutRef {
    /// A pinned pointer to the [`Sv39PageTable`] which this references.
    table: ManuallyDrop<Pin<&'static mut Sv39PageTable>>,
}

impl Sv39PageTableMutRef {
    /// Creates a new [`Sv39PageTableMutRef`] from a raw pointer, or returns
    /// `None` if the [`Sv39PageTableReferenceCounterHandle`] could not be
    /// acquired, or if another reference to (the value pointed to by)
    /// `table` already exists.
    ///
    /// # Safety
    ///
    /// `table` must point to a [`Sv39PageTable`] or be null.
    ///
    /// # Panics
    ///
    /// This function panics if `table` is null.
    unsafe fn new(table: *mut Sv39PageTable) -> Option<Self> {
        // SAFETY: By safety requirements of this function.
        let table_ref = unsafe { table.as_mut() };
        // SAFETY: Pointer is pinned, and is never moved out of.
        let pinned_table =
            unsafe { Pin::new_unchecked(table_ref.expect("Created ref to null page table!")) };
        let mut ref_lock = pinned_table.as_ref().acquire_reference_lock();

        if !ref_lock.claim_mut_reference() {
            return None;
        }

        // SAFETY: We just checked that no mutable references are held.
        unsafe {
            ref_lock.increment_reference_count();
        }

        Some(Self {
            table: ManuallyDrop::new(pinned_table),
        })
    }

    /// Creates a new [`Sv39PageTableMutRef`] from a raw pointer by repeatedly
    /// calling [`Self::new`] until it succeeds.
    ///
    /// # Safety
    ///
    /// `table` must point to a [`Sv39PageTable`] or be null.
    ///
    /// # Panics
    ///
    /// This function panics if `table` is null, or if it takes too long to
    /// succeed.
    #[allow(unused, reason = "May be used in the future")]
    unsafe fn new_blocking(table: *mut Sv39PageTable) -> Self {
        let mut attempts = 0;
        while attempts < MAX_LOCK_ACQUIRE_CYCLES {
            // SAFETY: By safety requirements of this function.
            if let Some(table_ref) = unsafe { Self::new(table) } {
                return table_ref;
            }
            attempts += 1;
        }
        panic!("Failed to obtain a mutable table reference in time.");
    }
}

impl Drop for Sv39PageTableMutRef {
    fn drop(&mut self) {
        let mut ref_table = self.table.as_ref().acquire_reference_lock();
        // SAFETY: This is a currently held mutable reference.
        unsafe {
            ref_table.decrement_reference_count();
        }
        // SAFETY: This is a mutable reference.
        unsafe {
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

/// A reference to a [`Sv39PageTable`]. Reference abstractions are
/// needed since reference counting is done manually.
///
/// See [`Sv39PageTableMutRef`] for a mutable reference.
#[derive(Debug)]
#[repr(transparent)]
struct Sv39PageTableRef {
    /// A pinned pointer to the [`Sv39PageTable`] which this references.
    table: ManuallyDrop<Pin<&'static Sv39PageTable>>,
}

impl Sv39PageTableRef {
    /// Creates a new [`Sv39PageTableRef`] from a raw pointer, or returns
    /// `None` if the [`Sv39PageTableReferenceCounterHandle`] could not be
    /// acquired, or if a mutable reference to (the value pointed to by)
    /// `table` already exists.
    ///
    /// # Safety
    ///
    /// `table` must point to a [`Sv39PageTable`] or be null.
    ///
    /// # Panics
    ///
    /// This function panics if `table` is null.
    fn new(table: *const Sv39PageTable) -> Option<Self> {
        // SAFETY: By safety requirements of this function.
        let table_ref = unsafe { table.as_ref() };
        // SAFETY: Pointer is pinned, and is never moved out of.
        let pinned_table =
            unsafe { Pin::new_unchecked(table_ref.expect("Created ref to null page table!")) };
        let mut ref_lock = pinned_table.acquire_reference_lock();
        if ref_lock.has_mut_reference() {
            return None;
        }
        // SAFETY: We just checked that no mutable reference is held, and none can be
        // made until we release this handle.
        unsafe {
            ref_lock.increment_reference_count();
        }
        Some(Self {
            table: ManuallyDrop::new(pinned_table),
        })
    }

    /// Creates a new [`Sv39PageTableRef`] from a raw pointer by repeatedly
    /// calling [`Self::new`] until it succeeds.
    ///
    /// # Safety
    ///
    /// `table` must point to a [`Sv39PageTable`] or be null.
    ///
    /// # Panics
    ///
    /// This function panics if `table` is null, or if it takes too long to
    /// succeed.
    #[allow(unused, reason = "May be used in the future")]
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
        let mut ref_table = self.table.as_ref().acquire_reference_lock();
        assert!(!ref_table.has_mut_reference());
        // SAFETY: No new mutable references should have been created while this
        // reference lived.
        unsafe {
            ref_table.decrement_reference_count();
        }
    }
}

impl Deref for Sv39PageTableRef {
    type Target = Pin<&'static Sv39PageTable>;

    fn deref(&self) -> &Self::Target {
        &self.table
    }
}

/// A mutex guard like reference to a [`Sv39PageTable`], used to guard
/// accesses to the reference counters of that table.
struct Sv39PageTableReferenceCounterHandle {
    /// The table whose references are being guarded.
    table: *mut Sv39PageTable,
}

impl Sv39PageTableReferenceCounterHandle {
    /// Code for an unclaimed reference counter (of any type).
    const UNSET: u8 = 0;
    /// Code for a claimed mutable reference counter.
    const MUT_REF_HELD: u8 = 1;
    /// Code for a claimed parent reference counter.
    const PARENT_REF_ALIVE: u8 = 1;

    /// Creates a new [`Sv39PageTableReferenceCounterHandle`] from a pointer to
    /// a [`Sv39PageTable`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that at most a single
    /// [`Sv39PageTableReferenceCounterHandle`] exists per [`Sv39PageTable`],
    /// across all threads (cores / harts).
    ///
    /// Further, `table` must point to a [`Sv39PageTable`] that will outlive
    /// this handle.
    ///
    /// This function is essentially only designed to be called by
    /// [`Sv39PageTable::acquire_reference_lock`].
    unsafe fn new(table: *mut Sv39PageTable) -> Self {
        let mut handle = Self { table };
        // SAFETY: This is a handle.
        unsafe {
            handle.increment_reference_count();
        }
        handle
    }

    /// Returns the number of (non-mutable) references to [`Self::table`].
    fn reference_count(&self) -> u8 {
        // SAFETY: By safety requirements of [`Self::new`].
        let table: &Sv39PageTable = unsafe { self.table.as_ref().unwrap() };
        table.entries[Sv39PageTable::REFERENCE_COUNT_INDEX_0].get_reserved()
            | (table.entries[Sv39PageTable::REFERENCE_COUNT_INDEX_1].get_reserved() << 2)
            | (table.entries[Sv39PageTable::REFERENCE_COUNT_INDEX_2].get_reserved() << 4)
            | (table.entries[Sv39PageTable::REFERENCE_COUNT_INDEX_3].get_reserved() << 6)
    }

    /// Returns `true` if a mutable reference to [`Self::table`] is currently
    /// held.
    fn has_mut_reference(&self) -> bool {
        // SAFETY: By safety requirements of [`Self::new`].
        let table: &Sv39PageTable = unsafe { self.table.as_ref().unwrap() };
        table.entries[Sv39PageTable::MUT_REFERENCE_COUNT_INDEX].get_reserved() > 0
    }

    /// Attempts to claim a mutable reference to [`Self::table`], returning
    /// `true` on a success, and `false` if there are other references to
    /// [`Self::table`] other than the one held by this handle itself.
    fn claim_mut_reference(&mut self) -> bool {
        // SAFETY: By safety requirements of [`Self::new`].
        let table: &Sv39PageTable = unsafe { self.table.as_ref().unwrap() };

        // Reference count will be at least 1 because of this handle
        if self.reference_count() > 1 {
            return false;
        }

        table.entries[Sv39PageTable::MUT_REFERENCE_COUNT_INDEX]
            .atomic_swap_reserved(0, Self::MUT_REF_HELD)
            .is_ok()
    }

    /// Releases a claimed mutable reference to [`Self::table`].
    ///
    /// # Safety
    ///
    /// This function must only be called when a [`Sv39PageTableMutRef`] is
    /// dropped, or demoted to a [`Sv39PageTableRef`]. Further, it must be
    /// called on a handle referring to the same underlying
    /// [`Sv39PageTable`].
    ///
    /// # Panics
    ///
    /// This function panics if a mutable reference is not currently held.
    unsafe fn release_mut_reference(&mut self) {
        // SAFETY: By safety requirements of [`Self::new`].
        let table: &Sv39PageTable = unsafe { self.table.as_ref().unwrap() };
        table.entries[Sv39PageTable::MUT_REFERENCE_COUNT_INDEX]
            .atomic_swap_reserved(Self::MUT_REF_HELD, Self::UNSET)
            .expect("Failed to release mut reference.");
    }

    /// Increments the number of (non-mutable) references held to
    /// [`Self::table`].
    ///
    /// # Safety
    ///
    /// This function does not check if mutable references are currently held.
    /// Therefore, when this function is called, either:
    ///
    /// - The caller must be a [`Sv39PageTableReferenceCounterHandle`].
    /// - The caller must ensure no mutable references are held (such as with
    ///   [`Self::has_mut_reference`]).
    ///
    /// # Panics
    ///
    /// If an overflow occurs as a result of incrementing the reference count.
    unsafe fn increment_reference_count(&mut self) {
        // SAFETY: By safety requirements of [`Self::new`].
        let table: &mut Sv39PageTable = unsafe { self.table.as_mut().unwrap() };
        for i in Sv39PageTable::REFERENCE_COUNTS {
            // SAFETY: Synchronized by accessing through this handle.
            match unsafe {
                table.entries[i].set_reserved_atomic(table.entries[i].get_reserved() + 1)
            } {
                Ok(()) => return,
                // SAFETY: Synchronized by accessing through this handle.
                Err(()) => unsafe {
                    table.entries[i].set_reserved_atomic(0).expect(
                        "Failed to zero bits out in page table reference count when carrying.",
                    );
                },
            }
        }
        panic!("Reference count for page table overflowed.");
    }

    /// Decrements the number of (non-mutable) references held to
    /// [`Self::table`].
    ///
    /// # Safety
    ///
    /// This function does not check if mutable references are currently held.
    /// Therefore, when this function is called, either:
    ///
    /// - The caller must be a [`Sv39PageTableReferenceCounterHandle`].
    /// - The caller must be a [`Sv39PageTableMutRef`].
    /// - The caller must ensure no mutable references are held (such as with
    ///   [`Self::has_mut_reference`]).
    ///
    /// # Panics
    ///
    /// If an underflow occurs as a result of decrementing the reference count.
    unsafe fn decrement_reference_count(&mut self) {
        // SAFETY: By safety requirements of [`Self::new`].
        let table: &mut Sv39PageTable = unsafe { self.table.as_mut().unwrap() };
        for i in Sv39PageTable::REFERENCE_COUNTS {
            let current = table.entries[i].get_reserved();
            if current == 0 {
                // SAFETY: Synchronized by accessing through this handle.
                unsafe {
                    table.entries[i].set_reserved_atomic(0b11).expect(
                        "Failed to fill bits in page table reference count when borrowing.",
                    );
                };
            } else {
                // SAFETY: Synchronized by accessing through this handle.
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

    /// Marks this table's parent reference as alive, preventing this table from
    /// freeing its underlying memory.
    ///
    /// # Safety
    ///
    /// Only this table's parent may call this function, and it may do so
    /// exactly once.
    ///
    /// # Panics
    ///
    /// Panics if the parent reference is already marked as alive at the time of
    /// calling.
    unsafe fn set_parent_reference_alive(&mut self) {
        // SAFETY: By safety requirements of [`Self::new`].
        let table: &mut Sv39PageTable = unsafe { self.table.as_mut().unwrap() };
        table.entries[Sv39PageTable::PARENT_REFERENCE_INDEX]
            .atomic_swap_reserved(Self::UNSET, Self::PARENT_REF_ALIVE)
            .expect("Failed to set parent reference as alive.");
    }

    /// Marks this table's parent reference as dead, possibly allowing this
    /// table to free its underlying memory.
    ///
    /// # Safety
    ///
    /// Only this table's parent may call this function, and it may do so
    /// exactly once.
    ///
    /// # Panics
    ///
    /// Panics if the parent reference is not marked as alive at the time of
    /// calling.
    unsafe fn clear_parent_reference_alive(&mut self) {
        // SAFETY: By safety requirements of [`Self::new`].
        let table: &mut Sv39PageTable = unsafe { self.table.as_mut().unwrap() };
        table.entries[Sv39PageTable::PARENT_REFERENCE_INDEX]
            .atomic_swap_reserved(Self::PARENT_REF_ALIVE, Self::UNSET)
            .expect("Failed to unset parent reference as alive.");
    }

    /// Returns `true` if this table is pointed to by a parent table.
    fn has_living_parent_reference(&self) -> bool {
        // SAFETY: By safety requirements of [`Self::new`].
        let table: &Sv39PageTable = unsafe { self.table.as_ref().unwrap() };
        table.entries[Sv39PageTable::PARENT_REFERENCE_INDEX].get_reserved()
            == Self::PARENT_REF_ALIVE
    }
}

impl Drop for Sv39PageTableReferenceCounterHandle {
    fn drop(&mut self) {
        // SAFETY: [`Self::table`] is initially converted from a reference.
        let table_ref = unsafe { self.table.as_mut().expect("Page table pointer is null!") };
        // SAFETY: Reference is uniquely owned, and should never be moved anyways.
        let table: Pin<&mut Sv39PageTable> = unsafe { Pin::new_unchecked(table_ref) };

        // SAFETY: This is a [`Sv39PageTableReferenceCounterHandle`].
        unsafe {
            self.decrement_reference_count();
        }

        if self.reference_count() == 0
            && !self.has_mut_reference()
            && !self.has_living_parent_reference()
        {
            // SAFETY: Either no more references to `table` exist, or all remaining pointers
            // to it are about to be dropped anyways.
            drop(unsafe { Box::from_raw(self.table) });
        } else {
            // SAFETY: By the safety requirements of [`Self::new`].
            unsafe {
                table.as_ref().release_reference_lock();
            }
        }
    }
}

/// A page table following the RISC-V Sv39 format.
/// See <https://five-embeddev.com/riscv-priv-isa-manual/Priv-v1.12/supervisor.html#sec:sv39>
#[derive(Debug)]
#[repr(align(4096))]
pub struct Sv39PageTable {
    /// The mappings in this page table.
    entries: [Sv39PageTableEntry; Self::NUM_ENTRIES],
    /// Phantom type to ensure pinned pointers to this table cannot be moved.
    _pin: PhantomPinned,
}

#[allow(unused, reason = "Unused functions are predected to be useful later.")]
impl Sv39PageTable {
    /// Index to the page table entry storing this table's level in its reserved
    /// bits.
    const LEVEL_INDEX: usize = 0;
    /// Index to the page table entry storing the least significant (1st and
    /// 2nd) bits of this table's reference count in its reserved bits.
    const REFERENCE_COUNT_INDEX_0: usize = 1;
    /// Index to the page table entry storing the 3rd and 4th bits of this
    /// table's reference count in its reserved bits.
    const REFERENCE_COUNT_INDEX_1: usize = 2;
    /// Index to the page table entry storing the 5th and 6th bits of this
    /// table's reference count in its reserved bits.
    const REFERENCE_COUNT_INDEX_2: usize = 3;
    /// Index to the page table entry storing the most significant (7th and 8st)
    /// bits of this table's reference count in its reserved bits.
    const REFERENCE_COUNT_INDEX_3: usize = 4;
    /// The reference count indices stored in a conveniently iterable array.
    const REFERENCE_COUNTS: [usize; 4] = [
        Self::REFERENCE_COUNT_INDEX_0,
        Self::REFERENCE_COUNT_INDEX_1,
        Self::REFERENCE_COUNT_INDEX_2,
        Self::REFERENCE_COUNT_INDEX_3,
    ];
    /// Index to the page table entry whose reserved bits serve as a bitflag
    /// signaling if a mutable reference is held to it.
    const MUT_REFERENCE_COUNT_INDEX: usize = 5;
    /// Index to the page table entry whose reserved bits serve as a bitflag
    /// signaling if a reference counter handle is held to it.
    const REFERENCE_LOCK_INDEX: usize = 6;
    /// Index to the page table entry whose reserved bits serve as a bitflag
    /// signaling if this page table has a parent table.
    const PARENT_REFERENCE_INDEX: usize = 7;

    /// (Maximum) number of mappings in this table.
    const NUM_ENTRIES: usize = 512;

    /// Creates a new root (level 2) page table.
    /// If a subtable is needed instead, consider [`Self::new_subtable`]
    /// instead.
    pub fn new() -> Pin<Box<Self>> {
        assert_eq!(usize::BITS, 64);
        // SAFETY: All zeroes is a valid initial state for a page table.
        let mut new_table: Pin<Box<Self>> =
            unsafe { Box::into_pin(Box::new_zeroed().assume_init()) };
        Pin::as_mut(&mut new_table)
            .set_level(2)
            .expect("Failed to set Sv39 page table level!");
        // SAFETY: Parent is the resulting Box<Self>. Rust can manage those references
        // as ususal.
        unsafe {
            new_table
                .as_ref()
                .acquire_reference_lock()
                .set_parent_reference_alive();
        }
        new_table
    }

    /// Creates a new subtable under this table.
    fn new_subtable(self: Pin<&Self>) -> Sv39PageTableMutRef {
        // SAFETY: All zeroes is a valid initial state for a page table.
        let boxed_table = unsafe { Box::new_zeroed().assume_init() };
        // SAFETY: Pin is made around the only reference to the boxed memory. It cannot
        // therefore will not move.
        let mut new_table: Pin<&mut Self> = unsafe { Pin::new_unchecked(Box::leak(boxed_table)) };
        Pin::as_mut(&mut new_table)
            .set_level(self.as_ref().level() - 1)
            .expect("Failed to set Sv39 page table level!");
        // SAFETY: Pointer will not be moved, as it is guarded by the
        // Sv39PageTableMutRef.
        let new_table_ref = unsafe { new_table.get_unchecked_mut() };
        // SAFETY: new_table_ref points to a box we just leaked, and so is well formed.
        unsafe {
            Sv39PageTableMutRef::new(new_table_ref)
                .expect("New table somehow already has references")
        }
    }

    /// Sets this page table up to map virtual addresses to the exact same
    /// physical address.
    pub fn flat_map(self: Pin<&mut Self>) {
        let level = self.as_ref().level();
        // SAFETY: We don't move out of inner_self.
        let mut inner_self = unsafe { self.get_unchecked_mut() };
        for (index, entry) in inner_self.entries.iter_mut().enumerate() {
            // SAFETY: Invalidates the entry.
            unsafe {
                entry.set_valid(false);
            }
            // SAFETY: Entry is invalid.
            unsafe {
                entry
                    .set_physical_page_number_for_level(level, index)
                    .expect("Sv39 page table has too many entries!");
            }
            // SAFETY: Entry is invalid.
            unsafe {
                entry.apply_permissions(PagePermissions::ReadWriteExecute);
            }
            entry.set_accessed();
            entry.set_dirty();
            // SAFETY: Entry is invalid.
            unsafe {
                entry.set_valid(true);
            }
        }
        emit_mmu_fence();
    }

    /// Retrieves the level of this page table.
    fn level(self: Pin<&Self>) -> u8 {
        let level = self.entries[Self::LEVEL_INDEX].get_reserved();
        assert!(level <= 2);
        level
    }

    /// Creates a new [`Sv39PageTableReferenceCounterHandle`] that references
    /// this table.
    ///
    /// # Panics
    ///
    /// This function panics if it cannot acquire locks to create the
    /// [`Sv39PageTableReferenceCounterHandle`] in time.
    #[must_use]
    fn acquire_reference_lock(self: Pin<&Self>) -> Sv39PageTableReferenceCounterHandle {
        let mut attempts: usize = 0;
        while attempts < MAX_LOCK_ACQUIRE_CYCLES {
            if self.entries[Self::REFERENCE_LOCK_INDEX]
                .atomic_swap_reserved(0, 1)
                .is_ok()
            {
                // SAFETY: Ref will not be moved out of, since it will be guarded.
                let self_ref = unsafe { Pin::into_inner_unchecked(self) };
                // SAFETY: Setting reference lock ensure the table will live long enough.
                return unsafe {
                    Sv39PageTableReferenceCounterHandle::new(
                        core::ptr::from_ref::<Self>(self_ref).cast_mut(),
                    )
                };
            }
            attempts += 1;
        }
        panic!("Failed to acquire page table reference lock in time.");
    }

    /// Releases a lock preventing additional
    /// [`Sv39PageTableReferenceCounterHandle`]s from being made referencing
    /// this page table.
    ///
    /// # Safety
    ///
    /// The caller must be a [`Sv39PageTableReferenceCounterHandle`] created by
    /// previously calling [`Self::acquire_reference_lock`] on this paget
    /// able.
    ///
    /// # Panics
    ///
    /// This function panics if the lock is not currently held.
    unsafe fn release_reference_lock(self: Pin<&Self>) {
        self.entries[Self::REFERENCE_LOCK_INDEX]
            .atomic_swap_reserved(1, 0)
            .expect("Failed to release page table lock.");
    }

    /// Sets the level of this table. This can cause some checks to fail,
    /// eventually leading to enexpected page faults if this is not set
    /// carefully. The valid values for `level` are 0, 1, or 2.
    ///
    /// # Errors
    ///
    /// This function returns an error if the value for `level` is not possible.
    fn set_level(self: Pin<&mut Self>, level: u8) -> Result<(), ()> {
        if level > 2 {
            return Err(());
        }
        // SAFETY: Unpinned value is only read from (not moved).
        unsafe { Pin::get_unchecked_mut(self).entries[0].set_reserved(level) }
    }

    /// Sets this page table as the currently active, root page table on this
    /// core/hart.
    pub fn activate(self: Pin<&mut Self>) {
        // TODO: Error if table level is not 2.
        // SAFETY: Just a small assembly wrapper.
        let result = unsafe { activate_page_table(Pin::get_ref(self.as_ref()), 0) };
        println!("Satp: {:#02x}", result);
        emit_mmu_fence();
    }

    /// Attempts to create a [`TaggedSv39PageTableEntry`] referencing the
    /// `index`th entry of this table, returning `None` if a reference to
    /// this table cannot be created.
    fn make_tagged_entry(self: Pin<&Self>, index: usize) -> Option<TaggedSv39PageTableEntry> {
        Some(TaggedSv39PageTableEntry {
            table: Sv39PageTableRef::new(Pin::get_ref(self))?,
            index,
        })
    }

    /// Maps from `virtual_address` to `physical_address` with this table,
    /// creating subtables as nescessary. The grain of the mapping is
    /// determined by `level`, where lower levels are more detailed.
    /// Particularly, `12 + level * 9` bits of detail are mapped, starting
    /// with the least significant bits.
    ///
    /// # Errors
    ///
    /// This function errors if:
    /// - The requested `level` is impossible.
    /// - The entry needed for the mapping is an active pointer to a subtable.
    /// - The entry needed is an active mapping.
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
        // SAFETY: Unpinned pointer is read from and not moved out of.
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
            let mut subtable = self.as_ref().new_subtable();
            let subtable_map_result =
                subtable
                    .as_mut()
                    .set_map(virtual_address, physical_address, level, permissions);
            if subtable_map_result.is_ok() {
                // SAFETY: Unpinned pointer is read from and not moved out of.
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

    /// Determines the resulting physical address of mapping `virtual_address`
    /// with this table, or an error describing what went wrong with the
    /// translation.
    ///
    /// # Errors
    ///
    /// This function errors if:
    /// - The entry corresponding to `virtual_address` is invalid.
    /// - The entry corresponding to `virtual_address` is a pointer, but this is
    ///   a level 0 table.
    /// - The upper bits of `virtual_address` are malformed. The upper bits must
    ///   match 39th least significant bit of `virtual_address`.
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
                TaggedSv39PageTableEntry::new(Sv39PageTableRef::new(Pin::get_ref(self)), index),
            ));
        }

        if page_table_entry.is_pointer() {
            if self.level() == 0 {
                return Err(VirtualAddressTranslationError::LevelZeroPointer(
                    TaggedSv39PageTableEntry::new(Sv39PageTableRef::new(Pin::get_ref(self)), index),
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
