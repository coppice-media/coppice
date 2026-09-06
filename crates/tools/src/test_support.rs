//! Test-only helpers shared by every module in this crate.
//!
//! Compiled under `cfg(test)` only; nothing here ships.

use std::sync::{Mutex, MutexGuard};

/// Serializes every test that writes an executable *and* spawns it.
///
/// `execve` fails with `ETXTBSY` ("Text file busy") while any process holds the
/// image open for writing. `fs::write` closes its descriptor promptly, but a
/// concurrent `Command::spawn` on another test thread forks a child that
/// inherits that descriptor and keeps it open until its own `exec`. Several
/// modules now write fake external binaries into temp dirs and run them
/// (`calibre`, `calibre_polish`, …), so the lock has to be **one lock for the
/// whole crate**: a per-module mutex serializes a module against itself and
/// leaves the cross-module window wide open, which showed up as
/// `Io(ExecutableFileBusy)` in whichever module lost the race.
static FAKE_BINARY_LOCK: Mutex<()> = Mutex::new(());

/// Take the crate-wide fake-binary lock; hold the guard for as long as the
/// fake binaries exist, i.e. for the fixture's whole lifetime.
///
/// Every test that writes an executable file and then spawns any process must
/// hold it, or it reintroduces the race for everyone else.
pub(crate) fn fake_binary_lock() -> MutexGuard<'static, ()> {
	// A failing test poisons the mutex; the guarded data is `()`, so recover
	// rather than cascade that failure into every other test.
	FAKE_BINARY_LOCK
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner())
}
