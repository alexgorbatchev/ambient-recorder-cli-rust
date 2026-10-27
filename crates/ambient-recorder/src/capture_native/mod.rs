mod properties;
use super::{
    Acquired, Backend, Buffer, Clock, MAX_FRAMES, MAX_STREAMS, Mixer, Monitor, Reader, Session,
    Timestamp, queue,
};
use crate::{Error, Result, microphone::Device};
use objc2::AnyThread;
use objc2_core_audio::*;
use objc2_core_audio_types::*;
use objc2_core_foundation::{CFData, CFDictionary, CFPropertyListCreateWithData};
use objc2_foundation::{NSArray, NSNumber, NSProcessInfo, NSString, NSUUID};
use properties::{address, scalar, status};
use std::{
    cell::UnsafeCell,
    ffi::c_void,
    ptr::{self, NonNull},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub struct Native;
impl Backend for Native {
    fn microphones(&mut self) -> Result<Vec<Device>> {
        properties::microphones()
    }
    fn monitor(&mut self) -> Result<Box<dyn Monitor>> {
        Ok(Box::new(DeviceMonitor::new()?))
    }
    fn open(&mut self, device: &Device) -> Result<Box<dyn Session>> {
        Ok(Box::new(Capture::open(device.clone())?))
    }
}

struct Callback {
    mixer: UnsafeCell<Mixer>,
    health: Arc<super::Health>,
    busy: AtomicBool,
}
unsafe extern "C-unwind" fn property_changed(
    _: u32,
    _: u32,
    _: NonNull<AudioObjectPropertyAddress>,
    data: *mut c_void,
) -> i32 {
    let callback = unsafe { &*data.cast::<Callback>() };
    callback.health.changed.store(true, Ordering::Release);
    0
}
unsafe extern "C-unwind" fn audio(
    _: u32,
    _: NonNull<AudioTimeStamp>,
    input: NonNull<AudioBufferList>,
    time: NonNull<AudioTimeStamp>,
    _: NonNull<AudioBufferList>,
    _: NonNull<AudioTimeStamp>,
    data: *mut c_void,
) -> i32 {
    let callback = unsafe { &*data.cast::<Callback>() };
    if callback
        .busy
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        callback.health.changed.store(true, Ordering::Release);
        return 0;
    }
    // Only this callback mutates the producer, protected against HAL reentrancy.
    let mixer = unsafe { &mut *callback.mixer.get() };
    let list = input.as_ptr();
    let count = unsafe { (*list).mNumberBuffers as usize };
    if count == 0 || count > MAX_STREAMS {
        mixer.mark_changed();
    } else {
        let native = unsafe {
            std::slice::from_raw_parts(ptr::addr_of!((*list).mBuffers).cast::<AudioBuffer>(), count)
        };
        let mut buffers: [Buffer<'_>; MAX_STREAMS] = std::array::from_fn(|_| Buffer {
            channels: 0,
            frames: 0,
            samples: None,
        });
        let mut valid = true;
        for (buffer, value) in buffers.iter_mut().zip(native) {
            let channels = value.mNumberChannels as usize;
            let bytes = value.mDataByteSize as usize;
            if channels == 0
                || !bytes.is_multiple_of(channels.saturating_mul(4))
                || bytes / channels / 4 > MAX_FRAMES
                || (!value.mData.is_null() && !value.mData.cast::<f32>().is_aligned())
            {
                valid = false;
                break;
            }
            *buffer = Buffer {
                channels,
                frames: bytes / channels / 4,
                samples: if value.mData.is_null() {
                    None
                } else {
                    Some(unsafe {
                        std::slice::from_raw_parts(value.mData.cast::<f32>(), bytes / 4)
                    })
                },
            };
        }
        if valid {
            let time = unsafe { time.as_ref() };
            let timestamp = Timestamp {
                host_time: time
                    .mFlags
                    .contains(AudioTimeStampFlags::HostTimeValid)
                    .then_some(time.mHostTime),
                sample_time: time
                    .mFlags
                    .contains(AudioTimeStampFlags::SampleTimeValid)
                    .then_some(time.mSampleTime),
            };
            mixer.push(&buffers[..count], timestamp);
        } else {
            mixer.mark_changed();
        }
    }
    callback.busy.store(false, Ordering::Release);
    0
}

