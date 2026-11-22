use orochi::{
    activity::residents::Notice,
    notify::{add_phone, inside, phones, remove_phone, shown, valid_push_id},
    resident::{apply_notes, split_notes},
};

fn notice(kind: &str, title: &str, body: &str) -> Notice {
    Notice {
        id: 1,
        kind: kind.into(),
        thread_id: Some("thread".into()),
        routine_id: None,
        prompt_id: None,
        title: title.into(),
        body: Some(body.into()),
        mac: None,
        phone: None,
        created_at: 0,
        read_at: None,
    }
}

#[test]
fn a_third_party_is_shown_no_more_of_a_notice_than_the_chosen_level_allows() {
    let n = notice("report", "Morning brief", &"secret details ".repeat(20));
    let (heading, text) = shown("title", &n);
    assert_eq!(heading, "Routine finished");
    assert_eq!(
        text, "Morning brief",
        "the title level carries the name and nothing else"
    );

    let (_, summary) = shown("summary", &n);
    assert!(summary.starts_with("Morning brief: secret details"));
    assert!(summary.chars().count() <= "Morning brief: ".len() + 120);

    let (_, full) = shown("full", &n);
    assert!(full.contains("secret details secret details secret details"));
    assert_eq!(
        shown("title", &notice("ask", "x", "y")).0,
        "Approval needed",
        "the heading is the kind of notice and never anything the agent wrote"
    );
}

#[test]
fn quiet_hours_may_wrap_midnight_and_an_empty_range_is_never_quiet() {
    let night = (23 * 60, 7 * 60);
    assert!(inside(night, 23 * 60 + 30));
    assert!(inside(night, 3 * 60));
    assert!(
        !inside(night, 7 * 60),
        "the end of the range is no longer quiet"
    );
    assert!(!inside(night, 12 * 60));
    let day = (13 * 60, 14 * 60);
    assert!(inside(day, 13 * 60) && !inside(day, 14 * 60));
    assert!(!inside((600, 600), 600));
}

#[test]
fn a_phone_is_known_by_a_random_id_that_names_nobody_and_can_be_removed() {
    let dir = tempfile::tempdir().unwrap();
    assert!(phones(dir.path()).is_empty());
    for bad in [
        "short",
        "has spaces in it that is long enough",
        "name@example.com-aaaaaaaa",
    ] {
        assert!(!valid_push_id(bad), "{bad}");
        assert!(add_phone(dir.path(), bad, "x").is_err());
    }
    add_phone(dir.path(), "3f9a1c0de2b74a65b3c1d0e9f8a7b6c5", "My iPhone").unwrap();
    add_phone(dir.path(), "3f9a1c0de2b74a65b3c1d0e9f8a7b6c5", "Renamed").unwrap();
    let all = phones(dir.path());
    assert_eq!(all.len(), 1, "registering twice is one phone");
    assert_eq!(all[0].name, "Renamed");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(orochi::notify::phones_path(dir.path()))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "the list of phones is the user's alone");
    }
    assert!(remove_phone(dir.path(), "3f9a1c0de2b74a65b3c1d0e9f8a7b6c5").unwrap());
    assert!(!remove_phone(dir.path(), "3f9a1c0de2b74a65b3c1d0e9f8a7b6c5").unwrap());
    assert!(phones(dir.path()).is_empty());
}

#[test]
fn a_residents_notes_are_the_last_notes_block_of_its_reply_and_the_rest_is_the_report() {
    let (report, notes) = split_notes("Found two things.\n\n<notes>\n- likes tea\n</notes>");
    assert_eq!(report, "Found two things.");
    assert_eq!(notes.as_deref(), Some("- likes tea"));
    assert_eq!(split_notes("Nothing to keep.").1, None);
    assert_eq!(
        split_notes("an unclosed <notes> block").1,
        None,
        "half a block is not notes"
    );
}

#[test]
fn keeping_notes_leaves_the_previous_file_beside_them_and_counts_what_changed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("NOTES.md"), "- a\n- b\n").unwrap();
    let changed = apply_notes(dir.path(), "ok\n<notes>\n- a\n- c\n- d\n</notes>").unwrap();
    assert_eq!(changed, Some((2, 1)), "two lines in, one out");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("NOTES.md")).unwrap(),
        "- a\n- c\n- d\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("NOTES.md.prev")).unwrap(),
        "- a\n- b\n",
        "the person can see what was there before"
    );
    assert_eq!(
        apply_notes(dir.path(), "again\n<notes>\n- a\n- c\n- d\n</notes>").unwrap(),
        None,
        "notes that did not change are not a change"
    );
    let huge = format!("<notes>{}</notes>", "x".repeat(70_000));
    assert!(
        apply_notes(dir.path(), &huge).is_err(),
        "a runaway reply cannot fill the disk"
    );
}

#[test]
fn the_login_agent_starts_the_keeper_at_load_and_does_not_keep_it_alive() {
    let plist = orochi::keeper::login_plist(
        std::path::Path::new("/Applications/Orochi & Co/orochi"),
        std::path::Path::new("/Users/me/.config/orochi/config.toml"),
        std::path::Path::new("/Users/me/.local/share/orochi"),
    );
    assert!(plist.contains("<string>dev.orochi.keeper</string>"));
    assert!(plist.contains("<string>--internal-keeper</string>"));
    assert!(
        plist.contains("/Applications/Orochi &amp; Co/orochi"),
        "a path is text in an XML file, so it is escaped"
    );
    let run_at_load = plist.split("<key>RunAtLoad</key>").nth(1).unwrap();
    assert!(run_at_load.trim_start().starts_with("<true/>"));
    let keep_alive = plist.split("<key>KeepAlive</key>").nth(1).unwrap();
    assert!(
        keep_alive.trim_start().starts_with("<false/>"),
        "the keeper ends by itself when nothing is waiting; launchd must not start it again"
    );
    assert!(plist.contains("/Users/me/.local/share/orochi/keeper/keeper.log"));
    assert!(
        !plist.contains("sudo") && !plist.contains("/Library/LaunchDaemons"),
        "a user agent, never a system one"
    );
}
