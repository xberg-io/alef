use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio_stream::StreamExt;
static SEEN: AtomicUsize = AtomicUsize::new(0);
async fn chat_stream() -> Result<impl tokio_stream::Stream<Item = Result<u32, &'static str>>, &'static str> {
    let mode = std::env::var("STREAM_MODE").unwrap();
    if mode == "immediate" {
        return Err("limited");
    }
    let mut items = vec![Ok(1), Ok(2)];
    if mode != "clean" {
        items.push(Err(if mode == "wrong" { "different" } else { "limited" }));
    }
    Ok(tokio_stream::iter(items).map(|item| {
        if item.is_ok() {
            SEEN.fetch_add(1, Ordering::SeqCst);
        }
        item
    }))
}
struct Wake;
impl std::task::Wake for Wake {
    fn wake(self: Arc<Self>) {}
}
fn main() {
    let waker = std::task::Waker::from(Arc::new(Wake));
    let mut context = std::task::Context::from_waker(&waker);
    let future = async { GENERATED_BODY };
    let mut future = std::pin::pin!(future);
    assert!(future.as_mut().poll(&mut context).is_ready());
    let expected = if std::env::var("STREAM_MODE").unwrap() == "immediate" {
        0
    } else {
        2
    };
    assert_eq!(SEEN.load(Ordering::SeqCst), expected);
}
