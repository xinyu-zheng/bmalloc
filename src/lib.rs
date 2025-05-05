#![feature(allocator_api)]
#![feature(alloc_layout_extra)]

use core::{
    alloc::{AllocError, Allocator, GlobalAlloc, Layout},
    cmp::self,
    ptr::{self, NonNull},
};

#[repr(C)]
#[derive(Default)]
pub struct ProfileStats {
    /// Heap size in bytes (including area unmapped to OS).
    pub heapsize_full: usize,
    /// Total bytes contained in free and unmapped blocks.
    pub free_bytes_full: usize,
    /// Amount of memory unmapped to OS.
    pub unmapped_bytes: usize,
    /// Number of bytes allocated since the recent collection.
    pub bytes_allocd_since_gc: usize,
    /// Number of bytes allocated before the recent collection.
    /// The value may wrap.
    pub allocd_bytes_before_gc: usize,
    /// Number of bytes not considered candidates for garbage collection.
    pub non_gc_bytes: usize,
    /// Garbage collection cycle number.
    /// The value may wrap.
    pub gc_no: usize,
    /// Number of marker threads (excluding the initiating one).
    pub markers_m1: usize,
    /// Approximate number of reclaimed bytes after recent collection.
    pub bytes_reclaimed_since_gc: usize,
    /// Approximate number of bytes reclaimed before the recent collection.
    /// The value may wrap.
    pub reclaimed_bytes_before_gc: usize,
    /// Number of bytes freed explicitly since the recent GC.
    pub expl_freed_bytes_since_gc: usize,
}

#[link(name = "gc")]
extern "C" {
    pub fn GC_malloc(nbytes: usize) -> *mut u8;

    pub fn GC_posix_memalign(mem_ptr: *mut *mut u8, align: usize, nbytes: usize) -> i32;

    pub fn GC_realloc(old: *mut u8, new_size: usize) -> *mut u8;

    pub fn GC_free(dead: *mut u8);

    pub fn GC_base(mem_ptr: *mut u8) -> *mut u8;

    pub fn GC_register_finalizer(
        ptr: *mut u8,
        finalizer: Option<unsafe extern "C" fn(*mut u8, *mut u8)>,
        client_data: *mut u8,
        old_finalizer: *mut extern "C" fn(*mut u8, *mut u8),
        old_client_data: *mut *mut u8,
    );

    pub fn GC_register_finalizer_no_order(
        ptr: *mut u8,
        finalizer: Option<unsafe extern "C" fn(*mut u8, *mut u8)>,
        client_data: *mut u8,
        old_finalizer: *mut extern "C" fn(*mut u8, *mut u8),
        old_client_data: *mut *mut u8,
    );

    pub fn GC_gcollect();

    pub fn GC_thread_is_registered() -> u32;

    pub fn GC_pthread_create(
        native: *mut libc::pthread_t,
        attr: *const libc::pthread_attr_t,
        f: extern "C" fn(_: *mut libc::c_void) -> *mut libc::c_void,
        value: *mut libc::c_void,
    ) -> libc::c_int;

    pub fn GC_pthread_join(native: libc::pthread_t, value: *mut *mut libc::c_void) -> libc::c_int;

    pub fn GC_pthread_exit(value: *mut libc::c_void) -> !;

    pub fn GC_pthread_detach(thread: libc::pthread_t) -> libc::c_int;

    pub fn GC_init();

    pub fn GC_keep_alive(ptr: *mut u8);

    pub fn GC_set_finalize_on_demand(state: i32);

    pub fn GC_set_finalizer_notifier(f: extern "C" fn());

    pub fn GC_should_invoke_finalizers() -> u32;

    pub fn GC_invoke_finalizers() -> u64;

    pub fn GC_get_gc_no() -> u64;
}

// Fast-path for low alignment values
pub const MIN_ALIGN: usize = 8;

#[derive(Debug)]
pub struct GcAllocator;

unsafe impl GlobalAlloc for GcAllocator {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { gc_malloc(layout) }
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        //unsafe { gc_free(ptr, layout) }
    }

    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        unsafe { gc_realloc(ptr, layout, new_size) }
    }
}

#[inline]
unsafe fn gc_malloc(layout: Layout) -> *mut u8 {
    if layout.align() <= MIN_ALIGN && layout.align() <= layout.size() {
        unsafe { crate::GC_malloc(layout.size()) as *mut u8 }
    } else {
        let mut out = ptr::null_mut();
        // posix_memalign requires that the alignment be a multiple of `sizeof(void*)`.
        // Since these are all powers of 2, we can just use max.
        unsafe {
            let align = layout.align().max(core::mem::size_of::<usize>());
            let ret = crate::GC_posix_memalign(&mut out, align, layout.size());
            if ret != 0 { ptr::null_mut() } else { out as *mut u8 }
        }
    }
}

#[inline]
unsafe fn gc_realloc(ptr: *mut u8, old_layout: Layout, new_size: usize) -> *mut u8 {
    if old_layout.align() <= MIN_ALIGN && old_layout.align() <= new_size {
        unsafe { crate::GC_realloc(ptr, new_size) as *mut u8 }
    } else {
        unsafe {
            let new_layout = Layout::from_size_align_unchecked(new_size, old_layout.align());

            let new_ptr = gc_malloc(new_layout);
            if !new_ptr.is_null() {
                let size = cmp::min(old_layout.size(), new_size);
                ptr::copy_nonoverlapping(ptr, new_ptr, size);
                gc_free(ptr, old_layout);
            }
            new_ptr
        }
    }
}

#[inline]
unsafe fn gc_free(ptr: *mut u8, _: Layout) {
    unsafe {
        crate::GC_free(ptr);
    }
}

unsafe impl Allocator for GcAllocator {
    #[inline]
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        match layout.size() {
            0 => Ok(NonNull::slice_from_raw_parts(layout.dangling(), 0)),
            size => unsafe {
                let ptr = gc_malloc(layout);
                let ptr = NonNull::new(ptr).ok_or(AllocError)?;
                Ok(NonNull::slice_from_raw_parts(ptr, size))
            },
        }
    }

    unsafe fn deallocate(&self, _: NonNull<u8>, _: Layout) {}
}
