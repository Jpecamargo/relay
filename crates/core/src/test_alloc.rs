//! Alocador contador único do binário de teste da lib (compartilhado por C1 e C2).
//! Conta só na thread que chamou `measure`, para não depender de outros testes.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
    static BYTES: Cell<usize> = const { Cell::new(0) };
}

/// Conta alocações só da thread que ligou `COUNTING`, para não depender de outros testes.
struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let _ = COUNTING.try_with(|c| {
            if c.get() {
                ALLOCS.with(|a| a.set(a.get() + 1));
                BYTES.with(|b| b.set(b.get() + l.size()));
            }
        });
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let _ = COUNTING.try_with(|c| {
            if c.get() {
                ALLOCS.with(|a| a.set(a.get() + 1));
                BYTES.with(|b| b.set(b.get() + n));
            }
        });
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static A: Counting = Counting;

pub fn measure<R>(f: impl FnOnce() -> R) -> (R, usize, usize) {
    ALLOCS.with(|a| a.set(0));
    BYTES.with(|b| b.set(0));
    COUNTING.with(|c| c.set(true));
    let r = f();
    COUNTING.with(|c| c.set(false));
    (r, ALLOCS.with(Cell::get), BYTES.with(Cell::get))
}
