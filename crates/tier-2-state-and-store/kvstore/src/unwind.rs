//! Go's panic unwinding inside a `defer recover()` scope.
//!
//! A Go panic (and `ConsumeGas` past the limit) stops the goroutine at once
//! and unwinds to the nearest `recover`: no code after it runs, whatever
//! the callers would have done with an error. The Rust port returns these
//! as error values, which a caller can wrap, replace or ignore. [`scope`]
//! records the first such event raised inside it, so the scope's owner
//! (runTx, gov's `safeExecuteHandler`, ...) can fail with it as Go does;
//! after a panic gas consumption stops (Go's meter is not touched again).
//! Outside every scope nothing is recorded.

use std::cell::RefCell;

/// The first unwinding event of a scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unwind {
    /// A Go panic with this value (the `go_panic` log).
    Panic(String),
    /// `ConsumeGas` past the limit (the meter keeps the details).
    OutOfGas,
}

thread_local! {
    static FRAMES: RefCell<Vec<Option<Unwind>>> = const { RefCell::new(Vec::new()) };
}

/// Pops the frame even if `f` unwinds (a Rust panic).
struct Frame;

impl Drop for Frame {
    fn drop(&mut self) {
        FRAMES.with(|f| {
            f.borrow_mut().pop();
        });
    }
}

/// Runs `f` in a new scope and returns its result with the first unwinding
/// event raised inside it (in this scope, not in nested ones).
pub fn scope<T>(f: impl FnOnce() -> T) -> (T, Option<Unwind>) {
    FRAMES.with(|fr| fr.borrow_mut().push(None));
    let guard = Frame;
    let out = f();
    let ev = FRAMES.with(|fr| fr.borrow_mut().last_mut().and_then(Option::take));
    drop(guard);
    (out, ev)
}

fn raise(ev: Unwind) {
    FRAMES.with(|fr| {
        if let Some(top) = fr.borrow_mut().last_mut()
            && top.is_none()
        {
            *top = Some(ev);
        }
    });
}

/// A Go panic with value `log` (no-op outside a scope, or after the
/// scope's first event).
pub fn raise_panic(log: &str) {
    raise(Unwind::Panic(log.to_owned()));
}

/// The first out-of-gas consumption of a meter.
pub(crate) fn raise_out_of_gas() {
    raise(Unwind::OutOfGas);
}

/// Whether the innermost scope has panicked: Go's goroutine is unwinding,
/// so nothing consumes gas any more.
#[must_use]
pub fn panicking() -> bool {
    FRAMES.with(|fr| matches!(fr.borrow().last(), Some(Some(Unwind::Panic(_)))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_event_wins_and_scopes_nest() {
        let ((), ev) = scope(|| {
            raise_panic("a");
            raise_panic("b");
        });
        assert_eq!(ev, Some(Unwind::Panic("a".into())));
        let ((inner, ()), outer) = scope(|| {
            let ((), inner) = scope(|| raise_panic("inner"));
            assert!(!panicking());
            (inner, ())
        });
        assert_eq!(inner, Some(Unwind::Panic("inner".into())));
        assert_eq!(outer, None);
    }

    #[test]
    fn nothing_recorded_outside_a_scope() {
        raise_panic("x");
        assert!(!panicking());
        let ((), ev) = scope(|| ());
        assert_eq!(ev, None);
    }

    #[test]
    fn out_of_gas_first_keeps_a_later_panic_out() {
        let ((), ev) = scope(|| {
            raise_out_of_gas();
            raise_panic("late");
            assert!(!panicking());
        });
        assert_eq!(ev, Some(Unwind::OutOfGas));
    }

    #[test]
    fn frame_popped_on_rust_panic() {
        let r = std::panic::catch_unwind(|| scope(|| panic!("boom")));
        assert!(r.is_err());
        FRAMES.with(|f| assert!(f.borrow().is_empty()));
    }
}
