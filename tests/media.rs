//! Pictures, sound and video a conversation produced: what counts as media, where an agent's
//! words may point, how it is kept and when it goes (`docs/conversation-tabs-design.md` §4).
use orochi::{
    acp::{ExecutionEvent, Progress, ToolUpdate},
    activity::{
        Activity, Origin,
        recorder::{Recorder, Seat},
    },
    media::{self, Kind, MediaRef, Source},
    types::*,
};
use std::path::Path;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0";

fn candidate() -> ExecutionCandidate {
    ExecutionCandidate {
        id: "claude:opus".into(),
        agent: "claude".into(),
        provider: Provider::ANTHROPIC,
        model: "opus".into(),
        reasoning_level: None,
        mode: None,
        session_strategy: "fresh".into(),
        context_strategy: "none".into(),
        success_probability: 0.8,
        expected_tokens: 1000.0,
        expected_cost: 1.0,
        confidence: 0.5,
        reasons: vec![],
        prediction: None,
        spoken_for: false,
    }
}

#[test]
fn what_is_media_is_read_from_the_bytes_and_html_is_not() {
    let kind = |bytes: &[u8]| media::sniff(bytes).map(|s| (s.mime, s.kind));
    assert_eq!(kind(PNG), Some(("image/png", Kind::Image)));
    assert_eq!(
        kind(b"\xff\xd8\xff\xe0\0\x10JFIF"),
        Some(("image/jpeg", Kind::Image))
    );
    assert_eq!(kind(b"GIF89a\x01\0"), Some(("image/gif", Kind::Image)));
    assert_eq!(
        kind(b"RIFF\0\0\0\0WEBPVP8 "),
        Some(("image/webp", Kind::Image))
    );
    assert_eq!(
        kind(b"RIFF\0\0\0\0WAVEfmt "),
        Some(("audio/wav", Kind::Audio))
    );
    assert_eq!(
        kind(b"\0\0\0\x18ftypmp42\0\0\0\0"),
        Some(("video/mp4", Kind::Video))
    );
    assert_eq!(
        kind(b"\0\0\0\x14ftypqt  \0\0\0\0"),
        Some(("video/quicktime", Kind::Video))
    );
    assert_eq!(
        kind(b"\x1a\x45\xdf\xa3\x01"),
        Some(("video/webm", Kind::Video))
    );
    assert_eq!(kind(b"ID3\x03\0\0\0"), Some(("audio/mpeg", Kind::Audio)));
    assert_eq!(
        kind(b"\0\0\0\x1cftypavif\0\0\0\0"),
        Some(("image/avif", Kind::Image))
    );
    assert_eq!(
        kind(b"<?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\"/>"),
        Some(("image/svg+xml", Kind::Image))
    );
    // Whatever the name or the agent's claim, these are not media.
    assert_eq!(
        kind(b"<!doctype html><html><script>alert(1)</script>"),
        None
    );
    assert_eq!(kind(b"%PDF-1.7\n"), None);
    assert_eq!(kind(b"PK\x03\x04"), None);
    assert_eq!(kind(b"just words"), None);
    assert_eq!(kind(b""), None);
}

#[test]
fn an_agent_may_name_files_in_its_tree_and_the_temp_directory_and_nowhere_else() {
    let work = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(work.path().join("out")).unwrap();
    std::fs::write(work.path().join("out/shot.png"), PNG).unwrap();
    std::fs::write(outside.path().join("secret.png"), PNG).unwrap();
    std::fs::write(temp.path().join("tmp.png"), PNG).unwrap();
    std::fs::write(work.path().join("notes.txt"), b"x").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        outside.path().join("secret.png"),
        work.path().join("link.png"),
    )
    .unwrap();
    let roots = [temp.path().to_path_buf()];
    let text = format!(
        "Saved `out/shot.png`, also ![x]({tmp}), see ({secret}) and link.png and missing.png \
         and notes.txt, plus /etc/hosts.png.",
        tmp = temp.path().join("tmp.png").display(),
        secret = outside.path().join("secret.png").display(),
    );
    let found = media::find_paths(&text, work.path(), &roots);
    let names: Vec<_> = found
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["shot.png", "tmp.png"], "{found:?}");
}

