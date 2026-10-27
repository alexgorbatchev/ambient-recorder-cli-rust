use super::*;
pub(super) fn device_fields(device: &Device) -> Value {
    json!({"microphone_id":device.id,"microphone_uid":device.uid,"microphone_name":device.name,"microphone_transport":device.transport,
        "microphone_type":device.terminal,"microphone_manufacturer":device.manufacturer,"microphone_model":device.model})
}
fn key(device: &Device) -> String {
    if device.uid.is_empty() {
        format!("id:{}", device.id)
    } else {
        device.uid.clone()
    }
}
impl Runner<'_> {
    pub(super) fn refresh(&mut self, force: bool) -> Result<()> {
        let changed = self.monitor.as_mut().is_some_and(|m| m.changed());
        if !force
            && !changed
            && self
                .refreshed
                .is_some_and(|at| at.elapsed() < self.config.timing.inventory)
        {
            return Ok(());
        }
        self.refreshed = Some(Instant::now());
        let devices: Vec<_> = self
            .backend
            .microphones()?
            .into_iter()
            .filter(|d| d.id != self.aggregate)
            .collect();
        for old in self.devices.clone() {
            if !devices
                .iter()
                .any(|d| key(d) == key(&old) && d.id == old.id)
            {
                self.log(Level::Info, "Microphone disconnected", device_fields(&old));
                self.failures.remove(&key(&old));
            }
        }
        for device in &devices {
            if !self
                .devices
                .iter()
                .any(|d| key(d) == key(device) && d.id == device.id)
            {
                self.log(
                    Level::Info,
                    if self.initialized {
                        "Microphone connected"
                    } else {
                        "Microphone available"
                    },
                    device_fields(device),
                );
            }
        }
        self.devices = devices;
        self.initialized = true;
        Ok(())
    }
    pub(super) fn preferred(&self) -> Vec<Device> {
        self.preferences
            .ordered(&self.devices)
            .into_iter()
            .filter(|d| {
                self.failures
                    .get(&key(d))
                    .is_none_or(|until| Instant::now() >= *until)
            })
            .cloned()
            .collect()
    }
    pub(super) fn failed(&mut self, device: &Device, error: &Error) {
        self.failures
            .insert(key(device), Instant::now() + self.config.timing.cooldown);
        let mut fields = device_fields(device);
        fields["error"] = json!(error.to_string());
        fields["retry_in"] = json!(format!("{:?}", self.config.timing.cooldown));
        self.log(
            Level::Warn,
            "Microphone capture failed; trying next preference",
            fields,
        );
    }
}
