use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug)]
struct VmConfig {
    logger: VmLoggerConfig,
    boot_source: VmBootSourceConfig,
    network: VmNetworkConfig,
    drives: VmDrivesConfig,
    machine: VmMachineConfig,
}

#[derive(Debug, Serialize, Deserialize)]
struct VmLoggerConfig {
    // TODO: will serde work with paths like this?
    log_path: PathBuf,
    // TODO: make this an enum, maybe use one from logging crate?
    level: String,
    show_level: bool,
    show_log_origin: bool,
}

impl Default for VmLoggerConfig {
    fn default() -> Self {
        Self {
            // TODO: generate random log names in default impl
            log_path: PathBuf::from("/tmp/log"),
            level: "Debug".to_owned(),
            show_level: true,
            show_log_origin: true,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct VmBootSourceConfig {
    kernel_image_path: PathBuf,
    initrd_path: PathBuf,
    boot_args: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct VmNetworkConfig {
    // TODO: use better types here
    iface_id: String,
    guest_mac: String,
    host_dev_name: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct VmDrivesConfig {
    drive_id: String,
    path_on_host: PathBuf,
    is_root_device: bool,
    is_read_only: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct VmMachineConfig {
    vcpu_count: u8,
    mem_size_mib: u32,
}
