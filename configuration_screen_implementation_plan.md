# Configuration Screen Implementation Plan for rvoip_sip_client

## Overview
Move network interface and port configuration from the registration screen to a dedicated configuration popup, and add audio device selection capabilities.

## Current State Analysis

### ✅ Existing Capabilities in sip-client Library

1. **Audio Device Enumeration**
   - `list_audio_devices(direction)` - Lists all available input/output devices
   - `get_audio_device(direction)` - Gets current device for input/output
   - `set_audio_device(direction, device_id)` - Sets audio device (TODO: needs implementation)
   - Returns `AudioDeviceInfo` with `id`, `name`, and `direction`

2. **Configuration Support**
   - `SipClientConfig` has `AudioConfig` with:
     - `input_device: Option<String>` - Input device name
     - `output_device: Option<String>` - Output device name
     - Audio processing settings (echo cancellation, noise suppression, AGC)
   - Builder pattern supports setting audio devices via:
     - `audio().input_device("device_name")`
     - `audio().output_device("device_name")`

3. **Network Configuration**
   - `local_address: SocketAddr` - For binding to specific interface/port
   - Currently configurable via `SipClientBuilder`

## Implementation Plan

### Phase 1: UI Components

#### 1.1 Add Gear Icon to Registration Screen
```rust
// In registration_screen.rs
// Add to the header or top-right corner
if ui.button("⚙").on_hover_text("Settings").clicked() {
    self.show_config_popup = true;
}
```

#### 1.2 Create Configuration Popup Structure
```rust
pub struct ConfigPopup {
    // Network settings
    pub selected_interface: String,
    pub port: String,
    pub available_interfaces: Vec<NetworkInterface>,
    
    // Audio settings
    pub selected_input_device: Option<String>,
    pub selected_output_device: Option<String>,
    pub available_input_devices: Vec<AudioDeviceInfo>,
    pub available_output_devices: Vec<AudioDeviceInfo>,
    
    // Audio processing
    pub echo_cancellation: bool,
    pub noise_suppression: bool,
    pub auto_gain_control: bool,
    
    // State
    pub is_open: bool,
    pub devices_loaded: bool,
}
```

### Phase 2: Backend Integration

#### 2.1 Audio Device Enumeration on Popup Open
```rust
impl ConfigPopup {
    pub async fn load_devices(&mut self, client: &SipClient) -> Result<()> {
        // Load audio devices
        self.available_input_devices = client
            .list_audio_devices(AudioDirection::Input)
            .await?;
        
        self.available_output_devices = client
            .list_audio_devices(AudioDirection::Output)
            .await?;
        
        // Get current devices
        let current_input = client
            .get_audio_device(AudioDirection::Input)
            .await?;
        self.selected_input_device = Some(current_input.id);
        
        let current_output = client
            .get_audio_device(AudioDirection::Output)
            .await?;
        self.selected_output_device = Some(current_output.id);
        
        self.devices_loaded = true;
        Ok(())
    }
}
```

#### 2.2 Complete set_audio_device Implementation
```rust
// In sip-client/src/simple.rs
pub async fn set_audio_device(&self, direction: AudioDirection, device_id: &str) -> Result<()> {
    // Verify device exists
    let devices = self.list_audio_devices(direction).await?;
    let device = devices.iter()
        .find(|d| d.id == device_id)
        .ok_or("Device not found")?;
    
    // Update audio manager
    match direction {
        AudioDirection::Input => {
            self.inner.audio_manager.set_input_device(device_id).await?;
        }
        AudioDirection::Output => {
            self.inner.audio_manager.set_output_device(device_id).await?;
        }
    }
    
    // Emit event
    self.inner.events.emit(SipClientEvent::AudioDeviceChanged {
        direction,
        old_device: self.get_audio_device(direction).await.ok().map(|d| d.name),
        new_device: Some(device.name.clone()),
    });
    
    Ok(())
}
```

### Phase 3: Configuration Persistence

#### 3.1 Settings Storage
```rust
#[derive(Serialize, Deserialize)]
pub struct AppSettings {
    // Network
    pub interface: Option<String>,
    pub port: u16,
    
    // Audio
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub echo_cancellation: bool,
    pub noise_suppression: bool,
    pub auto_gain_control: bool,
}

impl AppSettings {
    pub fn load() -> Result<Self> {
        let path = dirs::config_dir()
            .unwrap()
            .join("rvoip")
            .join("settings.json");
        
        if path.exists() {
            let contents = std::fs::read_to_string(path)?;
            Ok(serde_json::from_str(&contents)?)
        } else {
            Ok(Self::default())
        }
    }
    
    pub fn save(&self) -> Result<()> {
        let dir = dirs::config_dir()
            .unwrap()
            .join("rvoip");
        std::fs::create_dir_all(&dir)?;
        
        let path = dir.join("settings.json");
        let contents = serde_json::to_string_pretty(self)?;
        std::fs::write(path, contents)?;
        Ok(())
    }
}
```

### Phase 4: UI Implementation

