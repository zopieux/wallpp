wit_bindgen::generate!({
    path: "../../wit",
    world: "wallpaper-provider",
    pub_export_macro: true,
});
pub use exports::wallpp::provider::provider::Guest;
pub use wallpp::provider::types::*;

pub struct ConfigReader<'a> {
    config: &'a Config,
}

impl<'a> ConfigReader<'a> {
    pub fn new(config: &'a Config) -> Self {
        Self { config }
    }

    pub fn get_text(&self, key: &str) -> Option<&'a str> {
        for entry in self.config {
            if entry.key == key {
                if let ConfigValue::One(ScalarValue::Text(ref s)) = entry.value {
                    return Some(s);
                }
            }
        }
        None
    }

    pub fn get_choice(&self, key: &str) -> Option<&'a str> {
        for entry in self.config {
            if entry.key == key {
                if let ConfigValue::One(ScalarValue::Choice(ref s)) = entry.value {
                    return Some(s);
                }
            }
        }
        None
    }

    pub fn get_integer(&self, key: &str) -> Option<i64> {
        for entry in self.config {
            if entry.key == key {
                if let ConfigValue::One(ScalarValue::Integer(i)) = entry.value {
                    return Some(i);
                }
            }
        }
        None
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        for entry in self.config {
            if entry.key == key {
                if let ConfigValue::One(ScalarValue::Boolean(b)) = entry.value {
                    return Some(b);
                }
            }
        }
        None
    }

    pub fn get_text_list(&self, key: &str) -> Vec<&'a str> {
        for entry in self.config {
            if entry.key == key {
                if let ConfigValue::Many(ref items) = entry.value {
                    return items
                        .iter()
                        .filter_map(|item| match item {
                            ScalarValue::Text(ref s) | ScalarValue::Choice(ref s) => {
                                Some(s.as_str())
                            }
                            _ => None,
                        })
                        .collect();
                }
            }
        }
        Vec::new()
    }
}
