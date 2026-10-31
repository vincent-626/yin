use std::borrow::Cow;

use gpui_kit::{AssetSource, Result, SharedString};

// Icons beyond gpui-kit's default component set. Reference them with
// `Icon::empty().path("icons/<name>.svg")`.
gpui_kit::assets::icon_assets!(
    ExtraIcons,
    [
        GitBranch,
        FileDiff,
        GitGraph,
        Folder,
        GitPullRequest,
        Github,
        SquarePen,
        ExternalLink,
        Trash,
        MessageSquareText,
    ]
);

/// yin's icons first, then gpui-kit's defaults.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match ExtraIcons.load(path)? {
            Some(bytes) => Ok(Some(bytes)),
            None => gpui_kit::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = ExtraIcons.list(path)?;
        paths.extend(gpui_kit::assets::Assets.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}
