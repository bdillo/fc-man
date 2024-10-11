use std::{error::Error, path::Path};

use clap::Parser;
use fc_man::{args::CliArgs, image_builder::ImageBuilder};
use log::{info, LevelFilter};
use simplelog::{Config, SimpleLogger};

fn main() -> Result<(), Box<dyn Error>> {
    SimpleLogger::init(LevelFilter::Debug, Config::default()).expect("Failed to initialize logger");
    info!("Starting...");
    let args = CliArgs::parse();
    let image_builder = ImageBuilder::default();
    Ok(image_builder.build_image_from_base(Path::new(&args.base_fs))?)
}
