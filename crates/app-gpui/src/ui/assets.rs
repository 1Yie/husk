use std::borrow::Cow;

use gpui_kit::{AssetSource, SharedString};

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        match path {
            "images/husk.png" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../../assets/images/husk.png"
            )))),
            _ => gpui_kit::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        if "images/husk.png".starts_with(path) {
            paths.push("images/husk.png".into());
        }
        Ok(paths)
    }
}
