// make fs
// format fs
// mount fs
// copy from image
// chroot
// customize rootfs, openssh, openrc, etc
// grab initramfs, vmlinuz
// extract vmlinux from vmlinuz
// unmount fs
// done!

use log::debug;
use nix::{errno::Errno, libc::off_t, unistd::truncate};
use std::{
    fs::File,
    io,
    os::fd::{AsRawFd, RawFd},
    path::{Path, PathBuf},
    process::Command,
};
use thiserror::Error;

// TODO: move this out into some sort of context struct?
const VAR_DIR: &str = "/var/lib/fc-man";
const ROOTFS_DIR: &str = "rootfs";
const MOUT_DIR: &str = "mount";

const MKFS_EXT4: &str = "mkfs.ext4";

// TODO: make these not bad
#[derive(Error, Debug)]
enum ImageBuilderError {
    #[error("IO Error")]
    IoError(#[from] io::Error),
    #[error("Syscall error")]
    SyscallError(#[from] Errno),
}

/// VM image with paths to all related components needed to launch a vm
struct Image<'a> {
    rootfs_path: &'a Path,
    initrd_path: &'a Path,
    kernel_path: &'a Path,
}

/// An image's rootfs
struct ImageRootFs {
    path: PathBuf,
    // raw_fd: RawFd,
}

impl ImageRootFs {
    /// Create a new root fs with given name - this creates the file
    fn new(image_name: &str) -> Result<Self, ImageBuilderError> {
        let path = PathBuf::from(format!("{VAR_DIR}/{ROOTFS_DIR}/{image_name}"));
        debug!("Creating new image root fs at {:?}", &path);
        let file = File::create_new(&path)?;
        // let raw_fd = file.as_raw_fd();

        // Ok(Self { path, raw_fd })
        Ok(Self { path })
    }

    /// Allocate disk space for our image
    fn fallocate(&self, size: off_t) -> Result<(), ImageBuilderError> {
        debug!("Allocating {} bytes to file at {:?}", size, &self.path);
        Ok(truncate(&self.path, size)?)
    }

    /// Format our file to ext4
    fn format(&self) -> Result<(), ImageBuilderError> {
        // TODO: see if there's a better option than just shelling out to reduce implicit dependencies
        debug!("Executing command: {} {:?}", MKFS_EXT4, &self.path);
        let output = Command::new(MKFS_EXT4).arg(&self.path).output()?;

        if !output.stderr.is_empty() {
            debug!("{:?}", output.stderr);
        }

        Ok(())
    }
}

/// High level image builder
struct ImageBuilder<'a> {
    working_dir: &'a Path,
}

impl<'a> ImageBuilder<'a> {
    fn new() -> Self {
        Self {
            working_dir: Path::new(VAR_DIR),
        }
    }

    fn mount(&self, image_root_fs: &ImageRootFs) -> Result<(), ImageBuilderError> {
        todo!()
    }

    fn copy_base_filesystem(&self, base_fs_path: &Path) -> Result<(), ImageBuilderError> {
        todo!()
    }

    fn chroot(&self, image_root_fs: &ImageRootFs) -> Result<(), ImageBuilderError> {
        todo!()
    }
}
