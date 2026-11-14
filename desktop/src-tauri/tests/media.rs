//! The `orochi-media://` answer, without a webview (`docs/conversation-tabs-design.md` §4.4).
use orochi::media::{self, MediaRef, Source};
use orochi_desktop::media::serve;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n0123456789abcdefghij";

fn kept(data: &std::path::Path, bytes: &[u8]) -> String {
    media::keep(
        data,
        "t1",
        &MediaRef {
            name: "a.png".into(),
            source: Source::Inline(bytes.to_vec()),
        },
    )
    .unwrap()
    .unwrap()
    .sha
}

fn header<'a>(reply: &'a orochi_desktop::media::Reply, name: &str) -> Option<&'a str> {
    reply
        .headers
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, v)| v.as_str())
}

#[test]
fn a_stored_file_is_answered_by_thread_and_hash_with_the_type_its_bytes_have() {
    let data = tempfile::tempdir().unwrap();
    let sha = kept(data.path(), PNG);
    let reply = serve(data.path(), &format!("/t1/{sha}"), None);
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body, PNG);
    assert_eq!(header(&reply, "Content-Type"), Some("image/png"));
    assert_eq!(header(&reply, "X-Content-Type-Options"), Some("nosniff"));
    assert_eq!(header(&reply, "Accept-Ranges"), Some("bytes"));
    assert_eq!(
        header(&reply, "Content-Length"),
        Some(PNG.len().to_string().as_str())
    );
}

#[test]
fn a_range_gets_a_partial_answer_and_one_that_cannot_be_met_gets_416() {
    let data = tempfile::tempdir().unwrap();
    let sha = kept(data.path(), PNG);
    let path = format!("/t1/{sha}");
    let reply = serve(data.path(), &path, Some("bytes=8-11"));
    assert_eq!(reply.status, 206);
    assert_eq!(reply.body, b"0123");
    assert_eq!(
        header(&reply, "Content-Range"),
        Some(format!("bytes 8-11/{}", PNG.len()).as_str())
    );
    let open_ended = serve(data.path(), &path, Some("bytes=20-"));
    assert_eq!(open_ended.body, &PNG[20..]);
    let suffix = serve(data.path(), &path, Some("bytes=-4"));
    assert_eq!(suffix.body, &PNG[PNG.len() - 4..]);
    let past = serve(data.path(), &path, Some("bytes=999-"));
    assert_eq!(past.status, 416);
    assert_eq!(
        header(&past, "Content-Range"),
        Some(format!("bytes */{}", PNG.len()).as_str())
    );
    // Several ranges are not served; the player is told the range cannot be met.
    assert_eq!(serve(data.path(), &path, Some("bytes=0-1,4-5")).status, 416);
}

#[test]
fn nothing_but_a_thread_and_a_hash_is_ever_opened() {
    let data = tempfile::tempdir().unwrap();
    let sha = kept(data.path(), PNG);
    std::fs::write(data.path().join("activity.sqlite3"), b"secret").unwrap();
    for path in [
        "/".to_string(),
        "/t1".to_string(),
        format!("/t1/{sha}/extra"),
        format!("/t2/{sha}"),
        format!("/../{sha}"),
        "/t1/../activity.sqlite3".to_string(),
        "/t1/abcd".to_string(),
        format!("/t1/{}", "0".repeat(64)),
        format!("/%2e%2e/{sha}"),
    ] {
        let reply = serve(data.path(), &path, None);
        assert_eq!(reply.status, 404, "{path}");
        assert!(!reply.body.windows(6).any(|w| w == b"secret"));
    }
}

#[test]
fn a_file_that_stopped_being_media_on_disk_is_not_served_as_anything() {
    let data = tempfile::tempdir().unwrap();
    let sha = kept(data.path(), PNG);
    let stored = media::file(data.path(), "t1", &sha).unwrap();
    std::fs::write(&stored, b"<html><script>alert(1)</script></html>").unwrap();
    assert_eq!(serve(data.path(), &format!("/t1/{sha}"), None).status, 404);
}
