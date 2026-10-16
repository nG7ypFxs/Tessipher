#[cfg(tessipher_repository_fixtures)]
mod regression_tests {
    extern crate std;
    use crate::core::{Segment, MAX_MATRIX_BYTES};
    use crate::detect::{
        DefaultConfig, Error, GrayImage, Rect, SamplingStrategy, Stats, TrackHint, Tracker,
        TrackerState, FINDER_BYTES,
    };

    #[test]
    fn every_tracker_state_preserves_itself_on_terminal_errors() {
        let file = include_bytes!("../../../../fixtures/v1.pgm");
        let offset = file
            .iter()
            .enumerate()
            .filter(|(_, b)| **b == b'\n')
            .nth(2)
            .unwrap()
            .0
            + 1;
        let image = GrayImage::new(&file[offset..], 145, 145, 145).unwrap();
        let hint = TrackHint {
            roi: Rect {
                x: 0,
                y: 0,
                width: 145,
                height: 145,
            },
            image_width: 145,
            image_height: 145,
        };
        let config = DefaultConfig::default();
        let mut ds = [0; MAX_MATRIX_BYTES + 32 * FINDER_BYTES];
        let mut cs = [0; crate::core::SCRATCH_BYTES];
        let mut output = [0; 16];
        let mut segments = [Segment::EMPTY; 4];
        for state in [
            TrackerState::Searching,
            TrackerState::Candidate { hint, misses: 1 },
            TrackerState::Tracking { hint },
        ] {
            let mut tracker = Tracker { state };
            let mut stats = Stats::default();
            let small = DefaultConfig {
                max_pixel_reads: 1,
                ..config
            };
            assert_eq!(
                tracker.decode_with_stats(
                    image,
                    &small,
                    SamplingStrategy::CenterThenMajority5,
                    &mut ds,
                    &mut cs,
                    &mut output,
                    &mut segments,
                    &mut stats
                ),
                Err(Error::PixelBudget)
            );
            assert_eq!(tracker.state(), state);
            assert_eq!(stats.pixel_reads, 1);
            assert_eq!(
                tracker.decode_with_stats(
                    image,
                    &config,
                    SamplingStrategy::CenterThenMajority5,
                    &mut [],
                    &mut cs,
                    &mut output,
                    &mut segments,
                    &mut stats
                ),
                Err(Error::ScratchTooSmall)
            );
            assert_eq!(tracker.state(), state);
            assert_eq!(stats, Stats::default());
            assert_eq!(
                tracker.decode_with_stats(
                    image,
                    &config,
                    SamplingStrategy::CenterThenMajority5,
                    &mut ds,
                    &mut cs,
                    &mut [],
                    &mut segments,
                    &mut stats
                ),
                Err(Error::Decode(crate::core::Error::OutputTooSmall))
            );
            assert_eq!(tracker.state(), state);
            assert_eq!(stats.retries, 1);
            assert_eq!(
                tracker.decode_with_stats(
                    image,
                    &config,
                    SamplingStrategy::CenterThenMajority5,
                    &mut ds,
                    &mut cs,
                    &mut output,
                    &mut [],
                    &mut stats
                ),
                Err(Error::Decode(crate::core::Error::SegmentsTooSmall))
            );
            assert_eq!(tracker.state(), state);
            assert_eq!(stats.retries, 1);
        }
    }
}