fn authorize() -> Result<()> {
    use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};
    let version = NSProcessInfo::processInfo().operatingSystemVersion();
    if (version.majorVersion, version.minorVersion) < (14, 2) {
        return Err(Error::UnsupportedPlatform);
    }
    let media = unsafe { AVMediaTypeAudio }.ok_or(Error::UnsupportedPlatform)?;
    match unsafe { AVCaptureDevice::authorizationStatusForMediaType(media) } {
        AVAuthorizationStatus::Authorized => Ok(()),
        AVAuthorizationStatus::NotDetermined => {
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            let handler = block2::RcBlock::new(move |allowed: objc2::runtime::Bool| {
                let _ = tx.send(allowed.as_bool());
            });
            unsafe {
                AVCaptureDevice::requestAccessForMediaType_completionHandler(media, &handler);
            }
            match rx.recv_timeout(Duration::from_secs(60)) {
                Ok(true) => Ok(()),
                Ok(false) => Err(Error::Invalid(
                    "microphone permission denied; allow access in Privacy & Security".into(),
                )),
                Err(error) => Err(Error::Invalid(format!(
                    "waiting for microphone permission: {error}"
                ))),
            }
        }
        _ => Err(Error::Invalid(
            "microphone permission denied or restricted; allow access in Privacy & Security".into(),
        )),
    }
}
fn host_snapshot() -> (u64, chrono::DateTime<chrono::Utc>) {
    let before = chrono::Utc::now();
    let host = unsafe { AudioConvertHostTimeToNanos(AudioGetCurrentHostTime()) };
    let after = chrono::Utc::now();
    (host, before + (after - before) / 2)
}
fn dictionary(value: plist::Dictionary) -> Result<objc2_core_foundation::CFRetained<CFDictionary>> {
    let mut bytes = Vec::new();
    plist::Value::Dictionary(value)
        .to_writer_binary(&mut bytes)
        .map_err(|error| Error::Invalid(format!("aggregate description: {error}")))?;
    let data = CFData::from_bytes(&bytes);
    let value = unsafe {
        CFPropertyListCreateWithData(None, Some(&data), 0, ptr::null_mut(), ptr::null_mut())
    }
    .ok_or_else(|| Error::Invalid("Core Foundation rejected aggregate description".into()))?;
    value
        .downcast::<CFDictionary>()
        .map_err(|_| Error::Invalid("aggregate description is not a dictionary".into()))
}
fn key(value: &std::ffi::CStr) -> String {
    value
        .to_str()
        .expect("Core Audio ASCII dictionary key")
        .into()
}
fn entry(key_value: &std::ffi::CStr, value: plist::Value) -> plist::Dictionary {
    let mut dict = plist::Dictionary::new();
    dict.insert(key(key_value), value);
    dict
}