#### 4.1 Configuration Popup UI
```rust
impl ConfigPopup {
    pub fn show(&mut self, ctx: &egui::Context, client: &SipClient) {
        if !self.is_open { return; }
        
        egui::Window::new("Settings")
            .collapsible(false)
            .resizable(true)
            .show(ctx, |ui| {
                ui.heading("Network Configuration");
                
                ui.horizontal(|ui| {
                    ui.label("Interface:");
                    egui::ComboBox::from_label("")
                        .selected_text(&self.selected_interface)
                        .show_ui(ui, |ui| {
                            for interface in &self.available_interfaces {
                                ui.selectable_value(
                                    &mut self.selected_interface,
                                    interface.name.clone(),
                                    &interface.name
                                );
                            }
                        });
                });
                
                ui.horizontal(|ui| {
                    ui.label("Port:");
                    ui.text_edit_singleline(&mut self.port);
                });
                
                ui.separator();
                ui.heading("Audio Configuration");
                
                // Input device dropdown
                ui.horizontal(|ui| {
                    ui.label("Microphone:");
                    egui::ComboBox::from_label("mic")
                        .selected_text(
                            self.selected_input_device.as_ref()
                                .unwrap_or(&"Default".to_string())
                        )
                        .show_ui(ui, |ui| {
                            for device in &self.available_input_devices {
                                ui.selectable_value(
                                    &mut self.selected_input_device,
                                    Some(device.id.clone()),
                                    &device.name
                                );
                            }
                        });
                });
                
                // Output device dropdown
                ui.horizontal(|ui| {
                    ui.label("Speaker:");
                    egui::ComboBox::from_label("speaker")
                        .selected_text(
                            self.selected_output_device.as_ref()
                                .unwrap_or(&"Default".to_string())
                        )
                        .show_ui(ui, |ui| {
                            for device in &self.available_output_devices {
                                ui.selectable_value(
                                    &mut self.selected_output_device,
                                    Some(device.id.clone()),
                                    &device.name
                                );
                            }
                        });
                });
                
                ui.separator();
                ui.heading("Audio Processing");
                
                ui.checkbox(&mut self.echo_cancellation, "Echo Cancellation");
                ui.checkbox(&mut self.noise_suppression, "Noise Suppression");
                ui.checkbox(&mut self.auto_gain_control, "Automatic Gain Control");
                
                ui.separator();
                
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        self.save_settings();
                        self.apply_settings(client);
                        self.is_open = false;
                    }
                    
                    if ui.button("Cancel").clicked() {
                        self.is_open = false;
                    }
                });
            });
    }
}
```

### Phase 5: Apply Settings to Client

#### 5.1 When Creating New Client
```rust
// In registration_screen.rs or main app
let settings = AppSettings::load()?;

let client = SipClientBuilder::new()
    .sip_identity(&self.username)
    .local_address(format!("{}:{}", 
        settings.interface.unwrap_or("0.0.0.0".to_string()),
        settings.port
    ).parse()?)
    .audio(|a| a
        .input_device(settings.input_device.unwrap_or_default())
        .output_device(settings.output_device.unwrap_or_default())
        .echo_cancellation(settings.echo_cancellation)
        .noise_suppression(settings.noise_suppression)
        .auto_gain_control(settings.auto_gain_control)
    )
    .build()
    .await?;
```

#### 5.2 Apply Settings to Existing Client
```rust
impl ConfigPopup {
    pub async fn apply_settings(&self, client: &SipClient) -> Result<()> {
        // Apply audio device changes
        if let Some(ref device_id) = self.selected_input_device {
            client.set_audio_device(AudioDirection::Input, device_id).await?;
        }
        
        if let Some(ref device_id) = self.selected_output_device {
            client.set_audio_device(AudioDirection::Output, device_id).await?;
        }
        
        // Note: Network settings require client restart
        // Audio processing settings also require restart for now
        
        Ok(())
    }
}
```

## Implementation Steps

1. **Step 1: Add gear icon to registration screen** (30 min)
   - Add button with gear icon
   - Add state variable for popup visibility

2. **Step 2: Create ConfigPopup struct** (1 hour)
   - Define all fields
   - Implement new() and default()
   - Add to app state

3. **Step 3: Implement device enumeration** (2 hours)
   - Load devices when popup opens
   - Handle async loading with UI feedback
   - Cache device lists

4. **Step 4: Build configuration UI** (2 hours)
   - Network interface dropdown
   - Port input field
   - Audio device dropdowns
   - Audio processing checkboxes
   - Save/Cancel buttons

5. **Step 5: Implement settings persistence** (1 hour)
   - Create AppSettings struct
   - Implement load/save with serde_json
   - Handle missing config gracefully

6. **Step 6: Complete set_audio_device in sip-client** (2 hours)
   - Implement actual device switching in audio_manager
   - Test with real audio devices
   - Handle errors gracefully

7. **Step 7: Integration testing** (1 hour)
   - Test device enumeration
   - Test device switching
   - Test settings persistence
   - Test UI responsiveness

## Dependencies

- `egui` - UI framework (already in use)
- `serde` + `serde_json` - Settings persistence
- `dirs` - Finding config directory
- `tokio` - Async runtime (already in use)

## Testing Plan

1. **Unit Tests**
   - Settings load/save
   - Device enumeration
   - Configuration validation

2. **Integration Tests**
   - Audio device switching during call
   - Settings persistence across app restarts
   - Network interface changes (requires restart)

3. **Manual Testing**
   - UI responsiveness
   - Device list population
   - Settings application
   - Error handling for missing devices

## Future Enhancements

1. **Hot-reload for network settings** - Apply without restart
2. **Audio level meters** - Visual feedback for mic/speaker
3. **Test audio button** - Play test sound through selected speaker
4. **Advanced codec configuration** - Per-codec settings
5. **Profile management** - Save multiple configuration profiles
6. **Keyboard shortcuts** - Quick access to settings

## Notes

- Network interface/port changes currently require client restart
- Audio device changes can be applied immediately for new calls
- Audio processing settings (echo cancellation, etc.) require client recreation
- Consider adding "Apply requires restart" warning for certain settings