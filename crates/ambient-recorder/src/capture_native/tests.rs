use super::*;

mod session;

struct Input {
    callback: Box<Callback>,
    reader: Reader,
}

impl Input {
    fn new(channels: usize) -> Self {
        let (mixer, reader) = queue(channels);
        let health = mixer.health.clone();
        Self {
            callback: Box::new(Callback {
                mixer: UnsafeCell::new(mixer),
                health,
                busy: AtomicBool::new(false),
            }),
            reader,
        }
    }

    // The caller owns every sample allocation referenced by the list for this
    // synchronous call. The callback does not retain input or timestamp pointers.
    unsafe fn deliver(&mut self, list: &mut AudioBufferList, time: &mut AudioTimeStamp) {
        let mut output = AudioBufferList {
            mNumberBuffers: 0,
            mBuffers: [AudioBuffer {
                mNumberChannels: 0,
                mDataByteSize: 0,
                mData: ptr::null_mut(),
            }],
        };
        let timestamp = NonNull::from(time);
        let data = NonNull::from(self.callback.as_mut()).as_ptr().cast();
        // Every pointer is aligned, initialized, and remains live throughout
        // the callback; only the callback accesses its mixer during this call.
        assert_eq!(
            unsafe {
                audio(
                    0,
                    timestamp,
                    NonNull::from(list),
                    timestamp,
                    NonNull::from(&mut output),
                    timestamp,
                    data,
                )
            },
            0
        );
    }
}

fn samples(values: &mut [f32], channels: u32) -> AudioBufferList {
    AudioBufferList {
        mNumberBuffers: 1,
        mBuffers: [AudioBuffer {
            mNumberChannels: channels,
            mDataByteSize: std::mem::size_of_val(values).try_into().unwrap(),
            mData: values.as_mut_ptr().cast(),
        }],
    }
}

fn timestamp() -> AudioTimeStamp {
    // Apple's timestamp contains only numeric fields and transparent numeric
    // flag types; zero initializes an empty timestamp with no valid time flags.
    unsafe { std::mem::zeroed() }
}

#[test]
fn callback_transports_pcm_timestamps_and_null_input_silence() {
    let mut input = Input::new(2);
    let mut values = [0.25, 0.75, -0.5, -1.0];
    let mut list = samples(&mut values, 2);
    let mut time = AudioTimeStamp {
        mHostTime: 1234,
        mSampleTime: 56.0,
        mFlags: AudioTimeStampFlags::HostTimeValid | AudioTimeStampFlags::SampleTimeValid,
        ..timestamp()
    };
    // The list points into the owned, live values array.
    unsafe { input.deliver(&mut list, &mut time) };
    let mut pcm = [0.; 2];
    let chunk = input.reader.read(&mut pcm).unwrap().unwrap();
    assert_eq!(pcm, [0.5, -0.75]);
    assert_eq!(chunk.frames, 2);
    assert_eq!(chunk.timestamp.host_time, Some(1234));
    assert_eq!(chunk.timestamp.sample_time, Some(56.0));
    assert!(input.reader.read(&mut pcm).unwrap().is_none());
    assert!(!input.callback.busy.load(Ordering::Acquire));

    list.mBuffers[0].mData = ptr::null_mut();
    time.mFlags = AudioTimeStampFlags::empty();
    // A null Core Audio buffer represents silence; its frame size is retained.
    unsafe { input.deliver(&mut list, &mut time) };
    let chunk = input.reader.read(&mut pcm).unwrap().unwrap();
    assert_eq!(pcm, [0., 0.]);
    assert_eq!(chunk.frames, 2);
    assert_eq!(chunk.timestamp, Timestamp::default());
    assert!(!input.reader.changed());
}

#[test]
fn callback_rejects_invalid_descriptors_before_reading_samples() {
    for case in 0..6 {
        let mut input = Input::new(1);
        let mut values = [0.25, 0.5];
        let mut list = samples(&mut values, 1);
        match case {
            0 => list.mNumberBuffers = 0,
            1 => list.mNumberBuffers = (MAX_STREAMS + 1) as u32,
            2 => list.mBuffers[0].mNumberChannels = 0,
            3 => list.mBuffers[0].mDataByteSize = 3,
            4 => list.mBuffers[0].mDataByteSize = ((MAX_FRAMES + 1) * 4) as u32,
            5 => {
                list.mBuffers[0].mData = values.as_mut_ptr().cast::<u8>().wrapping_add(1).cast();
            }
            _ => unreachable!(),
        }
        // Invalid metadata must be rejected before dereferencing the deliberately
        // misaligned pointer or the oversized lengths in this fixture.
        unsafe { input.deliver(&mut list, &mut timestamp()) };
        assert!(input.reader.changed(), "case {case}");
        assert!(input.reader.read(&mut [0.; 2]).unwrap().is_none());
        assert!(!input.callback.busy.load(Ordering::Acquire));
    }
}

#[test]
fn callback_reentrancy_does_not_release_the_active_producer() {
    let mut input = Input::new(1);
    let mut values = [0.5];
    let mut list = samples(&mut values, 1);
    input.callback.busy.store(true, Ordering::Release);
    // The owned list is valid; the busy producer must prevent its consumption.
    unsafe { input.deliver(&mut list, &mut timestamp()) };
    assert!(input.reader.changed());
    assert!(input.reader.read(&mut [0.; 1]).unwrap().is_none());
    assert!(input.callback.busy.load(Ordering::Acquire));
}

#[test]
fn native_notifications_reach_capture_and_inventory_consumers() {
    let input = Input::new(1);
    let mut addr = properties::address(
        kAudioDevicePropertyDeviceIsAlive,
        kAudioObjectPropertyScopeGlobal,
    );
    // The boxed callback is stable and remains alive until after notification.
    assert_eq!(
        unsafe {
            property_changed(
                0,
                1,
                NonNull::from(&mut addr),
                (input.callback.as_ref() as *const Callback)
                    .cast_mut()
                    .cast(),
            )
        },
        0
    );
    assert!(input.reader.changed());

    let mut monitor = DeviceMonitor {
        flag: Some(Box::new(AtomicBool::new(false))),
        installed: Vec::new(),
    };
    let data = (monitor.flag.as_deref().unwrap() as *const AtomicBool).cast_mut();
    // This notification owns no HAL registration and references a live flag.
    assert_eq!(
        unsafe { devices_changed(0, 1, NonNull::from(&mut addr), data.cast()) },
        0
    );
    assert!(monitor.changed());
    assert!(!monitor.changed());
    monitor.close().unwrap();
    monitor.close().unwrap();
    assert!(!monitor.changed());
}
