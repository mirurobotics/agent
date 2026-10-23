// standard crates
use std::time::Duration;

// internal crates
use crate::test_utils::{clock::FakeClock, filesys::dirs as test_dirs};
use miru_agent::clock::{uptime, UptimeFile, UptimeSaver, UptimeState};
use miru_agent::filesys::{dirs, files, Dir, File, PathExt};

// external crates
use chrono::{DateTime, Utc};

async fn open(path: &File) -> UptimeFile {
    UptimeFile::new_with_default(path.clone(), UptimeState::default())
        .await
        .unwrap()
}

async fn saved_secs(path: &File) -> u64 {
    files::read_json::<UptimeState>(path).await.unwrap().secs
}

#[test]
fn counts_monotonic_time_from_zero_without_a_file() {
    let fake = FakeClock::new(1000);
    let uptime = UptimeSaver::new(fake.clock(), None).uptime();
    assert_eq!(uptime.now(), Duration::ZERO);

    fake.advance(90);
    fake.set_system(1);

    assert_eq!(uptime.now(), Duration::from_secs(90));
    assert_eq!(
        uptime.now_as_datetime(),
        DateTime::from_timestamp(90, 0).unwrap()
    );
}

#[test]
fn every_handle_reads_the_same_uptime() {
    let fake = FakeClock::new(1000);
    let saver = UptimeSaver::new(fake.clock(), None);
    let first = saver.uptime();
    fake.advance(30);
    let second = first.clone();
    fake.advance(5);

    assert_eq!(first.now(), Duration::from_secs(35));
    assert_eq!(second.now(), Duration::from_secs(35));
    assert_eq!(saver.uptime().now(), Duration::from_secs(35));
}

#[tokio::test]
async fn resumes_from_the_saved_total_after_a_restart() {
    let dir = test_dirs::temp("uptime-resume").unwrap();
    let path = dir.file("uptime.json");
    let fake = FakeClock::new(1000);
    let mut saver = UptimeSaver::new(fake.clock(), Some(open(&path).await));
    fake.advance(30);
    saver.save().await;
    drop(saver);

    let restarted = fake.restart();
    let uptime = UptimeSaver::new(restarted.clock(), Some(open(&path).await)).uptime();
    restarted.advance(10);

    assert_eq!(uptime.now(), Duration::from_secs(40));
}

#[tokio::test]
async fn save_writes_the_total() {
    let dir = test_dirs::temp("uptime-save-now").unwrap();
    let path = dir.file("uptime.json");
    let fake = FakeClock::new(1000);
    let mut saver = UptimeSaver::new(fake.clock(), Some(open(&path).await));
    fake.advance(7);

    saver.save().await;

    assert_eq!(saved_secs(&path).await, 7);
}

#[tokio::test]
async fn a_failed_save_is_retried_on_the_next_save() {
    let dir = test_dirs::temp("uptime-save-fail").unwrap();
    let path = dir.file("uptime.json");
    let fake = FakeClock::new(1000);
    let mut saver = UptimeSaver::new(fake.clock(), Some(open(&path).await));
    // make the path unwritable: a directory now sits there
    files::delete(&path).await.unwrap();
    dirs::create(&Dir::new(path.path().clone())).await.unwrap();
    fake.advance(7);
    saver.save().await;

    dirs::delete(&Dir::new(path.path().clone())).await.unwrap();
    saver.save().await;

    assert_eq!(saved_secs(&path).await, 7);
}

#[tokio::test]
async fn save_without_a_file_does_nothing() {
    let fake = FakeClock::new(1000);
    let mut saver = UptimeSaver::new(fake.clock(), None);
    fake.advance(7);

    saver.save().await;

    assert_eq!(saver.uptime().now(), Duration::from_secs(7));
}

#[tokio::test(start_paused = true)]
async fn the_spawned_saver_saves_every_interval() {
    let dir = test_dirs::temp("uptime-spawn-interval").unwrap();
    let path = dir.file("uptime.json");
    let fake = FakeClock::new(1000);
    let (handle, join) = UptimeSaver::new(fake.clock(), Some(open(&path).await)).spawn();
    fake.advance(42);

    // nothing is written before the first interval
    tokio::time::sleep(uptime::SAVE_INTERVAL - Duration::from_secs(1)).await;
    assert_eq!(saved_secs(&path).await, 0);

    let mut waited = 0;
    while saved_secs(&path).await != 42 {
        assert!(waited < 100, "the saver never wrote the uptime");
        tokio::time::sleep(Duration::from_secs(1)).await;
        waited += 1;
    }

    handle.shutdown().await;
    join.await.unwrap();
}

#[tokio::test]
async fn shutdown_saves_and_stops_the_saver() {
    let dir = test_dirs::temp("uptime-spawn-shutdown").unwrap();
    let path = dir.file("uptime.json");
    let fake = FakeClock::new(1000);
    let (handle, join) = UptimeSaver::new(fake.clock(), Some(open(&path).await)).spawn();
    fake.advance(7);

    handle.shutdown().await;

    assert_eq!(saved_secs(&path).await, 7);
    join.await.unwrap();
    // a second shutdown finds the task gone and returns at once
    handle.shutdown().await;
}

#[test]
fn as_datetime_saturates() {
    assert_eq!(
        uptime::as_datetime(Duration::from_secs(u64::MAX)),
        DateTime::<Utc>::MAX_UTC
    );
}