struct Capture {
    microphone: Device,
    tap: u32,
    aggregate: u32,
    io: AudioDeviceIOProcID,
    running: bool,
    watches: Vec<(u32, AudioObjectPropertyAddress)>,
    callback: Option<Box<Callback>>,
    reader: Option<Reader>,
    rate: f64,
    clock: Clock,
    calibrated: Instant,
    closed: bool,
}
impl Capture {
    fn open(microphone: Device) -> Result<Self> {
        authorize()?;
        if microphone.id == 0 || microphone.uid.is_empty() {
            return Err(Error::Invalid("microphone has no device identity".into()));
        }
        let (host, wall) = host_snapshot();
        let mut capture = Self {
            microphone,
            tap: 0,
            aggregate: 0,
            io: None,
            running: false,
            watches: Vec::new(),
            callback: None,
            reader: None,
            rate: 0.,
            clock: Clock::new(host, wall, 1.),
            calibrated: Instant::now(),
            closed: false,
        };
        if let Err(error) = capture.initialize() {
            capture.close()?;
            return Err(error);
        }
        Ok(capture)
    }
    fn initialize(&mut self) -> Result<()> {
        let description = unsafe {
            CATapDescription::initMonoGlobalTapButExcludeProcesses(
                CATapDescription::alloc(),
                &NSArray::<NSNumber>::new(),
            )
        };
        unsafe {
            description.setName(&NSString::from_str("Ambient Recorder Playback"));
            description.setPrivate(true);
            description.setMuteBehavior(CATapMuteBehavior::Unmuted);
        }
        status(
            unsafe { AudioHardwareCreateProcessTap(Some(&description), &mut self.tap) },
            "create playback tap",
        )?;
        let uid = self.microphone.uid.clone();
        let tap_uid = unsafe { description.UUID() }.UUIDString().to_string();
        let mut tap = entry(kAudioSubTapUIDKey, tap_uid.into());
        tap.insert(key(kAudioSubTapDriftCompensationKey), true.into());
        let mut aggregate = entry(
            kAudioAggregateDeviceUIDKey,
            NSUUID::new().UUIDString().to_string().into(),
        );
        aggregate.insert(
            key(kAudioAggregateDeviceNameKey),
            "Ambient Recorder Capture".into(),
        );
        aggregate.insert(key(kAudioAggregateDeviceIsPrivateKey), true.into());
        aggregate.insert(key(kAudioAggregateDeviceIsStackedKey), false.into());
        aggregate.insert(key(kAudioAggregateDeviceTapAutoStartKey), false.into());
        aggregate.insert(
            key(kAudioAggregateDeviceMainSubDeviceKey),
            uid.clone().into(),
        );
        aggregate.insert(
            key(kAudioAggregateDeviceSubDeviceListKey),
            plist::Value::Array(vec![plist::Value::Dictionary(entry(
                kAudioSubDeviceUIDKey,
                uid.into(),
            ))]),
        );
        aggregate.insert(
            key(kAudioAggregateDeviceTapListKey),
            plist::Value::Array(vec![plist::Value::Dictionary(tap)]),
        );
        let aggregate = dictionary(aggregate)?;
        status(
            unsafe {
                AudioHardwareCreateAggregateDevice(&aggregate, NonNull::from(&mut self.aggregate))
            },
            "create private aggregate",
        )?;
        self.rate = scalar(
            self.aggregate,
            kAudioDevicePropertyNominalSampleRate,
            kAudioObjectPropertyScopeGlobal,
        )?;
        if !self.rate.is_finite() || self.rate <= 0. || self.rate > 192_000. {
            return Err(Error::Invalid("invalid aggregate sample rate".into()));
        }
        let tap_format: AudioStreamBasicDescription = scalar(
            self.tap,
            kAudioTapPropertyFormat,
            kAudioObjectPropertyScopeGlobal,
        )?;
        let channels =
            properties::input_channels(self.microphone.id)? + tap_format.mChannelsPerFrame as usize;
        let streams = properties::configure(self.aggregate, channels, self.rate)?;
        let (mixer, reader) = queue(channels);
        let health = mixer.health.clone();
        self.callback = Some(Box::new(Callback {
            mixer: UnsafeCell::new(mixer),
            health,
            busy: AtomicBool::new(false),
        }));
        self.reader = Some(reader);
        self.watch(
            self.aggregate,
            kAudioDevicePropertyNominalSampleRate,
            kAudioObjectPropertyScopeGlobal,
        )?;
        for stream in streams {
            self.watch(
                stream,
                kAudioStreamPropertyVirtualFormat,
                kAudioObjectPropertyScopeGlobal,
            )?;
        }
        self.watch(
            self.microphone.id,
            kAudioDevicePropertyDeviceIsAlive,
            kAudioObjectPropertyScopeGlobal,
        )?;
        self.watch(
            self.microphone.id,
            kAudioDevicePropertyNominalSampleRate,
            kAudioObjectPropertyScopeGlobal,
        )?;
        self.watch(
            self.microphone.id,
            kAudioDevicePropertyStreamConfiguration,
            kAudioObjectPropertyScopeInput,
        )?;
        let data = (self.callback.as_deref().unwrap() as *const Callback).cast_mut();
        status(
            unsafe {
                AudioDeviceCreateIOProcID(
                    self.aggregate,
                    Some(audio),
                    data.cast(),
                    NonNull::from(&mut self.io),
                )
            },
            "register capture callback",
        )?;
        self.callback
            .as_ref()
            .unwrap()
            .health
            .changed
            .store(false, Ordering::Release);
        status(
            unsafe { AudioDeviceStart(self.aggregate, self.io) },
            "start capture; verify microphone and system-audio permissions",
        )?;
        self.running = true;
        Ok(())
    }
    fn watch(&mut self, object: u32, selector: u32, scope: u32) -> Result<()> {
        let mut addr = address(selector, scope);
        let data = (self.callback.as_deref().unwrap() as *const Callback).cast_mut();
        status(
            unsafe {
                AudioObjectAddPropertyListener(
                    object,
                    NonNull::from(&mut addr),
                    Some(property_changed),
                    data.cast(),
                )
            },
            "watch input property",
        )?;
        self.watches.push((object, addr));
        Ok(())
    }
}
impl Session for Capture {
    fn sample_rate(&self) -> f64 {
        self.rate
    }
    fn microphone(&self) -> &Device {
        &self.microphone
    }
    fn aggregate_id(&self) -> u32 {
        self.aggregate
    }
    fn read(&mut self, pcm: &mut [f32]) -> Result<Option<Acquired>> {
        if self.closed || pcm.len() < MAX_FRAMES {
            return Err(Error::Invalid(
                "capture is closed or read buffer is smaller than 8192 frames".into(),
            ));
        }
        let Some(chunk) = self.reader.as_mut().unwrap().read(pcm)? else {
            return Ok(None);
        };
        let host = chunk
            .timestamp
            .host_time
            .ok_or_else(|| Error::Invalid("acquired audio has no host timestamp".into()))?;
        let host = unsafe { AudioConvertHostTimeToNanos(host) };
        Ok(Some(Acquired {
            frames: chunk.frames,
            time: self.clock.acquired(Some(host), chrono::Utc::now()),
            timestamp: chunk.timestamp,
        }))
    }
    fn changed(&self) -> bool {
        self.reader.as_ref().is_some_and(Reader::changed)
    }
    fn dropped(&self) -> u64 {
        self.reader.as_ref().map_or(0, Reader::dropped)
    }
    fn recalibrate(&mut self) -> Result<chrono::Duration> {
        if self.calibrated.elapsed() < Duration::from_secs(1) {
            return Ok(chrono::Duration::zero());
        }
        let (host, wall) = host_snapshot();
        self.calibrated = Instant::now();
        let shift = wall - self.clock.acquired(Some(host), wall);
        Ok(if self.clock.refresh(host, wall)? {
            shift
        } else {
            chrono::Duration::zero()
        })
    }
    fn stop(&mut self) -> Result<()> {
        if self.running {
            status(
                unsafe { AudioDeviceStop(self.aggregate, self.io) },
                "stop capture",
            )?;
            self.running = false;
        }
        Ok(())
    }
    fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        let mut errors = Vec::new();
        if let Err(error) = self.stop() {
            errors.push(error.to_string());
        }
        if self.io.is_some() {
            if let Err(error) = status(
                unsafe { AudioDeviceDestroyIOProcID(self.aggregate, self.io) },
                "remove callback",
            ) {
                errors.push(error.to_string());
            }
            self.io = None;
        }
        if let Some(callback) = self.callback.as_deref() {
            let data = (callback as *const Callback).cast_mut();
            for (object, mut addr) in self.watches.drain(..) {
                if let Err(error) = status(
                    unsafe {
                        AudioObjectRemovePropertyListener(
                            object,
                            NonNull::from(&mut addr),
                            Some(property_changed),
                            data.cast(),
                        )
                    },
                    "remove property listener",
                ) {
                    errors.push(error.to_string());
                }
            }
        }
        if self.aggregate != 0 {
            if let Err(error) = status(
                unsafe { AudioHardwareDestroyAggregateDevice(self.aggregate) },
                "destroy aggregate",
            ) {
                errors.push(error.to_string());
            }
            self.aggregate = 0;
        }
        if self.tap != 0 {
            if let Err(error) = status(
                unsafe { AudioHardwareDestroyProcessTap(self.tap) },
                "destroy tap",
            ) {
                errors.push(error.to_string());
            }
            self.tap = 0;
        }
        if errors.is_empty() {
            self.callback.take();
            Ok(())
        } else {
            // HAL may still own callbacks after failed deregistration. Retaining
            // the context until process exit prevents use-after-free.
            if let Some(callback) = self.callback.take() {
                Box::leak(callback);
            }
            Err(Error::Cleanup(errors.join("; ")))
        }
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

struct DeviceMonitor {
    flag: Option<Box<AtomicBool>>,
    installed: Vec<AudioObjectPropertyAddress>,
}
unsafe extern "C-unwind" fn devices_changed(
    _: u32,
    _: u32,
    _: NonNull<AudioObjectPropertyAddress>,
    data: *mut c_void,
) -> i32 {
    unsafe { &*data.cast::<AtomicBool>() }.store(true, Ordering::Release);
    0
}
impl DeviceMonitor {
    fn new() -> Result<Self> {
        let mut monitor = Self {
            flag: Some(Box::new(AtomicBool::new(true))),
            installed: Vec::new(),
        };
        for selector in [
            kAudioHardwarePropertyDevices,
            kAudioHardwarePropertyDefaultInputDevice,
        ] {
            let mut addr = address(selector, kAudioObjectPropertyScopeGlobal);
            let data = (monitor.flag.as_deref().unwrap() as *const AtomicBool).cast_mut();
            if let Err(error) = status(
                unsafe {
                    AudioObjectAddPropertyListener(
                        kAudioObjectSystemObject as u32,
                        NonNull::from(&mut addr),
                        Some(devices_changed),
                        data.cast(),
                    )
                },
                "watch microphone inventory",
            ) {
                monitor.close()?;
                return Err(error);
            }
            monitor.installed.push(addr);
        }
        Ok(monitor)
    }
}
impl Monitor for DeviceMonitor {
    fn changed(&mut self) -> bool {
        self.flag
            .as_ref()
            .is_some_and(|flag| flag.swap(false, Ordering::AcqRel))
    }
    fn close(&mut self) -> Result<()> {
        let Some(flag) = self.flag.take() else {
            return Ok(());
        };
        let data = (&*flag as *const AtomicBool).cast_mut();
        let mut errors = Vec::new();
        for mut addr in self.installed.drain(..) {
            if let Err(error) = status(
                unsafe {
                    AudioObjectRemovePropertyListener(
                        kAudioObjectSystemObject as u32,
                        NonNull::from(&mut addr),
                        Some(devices_changed),
                        data.cast(),
                    )
                },
                "remove inventory listener",
            ) {
                errors.push(error.to_string());
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Box::leak(flag);
            Err(Error::Cleanup(errors.join("; ")))
        }
    }
}
impl Drop for DeviceMonitor {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
