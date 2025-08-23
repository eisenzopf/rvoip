//! PIDF XML generation and parsing

use quick_xml::events::{BytesDecl, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};
use std::io::Cursor;
use crate::types::{PresenceState, ExtendedStatus, BasicStatus};
#[cfg(test)]
use crate::types::DevicePresence;
use crate::error::{RegistrarError, Result};

/// PIDF (Presence Information Data Format) generator and parser
pub struct PidfGenerator;

impl PidfGenerator {
    pub fn new() -> Self {
        Self
    }
    
    /// Create PIDF XML document from presence state
    pub async fn create_pidf(&self, presence: &PresenceState) -> Result<String> {
        let mut writer = Writer::new(Cursor::new(Vec::new()));
        
        // XML declaration
        writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))
            .map_err(|e| RegistrarError::PidfError(e.to_string()))?;
        
        // Presence element with namespaces
        let mut presence_elem = BytesStart::new("presence");
        presence_elem.push_attribute(("xmlns", "urn:ietf:params:xml:ns:pidf"));
        presence_elem.push_attribute(("xmlns:dm", "urn:ietf:params:xml:ns:pidf:data-model"));
        presence_elem.push_attribute(("entity", format!("sip:{}", presence.user_id).as_str()));
        
        writer.write_event(Event::Start(presence_elem))
            .map_err(|e| RegistrarError::PidfError(e.to_string()))?;
        
        // Tuple element for each device
        for (idx, device) in presence.devices.iter().enumerate() {
            let mut tuple = BytesStart::new("tuple");
            tuple.push_attribute(("id", format!("device-{}", idx).as_str()));
            
            writer.write_event(Event::Start(tuple))
                .map_err(|e| RegistrarError::PidfError(e.to_string()))?;
            
            // Status
            writer.write_event(Event::Start(BytesStart::new("status")))
                .map_err(|e| RegistrarError::PidfError(e.to_string()))?;
            
            // Basic status
            writer.write_event(Event::Start(BytesStart::new("basic")))
                .map_err(|e| RegistrarError::PidfError(e.to_string()))?;
            let basic_text = match presence.basic_status {
                BasicStatus::Open => "open",
                BasicStatus::Closed => "closed",
            };
            writer.write_event(Event::Text(BytesText::new(basic_text)))
                .map_err(|e| RegistrarError::PidfError(e.to_string()))?;
            writer.write_event(Event::End(BytesStart::new("basic").to_end()))
                .map_err(|e| RegistrarError::PidfError(e.to_string()))?;
            
            writer.write_event(Event::End(BytesStart::new("status").to_end()))
                .map_err(|e| RegistrarError::PidfError(e.to_string()))?;
            
            // Contact (using instance_id as contact URI)
            writer.write_event(Event::Start(BytesStart::new("contact")))
                .map_err(|e| RegistrarError::PidfError(e.to_string()))?;
            writer.write_event(Event::Text(BytesText::new(&format!("sip:{}@device", device.instance_id))))
                .map_err(|e| RegistrarError::PidfError(e.to_string()))?;
            writer.write_event(Event::End(BytesStart::new("contact").to_end()))
                .map_err(|e| RegistrarError::PidfError(e.to_string()))?;
            
            // Note
            if let Some(note) = &presence.note {
                writer.write_event(Event::Start(BytesStart::new("note")))
                    .map_err(|e| RegistrarError::PidfError(e.to_string()))?;
                writer.write_event(Event::Text(BytesText::new(note)))
                    .map_err(|e| RegistrarError::PidfError(e.to_string()))?;
                writer.write_event(Event::End(BytesStart::new("note").to_end()))
                    .map_err(|e| RegistrarError::PidfError(e.to_string()))?;
            }
            
            // Timestamp
            let timestamp = BytesStart::new("timestamp");
            writer.write_event(Event::Start(timestamp))
                .map_err(|e| RegistrarError::PidfError(e.to_string()))?;
            writer.write_event(Event::Text(BytesText::new(
                &presence.last_updated.to_rfc3339()
            )))
                .map_err(|e| RegistrarError::PidfError(e.to_string()))?;
            writer.write_event(Event::End(BytesStart::new("timestamp").to_end()))
                .map_err(|e| RegistrarError::PidfError(e.to_string()))?;
            
            writer.write_event(Event::End(BytesStart::new("tuple").to_end()))
                .map_err(|e| RegistrarError::PidfError(e.to_string()))?;
        }
        
        // Person element for extended status
        if let Some(extended) = &presence.extended_status {
            let mut person = BytesStart::new("dm:person");
            person.push_attribute(("id", "person-1"));
            
            writer.write_event(Event::Start(person))
                .map_err(|e| RegistrarError::PidfError(e.to_string()))?;
            
            // Activities
            writer.write_event(Event::Start(BytesStart::new("dm:activities")))
                .map_err(|e| RegistrarError::PidfError(e.to_string()))?;
            
            let activity = match extended {
                ExtendedStatus::Available => "dm:available",
                ExtendedStatus::Away => "dm:away",
                ExtendedStatus::Busy => "dm:busy",
                ExtendedStatus::DoNotDisturb => "dm:do-not-disturb",
                ExtendedStatus::OnThePhone => "dm:on-the-phone",
                ExtendedStatus::InMeeting => "dm:in-meeting",
                ExtendedStatus::Offline => "dm:offline",
                ExtendedStatus::Custom(s) => s,
            };
            
            writer.write_event(Event::Empty(BytesStart::new(activity)))
                .map_err(|e| RegistrarError::PidfError(e.to_string()))?;
            
            writer.write_event(Event::End(BytesStart::new("dm:activities").to_end()))
                .map_err(|e| RegistrarError::PidfError(e.to_string()))?;
            
            writer.write_event(Event::End(BytesStart::new("dm:person").to_end()))
                .map_err(|e| RegistrarError::PidfError(e.to_string()))?;
        }
        
        writer.write_event(Event::End(BytesStart::new("presence").to_end()))
            .map_err(|e| RegistrarError::PidfError(e.to_string()))?;
        
        let xml = writer.into_inner().into_inner();
        Ok(String::from_utf8(xml)
            .map_err(|e| RegistrarError::PidfError(e.to_string()))?)
    }
    
    /// Parse PIDF XML document into presence state
    pub async fn parse_pidf(&self, xml: &str) -> Result<PresenceState> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);
        
        let mut presence = PresenceState {
            user_id: String::new(),
            basic_status: BasicStatus::Closed,
            extended_status: None,
            note: None,
            activities: Vec::new(),
            devices: Vec::new(),
            last_updated: chrono::Utc::now(),
            expires: None,
            priority: 0,
        };
        
        let mut buf = Vec::new();
        let mut in_basic = false;
        let mut in_note = false;
        let mut in_activities = false;
        
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    match e.name().as_ref() {
                        b"presence" => {
                            // Extract entity attribute
                            for attr in e.attributes() {
                                if let Ok(attr) = attr {
                                    if attr.key.as_ref() == b"entity" {
                                        let entity = String::from_utf8_lossy(&attr.value);
                                        presence.user_id = entity
                                            .strip_prefix("sip:")
                                            .unwrap_or(&entity)
                                            .to_string();
                                    }
                                }
                            }
                        }
                        b"basic" => in_basic = true,
                        b"note" => in_note = true,
                        b"dm:activities" => in_activities = true,
                        _ => {}
                    }
                }
                Ok(Event::Text(ref e)) => {
                    if in_basic {
                        let status_text = e.unescape()
                            .map_err(|err| RegistrarError::PidfError(err.to_string()))?
                            .to_string();
                        presence.basic_status = if status_text == "open" {
                            BasicStatus::Open
                        } else {
                            BasicStatus::Closed
                        };
                        in_basic = false;
                    } else if in_note {
                        presence.note = Some(e.unescape()
                            .map_err(|err| RegistrarError::PidfError(err.to_string()))?
                            .to_string());
                        in_note = false;
                    }
                }
                Ok(Event::Empty(ref e)) if in_activities => {
                    // Parse activity elements
                    let activity = e.name();
                    presence.extended_status = Some(match activity.as_ref() {
                        b"dm:available" => ExtendedStatus::Available,
                        b"dm:away" => ExtendedStatus::Away,
                        b"dm:busy" => ExtendedStatus::Busy,
                        b"dm:do-not-disturb" => ExtendedStatus::DoNotDisturb,
                        b"dm:on-the-phone" => ExtendedStatus::OnThePhone,
                        b"dm:in-meeting" => ExtendedStatus::InMeeting,
                        b"dm:offline" => ExtendedStatus::Offline,
                        _ => ExtendedStatus::Custom(String::from_utf8_lossy(activity.as_ref()).to_string()),
                    });
                }
                Ok(Event::End(ref e)) => {
                    match e.name().as_ref() {
                        b"dm:activities" => in_activities = false,
                        _ => {}
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(RegistrarError::PidfError(e.to_string())),
                _ => {}
            }
            buf.clear();
        }
        
        Ok(presence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_pidf_roundtrip() {
        let generator = PidfGenerator::new();
        
        let mut presence = PresenceState {
            user_id: "alice@example.com".to_string(),
            basic_status: BasicStatus::Open,
            extended_status: Some(ExtendedStatus::Available),
            note: Some("In a meeting".to_string()),
            activities: Vec::new(),
            devices: vec![
                DevicePresence {
                    instance_id: "device1".to_string(),
                    status: BasicStatus::Open,
                    note: None,
                    capabilities: Vec::new(),
                    device_type: Some("pc".to_string()),
                    last_seen: chrono::Utc::now(),
                }
            ],
            last_updated: chrono::Utc::now(),
            expires: None,
            priority: 0,
        };
        
        // Generate PIDF
        let xml = generator.create_pidf(&presence).await.unwrap();
        assert!(xml.contains("urn:ietf:params:xml:ns:pidf"));
        assert!(xml.contains("alice@example.com"));
        
        // Parse it back
        let parsed = generator.parse_pidf(&xml).await.unwrap();
        assert_eq!(parsed.user_id, presence.user_id);
        assert_eq!(parsed.basic_status, presence.basic_status);
        assert_eq!(parsed.note, presence.note);
    }
}