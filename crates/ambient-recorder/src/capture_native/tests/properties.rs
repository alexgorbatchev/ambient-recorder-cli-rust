use super::*;

#[test]
fn device_transport_labels_distinguish_connections_and_unknown_native_values() {
    for (native, label) in [
        (kAudioDeviceTransportTypeBuiltIn, "Built-in"),
        (kAudioDeviceTransportTypeAggregate, "Aggregate"),
        (kAudioDeviceTransportTypeVirtual, "Virtual"),
        (kAudioDeviceTransportTypePCI, "PCI"),
        (kAudioDeviceTransportTypeUSB, "USB"),
        (kAudioDeviceTransportTypeFireWire, "FireWire"),
        (kAudioDeviceTransportTypeBluetooth, "Bluetooth"),
        (kAudioDeviceTransportTypeBluetoothLE, "Bluetooth LE"),
        (kAudioDeviceTransportTypeHDMI, "HDMI"),
        (kAudioDeviceTransportTypeDisplayPort, "DisplayPort"),
        (kAudioDeviceTransportTypeAirPlay, "AirPlay"),
        (kAudioDeviceTransportTypeAVB, "AVB network"),
        (kAudioDeviceTransportTypeThunderbolt, "Thunderbolt"),
        (
            kAudioDeviceTransportTypeContinuityCaptureWired,
            "Continuity (wired)",
        ),
        (
            kAudioDeviceTransportTypeContinuityCaptureWireless,
            "Continuity (wireless)",
        ),
        (kAudioDeviceTransportTypeUnknown, "Unavailable"),
        (u32::MAX, "Unavailable"),
    ] {
        assert_eq!(transport(native), label, "native transport {native:#x}");
    }
}