#[test]
fn media_is_kept_once_by_its_hash_privately_and_only_found_by_a_plain_id() {
    let data = tempfile::tempdir().unwrap();
    let reference = |bytes: &[u8]| MediaRef {
        name: "shot.png".into(),
        source: Source::Inline(bytes.to_vec()),
    };
    let first = media::keep(data.path(), "t1", &reference(PNG))
        .unwrap()
        .unwrap();
    let again = media::keep(data.path(), "t1", &reference(PNG))
        .unwrap()
        .unwrap();
    assert_eq!(first.sha, again.sha);
    assert_eq!(first.mime, "image/png");
    let stored = media::file(data.path(), "t1", &first.sha).unwrap();
    assert!(stored.ends_with(format!("activity-media/t1/{}.png", first.sha)));
    assert_eq!(
        std::fs::read_dir(media::thread_dir(data.path(), "t1"))
            .unwrap()
            .count(),
        1
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&stored), 0o600);
        assert_eq!(mode(&media::thread_dir(data.path(), "t1")), 0o700);
    }
    assert!(
        media::file(data.path(), "t2", &first.sha).is_none(),
        "another thread's"
    );
    for (thread, sha) in [
        ("../t1", first.sha.as_str()),
        ("t1", "../../etc/passwd"),
        ("t1", "abcd"),
    ] {
        assert!(
            media::file(data.path(), thread, sha).is_none(),
            "{thread} {sha}"
        );
    }
    // What is not media is not kept, whatever it is called.
    let html = MediaRef {
        name: "evil.png".into(),
        source: Source::Inline(b"<html><script>1</script></html>".to_vec()),
    };
    assert!(media::keep(data.path(), "t1", &html).unwrap().is_none());
}

#[test]
fn base64_blocks_are_decoded_and_a_bare_uri_is_never_fetched() {
    let block = serde_json::json!({"type": "image", "mimeType": "image/png",
        "data": "iVBORw0KGgoAAAANSUhEUgAAAAE=", "uri": "file:///tmp/a/screen.png"});
    let reference = media::from_block(&block, "result").unwrap();
    assert_eq!(reference.name, "screen.png");
    let Source::Inline(bytes) = reference.source else {
        panic!()
    };
    assert!(bytes.starts_with(b"\x89PNG"));
    let only_uri = serde_json::json!({"type": "image", "uri": "https://example.com/x.png"});
    assert!(media::from_block(&only_uri, "result").is_none());
    assert!(media::from_block(&serde_json::json!({"type": "text", "text": "hi"}), "x").is_none());
}

fn thread_in(activity: &Activity, cwd: &Path) -> (String, String, String, String) {
    activity.project("proj", cwd).unwrap();
    let thread = activity
        .create_thread(
            "proj",
            cwd,
            Some("main"),
            "hash",
            Origin::Console,
            &Overrides::default(),
            "ask",
        )
        .unwrap();
    let turn = activity
        .queue_turn(&thread, "take a screenshot", &[], "auto", "terminal")
        .unwrap();
    let seat = activity
        .create_seat(&turn, 0, "implementer", true, false, None, None)
        .unwrap();
    let attempt = activity
        .create_attempt(&seat, &candidate(), false, None)
        .unwrap();
    (thread, turn, seat, attempt)
}

