use std::{fs, io, path::Path};

use log::debug;
use thiserror::Error;
use tokio::{process::Command, sync::mpsc::Receiver};
use uuid::Uuid;

use crate::{image_builder::Image, messages::VmCommands, utils::FIRECRACKER_BIN};

const FIRECRACKET_SOCKET_DIR: &str = "/run/firecracker";

// TODO: make this not bad
#[derive(Error, Debug)]
pub enum VmError {
    #[error("IO Error")]
    Io(#[from] io::Error),
}

#[derive(Debug)]
struct Vm {
    id: Uuid,
    image: Image,
    config: (),
    socket: (),
}

#[derive(Debug)]
struct VmConfig {
    logger: VmLoggerConfig,
}

#[derive(Debug)]
struct VmLoggerConfig {}

#[derive(Debug)]
struct VmBootSourceConfig {}

#[derive(Debug)]
struct VmNetworkConfig {}

/// Manager for vms
pub struct VmManager {
    rx: Receiver<VmCommands>,
}

impl VmManager {
    pub fn new(rx: Receiver<VmCommands>) -> Self {
        Self { rx }
    }

    fn setup_socket_dir(&self) -> Result<(), VmError> {
        let sockets_dir = Path::new(FIRECRACKET_SOCKET_DIR);

        if !Path::exists(sockets_dir) {
            debug!("Creating new dir {:?}", sockets_dir);
            fs::create_dir_all(sockets_dir)?;
        }

        Ok(())
    }

    pub async fn run(&mut self) -> Result<(), VmError> {
        self.setup_socket_dir()?;

        while let Some(m) = self.rx.recv().await {
            debug!("Received message: {:?}", m);
            match m {
                VmCommands::LaunchVm { image } => {
                    let vm_id = Uuid::new_v4();
                    tokio::spawn(async move {});
                    self.launch_vm(image).await;
                }
            }
        }
        todo!()
    }

    async fn launch_vm(&self, image: Image) -> Result<(), VmError> {
        //
        let mut child = Command::new(FIRECRACKER_BIN);
        todo!()
    }
}
