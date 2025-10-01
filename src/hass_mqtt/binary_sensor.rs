use crate::hass_mqtt::base::{Device, EntityConfig, Origin};
use crate::hass_mqtt::instance::{publish_entity_config, EntityInstance};
use crate::platform_api::DeviceCapability;
use crate::service::device::Device as ServiceDevice;
use crate::service::hass::{availability_topic, topic_safe_id, topic_safe_string, HassClient};
use crate::service::state::StateHandle;
use async_trait::async_trait;
use serde::Serialize;

#[derive(Serialize, Clone, Debug)]
pub struct BinarySensorConfig {
    #[serde(flatten)]
    pub base: EntityConfig,

    pub state_topic: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_class: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_on: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_off: Option<String>,
}

impl BinarySensorConfig {
    pub async fn publish(&self, state: &StateHandle, client: &HassClient) -> anyhow::Result<()> {
        publish_entity_config("binary_sensor", state, client, &self.base, self).await
    }

    pub async fn notify_state(&self, client: &HassClient, value: &str) -> anyhow::Result<()> {
        client.publish(&self.state_topic, value).await
    }
}

#[derive(Clone)]
pub struct AlarmEventSensor {
    sensor: BinarySensorConfig,
    device_id: String,
    state: StateHandle,
    instance_name: String,
}

impl AlarmEventSensor {
    pub async fn new(
        device: &ServiceDevice,
        state: &StateHandle,
        instance: &DeviceCapability,
    ) -> anyhow::Result<Self> {
        let unique_id = format!(
            "binary-sensor-{id}-{inst}",
            id = topic_safe_id(device),
            inst = topic_safe_string(&instance.instance)
        );

        // Determine device class and name based on event type
        let (device_class, name) = match instance.instance.as_str() {
            "lowBatteryEvent" => (Some("battery"), "Low Battery"),
            "lackWaterEvent" => (Some("problem"), "Water Level Alert"),
            "temperatureAlarmEvent" | "tempAlarmEvent" => (Some("problem"), "Temperature Alarm"),
            "humidityAlarmEvent" | "humAlarmEvent" => (Some("problem"), "Humidity Alarm"),
            s if s.ends_with("AlarmEvent") => (Some("problem"), "Alarm"),
            s if s.ends_with("Event") => (Some("problem"), "Alert"),
            _ => (None, "Event"),
        };

        let name = name.to_string();

        Ok(Self {
            sensor: BinarySensorConfig {
                base: EntityConfig {
                    availability_topic: availability_topic(),
                    name: Some(name),
                    entity_category: Some("diagnostic".to_string()),
                    origin: Origin::default(),
                    device: Device::for_device(device),
                    unique_id: unique_id.clone(),
                    device_class: device_class.map(|s| s.to_string()),
                    icon: None,
                },
                state_topic: format!("gv2mqtt/binary_sensor/{unique_id}/state"),
                device_class,
                payload_on: Some("ON".to_string()),
                payload_off: Some("OFF".to_string()),
            },
            device_id: device.id.to_string(),
            state: state.clone(),
            instance_name: instance.instance.to_string(),
        })
    }
}

#[async_trait]
impl EntityInstance for AlarmEventSensor {
    async fn publish_config(&self, state: &StateHandle, client: &HassClient) -> anyhow::Result<()> {
        self.sensor.publish(&state, &client).await
    }

    async fn notify_state(&self, client: &HassClient) -> anyhow::Result<()> {
        let device = self
            .state
            .device_by_id(&self.device_id)
            .await
            .expect("device to exist");

        if let Some(cap) = device.get_state_capability_by_instance(&self.instance_name) {
            // Try to extract alarm state from the capability state
            // Events typically have a value field indicating if the alarm is active
            let is_active = cap
                .state
                .pointer("/value")
                .and_then(|v| v.as_i64())
                .map(|v| v != 0)
                .unwrap_or(false);

            let state_value = if is_active { "ON" } else { "OFF" };
            return self.sensor.notify_state(&client, state_value).await;
        }

        log::trace!(
            "AlarmEventSensor::notify_state: didn't find state for {device} {instance}",
            instance = self.instance_name
        );
        Ok(())
    }
}

