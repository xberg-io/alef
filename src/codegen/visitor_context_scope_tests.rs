use std::cell::Cell;

#[path = "visitor_context_scope.rs"]
mod scope;
use scope::CallbackAccessorScope;

thread_local! {
    static ATTRIBUTE_SCOPE: CallbackAccessorScope<String> = const { CallbackAccessorScope::new() };
}

fn read(identity: usize) -> Option<String> {
    ATTRIBUTE_SCOPE.with(|scope| scope.read(identity))
}

#[test]
fn unused_accessor_never_reads_the_borrowed_context() {
    let calls = Cell::new(0);
    CallbackAccessorScope::with(
        &ATTRIBUTE_SCOPE,
        1,
        || {
            calls.set(calls.get() + 1);
            String::from("value")
        },
        || assert_eq!(read(2), None),
    );

    assert_eq!(read(1), None);
    assert_eq!(calls.get(), 0);
}

#[test]
fn nested_scopes_can_read_both_live_contexts_and_expire_independently() {
    let outer = String::from("outer");
    let inner = String::from("inner");
    CallbackAccessorScope::with(
        &ATTRIBUTE_SCOPE,
        1,
        || outer.clone(),
        || {
            CallbackAccessorScope::with(
                &ATTRIBUTE_SCOPE,
                2,
                || inner.clone(),
                || {
                    assert_eq!(read(1).as_deref(), Some("outer"));
                    assert_eq!(read(2).as_deref(), Some("inner"));
                },
            );
            assert_eq!(read(1).as_deref(), Some("outer"));
            assert_eq!(read(2), None);
        },
    );
}

#[test]
fn borrowed_contexts_are_unavailable_across_threads_and_after_unwinding() {
    let panic = std::panic::catch_unwind(|| {
        CallbackAccessorScope::with(
            &ATTRIBUTE_SCOPE,
            1,
            || String::from("secret"),
            || {
                std::thread::spawn(|| assert_eq!(read(1), None)).join().unwrap();
                panic!("unwind the callback");
            },
        );
    });
    assert!(panic.is_err());
    assert_eq!(read(1), None);
}
