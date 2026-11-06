use super::*;

#[test]
fn inactive_private_aggregate_owns_properties_listeners_and_callback_registration() {
    let uid = NSUUID::new().UUIDString().to_string();
    let name = format!("Ambient inactive fixture {uid}");
    let mut description = entry(kAudioAggregateDeviceUIDKey, uid.clone().into());
    description.insert(key(kAudioAggregateDeviceNameKey), name.clone().into());
    description.insert(key(kAudioAggregateDeviceIsPrivateKey), true.into());
    // No subdevices or taps: this fixture has no audio source and never starts IO.
    let description = dictionary(description).unwrap();
    let mut capture = session::session(Input::new(1), 0, chrono::Utc::now());
    status(
        unsafe {
            AudioHardwareCreateAggregateDevice(&description, NonNull::from(&mut capture.aggregate))
        },
        "create inactive fixture",
    )
    .unwrap();
    let id = capture.aggregate_id();
    assert_ne!(id, 0);
    let device = properties::describe(id, id);
    assert_eq!(device.uid, uid);
    assert_eq!(device.name, name);
    assert!(device.default);
    assert_eq!(device.transport, "Aggregate");
    assert_eq!(properties::input_channels(id).unwrap(), 0);
    assert!(matches!(
        properties::configure(id, 1, 48_000.),
        Err(Error::Invalid(message)) if message.contains("input stream count")
    ));

    capture
        .watch(
            id,
            kAudioDevicePropertyNominalSampleRate,
            kAudioObjectPropertyScopeGlobal,
        )
        .unwrap();
    let data = (capture.callback.as_deref().unwrap() as *const Callback).cast_mut();
    // Registration alone does not start IO; the boxed context stays owned until
    // Capture::close removes the callback and listener before destroying the HAL object.
    status(
        unsafe {
            AudioDeviceCreateIOProcID(id, Some(audio), data.cast(), NonNull::from(&mut capture.io))
        },
        "register inactive fixture callback",
    )
    .unwrap();
    assert!(capture.io.is_some());
    assert_eq!(capture.watches.len(), 1);
    assert!(!capture.running);
    capture.close().unwrap();
    capture.close().unwrap();
    assert!(capture.io.is_none());
    assert!(capture.watches.is_empty());
    assert!(capture.callback.is_none());
    assert_eq!(capture.aggregate_id(), 0);
}

#[test]
fn missing_microphone_initialization_releases_partial_native_resources_without_starting_io() {
    let mut capture = session::session(Input::new(1), 0, chrono::Utc::now());
    capture.microphone.uid = NSUUID::new().UUIDString().to_string();
    // Object zero is never a microphone. Even if HAL constructs a tap-only
    // aggregate for the missing UID, input_channels(0) fails before IO registration
    // or AudioDeviceStart. This deliberately never calls permission acquisition.
    let result = capture.initialize();
    assert!(matches!(result, Err(Error::Invalid(_))));
    assert_ne!(
        capture.tap, 0,
        "the fixture must exercise partial allocation"
    );
    assert!(capture.io.is_none());
    assert!(!capture.running);
    capture.close().unwrap();
    assert_eq!(capture.tap, 0);
    assert_eq!(capture.aggregate_id(), 0);
    assert!(capture.callback.is_none());
    capture.close().unwrap();
}

#[test]
fn invalid_native_property_reads_and_writes_preserve_operation_errors() {
    let scope = kAudioObjectPropertyScopeGlobal;
    assert!(matches!(
        properties::scalar::<u32>(0, kAudioDevicePropertyDeviceIsAlive, scope),
        Err(Error::Invalid(message)) if message.contains("read property: OSStatus")
    ));
    assert!(matches!(
        properties::input_channels(0),
        Err(Error::Invalid(message)) if message.contains("size property: OSStatus")
    ));
    assert!(matches!(
        properties::set(0, kAudioDevicePropertyNominalSampleRate, scope, &mut 48_000f64),
        Err(Error::Invalid(message)) if message.contains("set property: OSStatus")
    ));
}
