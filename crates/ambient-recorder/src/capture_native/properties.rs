use crate::{Error, Result, microphone::Device};
use objc2_core_audio::*;
use objc2_core_audio_types::AudioBufferList;
use objc2_core_foundation::{CFRetained, CFString};
use std::{
    ffi::c_void,
    mem::{MaybeUninit, size_of},
    ptr::{self, NonNull},
};

pub(super) fn status(code: i32, operation: &str) -> Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(Error::Invalid(format!(
            "Core Audio {operation}: OSStatus {code}"
        )))
    }
}
pub(super) fn address(selector: u32, scope: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: scope,
        mElement: kAudioObjectPropertyElementMain,
    }
}
pub(super) fn scalar<T: Copy>(id: u32, selector: u32, scope: u32) -> Result<T> {
    let mut addr = address(selector, scope);
    let mut out = MaybeUninit::<T>::uninit();
    let mut size = size_of::<T>() as u32;
    // All callers request the exact Core Audio C property type. HAL receives
    // an aligned output buffer, and no uninitialized value is read on failure.
    status(
        unsafe {
            AudioObjectGetPropertyData(
                id,
                NonNull::from(&mut addr),
                0,
                ptr::null(),
                NonNull::from(&mut size),
                NonNull::new(out.as_mut_ptr().cast()).unwrap(),
            )
        },
        "read property",
    )?;
    if size as usize != size_of::<T>() {
        return Err(Error::Invalid("Core Audio property size changed".into()));
    }
    Ok(unsafe { out.assume_init() })
}
pub(super) fn set<T>(id: u32, selector: u32, scope: u32, value: &mut T) -> Result<()> {
    let mut addr = address(selector, scope);
    status(
        unsafe {
            AudioObjectSetPropertyData(
                id,
                NonNull::from(&mut addr),
                0,
                ptr::null(),
                size_of::<T>() as u32,
                NonNull::from(value).cast(),
            )
        },
        "set property",
    )
}
pub(super) fn bytes(id: u32, selector: u32, scope: u32) -> Result<(Vec<u64>, usize)> {
    let mut addr = address(selector, scope);
    let mut size = 0;
    status(
        unsafe {
            AudioObjectGetPropertyDataSize(
                id,
                NonNull::from(&mut addr),
                0,
                ptr::null(),
                NonNull::from(&mut size),
            )
        },
        "size property",
    )?;
    if size == 0 {
        return Ok((Vec::new(), 0));
    }
    let capacity = size;
    let mut out = vec![0u64; (size as usize).div_ceil(size_of::<u64>())];
    status(
        unsafe {
            AudioObjectGetPropertyData(
                id,
                NonNull::from(&mut addr),
                0,
                ptr::null(),
                NonNull::from(&mut size),
                NonNull::new(out.as_mut_ptr().cast::<c_void>()).unwrap(),
            )
        },
        "read property data",
    )?;
    if size > capacity {
        return Err(Error::Invalid(
            "Core Audio property grew during read".into(),
        ));
    }
    Ok((out, size as usize))
}
pub(super) fn ids(id: u32, selector: u32, scope: u32) -> Result<Vec<u32>> {
    let (words, size) = bytes(id, selector, scope)?;
    if !size.is_multiple_of(size_of::<u32>()) {
        return Err(Error::Invalid("invalid Core Audio identifier list".into()));
    }
    if size == 0 {
        return Ok(Vec::new());
    }
    Ok(
        unsafe {
            std::slice::from_raw_parts(words.as_ptr().cast::<u32>(), size / size_of::<u32>())
        }
        .to_vec(),
    )
}
pub(super) fn input_channels(id: u32) -> Result<usize> {
    let (words, size) = bytes(
        id,
        kAudioDevicePropertyStreamConfiguration,
        kAudioObjectPropertyScopeInput,
    )?;
    if size < size_of::<u32>() {
        return Ok(0);
    }
    let list = words.as_ptr().cast::<AudioBufferList>();
    let count = unsafe { (*list).mNumberBuffers as usize };
    let offset = std::mem::offset_of!(AudioBufferList, mBuffers);
    if count > (size.saturating_sub(offset)) / size_of::<objc2_core_audio_types::AudioBuffer>() {
        return Err(Error::Invalid(
            "invalid Core Audio stream configuration".into(),
        ));
    }
    let buffers = unsafe {
        std::slice::from_raw_parts(
            ptr::addr_of!((*list).mBuffers).cast::<objc2_core_audio_types::AudioBuffer>(),
            count,
        )
    };
    Ok(buffers.iter().map(|b| b.mNumberChannels as usize).sum())
}
fn text(id: u32, selector: u32) -> String {
    let Ok(raw) = scalar::<*mut CFString>(id, selector, kAudioObjectPropertyScopeGlobal) else {
        return String::new();
    };
    NonNull::new(raw)
        .map(|raw| unsafe { CFRetained::from_raw(raw) }.to_string())
        .unwrap_or_default()
}
// Apple exposes these named constants in mixed case; retain the native names.
#[allow(non_upper_case_globals)]
fn transport(value: u32) -> &'static str {
    match value {
        kAudioDeviceTransportTypeBuiltIn => "Built-in",
        kAudioDeviceTransportTypeAggregate => "Aggregate",
        kAudioDeviceTransportTypeVirtual => "Virtual",
        kAudioDeviceTransportTypePCI => "PCI",
        kAudioDeviceTransportTypeUSB => "USB",
        kAudioDeviceTransportTypeFireWire => "FireWire",
        kAudioDeviceTransportTypeBluetooth => "Bluetooth",
        kAudioDeviceTransportTypeBluetoothLE => "Bluetooth LE",
        kAudioDeviceTransportTypeHDMI => "HDMI",
        kAudioDeviceTransportTypeDisplayPort => "DisplayPort",
        kAudioDeviceTransportTypeAirPlay => "AirPlay",
        kAudioDeviceTransportTypeAVB => "AVB network",
        kAudioDeviceTransportTypeThunderbolt => "Thunderbolt",
        kAudioDeviceTransportTypeContinuityCaptureWired => "Continuity (wired)",
        kAudioDeviceTransportTypeContinuityCaptureWireless => "Continuity (wireless)",
        _ => "Unavailable",
    }
}
#[allow(non_upper_case_globals)]
fn terminal(id: u32) -> Result<&'static str> {
    let streams = ids(
        id,
        kAudioDevicePropertyStreams,
        kAudioObjectPropertyScopeInput,
    )?;
    let mut kind = None;
    for stream in streams {
        let current: u32 = scalar(
            stream,
            kAudioStreamPropertyTerminalType,
            kAudioObjectPropertyScopeGlobal,
        )?;
        if kind.is_some_and(|kind| kind != current) {
            return Ok("Multiple input types");
        }
        kind = Some(current);
    }
    Ok(match kind.unwrap_or(kAudioStreamTerminalTypeUnknown) {
        kAudioStreamTerminalTypeMicrophone => "Microphone",
        kAudioStreamTerminalTypeHeadsetMicrophone => "Headset microphone",
        kAudioStreamTerminalTypeReceiverMicrophone => "Handset microphone",
        kAudioStreamTerminalTypeLine => "Line input",
        kAudioStreamTerminalTypeDigitalAudioInterface => "Digital audio interface",
        kAudioStreamTerminalTypeHDMI => "HDMI audio",
        kAudioStreamTerminalTypeDisplayPort => "DisplayPort audio",
        _ => "Unavailable",
    })
}
pub(super) fn describe(id: u32, default: u32) -> Device {
    Device {
        id,
        default: id == default,
        name: text(id, kAudioObjectPropertyName),
        uid: text(id, kAudioDevicePropertyDeviceUID),
        manufacturer: text(id, kAudioObjectPropertyManufacturer),
        model: text(id, kAudioDevicePropertyModelUID),
        transport: transport(
            scalar(
                id,
                kAudioDevicePropertyTransportType,
                kAudioObjectPropertyScopeGlobal,
            )
            .unwrap_or(kAudioDeviceTransportTypeUnknown),
        )
        .into(),
        terminal: terminal(id).unwrap_or("Unavailable").into(),
    }
}
pub(super) fn microphones() -> Result<Vec<Device>> {
    let default = scalar(
        kAudioObjectSystemObject as u32,
        kAudioHardwarePropertyDefaultInputDevice,
        kAudioObjectPropertyScopeGlobal,
    )
    .unwrap_or(0);
    let mut out = Vec::new();
    for id in ids(
        kAudioObjectSystemObject as u32,
        kAudioHardwarePropertyDevices,
        kAudioObjectPropertyScopeGlobal,
    )? {
        if scalar::<u32>(
            id,
            kAudioDevicePropertyDeviceIsAlive,
            kAudioObjectPropertyScopeGlobal,
        )
        .unwrap_or(0)
            != 0
            && input_channels(id).unwrap_or(0) > 0
        {
            out.push(describe(id, default));
        }
    }
    Ok(out)
}
pub(super) fn configure(id: u32, expected_channels: usize, rate: f64) -> Result<Vec<u32>> {
    use objc2_core_audio_types::*;
    let streams = ids(
        id,
        kAudioDevicePropertyStreams,
        kAudioObjectPropertyScopeInput,
    )?;
    if streams.is_empty() || streams.len() > super::super::MAX_STREAMS {
        return Err(Error::Invalid(
            "invalid aggregate input stream count".into(),
        ));
    }
    let mut channels = 0usize;
    for stream in &streams {
        let mut format: AudioStreamBasicDescription = scalar(
            *stream,
            kAudioStreamPropertyVirtualFormat,
            kAudioObjectPropertyScopeGlobal,
        )?;
        let noninterleaved = format.mFormatFlags & kAudioFormatFlagIsNonInterleaved;
        let stride = if noninterleaved != 0 {
            4
        } else {
            4 * format.mChannelsPerFrame
        };
        let flags = kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked | noninterleaved;
        if format.mFormatID != kAudioFormatLinearPCM
            || format.mFormatFlags != flags
            || format.mBitsPerChannel != 32
            || format.mBytesPerFrame != stride
            || format.mFramesPerPacket != 1
            || format.mBytesPerPacket != stride
        {
            let mut addr = address(
                kAudioStreamPropertyVirtualFormat,
                kAudioObjectPropertyScopeGlobal,
            );
            let mut settable = 0;
            status(
                unsafe {
                    AudioObjectIsPropertySettable(
                        *stream,
                        NonNull::from(&mut addr),
                        NonNull::from(&mut settable),
                    )
                },
                "query format writability",
            )?;
            if settable == 0 {
                return Err(Error::Invalid(
                    "aggregate input format cannot be converted to native float PCM".into(),
                ));
            }
            format.mFormatID = kAudioFormatLinearPCM;
            format.mFormatFlags = flags;
            format.mBitsPerChannel = 32;
            format.mSampleRate = rate;
            format.mBytesPerFrame = stride;
            format.mFramesPerPacket = 1;
            format.mBytesPerPacket = stride;
            set(
                *stream,
                kAudioStreamPropertyVirtualFormat,
                kAudioObjectPropertyScopeGlobal,
                &mut format,
            )?;
            format = scalar(
                *stream,
                kAudioStreamPropertyVirtualFormat,
                kAudioObjectPropertyScopeGlobal,
            )?;
        }
        if format.mFormatID != kAudioFormatLinearPCM
            || format.mFormatFlags != flags
            || format.mBitsPerChannel != 32
            || format.mBytesPerFrame != stride
            || format.mFramesPerPacket != 1
            || format.mBytesPerPacket != stride
            || format.mChannelsPerFrame == 0
        {
            return Err(Error::Invalid(
                "aggregate rejected native float PCM format".into(),
            ));
        }
        channels += format.mChannelsPerFrame as usize;
    }
    if channels != expected_channels {
        return Err(Error::Invalid(
            "aggregate channel count differs from microphone and tap".into(),
        ));
    }
    Ok(streams)
}
