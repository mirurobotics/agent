// standard crates
use std::time::Duration;

// internal crates
use crate::test_utils::{clock::FakeClock, filesys::dirs as test_dirs};
use miru_agent::clock::{uptime, Uptime, UptimeFile, UptimeState};
use miru_agent::filesys::{dirs, files, Dir, File, PathExt};

// external crates
use chrono::{DateTime, Utc};

async fn open(path: &File) -> UptimeFile {
    UptimeFile::new_with_default(path.clone(), UptimeState::default())
        .await
        .unwrap()
}

#[test]
fn counts_monotonic_time_from_zero_without_a_file() {
    let fake = FakeClock::new(1000);
    let uptime = Uptime::new(fake.clock(), None);
    assert_eq!(uptime.now(), Duration::ZERO);

    fake.advance(90);
    fake.set_system(1);

    assert_eq!(uptime.now(), Duration::from_secs(90));
    assert_eq!(
        uptime.now_as_datetime(),
        DateTime::from_timestamp(90, 0).unwrap()
    );
}

#[tokio::test]
async fn resumes_from_the_saved_total_after_a_restart() {
    let dir = test_dirs::temp("uptime-resume").unwrap();
    let path = dir.file("uptime.json");
    let fake = FakeClock::new(1000);
    let mut uptime = Uptime::new(fake.clock(), Some(open(&path).await));
    fake.advance(uptime::SAVE_INTERVAL.as_secs() + 30);
    uptime.save_if_due().await;
    drop(uptime);

    let restarted = fake.restart();
    let uptime = Uptime::new(restarted.clock(), Some(open(&path).await));
    restarted.advance(10);

    assert_eq!(
        uptime.now(),
        uptime::SAVE_INTERVAL + Duration::from_secs(40)
    );
}

#[tokio::test]
async fn saves_only_once_the_interval_has_passed() {
    let dir = test_dirs::temp("uptime-interval").unwrap();
    let path = dir.file("uptime.json");
    let fake = FakeClock::new(1000);
    let mut uptime = Uptime::new(fake.clock(), Some(open(&path).await));

    fake.advance(uptime::SAVE_INTERVAL.as_secs() - 1);
    uptime.save_if_due().await;
    assert_eq!(open(&path).await.read().secs, 0);

    fake.advance(1);
    uptime.save_if_due().await;
    assert_eq!(
        open(&path).await.read().secs,
        uptime::SAVE_INTERVAL.as_secs()
    );
}

#[tokio::test]
async fn a_failed_save_is_retried_on_the_next_call() {
    let dir = test_dirs::temp("uptime-save-fail").unwrap();
    let path = dir.file("uptime.json");
    let fake = FakeClock::new(1000);
    let mut uptime = Uptime::new(fake.clock(), Some(open(&path).await));
    // make the path unwritable: a directory now sits there
    files::delete(&path).await.unwrap();
    dirs::create(&Dir::new(path.path().clone())).await.unwrap();

    fake.advance(uptime::SAVE_INTERVAL.as_secs());
    uptime.save_if_due().await;

    dirs::delete(&Dir::new(path.path().clone())).await.unwrap();
    uptime.save_if_due().await;
    assert_eq!(
        files::read_json::<UptimeState>(&path).await.unwrap().secs,
        uptime::SAVE_INTERVAL.as_secs()
    );
}

#[test]
fn as_datetime_saturates() {
    assert_eq!(
        uptime::as_datetime(Duration::from_secs(u64::MAX)),
        DateTime::<Utc>::MAX_UTC
    );
}
