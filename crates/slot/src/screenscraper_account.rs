//! ScreenScraper player account stored on the device card.
//!
//! This follows Slot's small Config/*.conf persistence convention. The file is plain text;
//! callers should not log its contents and can remove it with `forget`.

use std::path::Path;

pub const FILE: &str = "Config/screenscraper.conf";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Account {
    pub username: String,
    pub password: String,
}

impl Account {
    pub fn load(root: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(root.join(FILE)).ok()?;
        let mut username = None;
        let mut password = None;
        for line in text.lines() {
            if let Some(value) = line.strip_prefix("username=") {
                username = Some(value.to_string());
            } else if let Some(value) = line.strip_prefix("password=") {
                password = Some(value.to_string());
            }
        }
        let account = Self { username: username?, password: password? };
        (!account.username.trim().is_empty()).then_some(account)
    }

    pub fn save(&self, root: &Path) -> std::io::Result<()> {
        let path = root.join(FILE);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = format!("username={}\npassword={}\n", self.username, self.password);
        slot_store::atomic_write(&path, contents.as_bytes())
    }

    pub fn forget(root: &Path) {
        let _ = std::fs::remove_file(root.join(FILE));
    }
}
