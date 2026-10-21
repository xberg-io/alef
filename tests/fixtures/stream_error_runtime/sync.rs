fn events() -> impl tokio_stream::Stream<Item = Result<u32, &'static str>> {
    let mode = std::env::var("STREAM_MODE").unwrap();
    let mut items = if mode == "immediate" {
        vec![]
    } else {
        vec![Ok(1), Ok(2)]
    };
    if mode != "clean" {
        items.push(Err(if mode == "wrong" { "different" } else { "limited" }));
    }
    tokio_stream::iter(items)
}
#[cfg(feature = "infallible")]
pub fn chat_stream() -> impl tokio_stream::Stream<Item = Result<u32, &'static str>> {
    events()
}
#[cfg(not(feature = "infallible"))]
pub fn chat_stream() -> Result<impl tokio_stream::Stream<Item = Result<u32, &'static str>>, &'static str> {
    if std::env::var("STREAM_MODE").unwrap() == "immediate" {
        Err("limited")
    } else {
        Ok(events())
    }
}
