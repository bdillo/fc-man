use std::path::Path;

use crate::image_builder::Image;

/// Messages for the image builder
#[derive(Debug)]
pub enum ImageBuilderCommands<'a> {
    BuildImage { base_fs: &'a Path },
}
/// Messages for the vm manager
#[derive(Debug)]
pub enum VmCommands {
    LaunchVm { image: Image },
}
