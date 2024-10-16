use std::{error::Error, path::Path};

use clap::Parser;
use fc_man::{
    args::CliArgs, image_builder::ImageBuilder, messages::VmCommands, vm_manager::VmManager,
};
use log::{info, LevelFilter};
use simplelog::{Config, SimpleLogger};
use tokio::sync::mpsc;

const VM_MANAGER_MESSAGE_CAPACITY: usize = 10;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    SimpleLogger::init(LevelFilter::Debug, Config::default()).expect("Failed to initialize logger");
    info!("Starting...");
    let args = CliArgs::parse();
    let (vm_tx, vm_rx) = mpsc::channel(VM_MANAGER_MESSAGE_CAPACITY);

    let image_builder = ImageBuilder::default();
    // let image = image_builder.build_image_from_base(Path::new(&args.base_fs))?;

    vm_tx.send(VmCommands::LaunchVm).await?;
    let mut vm_manager = VmManager::new(vm_rx);
    vm_manager.run().await;

    Ok(())
}
