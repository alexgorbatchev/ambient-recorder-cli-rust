use ambient_recorder::capture::{Buffer, Clock, Timestamp, queue};
use chrono::{TimeZone, Utc};

#[test]
fn realtime_queue_mixes_without_waiting_and_retains_acquisition_metadata() {
    let (mut mixer, mut reader) = queue(3);
    let mic = [0.6, -0.3];
    let playback = [0.3, 0., -0.6, 0.];
    let timestamp = Timestamp {
        host_time: Some(1234),
        sample_time: Some(4567.),
    };
    let buffers = [
        Buffer {
            channels: 1,
            frames: 2,
            samples: Some(&mic),
        },
        Buffer {
            channels: 2,
            frames: 2,
            samples: Some(&playback),
        },
    ];
    assert!(mixer.push(&buffers, timestamp));
    let mut pcm = [0.; 8192];
    let chunk = reader.read(&mut pcm).unwrap().unwrap();
    assert_eq!(chunk.frames, 2);
    assert_eq!(chunk.timestamp, timestamp);
    assert!((pcm[0] - 0.3).abs() < 0.00001 && (pcm[1] + 0.3).abs() < 0.00001);
    assert!(reader.read(&mut pcm).unwrap().is_none());
    for _ in 0..258 {
        mixer.push(&buffers, timestamp);
    }
    assert_eq!(reader.dropped(), 4);
    for _ in 0..256 {
        assert_eq!(reader.read(&mut pcm).unwrap().unwrap().frames, 2);
    }
    assert!(reader.read(&mut pcm).unwrap().is_none());
    assert!(!mixer.push(
        &[Buffer {
            channels: 1,
            frames: 1,
            samples: Some(&mic)
        }],
        timestamp
    ));
    assert!(reader.changed());
    assert!(!mixer.push(&buffers, timestamp));
}

#[test]
fn nonfinite_clipping_null_buffers_capacity_and_wall_corrections() {
    let (mut mixer, mut reader) = queue(2);
    let samples = [f32::NAN, f32::INFINITY, 4., 4.];
    assert!(mixer.push(
        &[Buffer {
            channels: 2,
            frames: 2,
            samples: Some(&samples)
        }],
        Timestamp::default()
    ));
    assert!(reader.read(&mut [0.; 1]).is_err());
    let mut pcm = [0.; 2];
    reader.read(&mut pcm).unwrap().unwrap();
    assert_eq!(pcm, [0., 1.]);
    assert!(mixer.push(
        &[Buffer {
            channels: 2,
            frames: 2,
            samples: None
        }],
        Timestamp::default()
    ));
    reader.read(&mut pcm).unwrap().unwrap();
    assert_eq!(pcm, [0., 0.]);
    let wall = Utc.with_ymd_and_hms(2026, 10, 14, 0, 0, 0).unwrap();
    let mut clock = Clock::new(1_000_000_000, wall, 1.);
    assert_eq!(
        clock.acquired(Some(1_500_000_000), wall),
        wall + chrono::Duration::milliseconds(500)
    );
    assert_eq!(clock.acquired(None, wall), wall);
    assert!(
        !clock
            .refresh(2_000_000_000, wall + chrono::Duration::seconds(1))
            .unwrap()
    );
    assert!(
        clock
            .refresh(3_000_000_000, wall + chrono::Duration::seconds(7))
            .unwrap()
    );
    assert_eq!(
        clock.acquired(Some(3_500_000_000), wall),
        wall + chrono::Duration::milliseconds(7500)
    );
}
