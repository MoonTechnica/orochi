use orochi::routines::{Schedule, Zone};

/// 2026-11-21 (a Saturday) 10:00 UTC.
const NOW: i64 = 1_795_255_200;

fn parse(text: &str) -> Schedule {
    Schedule::parse(text, NOW, Zone::Utc).unwrap_or_else(|e| panic!("{text}: {e}"))
}
fn at(day: u32, hour: u32, minute: u32) -> i64 {
    orochi::routines::Zone::Utc
        .at(2026, 11, day, hour, minute)
        .unwrap()
}

#[test]
fn the_reference_instant_is_a_saturday_morning() {
    let p = Zone::Utc.parts(NOW);
    assert_eq!(
        (p.year, p.month, p.day, p.hour, p.minute),
        (2026, 11, 21, 10, 0)
    );
    assert_eq!(p.weekday, 5, "Monday is 0, so Saturday is 5");
}

#[test]
fn a_clock_time_alone_means_its_next_occurrence_once() {
    assert_eq!(parse("at 15:00"), Schedule::Once { at: at(21, 15, 0) });
    assert_eq!(
        parse("at 09:00"),
        Schedule::Once { at: at(22, 9, 0) },
        "a time already past today is tomorrow's"
    );
    assert_eq!(
        parse("on 2026-11-24 at 09:30"),
        Schedule::Once { at: at(24, 9, 30) }
    );
    assert_eq!(parse("in 90m"), Schedule::Once { at: NOW + 5400 });
    assert_eq!(parse("in 2hours"), Schedule::Once { at: NOW + 7200 });
    assert_eq!(
        parse("at 15:00").next_after(NOW, Zone::Utc),
        Some(at(21, 15, 0))
    );
    assert_eq!(
        parse("at 15:00").next_after(at(21, 15, 0), Zone::Utc),
        None,
        "a one-shot is not due again"
    );
}

#[test]
fn daily_weekday_and_named_day_schedules_find_the_next_matching_day() {
    let daily = parse("daily at 08:00, 18:00");
    assert_eq!(daily.next_after(NOW, Zone::Utc), Some(at(21, 18, 0)));
    assert_eq!(
        daily.next_after(at(21, 18, 0), Zone::Utc),
        Some(at(22, 8, 0))
    );

    // Saturday: the next weekday morning is Monday the 23rd.
    let weekdays = parse("weekdays at 09:00");
    assert_eq!(weekdays.next_after(NOW, Zone::Utc), Some(at(23, 9, 0)));
    assert_eq!(
        parse("weekends at 09:00").next_after(NOW, Zone::Utc),
        Some(at(22, 9, 0))
    );

    let some = parse("every mon,thu at 10:00");
    assert_eq!(some.next_after(NOW, Zone::Utc), Some(at(23, 10, 0)));
    assert_eq!(
        some.next_after(at(23, 10, 0), Zone::Utc),
        Some(at(26, 10, 0))
    );
    assert_eq!(
        parse("every Monday at 10:00"),
        parse("every mon at 10:00"),
        "a spelled-out day is read by its first three letters"
    );
}

#[test]
fn an_interval_counts_from_when_it_was_written_and_a_window_from_its_start() {
    let every = parse("every 2h");
    assert_eq!(every.next_after(NOW, Zone::Utc), Some(NOW + 7200));
    assert_eq!(every.next_after(NOW + 7200, Zone::Utc), Some(NOW + 14_400));
    assert_eq!(
        every.next_after(NOW + 7199, Zone::Utc),
        Some(NOW + 7200),
        "waking late does not move the beat"
    );

    let windowed = parse("every 2h from 09:00 to 19:00");
    assert_eq!(windowed.next_after(NOW, Zone::Utc), Some(at(21, 11, 0)));
    assert_eq!(
        windowed.next_after(at(21, 19, 0), Zone::Utc),
        Some(at(22, 9, 0))
    );
}

