//! C5b: o tradutor não aloca em `translate` (alocador contador por thread, binário próprio
//! para não colidir com outro `#[global_allocator]`).

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use arrayvec::ArrayVec;
use relay_core::keymap::translator::{KeyEvent, KeymapTranslator};
use relay_core::types::KeymapConfig;

struct Counting;

thread_local! {
    static ALLOCS: Cell<u64> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static A: Counting = Counting;

#[test]
fn dez_mil_eventos_sem_alocar() {
    let mut t = KeymapTranslator::new(KeymapConfig::default());
    let mut out = ArrayVec::new();
    // Cmd+C, Cmd+Tab, Shift+Cmd+Z, Cmd+Backspace e uma tecla solta, em laço.
    let seq: [(u16, bool); 20] = [
        (0xE05B, true),
        (0x2E, true),
        (0x2E, false),
        (0xE05B, false),
        (0xE05B, true),
        (0x0F, true),
        (0x0F, false),
        (0xE05B, false),
        (0x2A, true),
        (0xE05B, true),
        (0x2C, true),
        (0x2C, false),
        (0xE05B, false),
        (0x2A, false),
        (0xE05B, true),
        (0x0E, true),
        (0x0E, false),
        (0xE05B, false),
        (0x1E, true),
        (0x1E, false),
    ];
    let before = ALLOCS.with(Cell::get);
    let mut n = 0;
    while n < 10_000 {
        for &(scan, down) in &seq {
            out.clear();
            let ev = KeyEvent {
                scan,
                down,
                autorepeat: false,
            };
            t.translate(ev, Some("WindowsTerminal.exe"), &mut out);
            n += 1;
        }
    }
    assert_eq!(ALLOCS.with(Cell::get) - before, 0);
}