#[test]
fn a_screenshot_a_tool_returned_and_a_file_a_reply_named_become_media_rows_and_go_with_the_thread()
{
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    std::fs::write(work.path().join("demo.png"), [PNG, b"named"].concat()).unwrap();
    let activity = Activity::open(data.path(), 30).unwrap();
    let (thread, turn, seat, attempt) = thread_in(&activity, work.path());
    {
        let mut recorder = Recorder::new(
            orochi::activity::share(Activity::open(data.path(), 30).unwrap()),
            Seat {
                thread: thread.clone(),
                turn,
                seat,
                attempt: Some(attempt),
            },
            true,
        );
        recorder.record(&ExecutionEvent::Progress(Progress::Tool(Box::new(
            ToolUpdate {
                id: "call-1".into(),
                title: Some("browser_screenshot".into()),
                status: Some("completed".into()),
                media: vec![MediaRef {
                    name: "page.png".into(),
                    source: Source::Inline([PNG, b"inline"].concat()),
                }],
                ..Default::default()
            },
        ))));
        // The same picture returned again is the same row.
        recorder.record(&ExecutionEvent::Progress(Progress::Tool(Box::new(
            ToolUpdate {
                id: "call-2".into(),
                status: Some("completed".into()),
                media: vec![MediaRef {
                    name: "page.png".into(),
                    source: Source::Inline([PNG, b"inline"].concat()),
                }],
                ..Default::default()
            },
        ))));
        recorder.record(&ExecutionEvent::Text(
            "All done, see demo.png and /etc/passwd.png".into(),
            None,
        ));
        recorder.record(&ExecutionEvent::Finished);
    }
    let reader = Activity::open_read_only(data.path()).unwrap();
    let rows: Vec<(String, String, i64)> = reader
        .prepare("SELECT name, mime, kept FROM v_media WHERE thread_id=?1 ORDER BY seq")
        .unwrap()
        .query_map([&thread], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![
            ("page.png".into(), "image/png".into(), 1),
            ("demo.png".into(), "image/png".into(), 1)
        ],
        "one row for the screenshot returned twice, one for the file the reply named, none for /etc"
    );
    let kept = std::fs::read_dir(media::thread_dir(data.path(), &thread))
        .unwrap()
        .count();
    assert_eq!(kept, 2);
    // The bytes are never in a row, and never in telemetry's file.
    let rows_with_bytes: i64 = reader
        .query_row(
            "SELECT count(*) FROM items WHERE data LIKE '%inline%' AND kind<>'media'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows_with_bytes, 0);

    let listed = activity.media(&thread).unwrap();
    assert_eq!(
        listed.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
        ["page.png", "demo.png"]
    );
    assert!(listed.iter().all(|m| m.kept && m.kind == "image"));

    assert!(activity.delete_thread(&thread).unwrap());
    assert!(
        !media::thread_dir(data.path(), &thread).exists(),
        "nothing outlives the thread"
    );
}

#[test]
fn a_thread_over_its_cap_drops_its_oldest_media_and_says_so() {
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let activity = Activity::open(data.path(), 30).unwrap();
    let (thread, ..) = thread_in(&activity, work.path());
    let big = |n: u8| media::Kept {
        sha: format!("{n:02x}").repeat(32),
        mime: "image/png".into(),
        ext: "png".into(),
        kind: Kind::Image,
        bytes: media::THREAD_CAP * 6 / 10,
        name: format!("{n}.png"),
        source: "inline",
        from: None,
        kept: true,
    };
    activity.keep_media(&thread, &big(1)).unwrap();
    activity.keep_media(&thread, &big(2)).unwrap();
    let reader = Activity::open_read_only(data.path()).unwrap();
    let state: Vec<(String, i64)> = reader
        .prepare("SELECT name, kept FROM media WHERE thread_id=?1 ORDER BY created_at, rowid")
        .unwrap()
        .query_map([&thread], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(state, vec![("1.png".into(), 0), ("2.png".into(), 1)]);
}

#[test]
fn media_directories_of_threads_that_are_gone_are_swept() {
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let activity = Activity::open(data.path(), 30).unwrap();
    let (thread, ..) = thread_in(&activity, work.path());
    let reference = MediaRef {
        name: "a.png".into(),
        source: Source::Inline(PNG.to_vec()),
    };
    media::keep(data.path(), &thread, &reference).unwrap();
    media::keep(data.path(), "ghost", &reference).unwrap();
    activity.prune().unwrap();
    assert!(
        media::thread_dir(data.path(), &thread).exists(),
        "a live thread keeps its media"
    );
    assert!(
        !media::thread_dir(data.path(), "ghost").exists(),
        "a thread that is gone does not"
    );
}