#[test]
fn cron_fields_match_as_cron_does_including_either_of_day_and_weekday() {
    let weekdays = parse("cron 0 8 * * 1-5");
    assert_eq!(weekdays.next_after(NOW, Zone::Utc), Some(at(23, 8, 0)));

    let steps = parse("cron */20 9 * * *");
    assert_eq!(
        steps.next_after(at(22, 9, 0), Zone::Utc),
        Some(at(22, 9, 20))
    );

    // Sunday is 0 and 7 alike.
    assert_eq!(
        parse("cron 0 12 * * 0").next_after(NOW, Zone::Utc),
        parse("cron 0 12 * * 7").next_after(NOW, Zone::Utc)
    );
    assert_eq!(
        parse("cron 0 12 * * 0").next_after(NOW, Zone::Utc),
        Some(at(22, 12, 0))
    );

    // The 24th or a Monday, whichever comes first: Monday the 23rd.
    let either = parse("cron 0 7 24 * 1");
    assert_eq!(either.next_after(NOW, Zone::Utc), Some(at(23, 7, 0)));

    // A leap day that only one month names is found years ahead.
    let leap = parse("cron 0 0 29 2 *");
    let next = leap.next_after(NOW, Zone::Utc).unwrap();
    assert_eq!(Zone::Utc.parts(next).year, 2028);
}

#[test]
fn what_cannot_be_read_is_refused_with_a_reason() {
    for bad in [
        "",
        "sometimes",
        "at 25:00",
        "at 9",
        "daily at",
        "every",
        "every foo at 10:00",
        "every 30s",
        "every 2h from 19:00 to 09:00",
        "in 0m",
        "in 5",
        "in 5x",
        "on 2020-01-01 at 09:00",
        "on 2026-13-01 at 09:00",
        "cron 0 8 * *",
        "cron 61 8 * * *",
        "cron */0 8 * * *",
    ] {
        assert!(
            Schedule::parse(bad, NOW, Zone::Utc).is_err(),
            "`{bad}` should be refused"
        );
    }
}

#[test]
fn the_shortest_gap_is_what_the_floor_on_how_often_a_routine_wakes_reads() {
    let gap = |text: &str| parse(text).shortest_gap(NOW, Zone::Utc);
    assert_eq!(gap("at 15:00"), None);
    assert_eq!(gap("every 15m"), Some(900));
    assert_eq!(gap("daily at 08:00"), Some(86_400));
    assert_eq!(gap("daily at 08:00, 08:05"), Some(300));
    assert_eq!(gap("cron * * * * *"), Some(60));
    assert_eq!(
        gap("weekdays at 09:00"),
        Some(86_400),
        "the weekend is a longer gap, not a shorter one"
    );
}

#[test]
fn a_schedule_round_trips_through_the_json_stored_beside_its_words() {
    for text in [
        "at 15:00",
        "daily at 08:00, 18:00",
        "every 2h from 09:00 to 19:00",
        "cron 0 8 * * 1-5",
    ] {
        let schedule = parse(text);
        let stored = serde_json::to_string(&schedule).unwrap();
        assert_eq!(serde_json::from_str::<Schedule>(&stored).unwrap(), schedule);
    }
}

#[test]
fn schedules_are_described_in_the_words_a_list_shows() {
    assert_eq!(
        parse("daily at 18:00, 08:00").describe(Zone::Utc),
        "daily at 08:00, 18:00"
    );
    assert_eq!(
        parse("weekdays at 09:00").describe(Zone::Utc),
        "weekdays at 09:00"
    );
    assert_eq!(
        parse("every mon,thu at 10:00").describe(Zone::Utc),
        "every mon,thu at 10:00"
    );
    assert_eq!(parse("every 90m").describe(Zone::Utc), "every 90m");
    assert_eq!(
        parse("every 2h from 09:00 to 19:00").describe(Zone::Utc),
        "every 2h from 09:00 to 19:00"
    );
}
