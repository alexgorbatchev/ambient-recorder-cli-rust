use ambient_recorder::recording::{Source, frame_duration};
use chrono::{TimeZone, Utc};
#[test]
fn converter_preserves_input_gaps_wall_corrections_and_exact_final_duration() {
    for rate in [16_000, 44_100, 48_000, 96_000] {
        let mut source = Source::new(f64::from(rate)).unwrap();
        let mut batches = Vec::new();
        let base = Utc.with_ymd_and_hms(2026, 10, 14, 1, 0, 0).unwrap();
        let mut persist = |at, pcm: &[f32]| {
            batches.push((at, pcm.len()));
            Ok(())
        };
        let frames = rate as usize / 10;
        source
            .write(base, &vec![0.1; frames], &mut persist)
            .unwrap();
        source
            .write(
                base + chrono::Duration::seconds(5),
                &vec![0.1; frames],
                &mut persist,
            )
            .unwrap();
        source
            .write(
                base + chrono::Duration::seconds(3),
                &vec![0.1; frames],
                &mut persist,
            )
            .unwrap();
        source.finish(&mut persist).unwrap();
        source.finish(&mut persist).unwrap();
        assert_eq!(batches.iter().map(|b| b.1).sum::<usize>(), 14_400);
        let mut output = 0;
        for (index, (at, count)) in batches.iter().enumerate() {
            let input_index = index.min(2);
            let expected = base + chrono::Duration::seconds([0, 5, 3][input_index])
                - chrono::Duration::milliseconds(100 * input_index as i64)
                + frame_duration(output, 48_000).unwrap();
            assert_eq!(*at, expected, "rate={rate} batch={index}");
            output += *count as u64;
        }
    }
}
