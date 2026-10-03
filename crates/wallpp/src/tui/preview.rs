use anyhow::Result;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use std::path::Path;

/// A loaded preview item containing decoded image, rendering protocol, and metadata.
pub struct PreviewItem {
    pub id: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub protocol: StatefulProtocol,
}

/// Helper to create and initialize the terminal image picker.
pub fn create_picker() -> Picker {
    Picker::from_query_stdio().unwrap_or_else(|_| Picker::from_fontsize((8, 16)))
}

/// Loads an image file from disk and constructs a StatefulProtocol for rendering.
pub fn load_protocol_from_file(picker: &Picker, path: &Path) -> Result<StatefulProtocol> {
    let img = image::open(path)?;
    Ok(picker.new_resize_protocol(img))
}

/// Decodes image bytes from memory and constructs a StatefulProtocol for rendering.
pub fn load_protocol_from_memory(picker: &Picker, data: &[u8]) -> Result<StatefulProtocol> {
    let img = image::load_from_memory(data)?;
    Ok(picker.new_resize_protocol(img))
}

/// Message emitted from background preview loading tasks.
pub enum PreviewTaskResult {
    /// 3 preview items loaded successfully for the given request generation.
    Success {
        generation: u64,
        items: Vec<PreviewItem>,
        normalized_config: Option<crate::wallpp::provider::types::Config>,
    },
    /// An error occurred during fetching or decoding.
    Error { generation: u64, message: String },
}
